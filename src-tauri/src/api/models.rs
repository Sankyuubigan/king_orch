use tauri::AppHandle;

use crate::infra;

#[tauri::command]
pub fn get_models_catalog(app: AppHandle) -> Vec<infra::CatalogEntry> {
    infra::load_catalog(&app)
}

#[tauri::command]
pub fn get_model_params(app: AppHandle, model_path: String) -> infra::ModelParams {
    let mut cfg = infra::load_config(&app);
    if let Some(params) = cfg.model_params.get(&model_path) {
        return params.clone();
    }
    let catalog = infra::load_catalog(&app);
    let file_name = std::path::Path::new(&model_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let mut params = infra::ModelParams::default();
    let mut found = false;
    for entry in catalog {
        if file_name.contains(&entry.name) || entry.download_url.contains(&file_name.to_string()) {
            params = entry.default_params.clone();
            found = true;
            break;
        }
    }
    if !found {
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
    get_model_params(app, model_path)
}

#[tauri::command]
pub fn add_model(app: AppHandle, path: String) -> infra::AppConfig {
    let mut cfg = infra::load_config(&app);
    if !cfg.models.contains(&path) {
        cfg.models.push(path.clone());
    }
    cfg.last_model = Some(path);
    infra::save_config(&app, &cfg);
    cfg
}