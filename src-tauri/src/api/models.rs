use tauri::AppHandle;

use crate::infra;

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

    let catalog = infra::load_catalog(&app);
    let file_name = std::path::Path::new(&model_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    
    let mut params = infra::ModelParams::default();

    // 1. Берем базовые параметры из каталога
    for entry in catalog {
        if file_name.contains(&entry.name) || entry.download_url.contains(&file_name.to_string()) {
            params = entry.default_params.clone();
            break;
        }
    }

    // 2. УМНОЕ ЧТЕНИЕ (Ground Truth): Перезаписываем настройки тем, что ВШИТО в сам файл .gguf.
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
pub fn add_model(app: AppHandle, path: String) -> Result<infra::AppConfig, String> {
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
pub fn get_mmproj_path(app: AppHandle, model_path: String) -> Option<String> {
    let cfg = infra::load_config(&app);
    if let Some(path) = cfg.mmproj_files.get(&model_path) {
        return Some(path.clone());
    }
    if let Some(mmp) = infra::auto_detect_mmproj(&model_path) {
        let mut cfg = infra::load_config(&app);
        cfg.mmproj_files.insert(model_path.clone(), mmp.clone());
        infra::save_config(&app, &cfg);
        return Some(mmp);
    }
    None
}