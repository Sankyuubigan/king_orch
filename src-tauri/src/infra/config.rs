//! Полный конфиг приложения (`app_config.json`) — хостовый `AppConfig`.
//!
//! ## Разделение с движковым плагином
//! Плагин `tauri-plugin-llama-engine` владеет движковыми ключами (models,
//! last_model, models_dir, model_params, mmproj_files, model_meta,
//! llamacpp_dir, engine_variant, каталог). Хосты остаются с полным `AppConfig`
//! (theme, context_size, allow_error_reports, translator_*, …). Оба пишут в
//! ОДИН файл field-preserving merge, поэтому ключи друг друга не затираются
//! (типы модели/параметров пере-экспортируются отсюда из плагина — единая
//! сериализация). Всё движковое (каталог, mmproj-хелперы, сэмплинг-параметры)
//! живёт в плагине и доступно хостом через фасад `crate::infra::*`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};

/// Единые движковые типы — из плагина (SSOT), чтоб хостовый `AppConfig`
/// сериализовался в тот же JSON, что и `EngineConfig` плагина.
pub use tauri_plugin_llama_engine::engine::config::{ModelMeta, ModelParams};

/// Одна вкладка рабочей области (браузерный UI). Сериализуется в `app_config.json`
/// как часть `AppConfig.tabs`. Типы: "main" (главная/новая вкладка), "chat"
/// (сессия чата — 1:1 с файлом сессии), "section" (Статика: история сессий,
/// студия агентов, настройки, логи), "webview" (встроенная страница, напр. 9Router).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TabState {
    pub id: String,
    #[serde(rename = "type")]
    pub tab_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AppConfig {
    /// Открытые вкладки рабочей области (браузерный UI). Пустые — на старте
    /// фронтенд создаёт одну главную вкладку.
    #[serde(default)]
    pub tabs: Vec<TabState>,
    /// id активной вкладки (если сохранённая вкладка была закрыта — фронтенд
    /// выбирает первую из списка).
    #[serde(default)]
    pub active_tab: Option<String>,
    pub models: Vec<String>,
    pub last_model: Option<String>,
    #[serde(default)]
    pub last_agent: Option<String>,
    #[serde(default)]
    pub models_dir: Option<String>,
    #[serde(default)]
    pub model_params: HashMap<String, ModelParams>,
    #[serde(default = "default_context_size")]
    pub context_size: u32,
    #[serde(default = "default_max_gen_tokens")]
    pub max_gen_tokens: u32,
    /// Бюджет думателя (reasoning) модели в токенах: 0 — думатель запрещён,
    /// N>0 — лимит размышлений до ответа (передаётся в llama-server
    /// --reasoning-budget). Размышления не попадают в content и в историю.
    #[serde(default = "default_reasoning_budget")]
    pub reasoning_budget: u32,
    /// Устарело (UI убран): всегда false. Старые ключи в app_config.json
    /// игнорируются при загрузке (см. load_config). BeeLlama включает KVarN
    /// через source.runtime в engine_sources.json.
    #[serde(default = "default_kv_quant_keys")]
    pub kv_quant_keys: bool,
    #[serde(default = "default_kv_quant_values")]
    pub kv_quant_values: bool,
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Устарело (UI убран): всегда "Auto". Старые значения ChatML/Gemma/…
    /// принудительно сбрасываются в load_config.
    #[serde(default = "default_prompt_format")]
    pub prompt_format: String,
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f32,
    #[serde(default = "default_show_advanced_features")]
    pub show_advanced_features: bool,
    #[serde(default = "default_show_folder_agents")]
    pub show_folder_agents: bool,
    #[serde(default)]
    pub mmproj_files: HashMap<String, String>,
    #[serde(default)]
    pub model_meta: HashMap<String, ModelMeta>,
    #[serde(default)]
    pub llamacpp_dir: Option<String>,
    /// Источник бинарей движка: "ggml-org" (дефолт) / "beellama" (KVarN).
    /// None = дефолт (ggml-org). См. engine_sources.json плагина.
    #[serde(default)]
    pub engine_source: Option<String>,
    /// Предпочтение юзера: какой бекенд движка использовать ("auto" / "cpu" /
    /// "cuda-12.4" / "cuda-13.3" / "vulkan" / "hip-radeon"). None = авто.
    #[serde(default)]
    pub engine_variant: Option<String>,
    #[serde(default = "default_allow_error_reports")]
    pub allow_error_reports: bool,
    /// Масштаб шрифта чата (1.0 = 100%, текущий размер по умолчанию).
    #[serde(default = "default_chat_font_scale")]
    pub chat_font_scale: f32,
    /// Путь к .gguf модели, выступающей в роли переводчика сообщений.
    #[serde(default)]
    pub translator_model: Option<String>,
    /// Целевой язык перевода: "ru" — русский, "en" — английский.
    #[serde(default = "default_translator_lang")]
    pub translator_lang: String,
    /// Рабочая директория для кодера (bash tool current_dir).
    #[serde(default)]
    pub workdir: Option<String>,
    /// Двухфазный режим генерации по умолчанию (для ВСЕХ агентов, а не только
    /// signal-агентов с YAML-флагом two_phase_thinking): Phase 1 — свободные
    /// размышления без грамматики, Phase 2 — ответ с enable_thinking=false
    /// (думатель выключен на уровне запроса). Чинит «пустой думатель» моделей
    /// вроде Gemma-4 (мысли жгут --reasoning-budget, ответ пуст).
    #[serde(default = "default_two_phase_default")]
    pub two_phase_default: bool,
}

fn default_translator_lang() -> String {
    "ru".to_string()
}

fn default_context_size() -> u32 { 24576 }
fn default_max_gen_tokens() -> u32 { 4096 }
fn default_reasoning_budget() -> u32 { 1500 }
fn default_kv_quant_keys() -> bool { false }
fn default_kv_quant_values() -> bool { false }
fn default_theme() -> String { "dark".to_string() }
fn default_prompt_format() -> String { "Auto".to_string() }
fn default_confidence_threshold() -> f32 { 0.8 }
fn default_show_advanced_features() -> bool { false }
fn default_show_folder_agents() -> bool { false }
fn default_allow_error_reports() -> bool { true }
fn default_chat_font_scale() -> f32 { 1.0 }
fn default_two_phase_default() -> bool { true }

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: None,
            models: Vec::new(),
            last_model: None,
            last_agent: None,
            models_dir: None,
            model_params: HashMap::new(),
            context_size: default_context_size(),
            max_gen_tokens: default_max_gen_tokens(),
            reasoning_budget: default_reasoning_budget(),
            kv_quant_keys: default_kv_quant_keys(),
            kv_quant_values: default_kv_quant_values(),
            theme: default_theme(),
            prompt_format: default_prompt_format(),
            confidence_threshold: default_confidence_threshold(),
            show_advanced_features: default_show_advanced_features(),
            show_folder_agents: default_show_folder_agents(),
            mmproj_files: HashMap::new(),
            model_meta: HashMap::new(),
            llamacpp_dir: None,
            engine_source: None,
            engine_variant: None,
            allow_error_reports: default_allow_error_reports(),
            chat_font_scale: default_chat_font_scale(),
            translator_model: None,
            translator_lang: default_translator_lang(),
            workdir: None,
            two_phase_default: default_two_phase_default(),
        }
    }
}

pub fn get_config_path(app: &AppHandle) -> PathBuf {
    let base = app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."));
    if !base.exists() {
        let _ = fs::create_dir_all(&base);
    }
    base.join("app_config.json")
}

/// Имя папки данных приложения. ДОЛЖНО совпадать с `identifier` из
/// tauri.conf.json (используется только для чтения конфига ДО создания
/// Tauri-приложения, когда AppHandle ещё недоступен). Плагин использует то же
/// имя через `set_app_data_dir_name`.
pub const APP_DATA_DIR_NAME: &str = "com.kingorch.app";

/// Папка данных приложения без AppHandle (APPDATA/<APP_DATA_DIR_NAME>).
pub fn app_data_dir_early() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(APP_DATA_DIR_NAME)
}

/// Читает конфиг ДО создания Tauri-приложения (main.rs: решение о телеметрии
/// принимается до регистрации плагина). Если файл не читается — default
/// (анонимные отчёты включены).
pub fn load_config_early() -> AppConfig {
    let path = app_data_dir_early().join("app_config.json");
    if let Ok(data) = fs::read_to_string(path) {
        let mut cfg: AppConfig = serde_json::from_str(&data).unwrap_or_default();
        sanitize_retired_fields(&mut cfg);
        cfg
    } else {
        AppConfig::default()
    }
}

pub fn load_config(app: &AppHandle) -> AppConfig {
    if let Ok(data) = fs::read_to_string(get_config_path(app)) {
        let mut cfg: AppConfig = serde_json::from_str(&data).unwrap_or_default();
        sanitize_retired_fields(&mut cfg);
        cfg
    } else {
        AppConfig::default()
    }
}

/// Старые ключи UI, снятые с юзера: всегда дефолт (false / Auto).
/// Сбрасывается при каждом чтении — мигрирует app_config.json без ручной чистки.
fn sanitize_retired_fields(cfg: &mut AppConfig) {
    cfg.kv_quant_keys = false;
    cfg.kv_quant_values = false;
    cfg.prompt_format = "Auto".to_string();
}

pub fn save_config(app: &AppHandle, config: &AppConfig) {
    let path = get_config_path(app);
    save_config_file(&path, config);
}

pub fn save_config_file(path: &Path, config: &AppConfig) {
    let mut root: serde_json::Value = fs::read_to_string(path)
        .ok()
        .and_then(|d| serde_json::from_str(&d).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let host_value = serde_json::to_value(config).unwrap_or_else(|_| serde_json::json!({}));
    if let (serde_json::Value::Object(root_map), serde_json::Value::Object(host_map)) =
        (&mut root, host_value)
    {
        for (k, v) in host_map {
            root_map.insert(k, v);
        }
    }
    if let Ok(data) = serde_json::to_string_pretty(&root) {
        let _ = fs::write(path, data);
    }
}

pub fn find_agents_dir(app: &AppHandle) -> PathBuf {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Dev-чекout: исходники в корне репозитория (target/<profile>/../../agents).
    // Приоритет исходников гарантирует, что в списке агентов всегда актуальный
    // набор из agents/, а не устаревшая копия ресурсов в target/ (Tauri копирует
    // ресурсы аддитивно и НЕ удаляет выпиленные из исходников файлы-призраки).
    // Паттерн тот же, что в find_mcp_servers_dir ниже.
    // 3 уровня вверх от exe_dir: target/release/ → target/ → src-tauri/ → корень проекта
    let repo_agents = exe_dir.join("..").join("..").join("..").join("agents");
    if repo_agents.exists() {
        return repo_agents;
    }

    for rel in ["agents", "../agents"] {
        if let Ok(path) = app.path().resolve(rel, BaseDirectory::Resource) {
            if path.exists() {
                return path;
            }
        }
    }
    if exe_dir.join("agents").exists() {
        return exe_dir.join("agents");
    }
    let path = PathBuf::from("agents");
    if path.exists() {
        return path;
    }
    app.path().resolve("agents", BaseDirectory::Resource)
        .unwrap_or_else(|_| PathBuf::from("agents"))
}

pub fn find_mcp_servers_dir(app: &AppHandle) -> PathBuf {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
    for dir in [
        // Dev-чекout: исходники в корне репозитория — приоритет как в find_agents_dir.
        exe_dir.join("..").join("..").join("src-tauri").join("mcp_servers"),
        exe_dir.join("mcp_servers"),
        resource_dir.join("mcp_servers"),
        PathBuf::from("src-tauri").join("mcp_servers"),
    ] {
        if dir.exists() {
            return dir;
        }
    }
    resource_dir.join("mcp_servers")
}

/// Директория с наборами задач для coding-бенчмарка LLM (tasks_for_test_llm/).
/// Паттерн поиска тот же, что в find_agents_dir / find_mcp_servers_dir:
/// сначала dev-чекout в корне репозитория, затем ресурсы приложения.
pub fn find_coding_tests_dir(app: &AppHandle) -> PathBuf {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
    for dir in [
        exe_dir.join("..").join("..").join("tasks_for_test_llm"),
        exe_dir.join("tasks_for_test_llm"),
        resource_dir.join("tasks_for_test_llm"),
        PathBuf::from("tasks_for_test_llm"),
    ] {
        if dir.exists() {
            return dir;
        }
    }
    app.path()
        .resolve("tasks_for_test_llm", BaseDirectory::Resource)
        .unwrap_or_else(|_| resource_dir.join("tasks_for_test_llm"))
}

/// Именованные пресеты параметров сэмплинга (sampling_presets.json)
pub type SamplingPresets = HashMap<String, ModelParams>;

/// Загружает пресеты из sampling_presets.json, ища в нескольких местах.
/// Если файл не найден — возвращает пустой HashMap (backward compatible).
pub fn load_sampling_presets(project_dir: &Path) -> SamplingPresets {
    let possible_paths = vec![
        project_dir.join("sampling_presets.json"),
        project_dir.join("src-tauri").join("sampling_presets.json"),
        project_dir.join("..").join("sampling_presets.json"),
        PathBuf::from("sampling_presets.json"),
    ];

    for path in &possible_paths {
        if let Ok(data) = fs::read_to_string(path) {
            if let Ok(presets) = serde_json::from_str::<SamplingPresets>(&data) {
                eprintln!("[config] sampling_presets.json загружен из {}", path.display());
                return presets;
            }
        }
    }
    eprintln!("[config] sampling_presets.json не найден (пробовали: {:?}), пресеты не загружены",
        possible_paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>());
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_config_file_preserves_external_fields() {
        let tmp = std::env::temp_dir().join(format!("king_orch_cfg_test_{}.json", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let initial_json = r#"{"nine_router":{"dir":"D:\\custom\\9router","port":20128}}"#;
        fs::write(&tmp, initial_json).unwrap();

        let cfg = AppConfig::default();
        save_config_file(&tmp, &cfg);

        let saved = fs::read_to_string(&tmp).unwrap();
        let val: serde_json::Value = serde_json::from_str(&saved).unwrap();
        let _ = fs::remove_file(&tmp);

        assert_eq!(
            val.get("nine_router").and_then(|nr| nr.get("dir")).and_then(|d| d.as_str()),
            Some("D:\\custom\\9router")
        );
    }
}