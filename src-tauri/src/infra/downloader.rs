use futures_util::StreamExt;
use std::cmp::min;
use std::fs::{self, File};
use std::io::{Cursor, Read, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::task::JoinSet;

#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    downloaded: u64,
    total: u64,
    /// Скорость скачивания, байт/сек (для плавного GUI). 0 = нет данных.
    speed_bps: f64,
}

const MIN_GGUF_SIZE: u64 = 1024 * 1024;
/// Как часто шлём события в GUI (между эмитами минимум 200 мс — бар ползёт плавно).
const GUI_EMIT_INTERVAL: Duration = Duration::from_millis(200);

// ── Параметры надёжной параллельной докачки ──
/// Число параллельных соединений (каждое получает свою полосу CDN).
const CHUNK_COUNT: usize = 8;
/// Макс. повторов на чанк при ошибке/обрыве/stall.
const MAX_CHUNK_RETRIES: usize = 10;
/// Базовая задержка retry (растёт экспоненциально).
const RETRY_BASE_MS: u64 = 500;
/// Если от сервера нет ни байта дольше этого — считаем соединение «зависшим»
/// и докачиваем остаток чанка заново (без перезапуска всей загрузки).
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
const PART_SUFFIX: &str = ".part";

/// Зеркала HuggingFace. Если основной источник упорно падает (заблокирован в
/// регионе, throttling), подменяем хост и качаем с зеркала.
const HF_MIRRORS: &[(&str, &str)] = &[
    ("hf.co", "hf-mirror.com"),
    ("huggingface.co", "hf-mirror.com"),
];

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

fn mirror_url(url: &str) -> Option<String> {
    for (from, to) in HF_MIRRORS {
        let prefix = format!("https://{}", from);
        if let Some(stripped) = url.strip_prefix(&prefix) {
            return Some(format!("https://{}{}", to, stripped));
        }
    }
    None
}

/// Резолвим финальный CDN-URL, точный размер файла и поддержку HTTP Range.
/// GET с `Range: bytes=0-0` заставляет сервер вернуть 206 + `Content-Range`
/// со ВСЕМ размером — за один запрос получаем и URL, и размер.
async fn resolve_target(client: &reqwest::Client, url: &str) -> Result<(String, u64, bool), String> {
    let resp = client
        .get(url)
        .header("User-Agent", UA)
        .header("Accept", "*/*")
        .header("Range", "bytes=0-0")
        .send()
        .await
        .map_err(|e| format!("Ошибка подключения: {}", e))?;

    let status = resp.status();
    if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
        let body = resp.text().await.unwrap_or_default();
        let preview: String = body.chars().take(500).collect();
        return Err(format!("Ошибка загрузки (HTTP {}): {}", status, preview));
    }

    let final_url = resp.url().to_string();
    let supports_range = status == reqwest::StatusCode::PARTIAL_CONTENT;

    // Тело не читаем (для 206 это 1 байт, для 200 — весь файл); просто берём заголовки.
    let total = if supports_range {
        if let Some(cr) = resp.headers().get(reqwest::header::CONTENT_RANGE) {
            if let Ok(s) = cr.to_str() {
                if let Some(total_str) = s.rsplit('/').next() {
                    total_str.trim().parse::<u64>().unwrap_or(0)
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            resp.content_length().unwrap_or(0)
        }
    } else {
        resp.content_length().unwrap_or(0)
    };

    Ok((final_url, total, supports_range))
}

#[tauri::command]
pub async fn download_model(app: AppHandle, url: String, save_path: String) -> Result<(), String> {
    eprintln!("[download] Старт загрузки: {} -> {}", url, save_path);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::default())
        .timeout(Duration::from_secs(60 * 60))
        .build()
        .map_err(|e| format!("Ошибка создания HTTP-клиента: {}", e))?;

    match run_download(&app, &client, &url, &save_path).await {
        Ok(()) => Ok(()),
        Err(e) => {
            if let Some(murl) = mirror_url(&url) {
                eprintln!(
                    "[download] Основной источник не удался ({}). Пробуем зеркало: {}",
                    e, murl
                );
                let _ = fs::remove_file(format!("{}{}", save_path, PART_SUFFIX));
                run_download(&app, &client, &murl, &save_path).await
            } else {
                Err(e)
            }
        }
    }
}

/// Один полный проход загрузки: резолв → параллельные чанки (или однопоток).
async fn run_download(
    app: &AppHandle,
    client: &reqwest::Client,
    url: &str,
    save_path: &str,
) -> Result<(), String> {
    let part_path = format!("{}{}", save_path, PART_SUFFIX);

    // Уже скачано и валидно?
    if let Ok(meta) = fs::metadata(save_path) {
        if meta.len() >= MIN_GGUF_SIZE {
            let mut head = [0u8; 4];
            if File::open(save_path)
                .and_then(|mut f| f.read_exact(&mut head))
                .is_ok()
                && &head == b"GGUF"
            {
                eprintln!("[download] Файл уже скачан и валиден: {}", save_path);
                return Ok(());
            }
            let _ = fs::remove_file(save_path);
        }
    }

    let (final_url, total, supports_range) = resolve_target(client, url).await?;

    if total == 0 || !supports_range {
        // Размер неизвестен или сервер не умеет Range — однопоточный fallback.
        return download_single(app, client, url, &part_path, save_path, total).await;
    }

    // Подготавливаем .part (sparse-файл нужного размера, чтобы писать в любой offset).
    {
        let f = File::create(&part_path).map_err(|e| format!("Ошибка создания файла: {}", e))?;
        f.set_len(total)
            .map_err(|e| format!("Ошибка выделения места на диске: {}", e))?;
    }

    let total = Arc::new(total);
    let final_url = Arc::new(tokio::sync::Mutex::new(final_url));
    let downloaded = Arc::new(AtomicU64::new(0));
    let abort = Arc::new(AtomicBool::new(false));
    let client = Arc::new(client.clone());
    let orig_url = url.to_string();

    // Таск-репортер прогресса (совместим с chat.ts:815 — тот же payload).
    let rep_app = app.clone();
    let rep_total = total.clone();
    let rep_dl = downloaded.clone();
    let reporter = tokio::spawn(async move {
        let mut last = Instant::now();
        let mut last_bytes = rep_dl.load(Ordering::SeqCst);
        loop {
            tokio::time::sleep(GUI_EMIT_INTERVAL).await;
            let now = Instant::now();
            let cur = rep_dl.load(Ordering::SeqCst);
            let dt = now.duration_since(last).as_secs_f64();
            let speed = if dt > 0.0 { (cur - last_bytes) as f64 / dt } else { 0.0 };
            let _ = rep_app.emit(
                "download_progress",
                DownloadProgress {
                    downloaded: cur,
                    total: *rep_total,
                    speed_bps: speed,
                },
            );
            last = now;
            last_bytes = cur;
            if cur >= *rep_total {
                break;
            }
        }
    });

    let chunk_size = (*total + CHUNK_COUNT as u64 - 1) / CHUNK_COUNT as u64;

    let mut set: JoinSet<Result<(), String>> = JoinSet::new();
    for i in 0..CHUNK_COUNT {
        let client = client.clone();
        let final_url = final_url.clone();
        let total = total.clone();
        let downloaded = downloaded.clone();
        let abort = abort.clone();
        let part_path = part_path.clone();
        let orig_url = orig_url.clone();

        set.spawn(async move {
            if abort.load(Ordering::SeqCst) {
                return Err(format!("чанк {} отменён", i));
            }

            let start0 = (i as u64) * chunk_size;
            let end = min(start0 + chunk_size, *total); // невключительно
            let mut written: u64 = 0; // сколько байт чанка уже записано
            let mut attempt: usize = 0;

            loop {
                if abort.load(Ordering::SeqCst) {
                    return Err(format!("чанк {} отменён", i));
                }
                let range_start = start0 + written;
                if range_start >= end {
                    return Ok(());
                }

                let url_now = final_url.lock().await.clone();
                let resp = match client
                    .get(&url_now)
                    .header("User-Agent", UA)
                    .header("Accept", "*/*")
                    .header("Range", format!("bytes={}-{}", range_start, end - 1))
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        attempt += 1;
                        if attempt > MAX_CHUNK_RETRIES {
                            return Err(format!("чанк {}: сетевая ошибка: {}", i, e));
                        }
                        sleep_backoff(attempt).await;
                        continue;
                    }
                };

                let status = resp.status();
                if status == reqwest::StatusCode::FORBIDDEN
                    || status == reqwest::StatusCode::UNAUTHORIZED
                {
                    // Возможно, протух временный токен CDN — перерезолвим ссылку.
                    attempt += 1;
                    if attempt > MAX_CHUNK_RETRIES {
                        return Err(format!("чанк {}: HTTP {} (доступ запрещён)", i, status));
                    }
                    if let Ok((nu, _, _)) = resolve_target(&client, &orig_url).await {
                        *final_url.lock().await = nu;
                    }
                    sleep_backoff(attempt).await;
                    continue;
                }
                if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
                    attempt += 1;
                    if attempt > MAX_CHUNK_RETRIES {
                        return Err(format!("чанк {}: HTTP {}", i, status));
                    }
                    sleep_backoff(attempt).await;
                    continue;
                }

                // Поток с детектом зависания: нет байт > STALL_TIMEOUT — обрываем и докачиваем остаток.
                let mut stream = resp.bytes_stream();
                let mut file = match tokio::fs::OpenOptions::new()
                    .write(true)
                    .open(&part_path)
                    .await
                {
                    Ok(f) => f,
                    Err(e) => return Err(format!("чанк {}: ошибка файла: {}", i, e)),
                };

                let mut stalled = false;
                while let Some(item) =
                    tokio::time::timeout(STALL_TIMEOUT, stream.next()).await.unwrap_or(None)
                {
                    match item {
                        Ok(bytes) => {
                            if bytes.is_empty() {
                                continue;
                            }
                            if let Err(e) = file
                                .seek(SeekFrom::Start(range_start + written))
                                .await
                            {
                                return Err(format!("чанк {}: seek: {}", i, e));
                            }
                            if let Err(e) = file.write_all(&bytes).await {
                                return Err(format!("чанк {}: запись: {}", i, e));
                            }
                            written += bytes.len() as u64;
                            downloaded.fetch_add(bytes.len() as u64, Ordering::SeqCst);
                        }
                        Err(_) => {
                            stalled = true;
                            break;
                        }
                    }
                }

                if stalled {
                    attempt += 1;
                    if attempt > MAX_CHUNK_RETRIES {
                        return Err(format!("чанк {}: неоднократный обрыв потока", i));
                    }
                    sleep_backoff(attempt).await;
                    continue;
                }

                // Поток корректно завершился — проверяем, весь ли чанк получен.
                if written >= (end - start0) {
                    return Ok(());
                }
                // Сервер закрыл соединение раньше конца чанка — докачиваем остаток.
                attempt += 1;
                if attempt > MAX_CHUNK_RETRIES {
                    return Err(format!("чанк {}: не докачан до конца", i));
                }
                sleep_backoff(attempt).await;
            }
        });
    }

    let mut fatal: Option<String> = None;
    while let Some(join) = set.join_next().await {
        match join {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                abort.store(true, Ordering::SeqCst);
                if fatal.is_none() {
                    fatal = Some(e);
                }
            }
            Err(e) => {
                abort.store(true, Ordering::SeqCst);
                if fatal.is_none() {
                    fatal = Some(format!("паника таска: {}", e));
                }
            }
        }
    }

    if let Some(e) = fatal {
        let _ = fs::remove_file(&part_path);
        return Err(e);
    }

    // Все чанки готовы — финализация + валидация GGUF.
    let _ = app.emit(
        "download_progress",
        DownloadProgress {
            downloaded: *total,
            total: *total,
            speed_bps: 0.0,
        },
    );
    reporter.abort();

    finalize(part_path, save_path, *total)
}

/// Валидация и атомарный rename .part -> финальный путь.
fn finalize(part_path: String, save_path: &str, total: u64) -> Result<(), String> {
    let meta = fs::metadata(&part_path)
        .map_err(|e| format!("Не удалось прочитать скачанный файл: {}", e))?;
    if total > 0 && meta.len() != total {
        let _ = fs::remove_file(&part_path);
        return Err(format!(
            "Размер не совпадает: скачано {} из {} байт",
            meta.len(),
            total
        ));
    }
    if meta.len() < MIN_GGUF_SIZE {
        let _ = fs::remove_file(&part_path);
        return Err(format!(
            "Скачанный файл подозрительно мал ({} байт)",
            meta.len()
        ));
    }
    let mut head = [0u8; 4];
    if File::open(&part_path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_err()
        || &head != b"GGUF"
    {
        let _ = fs::remove_file(&part_path);
        return Err("Скачанный файл не является GGUF-моделью".to_string());
    }
    fs::rename(&part_path, save_path)
        .map_err(|e| format!("Ошибка переименования файла: {}", e))?;
    eprintln!("[download] Готово: {} байт -> {}", meta.len(), save_path);
    Ok(())
}

async fn sleep_backoff(attempt: usize) {
    let ms = RETRY_BASE_MS.saturating_mul(1u64 << (attempt.saturating_sub(1)).min(6));
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// Однопоточный fallback (нет размера / нет поддержки Range): стрим с
/// детектом зависания и повторами (перезапуск с начала).
async fn download_single(
    app: &AppHandle,
    client: &reqwest::Client,
    url: &str,
    part_path: &str,
    save_path: &str,
    known_total: u64,
) -> Result<(), String> {
    eprintln!("[download] Режим однопоточной загрузки (нет Range)");
    for attempt in 1..=MAX_CHUNK_RETRIES {
        if attempt > 1 {
            sleep_backoff(attempt).await;
            eprintln!("[download] Повтор загрузки (#{})", attempt);
        }

        let resp = match client
            .get(url)
            .header("User-Agent", UA)
            .header("Accept", "*/*")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if attempt == MAX_CHUNK_RETRIES {
                    return Err(format!("Ошибка подключения: {}", e));
                }
                continue;
            }
        };
        if !resp.status().is_success() {
            if attempt == MAX_CHUNK_RETRIES {
                return Err(format!("HTTP {}", resp.status()));
            }
            continue;
        }

        let total = resp.content_length().unwrap_or(known_total);
        let mut file = match tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(part_path)
            .await
        {
            Ok(f) => f,
            Err(e) => return Err(format!("Ошибка создания файла: {}", e)),
        };

        let mut stream = resp.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut last = Instant::now();
        let mut last_bytes: u64 = 0;

        let mut ok = true;
        while let Some(item) = tokio::time::timeout(STALL_TIMEOUT, stream.next()).await.unwrap_or(None) {
            match item {
                Ok(bytes) => {
                    if bytes.is_empty() {
                        continue;
                    }
                    if let Err(e) = file.write_all(&bytes).await {
                        return Err(format!("Ошибка записи на диск: {}", e));
                    }
                    downloaded += bytes.len() as u64;
                    let now = Instant::now();
                    let dt = now.duration_since(last).as_secs_f64();
                    if dt >= 0.2 {
                        let speed = (downloaded - last_bytes) as f64 / dt;
                        let _ = app.emit(
                            "download_progress",
                            DownloadProgress {
                                downloaded,
                                total,
                                speed_bps: speed,
                            },
                        );
                        last = now;
                        last_bytes = downloaded;
                    }
                }
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }

        if ok && (total == 0 || downloaded >= total) {
            let _ = app.emit(
                "download_progress",
                DownloadProgress {
                    downloaded,
                    total,
                    speed_bps: 0.0,
                },
            );
            return finalize(part_path.to_string(), save_path, total);
        }
        // иначе — следующая итерация перезапустит с начала
    }
    Err("Не удалось докачать файл (однопоточный режим)".to_string())
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
        speed_bps: 0.0,
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
