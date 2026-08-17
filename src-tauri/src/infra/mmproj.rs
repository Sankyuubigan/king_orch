//! Автодокачка мультимодального проектора (mmproj).
//!
//! Модель, добавленная вручную (кнопка «Добавить модель»), не скачивает mmproj,
//! поэтому кнопка «скрепка» остаётся отключённой. Здесь мы сопоставляем модель с
//! каталогом (`models_catalog.json`) и, если для неё указан `mmproj_url`,
//! докачиваем проектор в папку модели и запоминаем путь в конфиге.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use tauri::AppHandle;
use tokio::sync::Mutex;

use super::config::find_catalog_entry_for_model;
use super::downloader::download_model;
use super::{auto_detect_mmproj, load_catalog, load_config, save_config};

/// Гонки двойного скачивания одного mmproj: фоновая докачка из
/// `updateAttachButtonState` и фолбэк `chat_request` при вложениях могут
/// стартовать одновременно. Сериализуем докачку по пути модели.
static IN_FLIGHT: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

/// Возвращает путь к mmproj для модели, скачивая его при необходимости.
/// Порядок: конфиг → автодетект в папке модели → каталог + докачка.
pub async fn ensure_mmproj_for_model(
    app: &AppHandle,
    model_path: &str,
) -> Result<Option<String>, String> {
    // Быстрый путь: путь уже в конфиге и файл существует.
    let cfg = load_config(app);
    if let Some(p) = cfg.mmproj_files.get(model_path) {
        if Path::new(p).exists() {
            return Ok(Some(p.clone()));
        }
    }

    // Сериализация докачки на одну модель (защита от гонки).
    let map = IN_FLIGHT.get_or_init(|| Mutex::new(HashMap::new()));
    let cell = {
        let mut map = map.lock().await;
        map.entry(model_path.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _permit = cell.lock().await;

    // После ожидания другой воркер мог уже докачать — перепроверяем конфиг.
    let cfg = load_config(app);
    if let Some(p) = cfg.mmproj_files.get(model_path) {
        if Path::new(p).exists() {
            return Ok(Some(p.clone()));
        }
    }

    // Пользователь мог сам положить mmproj рядом с моделью.
    if let Some(p) = auto_detect_mmproj(model_path) {
        let mut cfg = load_config(app);
        cfg.mmproj_files.insert(model_path.to_string(), p.clone());
        save_config(app, &cfg);
        return Ok(Some(p));
    }

    // Сопоставление с каталогом и докачка проектора.
    let catalog = load_catalog(app);
    if let Some(entry) = find_catalog_entry_for_model(&catalog, model_path) {
        if let Some(url) = &entry.mmproj_url {
            let model_dir = Path::new(model_path)
                .parent()
                .ok_or_else(|| "Не удалось определить папку модели".to_string())?;
            let mmp_name = url
                .split('/')
                .last()
                .and_then(|s| s.split('?').next())
                .filter(|s| !s.is_empty())
                .unwrap_or("mmproj.gguf");
            let mmp_path = model_dir.join(mmp_name);
            let mmp_path_str = mmp_path.to_string_lossy().to_string();
            if mmp_path.exists() {
                let mut cfg = load_config(app);
                cfg.mmproj_files.insert(model_path.to_string(), mmp_path_str.clone());
                save_config(app, &cfg);
                return Ok(Some(mmp_path_str));
            }
            download_model(app.clone(), url.clone(), mmp_path_str.clone()).await?;
            let mut cfg = load_config(app);
            cfg.mmproj_files.insert(model_path.to_string(), mmp_path_str.clone());
            save_config(app, &cfg);
            return Ok(Some(mmp_path_str));
        }
    }
    Ok(None)
}