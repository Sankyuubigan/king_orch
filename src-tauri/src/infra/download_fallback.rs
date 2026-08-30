//! Цепочка фоллбэков скачивания файлов.
//!
//! Порядок (каждый следующий — если предыдущий не сработал):
//!   1. reqwest (с прокси из env, streaming + resume)
//!   2. PowerShell Invoke-WebRequest (.NET WebClient, системный прокси, Schannel)
//!   3. MCP-инструмент download_file (Deno, использует PowerShell/WinINet)
//!
//! Логируется какой метод сработал.

use std::path::Path;

/// Скачивание файла с цепочкой фоллбэков.
///
/// `on_log` — для логирования какая ступень сработала.
/// `on_progress` — прогресс-бар (только для reqwest, для PowerShell/MCP нет потокового прогресса).
pub async fn download_with_fallback<L: Fn(String) + Sync, P: Fn(u64, u64) + Sync>(
    url: &str,
    dest: &Path,
    expected_size: Option<u64>,
    on_log: &L,
    on_progress: &P,
) -> Result<(), String> {
    // ── Уровень 1: reqwest (streaming + resume) ──
    on_log("🔄 [1/3] Скачивание через HTTP (reqwest)...".to_string());
    match download_via_reqwest_streaming(url, dest, expected_size, on_progress).await {
        Ok(bytes) => {
            on_log(format!("✅ [1/3] reqwest: скачано {} МБ", bytes / 1024 / 1024));
            return Ok(());
        }
        Err(e) => {
            on_log(format!("⚠️ [1/3] reqwest не смог: {}", e));
        }
    }

    // ── Уровень 2: PowerShell ──
    on_log("🔄 [2/3] Скачивание через PowerShell (.NET)...".to_string());
    match download_via_powershell(url, dest, on_log).await {
        Ok(bytes) => {
            on_log(format!("✅ [2/3] PowerShell: скачано {} МБ", bytes / 1024 / 1024));
            return Ok(());
        }
        Err(e) => {
            on_log(format!("⚠️ [2/3] PowerShell не смог: {}", e));
        }
    }

    // ── Уровень 3: MCP (Deno + PowerShell/WinINet) ──
    on_log("🔄 [3/3] Скачивание через MCP (Deno)...".to_string());
    match download_via_mcp(url, dest, on_log).await {
        Ok(bytes) => {
            on_log(format!("✅ [3/3] MCP: скачано {} МБ", bytes / 1024 / 1024));
            return Ok(());
        }
        Err(e) => {
            on_log(format!("⚠️ [3/3] MCP не смог: {}", e));
        }
    }

    Err("Все 3 метода скачивания не сработали. Проверьте подключение к интернету и настройки прокси.".to_string())
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
// Уровень 3: MCP-инструмент (Deno downloader.ts)
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
