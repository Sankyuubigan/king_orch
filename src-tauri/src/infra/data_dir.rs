//! Общее хранилище данных программы (`KingOrchData`).
//!
//! Одна папка на всё: движки (llamacpp, 9router, sdcpp), бандлы и новые
//! модели. Юзер выбирает путь один раз в настройках, плагины подхватывают
//! через фоллбэк в своих `*_dir()` (см. шаг 4 плана).
//!
//! Приоритет пути:
//! 1. Явный `data_dir` из конфига (юзер нажал «Изменить путь»).
//! 2. Автоматом: диск с макс. свободным местом + `\KingOrchData`.
//! 3. Фоллбэк: `<exe>/KingOrchData` (если диски не найдены).

use std::path::{Path, PathBuf};
use tauri::AppHandle;

use crate::infra::{self};

/// Имя папки хранилища. Всегда такое, независимо от того, что выбрал юзер.
pub const DATA_DIR_NAME: &str = "KingOrchData";

/// Нормализует путь от юзера: тримит слэши, проверяет что последний
/// компонент — `KingOrchData` (без учёта регистра), иначе приклеивает.
/// Чистая функция, без обращений к файловой системе.
pub fn normalize(raw: &str) -> PathBuf {
    let trimmed = raw.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return PathBuf::from(DATA_DIR_NAME);
    }
    let p = Path::new(trimmed);
    let needs_suffix = p
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| !n.eq_ignore_ascii_case(DATA_DIR_NAME))
        .unwrap_or(true);
    if needs_suffix {
        p.join(DATA_DIR_NAME)
    } else {
        p.to_path_buf()
    }
}

/// Диск с максимальным свободным местом (логика как в `get_auto_download_info`
/// плагина llama-engine). Возвращает букву диска или None, если дисков нет.
pub fn best_drive_letter() -> Option<String> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut best: Option<(String, u64)> = None;
    for disk in &disks {
        let mount = disk.mount_point().to_string_lossy().to_string();
        if mount.len() >= 2 && mount.as_bytes()[1] == b':' {
            let letter = mount[..1].to_uppercase();
            let available = disk.available_space();
            match &best {
                Some((_, best_avail)) if available <= *best_avail => {}
                _ => best = Some((letter, available)),
            }
        }
    }
    best.map(|(letter, _)| letter)
}

/// Дефолтный путь хранилища: `<best_drive>:\KingOrchData`, фоллбэк —
/// `<exe>/KingOrchData`. Используется только когда `data_dir` пуст.
pub fn best_drive_data_dir() -> PathBuf {
    match best_drive_letter() {
        Some(letter) => PathBuf::from(format!("{}:\\{}", letter, DATA_DIR_NAME)),
        None => {
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."));
            exe_dir.join(DATA_DIR_NAME)
        }
    }
}

/// Читает `data_dir` из конфига (сырое значение, без нормализации).
pub fn configured_data_dir(app: &AppHandle) -> Option<String> {
    let cfg = infra::load_config(app);
    cfg.data_dir.filter(|s| !s.trim().is_empty())
}

/// Реальный путь хранилища: явный из конфига (нормализованный) или
/// автоматический (best-drive). Без создания папки — только вычисление.
pub fn resolve(app: &AppHandle) -> PathBuf {
    match configured_data_dir(app) {
        Some(raw) => normalize(&raw),
        None => best_drive_data_dir(),
    }
}

/// Создаёт папку хранилища, если её нет. Ошибка возвращается наружу.
#[allow(dead_code)] // инфра-хелпер хранилища — на вырост
pub fn ensure_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = resolve(app);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Не удалось создать папку хранилища {}: {}", dir.display(), e))?;
    Ok(dir)
}

/// Применяет новый путь хранилища: нормализует, создаёт папку, пишет
/// `data_dir` и все производные подпапки в конфиг одной записью.
/// Возвращает нормализованный путь.
pub fn apply(app: &AppHandle, raw_path: &str) -> Result<PathBuf, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err("Путь не может быть пустым".to_string());
    }
    let dir = normalize(trimmed);
    if !dir.is_absolute() {
        return Err(format!("Путь должен быть абсолютным: {}", dir.display()));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Не удалось создать папку {}: {}", dir.display(), e))?;

    let mut cfg = infra::load_config(app);
    cfg.data_dir = Some(dir.to_string_lossy().to_string());
    // llamacpp_dir и models_dir — движковые ключи: пишутся в общий конфиг
    // плагина (SSOT), из хостового файла они вырезаются при сохранении.
    infra::set_engine_dirs(
        app,
        Some(dir.join("llamacpp").to_string_lossy().to_string()),
        Some(dir.join("models").to_string_lossy().to_string()),
    )?;
    cfg.sdcpp_dir = Some(dir.join("sdcpp").to_string_lossy().to_string());
    // Папки бандлов image-engine — КОРНИ пулов (25.10.2026: per-bundle
    // подпапки порождали перекачку общих файлов, плагин держит один пул).
    cfg.image_bundle_dir = Some(dir.join("image_models").to_string_lossy().to_string());
    cfg.video_bundle_dir = Some(dir.join("video_models").to_string_lossy().to_string());
    // `cloud_routers_dir` здесь больше не пишется: путь из per-app конфига
    // хоста разводил бы папку шлюза по приложениям — процесс-то один, и второй
    // проект его не видел бы. Вместо этого хост сообщает плагину МАШИННЫЙ
    // корень установки (`set_default_install_root`, вызов в main.rs setup,
    // 19.11.2026): плагин хранит его в своём общем конфиге first-wins, и
    // дефолт установки ложится в хранилище, оставаясь одним из любого проекта.
    // Ручной «Путь» в панели (`cloud_routers.<id>.dir`) сильнее умолчания.
    infra::save_config(app, &cfg)?;
    Ok(dir)
}

/// Материализует дефолты путей бандлов медиа-движков в app_config.json
/// (PRP 26.11.20, Шаг 3): если ключи `image_bundle_dir`/`video_bundle_dir`
/// отсутствуют, записывает вычисленный дефолт (`resolve()/image_models`,
/// `resolve()/video_models`) явным ключом. Одна запись при изменениях,
/// идемпотентно.
///
/// Зачем: без записи эффективная папка вычисляется при каждом чтении
/// (ключ `data_dir` → диск с макс. свободным местом → exe-фолбэк). На чистой
/// установке вставка флешки/смена дисков тихо меняла путь — скачанные веса
/// «пропадали» и перекачивались. Отсутствующее состояние = записанный путь.
///
/// `sdcpp_dir` здесь НЕ трогается: его плагиновый дефолт — `<exe>/sdcpp`,
/// другой паттерн, материализуется отдельно при необходимости.
pub fn materialize_media_defaults(app: &AppHandle) {
    let mut cfg = infra::load_config(app);
    let mut changed = false;
    if cfg.image_bundle_dir.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true) {
        let path = resolve(app).join("image_models");
        log::info!("[data_dir] материализован дефолт image_bundle_dir: {}", path.display());
        cfg.image_bundle_dir = Some(path.to_string_lossy().to_string());
        changed = true;
    }
    if cfg.video_bundle_dir.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true) {
        let path = resolve(app).join("video_models");
        log::info!("[data_dir] материализован дефолт video_bundle_dir: {}", path.display());
        cfg.video_bundle_dir = Some(path.to_string_lossy().to_string());
        changed = true;
    }
    if changed {
        if let Err(e) = infra::save_config(app, &cfg) {
            log::error!("[data_dir] не удалось сохранить материализованные дефолты бандлов: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_appends_suffix() {
        assert_eq!(normalize("D:\\foo"), PathBuf::from("D:\\foo\\KingOrchData"));
        assert_eq!(normalize("D:\\foo\\"), PathBuf::from("D:\\foo\\KingOrchData"));
        assert_eq!(normalize("D:\\foo\\bar"), PathBuf::from("D:\\foo\\bar\\KingOrchData"));
    }

    #[test]
    fn test_normalize_keeps_existing_suffix() {
        assert_eq!(normalize("D:\\foo\\KingOrchData"), PathBuf::from("D:\\foo\\KingOrchData"));
        assert_eq!(normalize("D:\\foo\\KingOrchData\\"), PathBuf::from("D:\\foo\\KingOrchData"));
        assert_eq!(normalize("D:\\foo\\kingorchdata"), PathBuf::from("D:\\foo\\kingorchdata"));
    }

    #[test]
    fn test_normalize_does_not_partial_match() {
        assert_eq!(normalize("D:\\MyKingOrchData"), PathBuf::from("D:\\MyKingOrchData\\KingOrchData"));
    }

    #[test]
    fn test_normalize_empty() {
        assert_eq!(normalize(""), PathBuf::from("KingOrchData"));
        assert_eq!(normalize("   "), PathBuf::from("KingOrchData"));
    }
}
