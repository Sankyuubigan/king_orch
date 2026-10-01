use tauri::AppHandle;
use std::time::Instant;

use crate::infra;
use crate::infra::data_dir;

#[tauri::command]
pub async fn get_config(app: AppHandle) -> Result<infra::AppConfig, String> {
    let started = Instant::now();
    let config = tokio::task::spawn_blocking(move || infra::load_config(&app))
        .await
        .map_err(|error| error.to_string())?;
    log::info!(
        "[BOOT] get_config {}ms, models={}",
        started.elapsed().as_millis(),
        config.models.len()
    );
    Ok(config)
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
        "agent_visibility" => {
            // Порядок элементов = порядок в выпадающем списке чата, поэтому
            // массив сохраняем как есть, без сортировки и без дедупликации
            // (последнее сделает фронт — там единственная копия состояния).
            if let Some(v) = value.as_array() {
                cfg.agent_visibility = v
                    .iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect();
            } else {
                log::warn!(
                    "set_config_value: agent_visibility ожидает массив строк, получен {} — значение проигнорировано",
                    value
                );
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
        other => {
            // Молчаливый no-op на неизвестном ключе — ложь (§2.2 core/rules):
            // опечатка в интерфейсе выглядела бы как «сохранилось».
            log::warn!("set_config_value: неизвестный ключ конфига «{}» — значение проигнорировано", other);
        }
    }
    if let Err(e) = infra::save_config(&app, &cfg) {
        log::error!("set_config_value: ошибка сохранения конфига: {}", e);
    }
}

/// Текущий путь общего хранилища (`KingOrchData`). Если в конфиге пусто —
/// вычисляется автоматом (диск с макс. свободным местом). Папка не создаётся.
#[tauri::command]
pub fn get_data_dir(app: AppHandle) -> String {
    data_dir::resolve(&app).to_string_lossy().to_string()
}

/// Устанавливает путь общего хранилища: нормализует (приклеивает
/// `KingOrchData` если в конце его нет), создаёт папку, пишет `data_dir`
/// и все производные подпапки в конфиг одной записью.
#[tauri::command]
pub fn set_data_dir(app: AppHandle, path: String) -> Result<String, String> {
    data_dir::apply(&app, &path).map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub fn reset_max_gen_tokens(app: AppHandle) -> u32 {
    let mut cfg = infra::load_config(&app);
    cfg.max_gen_tokens = infra::AppConfig::default().max_gen_tokens;
    if let Err(e) = infra::save_config(&app, &cfg) {
        log::error!("reset_max_gen_tokens: ошибка сохранения конфига: {}", e);
    }
    cfg.max_gen_tokens
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
    if let Err(e) = infra::save_config(&app, &cfg) {
        log::error!("set_last_model: ошибка сохранения конфига: {}", e);
    }
}

#[tauri::command]
pub fn set_tabs(app: AppHandle, tabs: Vec<infra::TabState>, active_tab: Option<String>) {
    let mut cfg = infra::load_config(&app);
    cfg.tabs = tabs;
    cfg.active_tab = active_tab;
    if let Err(e) = infra::save_config(&app, &cfg) {
        log::error!("set_tabs: ошибка сохранения конфига: {}", e);
    }
}

#[tauri::command]
pub fn set_theme(app: AppHandle, theme: String) {
    let mut cfg = infra::load_config(&app);
    cfg.theme = theme;
    if let Err(e) = infra::save_config(&app, &cfg) {
        log::error!("set_theme: ошибка сохранения конфига: {}", e);
    }
}