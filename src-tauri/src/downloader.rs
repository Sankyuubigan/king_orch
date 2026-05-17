use futures_util::StreamExt;
use std::fs::File;
use std::io::Write;
use tauri::{AppHandle, Emitter};

#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    downloaded: u64,
    total: u64,
}

#[tauri::command]
pub async fn download_model(app: AppHandle, url: String, save_path: String) -> Result<(), String> {
    let res = reqwest::get(&url)
        .await
        .map_err(|e| format!("Ошибка подключения: {}", e))?;
        
    let total_size = res.content_length().unwrap_or(0);
    
    let mut file = File::create(&save_path).map_err(|e| format!("Ошибка создания файла: {}", e))?;
    let mut stream = res.bytes_stream();
    let mut downloaded = 0;

    let mut last_emit = std::time::Instant::now();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Ошибка загрузки: {}", e))?;
        file.write_all(&chunk).map_err(|e| format!("Ошибка записи на диск: {}", e))?;
        downloaded += chunk.len() as u64;
        
        // Отправляем прогресс не чаще 10 раз в секунду, чтобы не перегружать UI
        if last_emit.elapsed().as_millis() > 100 {
            let _ = app.emit("download_progress", DownloadProgress {
                downloaded,
                total: total_size,
            });
            last_emit = std::time::Instant::now();
        }
    }
    
    // Финальный эмит
    let _ = app.emit("download_progress", DownloadProgress {
        downloaded,
        total: total_size,
    });

    Ok(())
}