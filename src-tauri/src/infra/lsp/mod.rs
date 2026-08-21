//! 🧩 LSP-интеграция (Language Server Protocol).
//!
//! `LspClient` — JSON-RPC 2.0 по stdio с Content-Length framing (протокол LSP).
//! Sidecar-процесс (`rust-analyzer`) запускается один раз на workspace root
//! и живёт до конца процесса (менеджер в `manager`). Все методы — блокирующие,
//! с таймаутом (зависший сервер не вешает оркестратор).
//!
//! Фазовый фолбэк: если сервер не установлен (нет в `bins/`, нет в PATH) —
//! честная ошибка «не установлен», а не молчание (правило 2.2).

pub mod manager;

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::infra::process_util::kill_process_tree;

/// Таймаут ожидания ответа LSP-сервера (сек). Первый запрос к rust-analyzer
/// долгий (индексация проекта), поэтому не слишком мал.
pub const LSP_DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Каноническое имя сервера в `bins/` (без расширения).
pub const LSP_SERVER_NAME: &str = "rust-analyzer";

/// Языковой сервер по расширению файла (для `textDocument/didOpen`).
pub fn language_id(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "py" => "python",
        "go" => "go",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "html" => "html",
        "css" => "css",
        "md" => "markdown",
        _ => "plaintext",
    }
}

/// Абсолютный путь файла → `file://` URI (для LSP). Экранирует спецсимволы.
pub fn path_to_uri(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let encoded: String = normalized
        .chars()
        .flat_map(|c| match c {
            ' ' => "%20".chars().collect::<Vec<_>>(),
            '#' => "%23".chars().collect::<Vec<_>>(),
            '?' => "%3F".chars().collect::<Vec<_>>(),
            '%' => "%25".chars().collect::<Vec<_>>(),
            other => vec![other],
        })
        .collect();
    if encoded.starts_with('/') {
        format!("file://{}", encoded)
    } else {
        format!("file:///{}", encoded)
    }
}

/// Сообщение LSP: JSON-RPC с Content-Length framing.
struct FramedMessage {
    content_length: usize,
    json: String,
}

/// Читает одно framed-сообщение из потока (заголовки + тело).
fn read_framed<R: Read>(reader: &mut R) -> std::io::Result<Option<FramedMessage>> {
    let mut bufreader = BufReader::new(reader);
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = bufreader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(val) = line.strip_prefix("Content-Length:") {
            content_length = val.trim().parse().ok();
        }
    }
    let Some(len) = content_length else {
        return Ok(None);
    };
    let mut body = vec![0u8; len];
    bufreader.read_exact(&mut body)?;
    let json = String::from_utf8_lossy(&body).to_string();
    Ok(Some(FramedMessage { content_length: len, json }))
}

/// LSP-клиент: обёртка над sidecar-процессом (JSON-RPC 2.0 по stdio).
pub struct LspClient {
    child: Child,
    stdin: ChildStdin,
    next_id: i64,
    /// Канал от фонового читателя stdout (parsed JSON-сообщения).
    receiver: mpsc::Receiver<Value>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    timeout: Duration,
    root_uri: String,
}

impl LspClient {
    /// Запустить LSP-сервер в режиме stdio и выполнить handshake (initialize/initialized).
    pub fn spawn(server_path: &Path, workspace_root: &Path, log_cb: impl Fn(String) + Send + Sync + 'static) -> Result<Self, String> {
        log_cb(format!("🚀 Запуск LSP-сервера: {}", server_path.display()));

        let mut cmd = Command::new(server_path);
        cmd.arg("--stdio")
            .current_dir(workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }

        let mut child = cmd.spawn().map_err(|e| format!("Не удалось запустить LSP-сервер: {}", e))?;
        let stdin = child.stdin.take().ok_or("Не удалось получить stdin LSP-сервера")?;
        let stdout = child.stdout.take().ok_or("Не удалось получить stdout LSP-сервера")?;
        let stderr = child.stderr.take().ok_or("Не удалось получить stderr LSP-сервера")?;

        let cb = Arc::new(log_cb);
        let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_buf = stderr_tail.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                cb(format!("[LSP Stderr] {}", line));
                let mut tail = stderr_buf.lock().unwrap();
                if tail.len() >= 15 { tail.pop_front(); }
                tail.push_back(line);
            }
        });

        let (sender, receiver) = mpsc::channel::<Value>();
        thread::spawn(move || {
            let mut reader = stdout;
            while let Ok(Some(framed)) = read_framed(&mut reader) {
                if let Ok(val) = serde_json::from_str::<Value>(&framed.json) {
                    if sender.send(val).is_err() { break; }
                }
            }
        });

        let root_uri = path_to_uri(workspace_root);
        let mut client = Self {
            child,
            stdin,
            next_id: 1,
            receiver,
            stderr_tail,
            timeout: Duration::from_secs(LSP_DEFAULT_TIMEOUT_SECS),
            root_uri,
        };
        client.initialize()?;
        Ok(client)
    }

    fn send_raw(&mut self, data: &Value) -> Result<(), String> {
        let text = serde_json::to_string(data).map_err(|e| e.to_string())?;
        let header = format!("Content-Length: {}\r\n\r\n", text.len());
        self.stdin.write_all(header.as_bytes()).map_err(|e| e.to_string())?;
        self.stdin.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send_raw(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send_raw(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        self.read_response(id)
    }

    fn read_response(&mut self, target_id: i64) -> Result<Value, String> {
        let deadline = Instant::now() + self.timeout;
        loop {
            let remaining = deadline.checked_duration_since(Instant::now()).unwrap_or_default();
            match self.receiver.recv_timeout(remaining) {
                Ok(msg) => {
                    if msg.get("id").and_then(|i| i.as_i64()) == Some(target_id) {
                        if let Some(err) = msg.get("error") {
                            return Err(format!("Ошибка LSP JSON-RPC: {}", err));
                        }
                        return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
                    }
                    // Чужие сообщения (не наш id) — игнорируем.
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    kill_process_tree(&mut self.child);
                    return Err(format!(
                        "⏱ LSP-сервер не ответил за {}с (таймаут вызова). Процесс принудительно остановлен.",
                        self.timeout.as_secs()
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let tail: Vec<String> = self.stderr_tail.lock().unwrap().iter().cloned().collect();
                    return Err(format!(
                        "LSP-сервер неожиданно закрыл поток stdout{}",
                        if tail.is_empty() { String::new() } else { format!(" (stderr: {})", tail.join(" | ")) }
                    ));
                }
            }
        }
    }

    /// Проверка живости процесса сервера (без блокировки).
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    fn initialize(&mut self) -> Result<(), String> {
        let params = json!({
            "processId": std::process::id(),
            "rootUri": self.root_uri,
            "capabilities": {
                "textDocument": {
                    "definition": { "linkSupport": true },
                    "references": {},
                    "diagnostic": {}
                }
            },
            "clientInfo": { "name": "king-orch", "version": env!("CARGO_PKG_VERSION") }
        });
        self.request("initialize", params)?;
        self.notify("initialized", json!({}))?;
        Ok(())
    }

    /// Открыть документ (didOpen) — сервер читает текст файла сам по uri.
    fn did_open(&mut self, uri: &str) -> Result<(), String> {
        self.notify("textDocument/didOpen", json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_id(Path::new(uri)),
                "version": 1,
                "text": ""
            }
        }))
    }

    /// `textDocument/definition` — определения символа в позиции.
    pub fn definition(&mut self, path: &Path, line: u64, character: u64) -> Result<Value, String> {
        let uri = path_to_uri(path);
        self.did_open(&uri)?;
        let result = self.request("textDocument/definition", json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }))?;
        Ok(result)
    }

    /// `textDocument/references` — все ссылки на символ в позиции.
    pub fn references(&mut self, path: &Path, line: u64, character: u64, include_declaration: bool) -> Result<Value, String> {
        let uri = path_to_uri(path);
        self.did_open(&uri)?;
        let result = self.request("textDocument/references", json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": include_declaration }
        }))?;
        Ok(result)
    }

    /// `textDocument/diagnostic` (pull-диагностика, LSP 3.17) — проблемы файла.
    pub fn diagnostics(&mut self, path: &Path) -> Result<Value, String> {
        let uri = path_to_uri(path);
        self.did_open(&uri)?;
        let result = self.request("textDocument/diagnostic", json!({
            "textDocument": { "uri": uri },
            "previousResultId": Value::Null
        }))?;
        Ok(result)
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        kill_process_tree(&mut self.child);
    }
}

/// Поиск исполняемого файла LSP-сервера: сначала `bins/`, потом PATH.
pub fn find_lsp_server(bins_dir: &Path) -> Option<PathBuf> {
    let exe = if cfg!(target_os = "windows") {
        format!("{}.exe", LSP_SERVER_NAME)
    } else {
        LSP_SERVER_NAME.to_string()
    };
    let local = bins_dir.join(&exe);
    if local.is_file() {
        return Some(local);
    }
    // PATH
    let paths = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&paths) {
        let p = dir.join(&exe);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Форматирование локаций (Location/LocationLink) в читаемый текст.
pub fn format_locations(value: &Value) -> String {
    let mut out = Vec::new();
    match value {
        Value::Null => out.push("(нет результата)".to_string()),
        Value::Array(items) => {
            if items.is_empty() {
                out.push("(ссылок/определений не найдено)".to_string());
            }
            for item in items {
                out.push(format_location(item));
            }
        }
        other => out.push(format!("{}", other)),
    }
    if out.is_empty() { out.push("(пусто)".to_string()); }
    out.join("\n")
}

fn format_location(loc: &Value) -> String {
    // LocationLink: {originSelectionRange, targetUri, targetRange, targetSelectionRange}
    // Location: {uri, range}
    let uri = loc
        .get("targetUri").or_else(|| loc.get("uri"))
        .and_then(|u| u.as_str())
        .unwrap_or("?");
    let range = loc.get("targetRange").or_else(|| loc.get("range"));
    let pos = match range {
        Some(r) => {
            let start = &r["start"];
            let line = start["line"].as_u64().unwrap_or(0);
            let character = start["character"].as_u64().unwrap_or(0);
            format!("{}:{}", line + 1, character + 1)
        }
        None => "?:?".to_string(),
    };
    let short = Path::new(uri)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| uri.to_string());
    format!("{} ({})", short, pos)
}

/// Форматирование диагностики в читаемый текст.
pub fn format_diagnostics(value: &Value) -> String {
    let items = value.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
    if items.is_empty() {
        return "Диагностика: ошибок не найдено".to_string();
    }
    let mut out = Vec::new();
    for d in items {
        let severity = match d["severity"].as_u64().unwrap_or(3) {
            1 => "ERROR",
            2 => "WARN",
            3 => "INFO",
            4 => "HINT",
            _ => "?",
        };
        let message = d["message"].as_str().unwrap_or("?");
        let line = d["range"]["start"]["line"].as_u64().unwrap_or(0) + 1;
        let character = d["range"]["start"]["character"].as_u64().unwrap_or(0) + 1;
        out.push(format!("[{}] {}:{} {}", severity, line, character, message));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_to_uri_handles_windows_and_spaces() {
        assert_eq!(path_to_uri(Path::new("C:/proj/src/main.rs")), "file:///C:/proj/src/main.rs");
        assert_eq!(path_to_uri(Path::new("/proj/a b.ts")), "file:///proj/a%20b.ts");
    }

    #[test]
    fn language_id_maps_extensions() {
        assert_eq!(language_id(Path::new("x.rs")), "rust");
        assert_eq!(language_id(Path::new("x.ts")), "typescript");
        assert_eq!(language_id(Path::new("x.unknown_xyz")), "plaintext");
    }

    #[test]
    fn format_locations_readable() {
        let locs = json!([
            {"uri": "file:///C:/proj/src/a.rs", "range": {"start": {"line": 4, "character": 2}}}
        ]);
        let s = format_locations(&locs);
        assert!(s.contains("a.rs"));
        assert!(s.contains("5:3"), "строка должна быть 1-based: {}", s);
    }

    #[test]
    fn find_lsp_server_prefers_bins_dir() {
        let d = std::env::temp_dir().join(format!("lsp_find_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // Если в bins/ лежит файл — он приоритетнее PATH (даже при rustup-шиме в PATH).
        let exe = d.join(if cfg!(windows) { "rust-analyzer.exe" } else { "rust-analyzer" });
        std::fs::write(&exe, b"dummy").unwrap();
        assert_eq!(find_lsp_server(&d).as_ref(), Some(&exe));
        std::fs::remove_file(&exe).unwrap();
        // После удаления локального файла результат зависит от PATH (может быть
        // rustup-шим или ничего) — здесь не проверяем конкретное значение.
        let _ = std::fs::remove_dir_all(&d);
    }
}
