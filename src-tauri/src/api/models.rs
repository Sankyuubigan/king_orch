use serde::Serialize;
use tauri::AppHandle;

use crate::infra;

#[derive(Serialize)]
pub struct AutoDownloadInfo {
    pub model_name: String,
    pub model_url: String,
    pub size_gb: Option<String>,
    pub save_path: String,
    pub free_space_gb: u64,
    pub drive_letter: String,
}

#[tauri::command]
pub fn get_auto_download_info(app: AppHandle) -> Result<AutoDownloadInfo, String> {
    let catalog = infra::load_catalog(&app);
    let default_entry = catalog
        .iter()
        .find(|e| e.is_default)
        .ok_or_else(|| "В каталоге не найдена модель по умолчанию".to_string())?;

    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut best_drive: Option<(String, u64)> = None;

    for disk in &disks {
        let mount = disk.mount_point().to_string_lossy().to_string();
        let available = disk.available_space();
        if mount.len() >= 2 && mount.as_bytes()[1] == b':' {
            let drive_letter = mount[..1].to_uppercase();
            if let Some((_, best_avail)) = best_drive {
                if available > best_avail {
                    best_drive = Some((drive_letter, available));
                }
            } else {
                best_drive = Some((drive_letter, available));
            }
        }
    }

    let (drive_letter, free_space) =
        best_drive.ok_or_else(|| "Не найден ни один диск для сохранения модели".to_string())?;

    let save_dir = format!("{}:\\llm_local_ai_models", drive_letter);
    let filename = default_entry
        .download_url
        .split('/')
        .last()
        .and_then(|s| s.split('?').next())
        .unwrap_or(&format!("{}.gguf", default_entry.name))
        .to_string();
    let save_path = format!("{}\\{}", save_dir, filename);

    Ok(AutoDownloadInfo {
        model_name: default_entry.name.clone(),
        model_url: default_entry.download_url.clone(),
        size_gb: default_entry.size_gb.clone(),
        save_path,
        free_space_gb: free_space / (1024 * 1024 * 1024),
        drive_letter,
    })
}

#[tauri::command]
pub async fn auto_download_default_model(app: AppHandle, save_path: String) -> Result<(), String> {
    let catalog = infra::load_catalog(&app);
    let default_entry = catalog
        .iter()
        .find(|e| e.is_default)
        .ok_or_else(|| "В каталоге не найдена модель по умолчанию".to_string())?;

    let parent_dir = std::path::Path::new(&save_path)
        .parent()
        .ok_or_else(|| "Неверный путь сохранения".to_string())?;
    std::fs::create_dir_all(parent_dir)
        .map_err(|e| format!("Не удалось создать директорию: {}", e))?;

    crate::infra::downloader::download_model(
        app.clone(),
        default_entry.download_url.clone(),
        save_path.clone(),
    )
    .await?;

    // Докачиваем multimodal-проектор (mmproj), если модель его поддерживает
    let mut mmproj_saved: Option<String> = None;
    if let Some(mmp_url) = &default_entry.mmproj_url {
        let mmp_name = mmp_url
            .split('/')
            .last()
            .and_then(|s| s.split('?').next())
            .unwrap_or("mmproj.gguf")
            .to_string();
        let mmp_path = format!(
            "{}{}{}",
            parent_dir.display(),
            std::path::MAIN_SEPARATOR,
            mmp_name
        );
        crate::infra::downloader::download_model(
            app.clone(),
            mmp_url.clone(),
            mmp_path.clone(),
        )
        .await?;
        mmproj_saved = Some(mmp_path);
    }

    let models_dir = parent_dir.to_str().map(|d| d.to_string());

    let mut cfg = infra::load_config(&app);
    if !cfg.models.contains(&save_path) {
        cfg.models.push(save_path.clone());
    }
    cfg.last_model = Some(save_path.clone());
    cfg.models_dir = models_dir;
    if let Some(mp) = mmproj_saved {
        cfg.mmproj_files.insert(save_path.clone(), mp);
    }
    cfg.model_meta.insert(
        save_path.clone(),
        infra::ModelMeta {
            uncen: default_entry.uncen.unwrap_or(false),
            vision: default_entry.vision.unwrap_or(false),
            audio: default_entry.audio.unwrap_or(false),
        },
    );
    infra::save_config(&app, &cfg);

    Ok(())
}

#[tauri::command]
pub fn get_models_catalog(app: AppHandle) -> Vec<infra::CatalogEntry> {
    infra::load_catalog(&app)
}

#[tauri::command]
pub fn get_model_params(app: AppHandle, model_path: String) -> infra::ModelParams {
    let mut cfg = infra::load_config(&app);
    // Если пользователь уже сохранял параметры для этой модели - отдаем их
    if let Some(params) = cfg.model_params.get(&model_path) {
        return params.clone();
    }

    let mut params = infra::ModelParams::default();

    // УМНОЕ ЧТЕНИЕ (Ground Truth): Перезаписываем настройки тем, что ВШИТО в сам файл .gguf.
    if let Some(temp) = infra::extract_f32_from_gguf(&model_path, "tokenizer.ggml.temp") {
        params.temperature = temp;
    }
    if let Some(top_k) = infra::extract_u32_from_gguf(&model_path, "tokenizer.ggml.top_k") {
        params.top_k = top_k;
    }
    if let Some(top_p) = infra::extract_f32_from_gguf(&model_path, "tokenizer.ggml.top_p") {
        params.top_p = top_p;
    }
    if let Some(min_p) = infra::extract_f32_from_gguf(&model_path, "tokenizer.ggml.min_p") {
        params.min_p = min_p;
    }
    if let Some(rep_pen) = infra::extract_f32_from_gguf(&model_path, "tokenizer.ggml.repetition_penalty") {
        params.repetition_penalty = rep_pen;
    }

    cfg.model_params.insert(model_path.clone(), params.clone());
    infra::save_config(&app, &cfg);
    
    params
}

#[tauri::command]
pub fn set_model_params(app: AppHandle, model_path: String, params: infra::ModelParams) {
    let mut cfg = infra::load_config(&app);
    cfg.model_params.insert(model_path, params);
    infra::save_config(&app, &cfg);
}

#[tauri::command]
pub fn reset_model_params(app: AppHandle, model_path: String) -> infra::ModelParams {
    let mut cfg = infra::load_config(&app);
    cfg.model_params.remove(&model_path);
    infra::save_config(&app, &cfg);
    get_model_params(app, model_path) // Пересчитает параметры из GGUF заново
}

#[tauri::command]
pub fn add_model(
    app: AppHandle,
    path: String,
    flags: Option<infra::ModelMeta>,
) -> Result<infra::AppConfig, String> {
    let meta = std::fs::metadata(&path)
        .map_err(|e| format!("Файл модели не найден: {}", e))?;
    if meta.len() < 1024 * 1024 {
        return Err(format!("Файл слишком маленький ({} байт) — это не GGUF-модель", meta.len()));
    }

    let mut cfg = infra::load_config(&app);
    if !cfg.models.contains(&path) {
        cfg.models.push(path.clone());
    }
    cfg.last_model = Some(path.clone());

    if let Some(mmp) = infra::auto_detect_mmproj(&path) {
        cfg.mmproj_files.insert(path.clone(), mmp);
    }

    if let Some(f) = flags {
        cfg.model_meta.insert(path.clone(), f);
    }

    infra::save_config(&app, &cfg);
    Ok(cfg)
}

#[tauri::command]
pub fn remove_model(app: AppHandle, path: String) -> Result<infra::AppConfig, String> {
    let mut cfg = infra::load_config(&app);
    cfg.models.retain(|m| m != &path);
    if cfg.last_model.as_deref() == Some(path.as_str()) {
        cfg.last_model = None;
    }
    cfg.model_params.remove(&path);
    cfg.mmproj_files.remove(&path);
    infra::save_config(&app, &cfg);
    Ok(cfg)
}

#[tauri::command]
pub fn delete_model_file(app: AppHandle, path: String) -> Result<infra::AppConfig, String> {
    match std::fs::remove_file(&path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("Не удалось удалить файл «{}»: {}", path, e)),
    }
    let mut cfg = infra::load_config(&app);
    cfg.models.retain(|m| m != &path);
    if cfg.last_model.as_deref() == Some(path.as_str()) {
        cfg.last_model = None;
    }
    cfg.model_params.remove(&path);
    cfg.mmproj_files.remove(&path);
    cfg.model_meta.remove(&path);
    infra::save_config(&app, &cfg);
    Ok(cfg)
}

#[tauri::command]
pub fn get_mmproj_path(app: AppHandle, model_path: String) -> Option<String> {
    let cfg = infra::load_config(&app);
    if let Some(path) = cfg.mmproj_files.get(&model_path) {
        if std::path::Path::new(path).exists() {
            return Some(path.clone());
        }
    }
    if let Some(mmp) = infra::auto_detect_mmproj(&model_path) {
        let mut cfg = infra::load_config(&app);
        cfg.mmproj_files.insert(model_path.clone(), mmp.clone());
        infra::save_config(&app, &cfg);
        return Some(mmp);
    }
    None
}

/// Возвращает актуальные возможности модели (vision/audio), читая их
/// НАПРЯМУЮ из `models_catalog.json` (живьём), а не из закэшированного
/// `model_meta`. Нужно для гейтинга вложений: аудио разрешаем только
/// моделям, которые реально его поддерживают в llama.cpp (Ultravox /
/// Qwen2.5-Omni), иначе отправка аудио ведёт к зависанию генерации
/// (см. баг с Gemma-4, у которой в каталоге ошибочно стоял `audio: true`).
#[derive(Serialize)]
pub struct ModelCapabilities {
    pub vision: bool,
    pub audio: bool,
    pub uncen: bool,
}

#[tauri::command]
pub fn get_model_capabilities(app: AppHandle, model_path: String) -> ModelCapabilities {
    let catalog = infra::load_catalog(&app);
    let cfg = infra::load_config(&app);
    let mut caps = ModelCapabilities { vision: false, audio: false, uncen: false };
    if let Some(entry) = infra::find_catalog_entry_for_model(&catalog, &model_path) {
        caps.vision = entry.vision.unwrap_or(false);
        caps.audio = entry.audio.unwrap_or(false);
        caps.uncen = entry.uncen.unwrap_or(false);
    }
    // Наличие mmproj даёт показ изображений даже без явного флага в каталоге.
    if cfg.mmproj_files.contains_key(&model_path) {
        caps.vision = true;
    }
    caps
}

/// Возвращает актуальные возможности (vision/audio/uncen) для ВСЕХ
/// установленных моделей, читая их напрямую из `models_catalog.json` и
/// наличия mmproj. Единый источник правды для отображения иконок
/// возможностей — чтобы исключить показ устаревшего закэшированного
/// `model_meta` (см. баг с нотой 🎵 у Gemma после правки каталога).
#[tauri::command]
pub fn get_all_capabilities(app: AppHandle) -> std::collections::HashMap<String, ModelCapabilities> {
    use std::collections::HashMap;
    let catalog = infra::load_catalog(&app);
    let cfg = infra::load_config(&app);
    let mut map: HashMap<String, ModelCapabilities> = HashMap::new();
    for model_path in &cfg.models {
        let mut caps = ModelCapabilities { vision: false, audio: false, uncen: false };
        if let Some(entry) = infra::find_catalog_entry_for_model(&catalog, model_path) {
            caps.vision = entry.vision.unwrap_or(false);
            caps.audio = entry.audio.unwrap_or(false);
            caps.uncen = entry.uncen.unwrap_or(false);
        }
        if cfg.mmproj_files.contains_key(model_path) {
            caps.vision = true;
        }
        map.insert(model_path.clone(), caps);
    }
    map
}

/// Возвращает путь к mmproj для модели, докачивая его по каталогу
/// (`models_catalog.json`), если файл не найден. Нужно для кнопки «скрепка»
/// у моделей, добавленных вручную (без скачивания через кнопку).
#[tauri::command]
pub async fn ensure_mmproj(app: AppHandle, model_path: String) -> Result<Option<String>, String> {
    infra::ensure_mmproj_for_model(&app, &model_path).await
}