use std::path::{Path, PathBuf};
use crate::infra::McpClient;
use crate::infra::bin_downloader;

/// Встроенные инструменты (единый источник — SSOT). Подмешиваются в промпт
/// каждого агента при РЕАЛЬНОМ вызове и в worst-case оценку контекста.
/// Схемы туду-инструментов (`todo_write` / `todo_list`). Это ОБЫЧНЫЕ инструменты,
/// НЕ built-in: подключаются только опционально (папка агента `coder`/`research`
/// либо явный `tools: ["todo"]` в .md), чтобы не жрать токены и не путать агентов,
/// которым чек-лист не нужен. Состояние хранится в сессии (thought с автором
/// `todo::<agent_id>`) и переживает компакцию контекста (Слой 2.2).
pub fn todo_tool_schemas() -> Vec<(String, String, serde_json::Value)> {
    vec![
        (
            "_todo".to_string(),
            "todo_write".to_string(),
            serde_json::json!({
                "name": "todo_write",
                "description": "Управление чек-листом задач агента. Используй для многошаговых задач, чтобы не терять план (он переживает сжатие контекста). Действия: add (нужен title), done/remove (по index или title), clear (очистить), list (показать).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["add", "done", "remove", "clear", "list"], "description": "add — добавить задачу; done/remove — отметить/удалить по index(1-based) или title; clear — очистить; list — показать список."},
                        "title": {"type": "string", "description": "Текст задачи (для add) или название для поиска (done/remove)."},
                        "index": {"type": "integer", "description": "Номер задачи (1-based) для done/remove."}
                    },
                    "required": ["action"]
                }
            }),
        ),
        (
            "_todo".to_string(),
            "todo_list".to_string(),
            serde_json::json!({
                "name": "todo_list",
                "description": "Показать текущий чек-лист задач агента (что сделано, что осталось).",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }),
        ),
    ]
}

pub fn builtin_tools() -> Vec<(String, String, serde_json::Value)> {
    vec![(
        "_builtin".to_string(),
        "emit_signal".to_string(),
        serde_json::json!({
            "name": "emit_signal",
            "description": "Сохранить сигнал/маркер в сессию. Другие агенты, экстрактор и phase_router увидят его. Принимает key (имя сигнала) и value (произвольный JSON-объект с данными).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": {"type": "string", "description": "Имя сигнала, например 'validator_report' или 'phase'"},
                    "value": {"type": "object", "description": "Произвольный JSON с данными сигнала"}
                },
                "required": ["key", "value"]
            }
        }),
    ), (
        "_builtin".to_string(),
        "read_spill".to_string(),
        serde_json::json!({
            "name": "read_spill",
            "description": "Дочитать полный результат большого инструмента, сохранённый в файл spills (локатор приходит в сообщении '[РЕЗУЛЬТАТ ИНСТРУМЕНТА сохранён в файл spills]'). Принимает path (путь к spill-файлу). Возвращает полное содержимое (обрезанное до 16К символов).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Путь к spill-файлу (например spills/spill_agent_0.txt)"}
                },
                "required": ["path"]
            }
        }),
    )]
}

/// Capability-шов (Слой 4.3): единая точка, где декларируется, что агент
/// *умеет* — какие встроенные инструменты, MCP-серверы и сабагенты ему доступны.
/// Ядро оркестратора спрашивает шов вместо хардкода; добавить/заменить
/// capability можно, не трогая цикл выполнения. Возвращает человекочитаемую
/// сводку (используется в промптах/логах и для согласования между агентами).
pub fn agent_capabilities(agent: &crate::domain::agent_manager::AgentProfile) -> Vec<String> {
    let mut caps = Vec::new();
    for t in &agent.tools {
        caps.push(format!("tool:{}", t));
    }
    for m in &agent.mcp_servers {
        caps.push(format!("mcp:{}", m));
    }
    for s in &agent.subagents {
        caps.push(format!("subagent:{}", s));
    }
    // Встроенные инструменты доступны всем агентам.
    for (_, name, _) in builtin_tools() {
        caps.push(format!("builtin:{}", name));
    }
    caps
}

/// Человекочитаемая сводка capability-шва для логов/промпта.
pub fn agent_capabilities_summary(agent: &crate::domain::agent_manager::AgentProfile) -> String {
    let caps = agent_capabilities(agent);
    if caps.is_empty() {
        return format!("Агент '{}' не имеет объявленных capability.", agent.id);
    }
    format!("Capability-шов агента '{}': {}", agent.id, caps.join(", "))
}

pub fn get_mcp_server_path(mcp_servers_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let possible_paths = vec![
        mcp_servers_dir.join(format!("{}.ts", name)),
        mcp_servers_dir.join(format!("{}.js", name)),
        PathBuf::from("src-tauri").join("mcp_servers").join(format!("{}.ts", name)),
        PathBuf::from("src-tauri").join("mcp_servers").join(format!("{}.js", name)),
    ];
    for path in possible_paths { if path.exists() { return Ok(path); } }
    Err(format!("MCP-сервер {} не найден", name))
}

fn find_or_download_runtime<L: Fn(String) + Clone + Send + Sync>(
    runtime_name: &str, bins_dir: &Path, log_cb: L,
) -> PathBuf {
    let target = env!("TARGET");
    let dev_name = format!("{}-{}.exe", runtime_name, target);
    let exe_name = format!("{}.exe", runtime_name);

    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        for p in vec![
            exe.join(&exe_name),
            exe.join(&dev_name),
            exe.join("bin").join(&dev_name),
            PathBuf::from("bin").join(&dev_name),
        ] {
            if p.exists() { return p; }
        }
    }

    match bin_downloader::ensure_runtime_bin(runtime_name, bins_dir, log_cb.clone()) {
        Ok(path) if path.exists() => return path,
        Ok(_) => {}
        Err(e) => log_cb(format!("❌ Не удалось скачать {}: {}", runtime_name, e)),
    }

    PathBuf::from(runtime_name)
}

/// Гранулярные права Deno для каждого MCP-сервера (принцип минимальных прав).
/// Держать в синхроне с фактическими операциями серверов в src-tauri/mcp_servers/.
fn deno_permissions(mcp_name: &str, bins_dir: &Path) -> Vec<String> {
    let bins = bins_dir.to_string_lossy().to_string();
    match mcp_name {
        // time — без прав (только вычисления)
        "time" => vec![],
        // Только чтение файлов
        "fs_read" | "local_rag" | "markdown_section_reader" | "ast_analyzer" => {
            vec!["--allow-read".to_string()]
        }
        // Только запись файлов
        "fs_write" => vec!["--allow-write".to_string()],
        // Сеть + чтение env (node:https читает NODE_EXTRA_CA_CERTS и proxy-переменные через node-compat)
        // web_search: кеш результатов (search_cache.json) и статистика отказов (search_stats.json) в bins_dir
        "web_search" | "docs_fetcher" | "knowledge_api" => vec![
            "--allow-net".to_string(),
            "--allow-env".to_string(),
            format!("--allow-read={}", bins),
            format!("--allow-write={}", bins),
        ],
        // Сеть + кеш пула инстансов (searxng_cache.json) в bins_dir + чтение KING_ORCH_BINS_DIR
        "searxng_search" => vec![
            "--allow-net".to_string(),
            format!("--allow-read={}", bins),
            format!("--allow-write={}", bins),
            "--allow-env".to_string(),
        ],
        // Вертикальные поиски (keyless REST API): только сеть + env (node-compat proxy-переменные)
        "github_search" | "academic_search" | "youtube_search" => vec![
            "--allow-net".to_string(),
            "--allow-env".to_string(),
        ],
        // Сеть + запуск yt-dlp + temp-файлы + env (bins_dir)
        "youtube_mcp" => vec![
            "--allow-net".to_string(),
            "--allow-run".to_string(),
            "--allow-read".to_string(),
            "--allow-write".to_string(),
            "--allow-env".to_string(),
        ],
        // Браузер: запуск Chrome + CDP (localhost) + npm-пакеты + профили/PDF в bins_dir
        "browser" => vec![
            "--allow-net".to_string(),
            "--allow-run".to_string(),
            "--allow-read".to_string(),
            "--allow-write".to_string(),
            "--allow-env".to_string(),
        ],
        // AST-карта: чтение проекта, запись .agents_workspace, сеть (npm-пакеты, токенизатор)
        "ast_treesitter" => vec![
            "--allow-read".to_string(),
            "--allow-write".to_string(),
            "--allow-net".to_string(),
        ],
        // deno_runner и любые неизвестные серверы — только запуск подпроцессов
        _ => vec!["--allow-run".to_string()],
    }
}

pub fn resolve_runtime_and_args<L: Fn(String) + Clone + Send + Sync>(
    log_cb: L, script_path: &Path, bins_dir: &Path,
) -> (PathBuf, Vec<String>) {
    let deno_path = find_or_download_runtime("deno", bins_dir, log_cb.clone());
    log_cb(format!("   🦎 Runtime: Deno | {}", script_path.display()));

    let mcp_name = script_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let mut args = vec!["run".to_string(), "--no-check".to_string(), "--no-config".to_string()];
    args.extend(deno_permissions(mcp_name, bins_dir));
    args.push(script_path.to_string_lossy().to_string());
    (deno_path, args)
}

fn ensure_mcp_deps<L: Fn(String) + Clone + Send + Sync>(
    mcp_name: &str, bins_dir: &Path, log_cb: &L,
) -> Vec<(&'static str, String)> {
    if mcp_name == "browser" {
        let mut envs: Vec<(&'static str, String)> = Vec::new();
        if let Some(bins_str) = bins_dir.to_str() {
            envs.push(("KING_ORCH_BINS_DIR", bins_str.to_string()));
        }
        // Сначала CloakBrowser (stealth-Chromium) для обхода антибот-защиты;
        // при любой ошибке — фолбэк на Chrome-for-Testing.
        match bin_downloader::ensure_cloak_browser(bins_dir, log_cb) {
            Ok(exe) => {
                envs.push(("KING_ORCH_CHROME_PATH", exe.to_string_lossy().to_string()));
                log_cb(format!("🕵️ Браузер: CloakBrowser (stealth) | {}", exe.display()));
            }
            Err(e) => {
                log_cb(format!("⚠️ CloakBrowser недоступен ({}), пробуем Chrome-for-Testing", e));
                match bin_downloader::ensure_chrome_bin(bins_dir, log_cb) {
                    Ok(exe) => {
                        envs.push(("KING_ORCH_CHROME_PATH", exe.to_string_lossy().to_string()));
                        log_cb(format!("🌐 Браузер: Chrome-for-Testing | {}", exe.display()));
                    }
                    Err(e2) => log_cb(format!("❌ Не удалось установить браузер: {}", e2)),
                }
            }
        }
        return envs;
    }
    if mcp_name == "youtube_mcp" {
        if let Ok(_yt_path) = bin_downloader::ensure_runtime_bin("yt-dlp", bins_dir, log_cb.clone()) {
            if let Some(bins_str) = bins_dir.to_str() {
                return vec![("KING_ORCH_BINS_DIR", bins_str.to_string())];
            }
        }
    }
    // web_search (search_cache/search_stats) и searxng_search (searxng_cache) хранят свои файлы в bins_dir.
    if mcp_name == "searxng_search" || mcp_name == "web_search" {
        if let Some(bins_str) = bins_dir.to_str() {
            return vec![("KING_ORCH_BINS_DIR", bins_str.to_string())];
        }
    }
    vec![]
}

pub fn load_mcp_servers<L: Fn(String) + Clone + Send + Sync + 'static>(
    log_cb: &L,
    mcp_servers_dir: &Path,
    bins_dir: &Path,
    mcp_names: &[String],
    mcp_clients: &mut std::collections::HashMap<String, McpClient>,
    all_tools: &mut Vec<(String, String, serde_json::Value)>,
) {
    for mcp_name in mcp_names {
        log_cb(format!("⏳ Инициализация MCP: {}", mcp_name));
        match get_mcp_server_path(mcp_servers_dir, mcp_name) {
            Ok(script_path) => {
                let (runtime_path, runtime_args) = resolve_runtime_and_args(log_cb.clone(), &script_path, bins_dir);
                let args_refs: Vec<&str> = runtime_args.iter().map(|s| s.as_str()).collect();
                let envs = ensure_mcp_deps(mcp_name, bins_dir, log_cb);
                let env_refs: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (*k, v.as_str())).collect();
                match McpClient::spawn_stub_with_env(&runtime_path.to_string_lossy(), &args_refs, &env_refs, log_cb.clone()) {
                    Ok(mut client) => {
                        match client.list_tools() {
                            Ok(tools) => {
                                let mut loaded = 0;
                                for tool in &tools {
                                    if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                                        all_tools.push((mcp_name.clone(), name.to_string(), tool.clone()));
                                        loaded += 1;
                                    }
                                }
                                mcp_clients.insert(mcp_name.clone(), client);
                                log_cb(format!("✅ MCP '{}' запущен. Инструментов: {}", mcp_name, loaded));
                            }
                            Err(e) => log_cb(format!("❌ Ошибка списка инструментов у '{}': {}", mcp_name, e))
                        }
                    }
                    Err(e) => log_cb(format!("❌ Критическая ошибка запуска MCP '{}': {}", mcp_name, e)),
                }
            }
            Err(e) => log_cb(format!("❌ Ошибка поиска файла сервера: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_manager::AgentProfile;

    fn make_agent(tools: &[&str], mcp: &[&str], subs: &[&str]) -> AgentProfile {
        AgentProfile {
            id: "agent_x".to_string(),
            name: "Agent X".to_string(),
            description: String::new(),
            system_prompt: String::new(),
            is_hidden: false,
            mode: "worker".to_string(),
            mcp_servers: mcp.iter().map(|s| s.to_string()).collect(),
            subagents: subs.iter().map(|s| s.to_string()).collect(),
            folder: None,
            single_report: false,
            tools: tools.iter().map(|s| s.to_string()).collect(),
            current_date: false,
        }
    }

    #[test]
    fn agent_capabilities_lists_declared_and_builtin() {
        let a = make_agent(&["custom_tool"], &["fs_read"], &["helper"]);
        let caps = agent_capabilities(&a);
        assert!(caps.iter().any(|c| c == "tool:custom_tool"));
        assert!(caps.iter().any(|c| c == "mcp:fs_read"));
        assert!(caps.iter().any(|c| c == "subagent:helper"));
        // Встроенные инструменты доступны всем агентам через шов.
        assert!(caps.iter().any(|c| c == "builtin:emit_signal"));
        assert!(caps.iter().any(|c| c == "builtin:read_spill"));
    }

    #[test]
    fn agent_capabilities_summary_is_readable() {
        let a = make_agent(&["custom_tool"], &["fs_read"], &[]);
        let s = agent_capabilities_summary(&a);
        assert!(s.contains("agent_x"));
        assert!(s.contains("tool:custom_tool"));
    }
}
