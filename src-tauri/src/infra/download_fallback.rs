//! Цепочка фоллбэков скачивания файлов.
//!
//! Порядок (каждый следующий — если предыдущий не сработал):
//!   1. reqwest (с прокси из env, streaming + resume)
//!   2. PowerShell Invoke-WebRequest (.NET WebClient, системный прокси, Schannel)
//!   3. bitsadmin /transfer (WinINet, системный прокси, §6.4 правил)
//!   4. MCP-инструмент download_file (Deno, использует PowerShell/WinINet)
//!   5. Chrome CDP (headless Chrome, BoringSSL — обход DPI)
//!
//! Логируется какой метод сработал.

use std::path::Path;
use std::time::Duration;

/// Скачивание файла с цепочкой фоллбэков.
///
/// `on_log` — для логирования какая ступень сработала.
/// `on_progress` — прогресс-бар (только для reqwest, для остальных нет потокового прогресса).
pub async fn download_with_fallback<L: Fn(String) + Sync, P: Fn(u64, u64) + Sync>(
    url: &str,
    dest: &Path,
    expected_size: Option<u64>,
    on_log: &L,
    on_progress: &P,
) -> Result<(), String> {
    // ── Уровень 1: reqwest (streaming + resume) ──
    on_log("🔄 [1/5] Скачивание через HTTP (reqwest)...".to_string());
    match download_via_reqwest_streaming(url, dest, expected_size, on_progress).await {
        Ok(bytes) => {
            on_log(format!("✅ [1/5] reqwest: скачано {} МБ", bytes / 1024 / 1024));
            return Ok(());
        }
        Err(e) => {
            on_log(format!("⚠️ [1/5] reqwest не смог: {}", e));
        }
    }

    // ── Уровень 2: PowerShell (.NET WebClient) ──
    on_log("🔄 [2/5] Скачивание через PowerShell (.NET)...".to_string());
    match download_via_powershell(url, dest, on_log).await {
        Ok(bytes) => {
            on_log(format!("✅ [2/5] PowerShell: скачано {} МБ", bytes / 1024 / 1024));
            return Ok(());
        }
        Err(e) => {
            on_log(format!("⚠️ [2/5] PowerShell не смог: {}", e));
        }
    }

    // ── Уровень 3: bitsadmin /transfer (WinINet, §6.4 правил) ──
    on_log("🔄 [3/5] Скачивание через bitsadmin (WinINet)...".to_string());
    match download_via_bits(url, dest, on_log).await {
        Ok(bytes) => {
            on_log(format!("✅ [3/5] bitsadmin: скачано {} МБ", bytes / 1024 / 1024));
            return Ok(());
        }
        Err(e) => {
            on_log(format!("⚠️ [3/5] bitsadmin не смог: {}", e));
        }
    }

    // ── Уровень 4: MCP (Deno + PowerShell/WinINet) ──
    on_log("🔄 [4/5] Скачивание через MCP (Deno)...".to_string());
    match download_via_mcp(url, dest, on_log).await {
        Ok(bytes) => {
            on_log(format!("✅ [4/5] MCP: скачано {} МБ", bytes / 1024 / 1024));
            return Ok(());
        }
        Err(e) => {
            on_log(format!("⚠️ [4/5] MCP не смог: {}", e));
        }
    }

    // ── Уровень 5: Chrome CDP (headless Chrome) ──
    on_log("🔄 [5/5] Скачивание через Chrome (CDP)...".to_string());
    match download_via_chrome_cdp(url, dest, on_log).await {
        Ok(bytes) => {
            on_log(format!("✅ [5/5] Chrome CDP: скачано {} МБ", bytes / 1024 / 1024));
            return Ok(());
        }
        Err(e) => {
            on_log(format!("⚠️ [5/5] Chrome CDP не смог: {}", e));
        }
    }

    Err("Все 5 методов скачивания не сработали. Проверьте подключение к интернету и настройки прокси.".to_string())
}

// ══════════════════════════════════════════════════════════════════════════════
// Уровень 1: reqwest streaming + resume
// ══════════════════════════════════════════════════════════════════════════════

async fn download_via_reqwest_streaming<P: Fn(u64, u64) + Sync>(
    url: &str,
    dest: &Path,
    expected_size: Option<u64>,
    on_progress: &P,
) -> Result<u64, String> {
    use futures_util::StreamExt;
    use std::io::Write;

    let part_path = dest.with_extension("zip.part");
    let resume_from = std::fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);

    let client = reqwest::Client::builder()
        .user_agent("king-orch-app/1.0")
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("Ошибка создания HTTP-клиента: {}", e))?;

    let mut req = client.get(url);
    if resume_from > 0 {
        req = req.header("Range", format!("bytes={}-", resume_from));
    }

    let resp = req.send().await.map_err(|e| {
        let msg = crate::infra::llm::chain_err(&e, 3);
        format!("Ошибка соединения: {}", msg)
    })?;

    let status = resp.status();
    let total = expected_size.unwrap_or(0);

    let mut file = if status == reqwest::StatusCode::PARTIAL_CONTENT && resume_from > 0 {
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&part_path)
            .map_err(|e| format!("Ошибка файла: {}", e))?
    } else {
        if !status.is_success() {
            return Err(format!("HTTP {}", status));
        }
        std::fs::File::create(&part_path).map_err(|e| format!("Ошибка файла: {}", e))?
    };

    let mut downloaded = resume_from;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            let msg = crate::infra::llm::chain_err(&e, 3);
            format!("Ошибка приёма данных: {}", msg)
        })?;
        file.write_all(&chunk).map_err(|e| format!("Ошибка записи: {}", e))?;
        downloaded += chunk.len() as u64;
        if total > 0 {
            on_progress(downloaded, total);
        }
    }

    drop(file);
    if total > 0 && downloaded < total {
        let _ = std::fs::remove_file(&part_path);
        return Err(format!("Загрузка прервалась: {} из {} байт", downloaded, total));
    }

    std::fs::rename(&part_path, dest).map_err(|e| format!("Ошибка финализации: {}", e))?;
    Ok(downloaded)
}

// ══════════════════════════════════════════════════════════════════════════════
// Уровень 2: PowerShell Invoke-WebRequest
// ══════════════════════════════════════════════════════════════════════════════

async fn download_via_powershell<L: Fn(String) + Sync>(
    url: &str,
    dest: &Path,
    on_log: &L,
) -> Result<u64, String> {
    let dest_str = dest.display().to_string();
    let url_escaped = url.replace('\'', "''");
    let dest_escaped = dest_str.replace('\'', "''");

    // PowerShell: Tls12 + SilentlyContinue (без прогресс-бара) + UseBasicParsing
    let ps_script = format!(
        "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
         $ProgressPreference = 'SilentlyContinue'; \
         Invoke-WebRequest -Uri '{url}' -OutFile '{dest}' -UseBasicParsing; \
         (Get-Item '{dest}').Length",
        url = url_escaped,
        dest = dest_escaped,
    );

    on_log(format!("   PowerShell: {}...", &url[..url.len().min(80)]));

    let output = tokio::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &ps_script,
        ])
        .output()
        .await
        .map_err(|e| format!("PowerShell не найден: {}", e))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let bytes: u64 = stdout.parse().unwrap_or(0);
        if bytes > 0 {
            Ok(bytes)
        } else {
            // PowerShell скачал, но не вернул размер — проверяем по файлу
            std::fs::metadata(dest)
                .map(|m| m.len())
                .map_err(|e| format!("Файл не создан после PowerShell: {}", e))
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("PowerShell: {}", stderr.trim()))
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Уровень 3: bitsadmin /transfer (WinINet, §6.4 правил)
// ══════════════════════════════════════════════════════════════════════════════

async fn download_via_bits<L: Fn(String) + Sync>(
    url: &str,
    dest: &Path,
    on_log: &L,
) -> Result<u64, String> {
    let dest_str = dest.display().to_string();
    let job_name = format!("king_orch_dl_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis());

    on_log(format!("   bitsadmin: {}...", &url[..url.len().min(80)]));

    // bitsadmin /transfer <job> /download /priority high <url> <dest>
    let output = tokio::process::Command::new("bitsadmin")
        .args([
            "/transfer",
            &job_name,
            "/download",
            "/priority",
            "high",
            url,
            &dest_str,
        ])
        .output()
        .await
        .map_err(|e| format!("bitsadmin не найден: {}", e))?;

    if output.status.success() {
        // bitsadmin не выводит размер — проверяем по файлу
        std::fs::metadata(dest)
            .map(|m| m.len())
            .map_err(|e| format!("Файл не создан после bitsadmin: {}", e))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(format!("bitsadmin: {} {}", stdout.trim(), stderr.trim()))
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Уровень 4: MCP-инструмент (Deno downloader.ts)
// ══════════════════════════════════════════════════════════════════════════════

async fn download_via_mcp<L: Fn(String) + Sync>(
    url: &str,
    dest: &Path,
    on_log: &L,
) -> Result<u64, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .ok_or_else(|| "Не удалось определить папку приложения".to_string())?;

    // Ищем downloader.ts
    let mcp_script = exe_dir.join("mcp_servers").join("downloader.ts");
    if !mcp_script.exists() {
        // Фолбэк: ищем рядом с src-tauri (dev-режим)
        let dev_script = exe_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("mcp_servers").join("downloader.ts"))
            .filter(|p| p.exists());
        match dev_script {
            Some(p) => {
                return run_mcp_download(url, dest, &p, on_log).await;
            }
            None => {
                return Err("MCP-сервер downloader.ts не найден".to_string());
            }
        }
    }
    run_mcp_download(url, dest, &mcp_script, on_log).await
}

async fn run_mcp_download<L: Fn(String) + Sync>(
    url: &str,
    dest: &Path,
    script: &Path,
    on_log: &L,
) -> Result<u64, String> {
    // Ищем deno
    let deno_path = find_deno()?;
    on_log(format!("   MCP: deno + {}", script.file_name().unwrap_or_default().to_string_lossy()));

    let dest_str = dest.display().to_string();

    // Запускаем MCP-сервер как CLI: передаём аргументы вместо stdin JSON-RPC
    let output = tokio::process::Command::new(&deno_path)
        .args([
            "run",
            "--allow-all",
            "--no-lock",
            script.to_str().unwrap_or(""),
            "--download-cli",
            url,
            &dest_str,
        ])
        .output()
        .await
        .map_err(|e| format!("Не удалось запустить deno: {}", e))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // Ответ: "OK:<bytes>" или "ERROR:<msg>"
        if let Some(bytes_str) = stdout.strip_prefix("OK:") {
            let bytes: u64 = bytes_str.trim().parse().unwrap_or(0);
            return Ok(bytes);
        }
        // Нет префикса OK — проверяем по размеру файла
        if stdout.contains("OK") || dest.exists() {
            return std::fs::metadata(dest)
                .map(|m| m.len())
                .map_err(|e| format!("Файл не создан: {}", e));
        }
        Err(format!("MCP: {}", stdout))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("MCP: {}", stderr.trim()))
    }
}

fn find_deno() -> Result<std::path::PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .ok_or_else(|| "Не удалось определить папку приложения".to_string())?;

    let suffix = if cfg!(target_os = "windows") { ".exe" } else { "" };

    // 1. bins/deno.exe (рядом с king_orch.exe)
    let bins_deno = exe_dir.join(format!("bins/deno{}", suffix));
    if bins_deno.exists() {
        return Ok(bins_deno);
    }

    // 2. llamacpp/deno.exe (рядом с king_orch.exe)
    let llamacpp_deno = exe_dir.join(format!("llamacpp/deno{}", suffix));
    if llamacpp_deno.exists() {
        return Ok(llamacpp_deno);
    }

    // 3. В PATH
    #[cfg(windows)]
    {
        let result = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "(Get-Command deno -ErrorAction SilentlyContinue).Source"])
            .output();
        if let Ok(out) = result {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                let p = std::path::PathBuf::from(path);
                if p.exists() {
                    return Ok(p);
                }
            }
        }
    }

    Err("Deno не найден (нужен для MCP-фоллбэка). Установите: https://deno.land".to_string())
}

// ══════════════════════════════════════════════════════════════════════════════
// Уровень 5: Chrome CDP (headless Chrome, BoringSSL — обход DPI)
// ══════════════════════════════════════════════════════════════════════════════

async fn download_via_chrome_cdp<L: Fn(String) + Sync>(
    url: &str,
    dest: &Path,
    on_log: &L,
) -> Result<u64, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    // 1. Найти Chrome/CloakBrowser
    let chrome_exe = find_chrome_exe_for_download()?;
    on_log(format!("   Chrome CDP: {}", chrome_exe.display()));

    // 2. Временная папка для профиля Chrome
    let temp_dir = std::env::temp_dir().join(format!("king_orch_chrome_dl_{}", 
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Ошибка создания temp-директории: {}", e))?;

    // 3. Запустить headless Chrome
    let mut child = tokio::process::Command::new(&chrome_exe)
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--disable-extensions",
            "--remote-debugging-port=0",
            &format!("--user-data-dir={}", temp_dir.display()),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Не удалось запустить Chrome: {}", e))?;

    // 4. Прочитать WebSocket URL из stdout
    let stdout = child.stdout.take().ok_or("Нет stdout у Chrome")?;
    let mut reader = tokio::io::BufReader::new(stdout).lines();
    let ws_url = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(line) = reader.next_line().await.map_err(|e| format!("Ошибка чтения stdout: {}", e))? {
            if line.contains("DevTools listening on") {
                return Ok(line.replace("DevTools listening on ", ""));
            }
        }
        Err("Chrome не вывел DevTools listening".to_string())
    })
    .await
    .map_err(|_| "Таймаут ожидания Chrome DevTools".to_string())??;

    on_log(format!("   Chrome CDP: WebSocket {}", &ws_url[..ws_url.len().min(60)]));

    // 5. Подключиться к CDP и скачать файл
    let result = cdp_download_file(&ws_url, url, dest, on_log).await;

    // 6. Убить Chrome
    let _ = child.kill().await;
    let _ = std::fs::remove_dir_all(&temp_dir);

    result
}

async fn cdp_download_file<L: Fn(String) + Sync>(
    ws_url: &str,
    download_url: &str,
    dest: &Path,
    on_log: &L,
) -> Result<u64, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    // Парсим URL для WebSocket
    let ws_addr = ws_url
        .strip_prefix("ws://")
        .or_else(|| ws_url.strip_prefix("wss://"))
        .ok_or_else(|| format!("Некорректный WebSocket URL: {}", ws_url))?;

    // TCP connect
    let stream = tokio::net::TcpStream::connect(ws_addr)
        .await
        .map_err(|e| format!("TCP connect к Chrome failed: {}", e))?;

    // HTTP Upgrade handshake (минимальный WebSocket)
    let key = base64_encode(&rand_bytes(16));
    let mut request = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
        ws_addr, key
    );

    let (mut reader, mut writer) = tokio::io::split(stream);
    writer.write_all(request.as_bytes()).await
        .map_err(|e| format!("WebSocket handshake write: {}", e))?;

    // Читаем HTTP 101 Switching Protocols
    let mut buf_reader = tokio::io::BufReader::new(&mut reader);
    let mut headers_done = false;
    while !headers_done {
        let mut line = String::new();
        buf_reader.read_line(&mut line).await
            .map_err(|e| format!("WebSocket handshake read: {}", e))?;
        if line.trim().is_empty() || line == "\r\n" {
            headers_done = true;
        }
    }

    on_log("   Chrome CDP: WebSocket connected".to_string());

    // Устанавливаем download behavior
    let set_download_cmd = serde_json::json!({
        "id": 1,
        "method": "Browser.setDownloadBehavior",
        "params": {
            "behavior": "allow",
            "downloadPath": dest.parent().unwrap_or(Path::new(".")).to_string_lossy(),
            "eventsEnabled": true
        }
    });
    send_cdp_message(&mut writer, &set_download_cmd.to_string()).await?;

    // Навигация на URL (начинает скачивание)
    let navigate_cmd = serde_json::json!({
        "id": 2,
        "method": "Page.navigate",
        "params": { "url": download_url }
    });
    send_cdp_message(&mut writer, &navigate_cmd.to_string()).await?;

    // Ждём завершения скачивания (таймаут 5 минут для больших файлов)
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    let mut got_event = false;

    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }

        match tokio::time::timeout(Duration::from_secs(5), buf_reader.read_line(&mut String::new())).await {
            Ok(Ok(0)) => break, // EOF
            Ok(Ok(_)) => {
                // WebSocket frame — читаем и проверяем на downloadProgress
                // Упрощённо: просто ждём и проверяем размер файла
                if dest.exists() {
                    let metadata = std::fs::metadata(dest)
                        .map_err(|e| format!("Ошибка чтения файла: {}", e))?;
                    if metadata.len() > 0 {
                        on_log(format!("   Chrome CDP: файл создан ({} байт)", metadata.len()));
                        got_event = true;
                        // Даём Chrome время завершить запись
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        break;
                    }
                }
            }
            _ => continue,
        }
    }

    if !got_event && dest.exists() {
        let metadata = std::fs::metadata(dest)
            .map_err(|e| format!("Ошибка чтения файла: {}", e))?;
        if metadata.len() > 0 {
            return Ok(metadata.len());
        }
    }

    if dest.exists() {
        let metadata = std::fs::metadata(dest)
            .map_err(|e| format!("Ошибка чтения файла: {}", e))?;
        if metadata.len() > 0 {
            return Ok(metadata.len());
        }
    }

    Err("Chrome CDP: файл не создан или пуст".to_string())
}

async fn send_cdp_message(writer: &mut (impl tokio::io::AsyncWrite + Unpin), msg: &str) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let payload = msg.as_bytes();
    let len = payload.len();

    // WebSocket frame: FIN + TEXT opcode
    let mut frame = vec![0x81u8]; // FIN=1, opcode=TEXT

    // Masking key (обязательно для клиентских фреймов)
    let mask = rand_bytes(4);

    if len < 126 {
        frame.push(0x80 | (len as u8)); // MASK=1, len
    } else if len < 65536 {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }

    frame.extend_from_slice(&mask);

    // Masking payload
    let masked: Vec<u8> = payload.iter().enumerate()
        .map(|(i, &b)| b ^ mask[i % 4])
        .collect();
    frame.extend_from_slice(&masked);

    writer.write_all(&frame).await
        .map_err(|e| format!("CDP send: {}", e))
}

fn find_chrome_exe_for_download() -> Result<std::path::PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .ok_or_else(|| "Не удалось определить папку приложения".to_string())?;

    let bins_dir = exe_dir.join("bins");

    // 1. CloakBrowser (preferred)
    let cloak_dir = bins_dir.join("cloak");
    if let Some(exe) = find_chrome_in_dir(&cloak_dir) {
        return Ok(exe);
    }

    // 2. Chrome-for-Testing
    let chrome_dir = bins_dir.join("chrome");
    if let Some(exe) = find_chrome_in_dir(&chrome_dir) {
        return Ok(exe);
    }

    // 3. Системный Chrome
    let paths = [
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
    ];
    for path in &paths {
        let p = std::path::PathBuf::from(path);
        if p.exists() {
            return Ok(p);
        }
    }

    Err("Chrome не найден. Скачайте CloakBrowser или Chrome-for-Testing.".to_string())
}

fn find_chrome_in_dir(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut stack: Vec<std::path::PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).ok()?;
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase());
                if name == Some("chrome.exe".to_string()) || name == Some("chromium.exe".to_string()) {
                    return Some(p);
                }
            }
        }
    }
    None
}

// ── Минимальные утилиты для WebSocket (без внешних зависимостей) ──

fn rand_bytes(n: usize) -> Vec<u8> {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let s = RandomState::new();
    let mut hasher = s.build_hasher();
    hasher.write_u64(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64);
    let seed = hasher.finish();
    (0..n).map(|i| ((seed >> (i * 8)) & 0xFF) as u8).collect()
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
