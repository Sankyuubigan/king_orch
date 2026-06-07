use tauri::AppHandle;

use crate::infra;

#[tauri::command]
pub fn get_config(app: AppHandle) -> infra::AppConfig {
    infra::load_config(&app)
}

#[tauri::command]
pub fn set_config_value(app: AppHandle, key: String, value: serde_json::Value) {
    let mut cfg = infra::load_config(&app);
    match key.as_str() {
        "confidence_threshold" => {
            if let Some(v) = value.as_f64() {
                cfg.confidence_threshold = v as f32;
            }
        }
        "show_advanced_features" => {
            if let Some(v) = value.as_bool() {
                cfg.show_advanced_features = v;
            }
        }
        _ => {}
    }
    infra::save_config(&app, &cfg);
}

#[tauri::command]
pub fn set_last_model(app: AppHandle, path: String) {
    let mut cfg = infra::load_config(&app);
    cfg.last_model = Some(path);
    infra::save_config(&app, &cfg);
}

#[tauri::command]
pub fn set_theme(app: AppHandle, theme: String) {
    let mut cfg = infra::load_config(&app);
    cfg.theme = theme;
    infra::save_config(&app, &cfg);
}

#[tauri::command]
pub fn set_prompt_format(app: AppHandle, format: String) {
    let mut cfg = infra::load_config(&app);
    cfg.prompt_format = format;
    infra::save_config(&app, &cfg);
}