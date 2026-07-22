use futures_util::StreamExt;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use tauri::{AppHandle, Emitter};

#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    downloaded: u64,
    total: u64,
}

const MIN_GGUF_SIZE: u64 = 1024 * 1024;

#[tauri::command]
pub async fn download_model(app: AppHandle, url: String, save_path: String) -> Result<(), String> {
    eprintln!("[download] Старт загрузки: {} -> {}", url, save_path);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::default())
        .timeout(std::time::Duration::from_secs(60 * 60))
        .build()
        .map_err(|e| format!("Ошибка создания HTTP-клиента: {}", e))?;

    let res = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Accept", "*/*")
        .send()
        .await
        .map_err(|e| format!("Ошибка подключения: {}", e))?;

    eprintln!("[download] Финальный URL после редиректов: {}", res.url());
    let status = res.status();
    if !status.is_success() {
        let body = res
            .text()
            .await
            .unwrap_or_default();
        let preview: String = body.chars().take(500).collect();
        eprintln!("[download] Ошибка HTTP {} при загрузке {}: {}", status, url, preview);
        let _ = std::fs::remove_file(&save_path);
        return Err(format!("Ошибка загрузки (HTTP {}): {}", status, preview));
    }

    let total_size = res.content_length().unwrap_or(0);
    eprintln!("[download] HTTP OK, размер: {} байт", total_size);

    let mut file = File::create(&save_path).map_err(|e| format!("Ошибка создания файла: {}", e))?;
    let mut stream = res.bytes_stream();
    let mut downloaded: u64 = 0;

    let mut last_emit = std::time::Instant::now();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_file(&save_path);
                eprintln!("[download] Ошибка потока: {}", e);
                return Err(format!("Ошибка загрузки: {}", e));
            }
        };
        if let Err(e) = file.write_all(&chunk) {
            let _ = std::fs::remove_file(&save_path);
            eprintln!("[download] Ошибка записи на диск: {}", e);
            return Err(format!("Ошибка записи на диск: {}", e));
        }
        downloaded += chunk.len() as u64;

        if last_emit.elapsed().as_millis() > 100 {
            let _ = app.emit("download_progress", DownloadProgress {
                downloaded,
                total: total_size,
            });
            last_emit = std::time::Instant::now();
        }
    }

    let _ = app.emit("download_progress", DownloadProgress {
        downloaded,
        total: total_size,
    });

    if total_size > 0 && downloaded < total_size {
        let _ = std::fs::remove_file(&save_path);
        eprintln!("[download] Недокачано: {} из {} байт", downloaded, total_size);
        return Err(format!("Загрузка прервалась: скачано {} из {} байт", downloaded, total_size));
    }

    if downloaded < MIN_GGUF_SIZE {
        let _ = std::fs::remove_file(&save_path);
        eprintln!("[download] Подозрительно маленький файл: {} байт", downloaded);
        return Err(format!("Скачанный файл подозрительно мал ({} байт) — возможно, это не GGUF-модель", downloaded));
    }

    let mut head = [0u8; 4];
    if File::open(&save_path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_err()
        || &head != b"GGUF"
    {
        let _ = std::fs::remove_file(&save_path);
        eprintln!("[download] Файл не является GGUF (magic: {:?})", head);
        return Err("Скачанный файл не является GGUF-моделью".to_string());
    }

    eprintln!("[download] Готово: {} байт", downloaded);
    Ok(())
}

#[tauri::command]
pub async fn download_binary(app: AppHandle, url: String, save_path: String, extract_zip: bool) -> Result<(), String> {
    eprintln!("[download_binary] Старт: {} -> {}", url, save_path);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::default())
        .timeout(std::time::Duration::from_secs(60 * 60))
        .build()
        .map_err(|e| format!("Ошибка создания HTTP-клиента: {}", e))?;

    let res = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await
        .map_err(|e| format!("Ошибка подключения: {}", e))?;

    let status = res.status();
    if !status.is_success() {
        return Err(format!("HTTP {} при скачивании {}", status, url));
    }

    let total_size = res.content_length().unwrap_or(0);
    let bytes = res.bytes().await.map_err(|e| format!("Ошибка чтения: {}", e))?;

    let _ = app.emit("download_progress", DownloadProgress {
        downloaded: bytes.len() as u64,
        total: total_size,
    });

    if extract_zip {
        let reader = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(reader)
            .map_err(|e| format!("Ошибка открытия zip: {}", e))?;

        let save_dir = std::path::Path::new(&save_path).parent().unwrap_or(std::path::Path::new("."));
        fs::create_dir_all(save_dir).map_err(|e| format!("Ошибка создания директории: {}", e))?;

        let target_name = std::path::Path::new(&save_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("binary.exe");

        let mut extracted = false;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| format!("Ошибка zip: {}", e))?;
            let entry_name = entry.name().to_string();
            if entry_name.ends_with(target_name) || entry_name.ends_with(".exe") {
                let mut out = fs::File::create(&save_path)
                    .map_err(|e| format!("Ошибка создания {}: {}", save_path, e))?;
                std::io::copy(&mut entry, &mut out).map_err(|e| format!("Ошибка распаковки: {}", e))?;
                extracted = true;
                break;
            }
        }
        if !extracted {
            return Err(format!("{} не найден внутри zip", target_name));
        }
    } else {
        fs::write(&save_path, &bytes).map_err(|e| format!("Ошибка записи: {}", e))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(file) = fs::File::open(&save_path) {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o755)).ok();
        }
    }

    eprintln!("[download_binary] Готово: {} байт", bytes.len());
    Ok(())
}
