use std::path::{Path, PathBuf};

/// Находит путь к MCP-серверу по имени, проверяя несколько возможных расположений
pub fn get_mcp_server_path(mcp_servers_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let possible_paths = vec![
        mcp_servers_dir.join(format!("{}.cjs", name)),
        mcp_servers_dir.join(format!("{}.js", name)),
        mcp_servers_dir.join(format!("{}.ts", name)),
        PathBuf::from("src-tauri").join("mcp_servers").join(format!("{}.cjs", name)),
        PathBuf::from("src-tauri").join("mcp_servers").join(format!("{}.js", name)),
        PathBuf::from("src-tauri").join("mcp_servers").join(format!("{}.ts", name)),
    ];
    for path in possible_paths { if path.exists() { return Ok(path); } }
    Err(format!("MCP-сервер {} не найден", name))
}

/// Определяет рантайм (Node.js или Deno) и аргументы запуска по расширению скрипта
pub fn resolve_runtime_and_args<L: Fn(String) + Clone + Send + Sync>(log_cb: L, script_path: &Path) -> (PathBuf, Vec<String>) {
    let ext = script_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let target = env!("TARGET");
    if ext == "ts" || ext == "mts" {
        let dev_name = format!("deno-{}.exe", target);
        let mut deno_path = PathBuf::from("deno");
        if let Ok(mut exe) = std::env::current_exe() {
            exe.pop();
            for p in vec![exe.join("deno.exe"), exe.join(&dev_name), exe.join("bin").join(&dev_name), PathBuf::from("bin").join(&dev_name)] {
                if p.exists() { deno_path = p; break; }
            }
        }
        log_cb(format!("   🦎 Runtime: Deno | {}", script_path.display()));
        (deno_path, vec!["run".to_string(), "--allow-run".to_string(), "--no-check".to_string(), "--no-config".to_string(), script_path.to_string_lossy().to_string()])
    } else {
        let dev_name = format!("node-{}.exe", target);
        let mut node_path = PathBuf::from("node");
        if let Ok(mut exe) = std::env::current_exe() {
            exe.pop();
            for p in vec![exe.join("node.exe"), exe.join(&dev_name), exe.join("bin").join(&dev_name), PathBuf::from("bin").join(&dev_name)] {
                if p.exists() { node_path = p; break; }
            }
        }
        log_cb(format!("   🟢 Runtime: Node | {}", script_path.display()));
        (node_path, vec![script_path.to_string_lossy().to_string()])
    }
}

/// Загружает все MCP-серверы, указанные в конфиге агента, и собирает доступные инструменты
pub fn load_mcp_servers<L: Fn(String) + Clone + Send + Sync + 'static>(
    log_cb: &L,
    mcp_servers_dir: &Path,
    mcp_names: &[String],
    mcp_clients: &mut std::collections::HashMap<String, crate::mcp_client::McpClient>,
    all_tools: &mut Vec<(String, String, serde_json::Value)>,
) {
    for mcp_name in mcp_names {
        log_cb(format!("⏳ Инициализация MCP: {}", mcp_name));
        match get_mcp_server_path(mcp_servers_dir, mcp_name) {
            Ok(script_path) => {
                let (runtime_path, runtime_args) = resolve_runtime_and_args(log_cb.clone(), &script_path);
                let args_refs: Vec<&str> = runtime_args.iter().map(|s| s.as_str()).collect();
                match crate::mcp_client::McpClient::spawn_stub(&runtime_path.to_string_lossy(), &args_refs, log_cb.clone()) {
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