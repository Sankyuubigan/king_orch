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
        "context_size" => {
            if let Some(v) = value.as_u64() {
                cfg.context_size = v as u32;
            }
        }
        "max_gen_tokens" => {
            if let Some(v) = value.as_u64() {
                cfg.max_gen_tokens = v as u32;
            }
        }
        "kv_quant_keys" => {
            if let Some(v) = value.as_bool() {
                cfg.kv_quant_keys = v;
            }
        }
        "kv_quant_values" => {
            if let Some(v) = value.as_bool() {
                cfg.kv_quant_values = v;
            }
        }
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
        "show_folder_agents" => {
            if let Some(v) = value.as_bool() {
                cfg.show_folder_agents = v;
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