use std::process::{Command, Child, Stdio};
use std::io::{BufReader, BufRead, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use serde_json::{Value, json};

use crate::infra::process_util::kill_process_tree;

/// Общий пул MCP-клиентов на весь запрос: каждый уникальный сервер (по имени)
/// стартует deno ровно один раз и гасится один раз в конце запроса, а не по
/// N раз на каждого агента/сабагента (это давало вспышки чёрных окон taskkill
/// и лишнюю задержку на старт deno).
pub type SharedMcpClient = Arc<Mutex<McpClient>>;
pub type McpPool = Arc<Mutex<HashMap<String, SharedMcpClient>>>;

/// Таймаут ожидания ответа от MCP-сервера (сек). Защищает оркестратор от
/// вечного висняка при зависшем Deno-сервере. Переопределяется через
/// `spawn_stub_with_env_timeout` (в перспективе — из `app_config.json`).
pub const MCP_DEFAULT_TIMEOUT_SECS: u64 = 120;

pub struct McpClient {
    child: Child,
    stdin: std::process::ChildStdin,
    next_id: Arc<Mutex<i64>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    /// Канал от фонового читателя stdout. Строки складываются сюда, а
    /// `read_response` забирает их с таймаутом (вместо блокирующего `read_line`).
    receiver: mpsc::Receiver<String>,
    timeout: Duration,
}

impl McpClient {
    pub fn spawn_stub(cmd_path: &str, args: &[&str], log_cb: impl Fn(String) + Send + Sync + 'static) -> Result<Self, String> {
        Self::spawn_stub_with_env(cmd_path, args, &[], log_cb)
    }

    pub fn spawn_stub_with_env(
        cmd_path: &str, args: &[&str], envs: &[(&str, &str)],
        log_cb: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        Self::spawn_stub_with_env_timeout(cmd_path, args, envs, Duration::from_secs(MCP_DEFAULT_TIMEOUT_SECS), log_cb)
    }

    pub fn spawn_stub_with_env_timeout(
        cmd_path: &str, args: &[&str], envs: &[(&str, &str)],
        timeout: Duration,
        log_cb: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        log_cb(format!("🚀 Запуск MCP сервера: {} {:?}", cmd_path, args));

        let mut cmd = Command::new(cmd_path);
        cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (k, v) in envs { cmd.env(k, v); }

        #[cfg(target_os = "windows")]
        { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }

        let mut child = cmd.spawn().map_err(|e| format!("Не удалось запустить MCP-сервер: {}", e))?;
        let stdin = child.stdin.take().ok_or("Не удалось получить stdin")?;
        let stdout = child.stdout.take().ok_or("Не удалось получить stdout")?;
        let stderr = child.stderr.take().ok_or("Не удалось получить stderr")?;

        let cb = Arc::new(log_cb);
        let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_buf = stderr_tail.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                cb(format!("[MCP Stderr] {}", line));
                let mut tail = stderr_buf.lock().unwrap();
                if tail.len() >= 15 { tail.pop_front(); }
                tail.push_back(line);
            }
        });

        // Фоновый читатель stdout -> mpsc канал. Позволяет читать ответы с таймаутом,
        // а не блокироваться навсегда при зависшем сервере.
        let (sender, receiver) = mpsc::channel::<String>();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().flatten() {
                if sender.send(line).is_err() { break; } // получатель отвалился — завершаем
            }
        });

        let mut client = Self { child, stdin, next_id: Arc::new(Mutex::new(1)), stderr_tail, receiver, timeout };
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
        let deadline = Instant::now() + self.timeout;
        loop {
            let remaining = deadline.checked_duration_since(Instant::now()).unwrap_or_default();
            match self.receiver.recv_timeout(remaining) {
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() { continue; }
                    if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                        if val.get("id").and_then(|id| id.as_i64()) == Some(target_id) {
                            return Ok(val);
                        }
                        // Не наш id (напр. notification без поля id) — продолжаем ждать.
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    kill_process_tree(&mut self.child);
                    return Err(format!(
                        "⏱ MCP-сервер не ответил за {}с (таймаут вызова). Процесс принудительно остановлен.",
                        self.timeout.as_secs()
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let tail: Vec<String> = self.stderr_tail.lock().unwrap().iter().cloned().collect();
                    if tail.is_empty() {
                        return Err("MCP-сервер неожиданно закрыл поток stdout (stderr пуст)".to_string());
                    }
                    return Err(format!("MCP-сервер неожиданно закрыл поток stdout. Причина (последний stderr): {}", tail.join(" | ")));
                }
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
    fn drop(&mut self) {
        kill_process_tree(&mut self.child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Заведомо «немой» сервер (Windows): ждёт 600с, ничего не пишет в stdout.
    /// Если таймаут в read_response работает — конструктор вернёт Err примерно
    /// за 2с, а не повиснет навсегда.
    #[test]
    fn mcp_call_times_out_instead_of_hanging() {
        let dummy = "cmd";
        // «Немой» сервер: timeout блокирует 30с, ничего валидного в stdout не шлёт.
        let args = ["/c", "timeout", "/t", "30", "/nobreak"];
        let args_refs: Vec<&str> = args.iter().map(|s| *s).collect();
        let start = Instant::now();
        let res = McpClient::spawn_stub_with_env_timeout(
            dummy, &args_refs, &[], Duration::from_secs(2), |_| {},
        );
        let elapsed = start.elapsed();
        assert!(res.is_err(), "ожидалась ошибка таймаута, а не успешный коннект к 'timeout'");
        // Возвращаемся быстро (с запасом на старт cmd), а не через 600с.
        assert!(elapsed < Duration::from_secs(30), "таймаут не сработал, прошло {:?}", elapsed);
    }
}
