use tauri::AppHandle;

use crate::infra;

#[tauri::command]
pub fn get_config(app: AppHandle) -> infra::AppConfig {
    let mut cfg = infra::load_config(&app);
    backfill_model_meta(&app, &mut cfg);
    cfg
}

/// Дозаполняет `model_meta` для уже установленных моделей по сопоставлению
/// имени файла с каталогом (чтобы иконки возможностей отображались без
/// повторного скачивания). Vision также выводится из наличия mmproj.
fn backfill_model_meta(app: &AppHandle, cfg: &mut infra::AppConfig) {
    let catalog = infra::load_catalog(app);
    let mut changed = false;
    for model_path in &cfg.models {
        if cfg.model_meta.contains_key(model_path) {
            continue;
        }
        let mut meta = infra::ModelMeta::default();
        if let Some(entry) = infra::find_catalog_entry_for_model(&catalog, model_path) {
            meta.uncen = entry.uncen.unwrap_or(false);
            meta.vision = entry.vision.unwrap_or(false);
            meta.audio = entry.audio.unwrap_or(false);
        }
        if cfg.mmproj_files.contains_key(model_path) {
            meta.vision = true;
        }
        if meta.uncen || meta.vision || meta.audio {
            cfg.model_meta.insert(model_path.clone(), meta);
            changed = true;
        }
    }
    if changed {
        infra::save_config(app, cfg);
    }
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
        "last_agent" => {
            if let Some(v) = value.as_str() {
                cfg.last_agent = Some(v.to_string());
            }
        }
        "allow_error_reports" => {
            if let Some(v) = value.as_bool() {
                cfg.allow_error_reports = v;
                // Мгновенный переключатель облачной отправки в плагине логов
                // (panic-хук и события больше не отправляются сразу же).
                tauri_plugin_logs::set_reporting_enabled(v);
            }
        }
        "chat_font_scale" => {
            if let Some(v) = value.as_f64() {
                cfg.chat_font_scale = v as f32;
            }
        }
        "translator_model" => {
            if let Some(v) = value.as_str() {
                cfg.translator_model = Some(v.to_string());
            }
        }
        "translator_lang" => {
            if let Some(v) = value.as_str() {
                cfg.translator_lang = v.to_string();
            }
        }
        "workdir" => {
            if value.is_null() {
                cfg.workdir = None;
            } else if let Some(v) = value.as_str() {
                cfg.workdir = Some(v.to_string());
            }
        }
        _ => {}
    }
    infra::save_config(&app, &cfg);
}

#[tauri::command]
pub fn set_last_model(app: AppHandle, path: String) {
    let mut cfg = infra::load_config(&app);
    // 🛡 Проектор mmproj нельзя выбирать активной моделью (см. is_mmproj_file).
    // Пропускаем игнор в лог, конфиг не трогаем.
    if infra::is_mmproj_file(&path) {
        log::warn!("set_last_model: отклонён выбор mmproj «{}»", path);
        return;
    }
    cfg.last_model = Some(path);
    infra::save_config(&app, &cfg);
}

#[tauri::command]
pub fn set_tabs(app: AppHandle, tabs: Vec<infra::TabState>, active_tab: Option<String>) {
    let mut cfg = infra::load_config(&app);
    cfg.tabs = tabs;
    cfg.active_tab = active_tab;
    infra::save_config(&app, &cfg);
}

#[tauri::command]
pub fn set_theme(app: AppHandle, theme: String) {
    let mut cfg = infra::load_config(&app);
    cfg.theme = theme;
    infra::save_config(&app, &cfg);
}