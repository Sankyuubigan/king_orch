//! One-shot кастомные тулы команд агентов — узел `custom_tool_call`.
//!
//! Детерминированный бизнес-код команды (генератор расклада, лукап базы и т.п.)
//! живёт В ПАПКЕ КОМАНДЫ (`agents/<команда>/tools/`), а не в движке: движок
//! остаётся generic, команде не нужен Rust-релиз, чтобы добавить тул. Контракт:
//!
//! - `tool_file` — путь к скрипту относительно папки YAML-графа (прецедент
//!   `config.facts_file`); обязан оставаться внутри `agents/`;
//! - `args` — плоский JSON-объект, значения ТОЛЬКО строки (шаблоны проходят
//!   `resolve_template`); передаются процессу ОДНИМ argv-токеном после скрипта
//!   (Deno: `Deno.args[0]`), без shell;
//! - stdout процесса обязан быть JSON-объектом — это `NodeResult.output`,
//!   доступный шаблонам как `{{ nodes.<id>.output.<key> }}`;
//! - права Deno минимальны: только `--allow-read` своих папок (папка тула,
//!   папка графа и корень команды), НЕТ сети/записи/запуска;
//! - любой отказ (нет файла, выход за `agents/`, не-JSON stdout, ненулевой
//!   exit, таймаут) — честная ошибка узла, workflow останавливается;
//!   пустой/нулевой output «по умолчанию» запрещён.
//!
//! `output_message: true` создаёт сообщение в чат: content = резолвнутый
//! `node.system_message`, а без него — авто-текст-перечень карт из output;
//! вложения собираются из `output.cards[].image_file` относительно
//! `attachments_dir` (отсутствующий файл — ошибка узла, не тихий пропуск).

use super::nodes::NodeResult;
use super::parser::{NodeDef, WorkflowDef};
use super::{WorkflowContext, WorkflowRunner};
use crate::infra::{ChatAttachment, ChatMessage};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Дедлайн one-shot процесса (§2.7 PRP): зависший тул — честная ошибка узла.
const TOOL_TIMEOUT: Duration = Duration::from_secs(30);
/// CREATE_NO_WINDOW: консольное окно Deno не должно мигать у пользователя (§6.8).
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Лимит дампа stdout/stderr в лог (§2.5).
const LOG_DUMP_LIMIT: usize = 500;

pub fn execute_node<L, S, C>(
    node: &NodeDef,
    workflow: &WorkflowDef,
    context: &mut WorkflowContext,
    runner: &mut WorkflowRunner<L, S, C>,
) -> Result<NodeResult, String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&crate::infra::SubCall) + Clone + Send + Sync + 'static,
{
    let tool_file = require_tool_file(node)?;

    let workflow_dir = Path::new(&workflow.parent_dir);
    let script_path = resolve_tool_path(tool_file, workflow_dir, runner.agents_dir)?;
    let script_dir = script_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| workflow_dir.to_path_buf());

    let args_json = build_args_json(&node.args, context)?;

    (runner.log_cb)(format!(
        "[custom_tool:{}] запуск тула {} args={}",
        node.id,
        script_path.display(),
        args_json
    ));

    let deno_path = resolve_deno(runner.bins_dir, &runner.log_cb)?;
    let started = Instant::now();
    let (stdout, stderr, exit_code) =
        run_deno(&deno_path, &script_dir, workflow_dir, &script_path, &args_json)?;

    for line in String::from_utf8_lossy(&stderr).lines() {
        if !line.trim().is_empty() {
            log::warn!("[custom_tool:{}] stderr: {}", node.id, line);
            (runner.log_cb)(format!("[custom_tool:{}] stderr: {}", node.id, line));
        }
    }

    if !exit_code.success() {
        return Err(format!(
            "[custom_tool:{}] тул {} завершился с кодом {}: {}",
            node.id,
            script_path.display(),
            exit_code,
            dump(&String::from_utf8_lossy(&stderr))
        ));
    }

    let stdout_text = String::from_utf8_lossy(&stdout);
    let output: serde_json::Value = serde_json::from_str(stdout_text.trim()).map_err(|e| {
        format!(
            "[custom_tool:{}] stdout тула {} — не JSON ({}): {}",
            node.id,
            script_path.display(),
            e,
            dump(&stdout_text)
        )
    })?;
    if !output.is_object() {
        return Err(format!(
            "[custom_tool:{}] stdout тула {} обязан быть JSON-объектом, получено: {}",
            node.id,
            script_path.display(),
            dump(&stdout_text)
        ));
    }

    (runner.log_cb)(format!(
        "[custom_tool:{}] тул готов за {} мс: {}",
        node.id,
        started.elapsed().as_millis(),
        dump(&stdout_text)
    ));

    if node.output_message {
        let attachments = build_attachments(&output, workflow_dir, node.attachments_dir.as_deref())?;
        let content = match node.system_message.as_deref().map(str::trim) {
            Some(text) if !text.is_empty() => context.resolve_template(text),
            _ => auto_message_content(&output),
        };
        let msg = ChatMessage {
            id: Some(format!("msg_{}", runner.msg_counter)),
            msg_type: "message".to_string(),
            content,
            sub_calls: None,
            author: Some("system".to_string()),
            model: None,
            time_sec: None,
            attachments: if attachments.is_empty() { None } else { Some(attachments) },
            phase: None,
            author_name: Some(workflow.name.clone()),
        };
        context.messages.push(msg);
        *runner.msg_counter += 1;
        context.output_emitted = true;
    }

    Ok(NodeResult {
        output,
        next_node: None,
        next_nodes: vec![],
    })
}

/// `tool_file` обязателен и не может быть пустой строкой.
fn require_tool_file(node: &NodeDef) -> Result<&str, String> {
    node.tool_file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("[custom_tool:{}] поле tool_file обязательно", node.id))
}

/// Резолвит `tool_file` от папки графа и проверяет, что путь не вышел за `agents/`.
fn resolve_tool_path(
    tool_file: &str,
    workflow_dir: &Path,
    agents_dir: &Path,
) -> Result<PathBuf, String> {
    let path = workflow_dir.join(tool_file);
    let canonical = path.canonicalize().map_err(|e| {
        format!(
            "Файл тула недоступен: {} ({})",
            path.display(),
            e
        )
    })?;
    let agents_root = agents_dir.canonicalize().map_err(|e| {
        format!("Корень agents/ недоступен: {} ({})", agents_dir.display(), e)
    })?;
    if !canonical.starts_with(&agents_root) {
        return Err(format!(
            "tool_file '{}' выходит за пределы agents/: {}",
            tool_file,
            canonical.display()
        ));
    }
    Ok(canonical)
}

/// Собирает args в плоский JSON-объект: значения ТОЛЬКО строки, шаблоны
/// резолвятся контекстом (`{{ nodes.X.output.Y }}` и т.п.).
fn build_args_json(
    args: &Option<HashMap<String, serde_yaml::Value>>,
    context: &WorkflowContext,
) -> Result<String, String> {
    let mut json = serde_json::Map::new();
    if let Some(args) = args {
        for (key, value) in args {
            let s = match value {
                serde_yaml::Value::String(s) => context.resolve_template(s),
                other => {
                    return Err(format!(
                        "args.{}: значение обязано быть строкой (шаблоны резолвит движок), получено {:?}",
                        key, other
                    ))
                }
            };
            json.insert(key.clone(), serde_json::Value::String(s));
        }
    }
    serde_json::to_string(&serde_json::Value::Object(json))
        .map_err(|e| format!("Не удалось сериализовать args: {}", e))
}

/// Находит deno.exe в bins_dir; при отсутствии — докачивает (`ensure_runtime_bin`).
fn resolve_deno<L>(bins_dir: &Path, log_cb: &L) -> Result<PathBuf, String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
{
    let exe = bins_dir.join(if cfg!(windows) { "deno.exe" } else { "deno" });
    if exe.exists() {
        return Ok(exe);
    }
    crate::infra::bin_downloader::ensure_runtime_bin("deno", bins_dir, |msg| log_cb(msg))
}

/// Запускает one-shot Deno с минимальными правами чтения и дедлайном.
///
/// stdout/stderr читаются фоновыми потоками (зависший тул с переполненным
/// пайпом не должен упереться в deadlock), главный поток ждёт завершения
/// до дедлайна и убивает процесс при таймауте.
fn run_deno(
    deno_path: &Path,
    script_dir: &Path,
    workflow_dir: &Path,
    script_path: &Path,
    args_json: &str,
) -> Result<(Vec<u8>, Vec<u8>, std::process::ExitStatus), String> {
    let mut allow_read: Vec<String> = Vec::new();
    for dir in [script_dir, workflow_dir, workflow_dir.parent().unwrap_or(workflow_dir)] {
        let s = dir.to_string_lossy().to_string();
        if !allow_read.contains(&s) {
            allow_read.push(s);
        }
    }

    let mut cmd = Command::new(deno_path);
    cmd.arg("run")
        .arg("--no-check")
        .arg("--no-config")
        .arg(format!("--allow-read={}", allow_read.join(",")))
        .arg(script_path)
        .arg(args_json)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Не удалось запустить Deno ({}): {}", deno_path.display(), e))?;

    let mut stdout_pipe = child.stdout.take().expect("stdout piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() > TOOL_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "Тул превысил таймаут {} сек и был остановлен",
                        TOOL_TIMEOUT.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("Ошибка ожидания тула: {}", e)),
        }
    };

    let stdout = stdout_handle
        .join()
        .map_err(|_| "Поток чтения stdout упал".to_string())?;
    let stderr = stderr_handle
        .join()
        .map_err(|_| "Поток чтения stderr упал".to_string())?;
    Ok((stdout, stderr, status))
}

/// Вложения из `output.cards[].image_file` относительно `attachments_dir`.
fn build_attachments(
    output: &serde_json::Value,
    workflow_dir: &Path,
    attachments_dir: Option<&str>,
) -> Result<Vec<ChatAttachment>, String> {
    let mut attachments = Vec::new();
    let Some(cards) = output.get("cards").and_then(|v| v.as_array()) else {
        return Ok(attachments);
    };
    let dir_rel = attachments_dir.map(str::trim).filter(|s| !s.is_empty());
    for card in cards {
        let image_file = card
            .get("image_file")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(image_file) = image_file else {
            continue;
        };
        let Some(dir_rel) = dir_rel else {
            return Err(
                "output_message=true требует attachments_dir, когда output.cards содержит image_file"
                    .to_string(),
            );
        };
        let path = workflow_dir.join(dir_rel).join(image_file);
        if !path.is_file() {
            return Err(format!("Файл карты недоступен: {}", path.display()));
        }
        attachments.push(ChatAttachment {
            file_name: image_file.to_string(),
            mime_type: attachment_mime(&path)?.to_string(),
            data_base64: String::new(),
            file_path: Some(path.to_string_lossy().to_string()),
            is_dir: Some(false),
        });
    }
    Ok(attachments)
}

fn attachment_mime(path: &Path) -> Result<&'static str, String> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => Ok("image/jpeg"),
        Some("png") => Ok("image/png"),
        Some("webp") => Ok("image/webp"),
        other => Err(format!(
            "Недопустимое расширение вложения {:?}: {}",
            other,
            path.display()
        )),
    }
}

/// Авто-текст сообщения без `node.system_message`: перечень карт из output.
/// Движок не знает про Таро — имена берутся из поля `name_ru` (фоллбэк `name`).
fn auto_message_content(output: &serde_json::Value) -> String {
    let names: Vec<&str> = output
        .get("cards")
        .and_then(|v| v.as_array())
        .map(|cards| {
            cards
                .iter()
                .filter_map(|c| {
                    c.get("name_ru")
                        .or_else(|| c.get("name"))
                        .and_then(|v| v.as_str())
                })
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        "Карты разложены".to_string()
    } else {
        format!("Карты: {}", names.join(", "))
    }
}

fn dump(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= LOG_DUMP_LIMIT {
        text.to_string()
    } else {
        let cut: String = text.chars().take(LOG_DUMP_LIMIT).collect();
        format!("{}… [обрезано]", cut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow_engine::parser::NodeType;

    fn workflow_def() -> WorkflowDef {
        serde_yaml::from_str(
            "name: Тест\nentry: user_message\nnodes:\n  - id: user_message\n    type: user_message\n",
        )
        .expect("граф распарсен")
    }

    #[test]
    fn yaml_roundtrip_keeps_custom_tool_fields() {
        let yaml = r#"id: draw_cards
type: custom_tool_call
tool_file: ../tools/draw_card_numbers.ts
args:
  count: "3"
attachments_dir: ../database/cards
output_message: true
"#;
        let node: NodeDef = serde_yaml::from_str(yaml).expect("нода распарсена");
        assert_eq!(node.node_type, NodeType::CustomToolCall);
        assert_eq!(node.tool_file.as_deref(), Some("../tools/draw_card_numbers.ts"));
        assert_eq!(node.args.as_ref().unwrap()["count"], "3");
        assert_eq!(node.attachments_dir.as_deref(), Some("../database/cards"));
        assert!(node.output_message);

        let serialized = serde_yaml::to_string(&node).expect("сериализована");
        let back: NodeDef = serde_yaml::from_str(&serialized).expect("обратно распарсена");
        assert_eq!(back.tool_file, node.tool_file);
        assert_eq!(back.args, node.args);
        assert_eq!(back.attachments_dir, node.attachments_dir);
        assert!(back.output_message);
    }

    #[test]
    fn custom_fields_absent_in_other_node_types() {
        let yaml = "id: w\ntype: llm_worker\nagent: someone\ntask: do\n";
        let node: NodeDef = serde_yaml::from_str(yaml).expect("нода распарсена");
        assert!(node.tool_file.is_none());
        assert!(node.args.is_none());
        assert!(node.attachments_dir.is_none());
        assert!(!node.output_message);
        let serialized = serde_yaml::to_string(&node).expect("сериализована");
        assert!(!serialized.contains("tool_file"), "поля custom_tool не должны попадать в чужие ноды: {}", serialized);
    }

    fn tool_node(tool_file: Option<&str>, args_yaml: Option<&str>) -> NodeDef {
        let mut yaml = String::from("id: tool_node\ntype: custom_tool_call\n");
        if let Some(f) = tool_file {
            yaml.push_str(&format!("tool_file: {}\n", f));
        }
        if let Some(a) = args_yaml {
            yaml.push_str("args:\n");
            yaml.push_str(a);
        }
        serde_yaml::from_str(&yaml).expect("нода распарсена")
    }

    #[test]
    fn tool_file_is_required() {
        let absent = tool_node(None, None);
        let err = require_tool_file(&absent).expect_err("нет tool_file — ошибка");
        assert!(err.contains("tool_file обязательно"), "{}", err);

        let empty: NodeDef =
            serde_yaml::from_str("id: t\ntype: custom_tool_call\ntool_file: \"  \"\n")
                .expect("нода распарсена");
        let err = require_tool_file(&empty).expect_err("пустой tool_file — тоже ошибка");
        assert!(err.contains("tool_file обязательно"), "{}", err);
    }

    #[test]
    fn args_non_string_value_is_rejected() {
        let node: NodeDef =
            serde_yaml::from_str("id: t\ntype: custom_tool_call\ntool_file: x.ts\nargs:\n  count: 3\n")
                .expect("нода");
        let context = WorkflowContext::new("test".to_string(), vec![], vec![]);
        let err = build_args_json(&node.args, &context).expect_err("не-строка обязана упасть");
        assert!(err.contains("count"), "ошибка называет поле: {}", err);
    }

    #[test]
    fn args_templates_resolved_and_serialized() {
        let node: NodeDef = serde_yaml::from_str(
            "id: t\ntype: custom_tool_call\ntool_file: x.ts\nargs:\n  count: \"{{ user_message }}\"\n",
        )
        .expect("нода");
        let mut context = WorkflowContext::new("3".to_string(), vec![], vec![]);
        let json = build_args_json(&node.args, &mut context).expect("args собраны");
        assert_eq!(json, r#"{"count":"3"}"#);
    }

    #[test]
    fn tool_path_resolves_relative_to_graph_dir() {
        let agents_dir = std::env::temp_dir().join("king_orch_custom_tool_test");
        let graph_dir = agents_dir.join("tarot-master/transitions");
        let tools_dir = agents_dir.join("tarot-master/tools");
        std::fs::create_dir_all(&graph_dir).expect("создано");
        std::fs::create_dir_all(&tools_dir).expect("создано");
        let script = tools_dir.join("draw.ts");
        std::fs::write(&script, "// tool").expect("скрипт записан");

        let resolved = resolve_tool_path("../tools/draw.ts", &graph_dir, &agents_dir)
            .expect("путь резолвится");
        assert!(resolved.starts_with(agents_dir.canonicalize().unwrap()));
        assert!(resolved.ends_with("draw.ts"));

        let outside = std::env::temp_dir().join("outside_tool.ts");
        std::fs::write(&outside, "// tool").expect("внешний скрипт записан");
        let err = resolve_tool_path(
            outside.to_str().expect("utf8"),
            &graph_dir,
            &agents_dir,
        )
        .expect_err("выход за agents/ обязан быть отклонён");
        assert!(err.contains("выходит за пределы agents/"), "{}", err);
    }

    #[test]
    fn attachments_built_from_output_cards() {
        let output: serde_json::Value = serde_json::from_str(
            r#"{"count": 2, "cards": [
                {"name_ru": "Шут", "image_file": "RWS_Tarot_00_Fool.jpg"},
                {"name_ru": "Маг", "image_file": "RWS_Tarot_01_Magician.jpg"}
            ]}"#,
        )
        .expect("output распарсен");
        let base = std::env::temp_dir().join("king_orch_attachments_test");
        let cards_dir = base.join("cards");
        std::fs::create_dir_all(&cards_dir).expect("создано");
        std::fs::write(cards_dir.join("RWS_Tarot_00_Fool.jpg"), b"jpg").expect("файл 1");
        std::fs::write(cards_dir.join("RWS_Tarot_01_Magician.jpg"), b"jpg").expect("файл 2");

        let attachments =
            build_attachments(&output, &base, Some("cards")).expect("вложения собраны");
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0].file_name, "RWS_Tarot_00_Fool.jpg");
        assert_eq!(attachments[0].mime_type, "image/jpeg");
        assert!(attachments[0].file_path.as_deref().unwrap_or("").ends_with("RWS_Tarot_00_Fool.jpg"));
        assert_eq!(attachments[0].data_base64, "");
        assert_eq!(attachments[0].is_dir, Some(false));
    }

    #[test]
    fn missing_card_file_is_honest_error() {
        let output: serde_json::Value = serde_json::from_str(
            r#"{"cards": [{"name_ru": "Шут", "image_file": "missing.jpg"}]}"#,
        )
        .expect("output распарсен");
        let base = std::env::temp_dir().join("king_orch_attachments_missing");
        std::fs::create_dir_all(&base).expect("создано");
        let err = build_attachments(&output, &base, Some("cards"))
            .expect_err("отсутствующий файл — ошибка узла");
        assert!(err.contains("недоступен"), "{}", err);
    }

    #[test]
    fn attachments_without_dir_is_error_when_cards_present() {
        let output: serde_json::Value = serde_json::from_str(
            r#"{"cards": [{"name_ru": "Шут", "image_file": "x.jpg"}]}"#,
        )
        .expect("output распарсен");
        let err = build_attachments(&output, Path::new("."), None)
            .expect_err("output_message без attachments_dir и с карточками — ошибка");
        assert!(err.contains("attachments_dir"), "{}", err);
    }

    #[test]
    fn auto_message_lists_card_names() {
        let one: serde_json::Value =
            serde_json::from_str(r#"{"cards": [{"name_ru": "Шут"}]}"#).expect("output");
        assert_eq!(auto_message_content(&one), "Карты: Шут");
        let three: serde_json::Value = serde_json::from_str(
            r#"{"cards": [{"name_ru": "Шут"}, {"name_ru": "Маг"}, {"name": "World"}]}"#,
        )
        .expect("output");
        assert_eq!(auto_message_content(&three), "Карты: Шут, Маг, World");
        let empty: serde_json::Value = serde_json::from_str("{}").expect("output");
        assert_eq!(auto_message_content(&empty), "Карты разложены");
    }

    /// Реальный Deno + фикстурный скрипт: verify stdout-парсинг. Запуск вручную:
    /// `cargo test -- --ignored` (по явной просьбе пользователя). Если deno нет
    /// в bins_dir, ensure_runtime_bin скачает его (одноразово, ~30 МБ).
    #[test]
    #[ignore]
    fn real_deno_executes_fixture_tool() {
        let log = |msg: String| println!("[log] {}", msg);
        let bins_dir = std::env::temp_dir().join("king_orch_bins");
        std::fs::create_dir_all(&bins_dir).expect("создано");
        let deno = resolve_deno(&bins_dir, &log).expect("deno доступен");

        let root = std::env::temp_dir().join("king_orch_custom_tool_deno_test");
        std::fs::create_dir_all(&root).expect("создано");
        std::fs::write(
            root.join("fixture_tool.ts"),
            r#"const args = JSON.parse(Deno.args[0]);
console.log(JSON.stringify({ received: args.n, doubled: Number(args.n) * 2 }));
"#,
        )
        .expect("скрипт записан");

        let (stdout, stderr, status) =
            run_deno(&deno, &root, &root, &root.join("fixture_tool.ts"), r#"{"n":"2"}"#)
                .expect("тул выполнен");
        assert!(stderr.is_empty(), "stderr: {}", String::from_utf8_lossy(&stderr));
        assert!(status.success(), "exit: {}", status);
        let output: serde_json::Value = serde_json::from_str(
            String::from_utf8_lossy(&stdout).trim(),
        )
        .expect("stdout — JSON");
        assert_eq!(output["received"], "2");
        assert_eq!(output["doubled"], 4);
    }

    /// Ненулевой exit тула → stderr с ошибкой, stdout пуст (verify честного отказа).
    #[test]
    #[ignore]
    fn real_deno_tool_failure_is_reported() {
        let log = |msg: String| println!("[log] {}", msg);
        let bins_dir = std::env::temp_dir().join("king_orch_bins");
        std::fs::create_dir_all(&bins_dir).expect("создано");
        let deno = resolve_deno(&bins_dir, &log).expect("deno доступен");

        let root = std::env::temp_dir().join("king_orch_custom_tool_deno_test");
        std::fs::create_dir_all(&root).expect("создано");
        std::fs::write(
            root.join("failing_tool.ts"),
            "console.error('карта не найдена');\nDeno.exit(1);\n",
        )
        .expect("скрипт записан");

        let (stdout, stderr, status) =
            run_deno(&deno, &root, &root, &root.join("failing_tool.ts"), "{}")
                .expect("процесс запущен");
        assert!(!status.success());
        assert!(stdout.is_empty());
        assert!(String::from_utf8_lossy(&stderr).contains("карта не найдена"));
    }

    // ── Гейты контента команды «Таролог» (PRP §4) ──

    /// Корень проекта (src-tauri/..) — для чтения боевых файлов из agents/.
    fn project_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .to_path_buf()
    }

    /// gate_cards_json: manifest карт — валидный JSON, ровно 78 записей,
    /// number уникальны и покрывают 0..77, каждый image_file существует на диске,
    /// тексты непусты, id/arcana/rank согласованы с зафиксированной нумерацией.
    #[test]
    fn production_cards_json_gate() {
        let team = project_root().join("agents/tarot-master");
        let manifest_path = team.join("database/cards.json");
        let text = std::fs::read_to_string(&manifest_path)
            .expect("cards.json читается (agents/tarot-master/database/)");
        let cards: Vec<serde_json::Value> = serde_json::from_str(&text).expect("cards.json — JSON-массив");

        assert_eq!(cards.len(), 78, "в manifest ровно 78 карт");

        let mut numbers = std::collections::HashSet::new();
        let mut ids = std::collections::HashSet::new();
        for card in &cards {
            let number = card["number"].as_u64().expect("number — целое") as i64;
            assert!(
                (0..=77).contains(&number),
                "number {} вне диапазона 0..77",
                number
            );
            assert!(numbers.insert(number), "number {} дублируется", number);

            let id = card["id"].as_str().expect("id — строка");
            assert!(!id.is_empty() && ids.insert(id.to_string()), "id {} пуст или дублируется", id);

            // Нумерация зафиксирована в PRP §3.1: 0–21 старшие, 22–35 жезлы,
            // 36–49 кубки, 50–63 мечи, 64–77 пентакли.
            let arcana = card["arcana"].as_str().expect("arcana — строка");
            let expected_arcana = match number {
                0..=21 => "major",
                22..=35 => "wands",
                36..=49 => "cups",
                50..=63 => "swords",
                _ => "pentacles",
            };
            assert_eq!(arcana, expected_arcana, "arcana не согласована с number {}", number);

            let rank = card["rank"].as_str().expect("rank — строка");
            assert!(!rank.is_empty(), "rank пуст у number {}", number);
            if number < 22 {
                assert_eq!(
                    rank,
                    number.to_string(),
                    "у старшего аркана rank обязан совпадать с number"
                );
            }

            for field in ["name_en", "name_ru", "keywords_ru", "upright_ru", "reversed_ru", "image_file"] {
                let value = card[field].as_str().unwrap_or_else(|| {
                    panic!("у number {} поле {} отсутствует или не строка", number, field)
                });
                assert!(
                    !value.trim().is_empty(),
                    "у number {} поле {} пустое",
                    number,
                    field
                );
            }
            assert!(
                card["upright_ru"].as_str().unwrap_or("").chars().count() >= 80,
                "upright_ru у number {} подозрительно короткий",
                number
            );
            assert!(
                card["reversed_ru"].as_str().unwrap_or("").chars().count() >= 80,
                "reversed_ru у number {} подозрительно короткий",
                number
            );

            let image_file = card["image_file"].as_str().expect("image_file — строка");
            let image_path = team.join("database/cards").join(image_file);
            assert!(
                image_path.is_file(),
                "файл карты {} не найден в database/cards/",
                image_path.display()
            );
        }
        assert_eq!(numbers.len(), 78, "number должны покрывать 0..77 без пропусков");
        assert_eq!(ids.len(), 78, "id уникальны");
    }

    /// Вспомогательная сборка путей боевой команды «Таролог».
    fn tarot_team_dirs() -> (PathBuf, PathBuf, PathBuf) {
        let team = project_root().join("agents/tarot-master");
        (team.join("tools"), team.join("transitions"), team)
    }

    /// gate_tool_contract (реальный Deno): draw с count="3" отвечает
    /// 3 уникальными числами 0..77 и orientations длиной 3.
    #[test]
    #[ignore]
    fn real_deno_draw_tool_contract() {
        let log = |msg: String| println!("[log] {}", msg);
        let bins_dir = std::env::temp_dir().join("king_orch_bins");
        std::fs::create_dir_all(&bins_dir).expect("создано");
        let deno = resolve_deno(&bins_dir, &log).expect("deno доступен");
        let (tools, transitions, _) = tarot_team_dirs();

        let (stdout, stderr, status) = run_deno(
            &deno,
            &tools,
            &transitions,
            &tools.join("draw_card_numbers.ts"),
            r#"{"count":"3"}"#,
        )
        .expect("тул выполнен");
        assert!(status.success(), "exit: {}, stderr: {}", status, String::from_utf8_lossy(&stderr));
        let output: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&stdout).trim()).expect("stdout — JSON");
        let numbers: Vec<u64> = output["numbers"]
            .as_array()
            .expect("numbers — массив")
            .iter()
            .map(|v| v.as_u64().expect("число"))
            .collect();
        assert_eq!(numbers.len(), 3, "три уникальных числа");
        let unique: std::collections::HashSet<u64> = numbers.iter().copied().collect();
        assert_eq!(unique.len(), 3, "числа уникальны: {:?}", numbers);
        for n in &numbers {
            assert!(n <= &77, "число {} вне 0..77", n);
        }
        let orientations = output["orientations"].as_array().expect("orientations — массив");
        assert_eq!(orientations.len(), 3, "ориентация на каждую карту");
        for o in orientations {
            let s = o.as_str().expect("orientation — строка");
            assert!(s == "upright" || s == "reversed", "orientation '{}'", s);
        }
    }

    /// gate_tool_contract (реальный Deno): lookup в форме движка — числа и
    /// ориентации, сериализованные в строки — отвечает cards с непустыми
    /// name_ru, orientation и image_file; позиции соответствуют count.
    #[test]
    #[ignore]
    fn real_deno_lookup_tool_string_form_contract() {
        let log = |msg: String| println!("[log] {}", msg);
        let bins_dir = std::env::temp_dir().join("king_orch_bins");
        std::fs::create_dir_all(&bins_dir).expect("создано");
        let deno = resolve_deno(&bins_dir, &log).expect("deno доступен");
        let (tools, transitions, _) = tarot_team_dirs();

        let (stdout, stderr, status) = run_deno(
            &deno,
            &tools,
            &transitions,
            &tools.join("lookup_tarot_cards.ts"),
            r#"{"numbers":"[3,21,57]","orientations":"[\"upright\",\"reversed\",\"upright\"]"}"#,
        )
        .expect("тул выполнен");
        assert!(status.success(), "exit: {}, stderr: {}", status, String::from_utf8_lossy(&stderr));
        let output: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&stdout).trim()).expect("stdout — JSON");

        assert_eq!(output["count"], 3);
        let positions = output["positions"].as_array().expect("positions — массив");
        assert_eq!(positions.len(), 3, "позиция на каждую карту");
        let cards = output["cards"].as_array().expect("cards — массив");
        assert_eq!(cards.len(), 3);
        let expected_numbers = [3u64, 21, 57];
        for (i, card) in cards.iter().enumerate() {
            assert_eq!(card["number"].as_u64(), Some(expected_numbers[i]), "карта по номеру");
            assert!(!card["name_ru"].as_str().unwrap_or("").is_empty(), "name_ru непуст");
            assert!(!card["name_en"].as_str().unwrap_or("").is_empty(), "name_en непуст");
            assert!(!card["upright_ru"].as_str().unwrap_or("").is_empty(), "upright_ru непуст");
            assert!(!card["reversed_ru"].as_str().unwrap_or("").is_empty(), "reversed_ru непуст");
            let orientation = card["orientation"].as_str().expect("orientation — строка");
            let expected = if i == 1 { "reversed" } else { "upright" };
            assert_eq!(orientation, expected, "orientation проставлена из параллельного массива");
            assert!(!card["image_file"].as_str().unwrap_or("").is_empty(), "image_file непуст");
        }
    }

    /// gate_tool_contract (реальный Deno): несуществующий номер → exit≠0
    /// и ошибка в stderr (честный отказ, не тихий пропуск).
    #[test]
    #[ignore]
    fn real_deno_lookup_tool_rejects_unknown_number() {
        let log = |msg: String| println!("[log] {}", msg);
        let bins_dir = std::env::temp_dir().join("king_orch_bins");
        std::fs::create_dir_all(&bins_dir).expect("создано");
        let deno = resolve_deno(&bins_dir, &log).expect("deno доступен");
        let (tools, transitions, _) = tarot_team_dirs();

        let (stdout, stderr, status) = run_deno(
            &deno,
            &tools,
            &transitions,
            &tools.join("lookup_tarot_cards.ts"),
            r#"{"numbers":"[999]","orientations":"[\"upright\"]"}"#,
        )
        .expect("процесс запущен");
        assert!(!status.success(), "несуществующий номер обязан дать exit≠0");
        assert!(stdout.is_empty(), "ошибочный тул не пишет stdout");
        let stderr_text = String::from_utf8_lossy(&stderr);
        assert!(
            stderr_text.contains("999") || stderr_text.to_lowercase().contains("числом"),
            "stderr объясняет отказ: {}",
            stderr_text
        );
    }
}
