use std::process::{Command, Child, Stdio};
use std::io::{BufReader, BufRead, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use serde_json::{Value, json};

pub struct McpClient {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout_reader: BufReader<std::process::ChildStdout>,
    next_id: Arc<Mutex<i64>>,
}

impl McpClient {
    /// Спавн MCP сервера через коллбэк логирования (без привязки к AppHandle)
    pub fn spawn_stub(cmd_path: &str, args: &[&str], log_cb: impl Fn(String) + Send + Sync + 'static) -> Result<Self, String> {
        log_cb(format!("🚀 Запуск MCP сервера: {} {:?}", cmd_path, args));
        
        let mut cmd = Command::new(cmd_path);
        cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }

        let mut child = cmd.spawn().map_err(|e| format!("Не удалось запустить MCP-сервер: {}", e))?;
        let stdin = child.stdin.take().ok_or("Не удалось получить stdin")?;
        let stdout = child.stdout.take().ok_or("Не удалось получить stdout")?;
        let stderr = child.stderr.take().ok_or("Не удалось получить stderr")?;

        let cb = Arc::new(log_cb);
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() { cb(format!("[MCP Stderr] {}", line)); }
        });

        let stdout_reader = BufReader::new(stdout);
        let mut client = Self { child, stdin, stdout_reader, next_id: Arc::new(Mutex::new(1)) };
        client.initialize()?;
        Ok(client)
    }

    fn send_raw(&mut self, data: &Value) -> Result<(), String> {
        let text = serde_json::to_string(data).map_err(|e| e.to_string())?;
        self.stdin.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        self.stdin.write_all(b"\n").map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn read_response(&mut self, target_id: i64) -> Result<Value, String> {
        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = self.stdout_reader.read_line(&mut line).map_err(|e| e.to_string())?;
            if bytes_read == 0 { return Err("MCP-сервер неожиданно закрыл поток stdout".to_string()); }
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }
            if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                if val.get("id").and_then(|id| id.as_i64()) == Some(target_id) { return Ok(val); }
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = { let mut id_lock = self.next_id.lock().unwrap(); let current = *id_lock; *id_lock += 1; current };
        self.send_raw(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        let response = self.read_response(id)?;
        if let Some(error) = response.get("error") { return Err(format!("Ошибка MCP JSON-RPC: {}", error)); }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    fn initialize(&mut self) -> Result<(), String> {
        let params = json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "king-orch-client", "version": "1.0.0" } });
        self.call("initialize", params)?;
        self.send_raw(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        Ok(())
    }

    pub fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        let result = self.call("tools/list", json!({}))?;
        Ok(result.get("tools").and_then(|t| t.as_array()).cloned().unwrap_or_default())
    }

    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<String, String> {
        let result = self.call("tools/call", json!({ "name": name, "arguments": arguments }))?;
        if let Some(content_array) = result.get("content").and_then(|c| c.as_array()) {
            let mut output = String::new();
            for item in content_array { if let Some(text) = item.get("text").and_then(|t| t.as_str()) { output.push_str(text); } }
            Ok(output)
        } else { Err("Некорректный формат ответа tools/call".to_string()) }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) { let _ = self.child.kill(); }
}