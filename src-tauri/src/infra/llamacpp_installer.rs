//! Установка движка llamacpp: скачивание CUDA runtime DLL
//! из официальных релизов ggml-org/llama.cpp (cudart-llama-bin-win-...).
//! Маркер версии — engine_meta.json рядом с DLL.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

/// Имя CUDA-варианта в ассетах llama.cpp. Наш exe слинкован с
/// cublas64_12.dll — качаем всегда CUDA 12.4 (драйвер 13.x совместим).
pub const CUDA_VARIANT: &str = "cuda-12.4";
const ASSET_PREFIX: &str = "cudart-llama-bin-win-cuda-12";
const LLAMA_CPP_API: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest";
const METADATA_FILE: &str = "engine_meta.json";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EngineMeta {
    pub tag: String,
    pub cuda: String,
    pub installed_at: String,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// Папка движка по умолчанию — рядом с exe (в папке установки программы)
pub fn default_dir(exe_dir: &Path) -> PathBuf {
    exe_dir.join("llamacpp")
}

pub fn meta_path(dir: &Path) -> PathBuf {
    dir.join(METADATA_FILE)
}

/// Установлен ли движок: главная CUDA DLL на месте
pub fn is_installed(dir: &Path) -> bool {
    dir.join("cublas64_12.dll").exists()
}

/// Метаданные установленного движка (None если не установлен)
pub fn installed_meta(dir: &Path) -> Option<EngineMeta> {
    if !is_installed(dir) {
        return None;
    }
    let data = fs::read_to_string(meta_path(dir)).ok()?;
    serde_json::from_str(&data).ok()
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("king-orch-app/1.0")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("Ошибка создания HTTP-клиента: {}", e))
}

async fn fetch_latest_release(client: &reqwest::Client) -> Result<GitHubRelease, String> {
    let resp = client
        .get(LLAMA_CPP_API)
        .send()
        .await
        .map_err(|e| format!("Ошибка запроса GitHub API: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("GitHub API вернул HTTP {}", status));
    }
    resp.json::<GitHubRelease>()
        .await
        .map_err(|e| format!("Ошибка парсинга ответа GitHub: {}", e))
}

fn find_cudart_asset(release: &GitHubRelease) -> Option<&GitHubAsset> {
    let exact = format!("{}-x64.zip", ASSET_PREFIX);
    release
        .assets
        .iter()
        .find(|a| a.name == exact || (a.name.starts_with(ASSET_PREFIX) && a.name.ends_with("-x64.zip")))
        .or_else(|| {
            release
                .assets
                .iter()
                .find(|a| a.name.starts_with(ASSET_PREFIX) && a.name.ends_with(".zip"))
        })
}

fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path).map_err(|e| format!("Ошибка чтения файла: {}", e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("Ошибка чтения: {}", e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hex: String = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect();
    Ok(hex)
}

/// Скачивание ассета с докачкой (HTTP Range) и прогрессом
async fn download_asset<L: Fn(String), P: Fn(u64, u64)>(
    client: &reqwest::Client,
    asset: &GitHubAsset,
    dest_zip: &Path,
    on_log: &L,
    on_progress: &P,
) -> Result<(), String> {
    let part_path = dest_zip.with_extension("zip.part");
    let resume_from = fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);

    let mut req = client.get(&asset.browser_download_url);
    if resume_from > 0 {
        req = req.header("Range", format!("bytes={}-", resume_from));
        on_log(format!("Докачка архива с {} МБ...", resume_from / 1024 / 1024));
    } else {
        on_log(format!(
            "📥 Скачивание {} ({} МБ)...",
            asset.name,
            asset.size / 1024 / 1024
        ));
    }

    let resp = req.send().await.map_err(|e| format!("Ошибка загрузки: {}", e))?;
    let status = resp.status();
    let total = asset.size;

    let mut file = if status == reqwest::StatusCode::PARTIAL_CONTENT && resume_from > 0 {
        fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&part_path)
            .map_err(|e| format!("Ошибка создания файла: {}", e))?
    } else {
        if status != reqwest::StatusCode::OK {
            return Err(format!("Сервер вернул HTTP {}", status));
        }
        fs::File::create(&part_path).map_err(|e| format!("Ошибка создания файла: {}", e))?
    };

    let mut downloaded = resume_from;
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Ошибка приёма данных: {}", e))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Ошибка записи на диск: {}", e))?;
        downloaded += chunk.len() as u64;
        if total > 0 {
            on_progress(downloaded, total);
        }
    }
    drop(file);

    if total > 0 && downloaded < total {
        let _ = fs::remove_file(&part_path);
        return Err(format!("Загрузка прервалась: скачано {} из {} байт", downloaded, total));
    }

    fs::rename(&part_path, dest_zip).map_err(|e| format!("Ошибка финализации файла: {}", e))?;
    on_log(format!("✅ Скачано: {} МБ", downloaded / 1024 / 1024));
    Ok(())
}

/// Извлечение всех .dll из архива в dest_dir
fn extract_dlls<L: Fn(String)>(zip_path: &Path, dest_dir: &Path, on_log: &L) -> Result<u32, String> {
    on_log("📦 Распаковка CUDA DLL...".to_string());
    let data = fs::read(zip_path).map_err(|e| format!("Ошибка чтения zip: {}", e))?;
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|e| format!("Ошибка чтения zip: {}", e))?;

    let mut count = 0u32;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Ошибка чтения zip-записи: {}", e))?;
        let name = entry.name().to_string();
        if !name.to_lowercase().ends_with(".dll") {
            continue;
        }
        let file_name = name.rsplit(['/', '\\']).next().unwrap_or(&name);
        let out_path = dest_dir.join(file_name);
        let mut out_file = fs::File::create(&out_path)
            .map_err(|e| format!("Ошибка создания {}: {}", file_name, e))?;
        std::io::copy(&mut entry, &mut out_file)
            .map_err(|e| format!("Ошибка распаковки {}: {}", file_name, e))?;
        count += 1;
    }
    Ok(count)
}

/// Установка (или обновление) движка llamacpp
pub async fn install<L: Fn(String), P: Fn(u64, u64)>(
    dir: &Path,
    on_log: L,
    on_progress: P,
) -> Result<EngineMeta, String> {
    fs::create_dir_all(dir).map_err(|e| format!("Не удалось создать папку {}: {}", dir.display(), e))?;

    on_log("🔄 Получение информации о последнем релизе llama.cpp...".to_string());
    let client = http_client()?;
    let release = fetch_latest_release(&client).await?;

    let asset = find_cudart_asset(&release)
        .ok_or_else(|| format!("В релизе {} не найден ассет CUDA runtime (cudart-llama-bin-win-cuda-12*)", release.tag_name))?;

    let zip_path = dir.join("cudart.zip");
    download_asset(&client, asset, &zip_path, &on_log, &on_progress).await?;

    if let Some(digest) = &asset.digest {
        let expected = digest.strip_prefix("sha256:").unwrap_or(digest);
        let actual = sha256_file(&zip_path)?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = fs::remove_file(&zip_path);
            return Err(format!(
                "Контрольная сумма не совпала! Ожидалось {}, получено {}. Загрузка повреждена.",
                expected, actual
            ));
        }
        on_log("✅ Контрольная сумма SHA-256 подтверждена".to_string());
    }

    // Удаляем старые DLL перед распаковкой (чтобы не осталось хвостов прошлой версии)
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("dll") {
                let _ = fs::remove_file(&path);
            }
        }
    }

    let dll_count = extract_dlls(&zip_path, dir, &on_log)?;
    let _ = fs::remove_file(&zip_path);

    let meta = EngineMeta {
        tag: release.tag_name.clone(),
        cuda: CUDA_VARIANT.to_string(),
        installed_at: chrono_now(),
    };
    let data = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    fs::write(meta_path(dir), data).map_err(|e| format!("Ошибка записи метаданных: {}", e))?;

    on_log(format!(
        "✅ Движок llamacpp установлен: {} ({}). Извлечено {} DLL.",
        release.tag_name, CUDA_VARIANT, dll_count
    ));
    Ok(meta)
}

/// Проверка наличия обновления движка (только проверка, не установка)
pub async fn check_update<L: Fn(String)>(dir: &Path, on_log: L) -> Result<Option<String>, String> {
    let meta = match installed_meta(dir) {
        Some(m) => m,
        None => return Ok(None),
    };
    let client = http_client()?;
    let release = fetch_latest_release(&client).await?;
    if release.tag_name != meta.tag {
        on_log(format!(
            "🔄 Доступно обновление движка llamacpp: {} → {}",
            meta.tag, release.tag_name
        ));
        Ok(Some(release.tag_name))
    } else {
        on_log(format!("Движок llamacpp актуален: {}", meta.tag));
        Ok(None)
    }
}

/// Удаление движка (освобождает ~1 ГБ)
pub fn remove<L: Fn(String)>(dir: &Path, on_log: &L) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|e| format!("Ошибка чтения папки: {}", e))? {
        let entry = entry.map_err(|e| format!("Ошибка чтения папки: {}", e))?;
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(&path).map_err(|e| format!("Ошибка удаления папки: {}", e))?;
        } else {
            fs::remove_file(&path).map_err(|e| format!("Ошибка удаления файла: {}", e))?;
        }
    }
    let _ = fs::remove_dir(dir);
    on_log("🗑️ Движок llamacpp удалён. GPU-ускорение отключено.".to_string());
    Ok(())
}

fn chrono_now() -> String {
    let now = std::time::SystemTime::now();
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}
