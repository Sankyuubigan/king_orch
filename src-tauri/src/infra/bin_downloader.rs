use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

const NODE_URL: &str = "https://nodejs.org/dist/v22.14.0/win-x64/node.exe";
const DENO_URL: &str = "https://github.com/denoland/deno/releases/download/v2.2.8/deno-x86_64-pc-windows-msvc.zip";
const YT_DLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";

pub fn get_bins_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("bins")
}

fn bin_filename(name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{}.exe", name)
    } else {
        name.to_string()
    }
}

fn bin_url(name: &str) -> Result<&'static str, String> {
    match name {
        "node" => Ok(NODE_URL),
        "deno" => Ok(DENO_URL),
        "yt-dlp" => Ok(YT_DLP_URL),
        _ => Err(format!("Неизвестный бинарник: {}", name)),
    }
}

fn is_zip(name: &str) -> bool {
    name == "deno"
}

fn download_file_sync(url: &str, dest: &Path, log_cb: &dyn Fn(String)) -> Result<(), String> {
    log_cb(format!("📥 Скачивание {}...", url));

    let resp = reqwest::blocking::get(url)
        .map_err(|e| format!("Ошибка подключения {}: {}", url, e))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {} при скачивании {}", status, url));
    }

    let total = resp.content_length().unwrap_or(0);
    let bytes = resp.bytes().map_err(|e| format!("Ошибка чтения ответа: {}", e))?;

    log_cb(format!("📥 Скачано {} МБ", bytes.len() as f64 / 1024.0 / 1024.0));

    fs::write(dest, &bytes).map_err(|e| format!("Ошибка записи {}: {}", dest.display(), e))?;

    if total > 0 && (bytes.len() as u64) < total {
        return Err(format!("Недокачано: {} из {} байт", bytes.len(), total));
    }

    Ok(())
}

fn extract_zip_entry(zip_bytes: &[u8], bins_dir: &Path, target_exe: &str, log_cb: &dyn Fn(String)) -> Result<(), String> {
    log_cb("📦 Распаковка zip...".to_string());

    let reader = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| format!("Ошибка открытия zip: {}", e))?;

    let dest = bins_dir.join(target_exe);

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)
            .map_err(|e| format!("Ошибка чтения zip-записи: {}", e))?;

        let entry_name = entry.name().to_string();
        let entry_lower = entry_name.to_lowercase();
        let target_lower = target_exe.to_lowercase();

        if entry_lower == target_lower || entry_lower.ends_with(&target_lower) {
            let mut out = fs::File::create(&dest)
                .map_err(|e| format!("Ошибка создания {}: {}", dest.display(), e))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Ошибка распаковки: {}", e))?;
            log_cb(format!("✅ Распакован {}", entry_name));
            return Ok(());
        }
    }

    Err(format!("{} не найден внутри zip-архива", target_exe))
}

pub fn ensure_runtime_bin(name: &str, bins_dir: &Path, log_cb: impl Fn(String)) -> Result<PathBuf, String> {
    let bins_dir = bins_dir.to_path_buf();
    let bin_name = bin_filename(name);
    let bin_path = bins_dir.join(&bin_name);

    if bin_path.exists() {
        return Ok(bin_path);
    }

    fs::create_dir_all(&bins_dir)
        .map_err(|e| format!("Ошибка создания {}: {}", bins_dir.display(), e))?;

    let url = bin_url(name)?;

    log_cb(format!("🔄 Первый запуск: скачиваем {}... Это займёт около минуты", name));

    if is_zip(name) {
        let resp = reqwest::blocking::get(url)
            .map_err(|e| format!("Ошибка подключения {}: {}", url, e))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(format!("HTTP {} при скачивании {}", status, url));
        }

        let bytes = resp.bytes().map_err(|e| format!("Ошибка чтения ответа: {}", e))?;
        log_cb(format!("📥 Скачано {} МБ, распаковка...", bytes.len() as f64 / 1024.0 / 1024.0));

        extract_zip_entry(&bytes, &bins_dir, &bin_name, &log_cb)?;
    } else {
        download_file_sync(url, &bin_path, &log_cb)?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755)).ok();
    }

    log_cb(format!("✅ {} установлен", name));

    Ok(bin_path)
}
