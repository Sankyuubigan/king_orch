use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

const DENO_URL: &str = "https://github.com/denoland/deno/releases/download/v2.9.5/deno-x86_64-pc-windows-msvc.zip";
const YT_DLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
const CHROME_JSON_URL: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";
// CloakBrowser (stealth-Chromium): бесплатный релиз v146 без лицензионного ключа
const CLOAK_RELEASE_VERSION: &str = "chromium-v146.0.7680.177.5";
const CLOAK_ZIP_URL: &str = "https://github.com/CloakHQ/CloakBrowser/releases/download/chromium-v146.0.7680.177.5/cloakbrowser-windows-x64.zip";
const CLOAK_ZIP_ENTRY: &str = "cloakbrowser-windows-x64.zip";
const CLOAK_SUMS_URL: &str = "https://github.com/CloakHQ/CloakBrowser/releases/download/chromium-v146.0.7680.177.5/SHA256SUMS";

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

/// Распаковка ВСЕХ записей zip (для Chrome-for-Testing — многофайловый архив).
fn extract_zip_all(zip_bytes: &[u8], dest_dir: &Path, log_cb: &dyn Fn(String)) -> Result<(), String> {
    log_cb("📦 Распаковка архива...".to_string());
    fs::create_dir_all(dest_dir).map_err(|e| format!("Ошибка создания {}: {}", dest_dir.display(), e))?;

    let reader = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| format!("Ошибка открытия zip: {}", e))?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)
            .map_err(|e| format!("Ошибка чтения zip-записи: {}", e))?;

        let entry_name = entry.name().replace('\\', "/");

        if entry_name.ends_with('/') {
            fs::create_dir_all(dest_dir.join(&entry_name)).ok();
            continue;
        }

        // защита от zip-slip
        if entry_name.starts_with('/') || entry_name.split('/').any(|p| p == "..") {
            return Err(format!("Недопустимый путь в архиве: {}", entry_name));
        }

        let out_path = dest_dir.join(&entry_name);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Ошибка создания {}: {}", parent.display(), e))?;
        }
        let mut out = fs::File::create(&out_path)
            .map_err(|e| format!("Ошибка создания {}: {}", out_path.display(), e))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("Ошибка распаковки {}: {}", entry_name, e))?;
    }

    log_cb(format!("✅ Распаковано {} записей", archive.len()));
    Ok(())
}

fn find_chrome_exe(dir: &Path) -> Option<PathBuf> {
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = fs::read_dir(&d).ok()?;
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().map(|n| n.to_string_lossy().to_lowercase()) == Some("chrome.exe".to_string()) {
                return Some(p);
            }
        }
    }
    None
}

/// Авто-докачка Chrome-for-Testing (стабильный канал, win64) в bins/chrome/.
/// Возвращает путь к chrome.exe.
pub fn ensure_chrome_bin<L: Fn(String)>(bins_dir: &Path, log_cb: &L) -> Result<PathBuf, String> {
    let chrome_dir = bins_dir.join("chrome");
    if let Some(exe) = find_chrome_exe(&chrome_dir) {
        return Ok(exe);
    }

    fs::create_dir_all(&chrome_dir).map_err(|e| format!("Ошибка создания {}: {}", chrome_dir.display(), e))?;
    log_cb("🔄 Первый запуск браузера: скачиваем Chrome-for-Testing (~150 МБ)...".to_string());

    // 1. Актуальная стабильная версия и URL win64-сборки
    let json = reqwest::blocking::get(CHROME_JSON_URL)
        .map_err(|e| format!("Ошибка запроса {}: {}", CHROME_JSON_URL, e))?
        .text()
        .map_err(|e| format!("Ошибка чтения JSON: {}", e))?;

    let root: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| format!("Ошибка парсинга JSON: {}", e))?;

    let stable = &root["channels"]["Stable"];
    let version = stable["version"].as_str().unwrap_or("?").to_string();
    let mut win64_url: Option<String> = None;
    if let Some(downloads) = stable["downloads"]["chrome"].as_array() {
        for d in downloads {
            if d["platform"].as_str() == Some("win64") {
                if let Some(u) = d["url"].as_str() {
                    win64_url = Some(u.to_string());
                }
            }
        }
    }
    let url = win64_url.ok_or_else(|| format!("Нет win64-сборки Chrome for Testing (v{})", version))?;

    // 2. Скачивание (большой файл — длинный таймаут)
    log_cb(format!("📥 Chrome-for-Testing v{} — скачивание...", version));
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(900))
        .build()
        .map_err(|e| format!("Ошибка создания HTTP-клиента: {}", e))?;
    let resp = client.get(&url).send()
        .map_err(|e| format!("Ошибка подключения {}: {}", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {} при скачивании {}", status, url));
    }
    let bytes = resp.bytes().map_err(|e| format!("Ошибка чтения ответа: {}", e))?;
    log_cb(format!("📥 Скачано {} МБ", bytes.len() as f64 / 1024.0 / 1024.0));

    let zip_path = chrome_dir.join("chrome-for-testing.zip");
    fs::write(&zip_path, &bytes).map_err(|e| format!("Ошибка записи {}: {}", zip_path.display(), e))?;

    // 3. Распаковка целиком
    extract_zip_all(&bytes, &chrome_dir, log_cb)?;
    fs::remove_file(&zip_path).ok();

    let exe = find_chrome_exe(&chrome_dir)
        .ok_or_else(|| "chrome.exe не найден после распаковки".to_string())?;
    log_cb(format!("✅ Chrome-for-Testing v{} установлен", version));

    Ok(exe)
}

fn find_cloak_exe(dir: &Path) -> Option<PathBuf> {
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = fs::read_dir(&d).ok()?;
        for entry in entries.flatten() {
            let p = entry.path();
            let lower = p.file_name().map(|n| n.to_string_lossy().to_lowercase());
            if p.is_dir() {
                stack.push(p);
            } else if lower == Some("chrome.exe".to_string()) || lower == Some("chromium.exe".to_string()) {
                return Some(p);
            }
        }
    }
    None
}

/// Авто-докачка CloakBrowser (stealth-Chromium, бесплатная v146-сборка без ключа)
/// в bins/cloak/. Проверяет SHA-256 по манифесту релиза. Возвращает путь к exe.
pub fn ensure_cloak_browser<L: Fn(String)>(bins_dir: &Path, log_cb: &L) -> Result<PathBuf, String> {
    let cloak_dir = bins_dir.join("cloak");
    if let Some(exe) = find_cloak_exe(&cloak_dir) {
        return Ok(exe);
    }

    fs::create_dir_all(&cloak_dir).map_err(|e| format!("Ошибка создания {}: {}", cloak_dir.display(), e))?;
    log_cb("🕵️ Первый запуск: скачиваем CloakBrowser (stealth-Chromium, ~536 МБ)...".to_string());

    // 1. Ожидаемый SHA-256 из манифеста релиза
    let sums_text = reqwest::blocking::get(CLOAK_SUMS_URL)
        .map_err(|e| format!("Ошибка запроса {}: {}", CLOAK_SUMS_URL, e))?
        .text()
        .map_err(|e| format!("Ошибка чтения SHA256SUMS: {}", e))?;

    let expected_sha: Option<String> = sums_text.lines()
        .find(|l| l.trim().ends_with(CLOAK_ZIP_ENTRY))
        .and_then(|l| l.split_whitespace().next())
        .map(|s| s.to_lowercase());
    if expected_sha.is_none() {
        return Err(format!("{} не найден в SHA256SUMS релиза {}", CLOAK_ZIP_ENTRY, CLOAK_RELEASE_VERSION));
    }

    // 2. Скачивание (большой файл — длинный таймаут)
    log_cb(format!("📥 CloakBrowser {} — скачивание...", CLOAK_RELEASE_VERSION));
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(1800))
        .build()
        .map_err(|e| format!("Ошибка создания HTTP-клиента: {}", e))?;
    let resp = client.get(CLOAK_ZIP_URL).send()
        .map_err(|e| format!("Ошибка подключения {}: {}", CLOAK_ZIP_URL, e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {} при скачивании {}", status, CLOAK_ZIP_URL));
    }
    let bytes = resp.bytes().map_err(|e| format!("Ошибка чтения ответа: {}", e))?;
    log_cb(format!("📥 Скачано {} МБ", bytes.len() as f64 / 1024.0 / 1024.0));

    // 3. Проверка SHA-256 (защита от битой/подменённой сборки)
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    let actual_sha = format!("{:x}", hasher.finalize());
    if Some(actual_sha.clone()) != expected_sha {
        return Err(format!(
            "SHA-256 не совпал: ожидалось {}, получено {}. Повторите позже.",
            expected_sha.unwrap_or_default(), actual_sha
        ));
    }
    log_cb("🔐 SHA-256 совпал с манифестом релиза".to_string());

    // 4. Распаковка
    let zip_path = cloak_dir.join("cloakbrowser.zip");
    fs::write(&zip_path, &bytes).map_err(|e| format!("Ошибка записи {}: {}", zip_path.display(), e))?;
    extract_zip_all(&bytes, &cloak_dir, log_cb)?;
    fs::remove_file(&zip_path).ok();

    let exe = find_cloak_exe(&cloak_dir)
        .ok_or_else(|| "Бинарь CloakBrowser не найден после распаковки".to_string())?;
    log_cb(format!("✅ CloakBrowser {} установлен", CLOAK_RELEASE_VERSION));

    Ok(exe)
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
