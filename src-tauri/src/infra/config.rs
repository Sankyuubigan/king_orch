//! Полный конфиг приложения (`app_config.json`) — хостовый `AppConfig`.
//!
//! ## Разделение с движковым плагином (два файла, один SSOT на ключ)
//! Плагин `tauri-plugin-llama-engine` владеет движковыми ключами (models,
//! last_model, models_dir, model_params, mmproj_files, model_meta,
//! llamacpp_dir, engine_source, engine_variant) в СОБСТВЕННОМ общем файле
//! `%APPDATA%/llama-engine/app_config.json` (SSOT для всех хостов плагина).
//! Хост хранит только хостовые ключи (theme, context_size, tabs, …) в своём
//! `%APPDATA%/com.kingorch.app/app_config.json`.
//!
//! Стык: [`load_config`] читает хостовый файл и накатывает движковые ключи
//! из файла плагина (оверлей тем же лоадером, что читает панель моделей —
//! чат и плашка видят один список), а [`save_config_file`] вырезает
//! движковые ключи из хостового файла, чтобы вторая копия правды не могла
//! возникнуть снова. Запись движковых ключей хостом — только через
//! [`set_engine_last_model`] / [`set_engine_dirs`] (инцидент 25.10.26:
//! добавленная в панели модель не появлялась в выпадашке чата, потому что
//! хост читал устаревшую копию из своего файла).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};

/// Единые движковые типы — из плагина (SSOT), чтоб хостовый `AppConfig`
/// сериализовался в тот же JSON, что и `EngineConfig` плагина.
pub use tauri_plugin_llama_engine::engine::config::{ModelMeta, ModelParams};

use tauri_plugin_llama_engine::engine::config::{
    self as engine_config, EngineConfig,
};

/// Движковые ключи `EngineConfig` плагина — имена полей в JSON. Владелец —
/// общий файл плагина; в хостовом файле их быть не должно (strip на записи).
pub const ENGINE_KEYS: &[&str] = &[
    "models",
    "last_model",
    "models_dir",
    "model_params",
    "mmproj_files",
    "model_meta",
    "llamacpp_dir",
    "engine_source",
    "engine_variant",
];

/// Одна вкладка рабочей области (браузерный UI). Сериализуется в `app_config.json`
/// как часть `AppConfig.tabs`. Типы: "main" (главная/новая вкладка), "chat"
/// (сессия чата — 1:1 с файлом сессии), "section" (Статика: история сессий,
/// студия агентов, настройки, логи), "webview" (встроенная страница, напр. 9Router).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TabState {
    pub id: String,
    #[serde(rename = "type")]
    pub tab_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "sessionId")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub last_model: Option<String>,
    #[serde(default)]
    pub last_agent: Option<String>,
    /// Какие entry points показывать в выпадающем списке агентов.
    ///
    /// ЕДИНСТВЕННЫЙ источник правды о ВЫБОРЕ ПОЛЬЗОВАТЕЛЯ: список хранит ПОРЯДОК
    /// показа (сверху вниз как в чате), поэтому сортировать его где-либо ещё нельзя.
    /// Пустой массив — валидное состояние «юзер ничего не выбрал».
    ///
    /// Второй, независимый сигнал — `visible` в frontmatter `.md` агента и в корне
    /// YAML-workflow: это решение АВТОРА «предлагать ли запись в каталоге». Он не
    /// подменяет этот список, а лишь убирает запись из правой колонки каталога
    /// (см. `utils/agent-visibility.ts`).
    #[serde(default = "default_agent_visibility")]
    pub agent_visibility: Vec<String>,
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
    #[serde(default)]
    pub mmproj_files: HashMap<String, String>,
    #[serde(default)]
    pub model_meta: HashMap<String, ModelMeta>,
    #[serde(default)]
    pub llamacpp_dir: Option<String>,
    /// Источник бинарей движка: "ggml-org" (дефолт) / "beellama" (KVarN) /
    /// "prismml" (модели Bonsai). None = дефолт (ggml-org). См. engine_sources.json плагина.
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
    /// Корень общего хранилища данных (`KingOrchData`): движки (llamacpp,
    /// 9router, sdcpp), бандлы и новые модели. None = вычисляется автоматом
    /// (диск с макс. свободным местом, см. infra::data_dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
    /// Папка движка stable-diffusion.cpp (ключ плагина image-engine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdcpp_dir: Option<String>,
    /// Папка бандла весов ImageGEN (ключ плагина image-engine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_bundle_dir: Option<String>,
    /// Папка видео-бандлов ImageGEN (ключ плагина image-engine; MiniMax H3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_bundle_dir: Option<String>,
    /// Путь к soffice.exe (LibreOffice) для Excel-тулов: пересчёт формул и
    /// PDF-рендер. None = авто-детект (PATH → стандартные пути установки,
    /// см. ko_excel_tools::soffice::detect_soffice). Пересчёт опционален:
    /// статические гейты формул работают и без него.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libreoffice_path: Option<String>,
}

fn default_translator_lang() -> String {
    "ru".to_string()
}

fn default_context_size() -> u32 { 24576 }
fn default_max_gen_tokens() -> u32 { 2048 }
fn default_reasoning_budget() -> u32 { 1500 }
fn default_kv_quant_keys() -> bool { false }
fn default_kv_quant_values() -> bool { false }
fn default_theme() -> String { "dark".to_string() }
fn default_prompt_format() -> String { "Auto".to_string() }
fn default_confidence_threshold() -> f32 { 0.8 }
fn default_show_advanced_features() -> bool { false }
/// Новому пользователю показываем только базового агента: остальное он
/// добавляет сам в Настройках → «Агенты в чате».
fn default_agent_visibility() -> Vec<String> { vec!["basic".to_string()] }
fn default_allow_error_reports() -> bool { true }
fn default_chat_font_scale() -> f32 { 1.0 }

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: None,
            models: Vec::new(),
            last_model: None,
            last_agent: None,
            agent_visibility: default_agent_visibility(),
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
            data_dir: None,
            sdcpp_dir: None,
            image_bundle_dir: None,
            video_bundle_dir: None,
            libreoffice_path: None,
        }
    }
}

pub fn get_config_path(_app: &AppHandle) -> PathBuf {
    let base = crate::infra::storage::app_data_root();
    // Корень уже пробирован настоящей пробой, поэтому недоступность здесь —
    // уже не молчание: причина написана в лог при выборе корня.
    if let Err(e) = fs::create_dir_all(&base) {
        log::error!(
            "get_config_path: каталог данных {} недоступен ({}). Конфиг не будет сохранён.",
            base.display(),
            e
        );
    }
    base.join("app_config.json")
}

/// Имя папки данных приложения. ДОЛЖНО совпадать с `identifier` из
/// tauri.conf.json (используется только для чтения конфига ДО создания
/// Tauri-приложения, когда AppHandle ещё недоступен). Движковые ключи сюда
/// не входят — ими владеет плагин llama-engine в собственном каталоге.
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
/// принимается до регистрации плагина). Если файла нет — default (анонимные
/// отчёты включены).
///
/// Чтение идёт через `ko-json-store`, а не `fs::read_to_string`: если запись
/// оборвалась на середине, значение лежит в журнале, и прямой `fs` его не видит
/// — пользователь получил бы настройки по умолчанию вместо своих.
pub fn load_config_early() -> AppConfig {
    let mut cfg = parse_config(ko_json_store::read_text(&app_data_dir_early().join("app_config.json")));
    apply_engine_keys(&mut cfg, &engine_config::load_config_early());
    cfg
}

pub fn load_config(app: &AppHandle) -> AppConfig {
    let mut cfg = parse_config(ko_json_store::read_text(&get_config_path(app)));
    apply_engine_keys(&mut cfg, &engine_config::load_config(app));
    cfg
}

/// Накатывает движковые ключи из конфига плагина поверх хостовых значений.
/// Копии движковых ключей в хостовом файле — остатки старой схемы (до переезда
/// плагина в собственный каталог) и выигрывать никогда не должны.
fn apply_engine_keys(cfg: &mut AppConfig, eng: &EngineConfig) {
    cfg.models = eng.models.clone();
    cfg.last_model = eng.last_model.clone();
    cfg.models_dir = eng.models_dir.clone();
    cfg.model_params = eng.model_params.clone();
    cfg.mmproj_files = eng.mmproj_files.clone();
    cfg.model_meta = eng.model_meta.clone();
    cfg.llamacpp_dir = eng.llamacpp_dir.clone();
    cfg.engine_source = eng.engine_source.clone();
    cfg.engine_variant = eng.engine_variant.clone();
}

/// Разбор прочитанного текста конфига. Разбор не удался — дефолт, но причина
/// уходит в лог: «конфиг сломан» и «конфига нет» выглядели бы одинаково, и
/// пользователь решил бы, что настройки сохранены (core rules §2.2).
fn parse_config(read: Result<Option<String>, std::io::Error>) -> AppConfig {
    match read {
        Ok(Some(data)) => {
            match serde_json::from_str::<AppConfig>(&data) {
                Ok(mut cfg) => {
                    sanitize_retired_fields(&mut cfg);
                    cfg
                }
                Err(e) => {
                    log::error!("load_config: конфиг не разобран ({}). Беру значения по умолчанию; файл оставлен как есть.", e);
                    AppConfig::default()
                }
            }
        }
        Ok(None) => AppConfig::default(),
        Err(e) => {
            log::error!("load_config: конфиг не прочитан ({}). Беру значения по умолчанию.", e);
            AppConfig::default()
        }
    }
}

/// Старые ключи UI, снятые с юзера: всегда дефолт (false / Auto).
/// Сбрасывается при каждом чтении — мигрирует app_config.json без ручной чистки.
fn sanitize_retired_fields(cfg: &mut AppConfig) {
    cfg.kv_quant_keys = false;
    cfg.kv_quant_values = false;
    cfg.prompt_format = "Auto".to_string();
}

pub fn save_config(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let path = get_config_path(app);
    save_config_file(&path, config)
}

pub fn save_config_file(path: &Path, config: &AppConfig) -> Result<(), String> {
    let host_value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    ko_json_store::update_json(path, move |root: &mut serde_json::Value| {
        if !root.is_object() {
            *root = serde_json::json!({});
        }
        if let (serde_json::Value::Object(root_map), serde_json::Value::Object(host_map)) =
            (root, host_value)
        {
            for (k, v) in host_map {
                root_map.insert(k, v);
            }
            // Движковые ключи владеются общим файлом плагина (см. модульный
            // док): копия в хостовом файле — вторая правда, из-за которой
            // чат и панель моделей расходились. Вырезаем даже если остались
            // с прошлых версий.
            for k in ENGINE_KEYS {
                root_map.remove(*k);
            }
        }
    })
}

/// Записывает `last_model` (выбор модели в выпадашке чата) в общий конфиг
/// движка — единый источник правды, который читает и панель моделей.
pub fn set_engine_last_model(app: &AppHandle, path: &str) -> Result<(), String> {
    let mut eng = engine_config::load_config(app);
    eng.last_model = Some(path.to_string());
    engine_config::save_config(app, &eng)
}

/// Записывает движковые пути (`llamacpp_dir`, `models_dir`) в общий конфиг
/// движка (смена хранилища `KingOrchData` пересчитывает производные папки).
pub fn set_engine_dirs(
    app: &AppHandle,
    llamacpp_dir: Option<String>,
    models_dir: Option<String>,
) -> Result<(), String> {
    let mut eng = engine_config::load_config(app);
    eng.llamacpp_dir = llamacpp_dir;
    eng.models_dir = models_dir;
    engine_config::save_config(app, &eng)
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
    fn test_tab_state_accepts_camel_and_snake_case() {
        let camel: TabState = serde_json::from_str(
            r#"{"id":"t1","type":"chat","sessionId":"s1"}"#,
        )
        .unwrap();
        assert_eq!(camel.session_id.as_deref(), Some("s1"));

        let snake: TabState = serde_json::from_str(
            r#"{"id":"t1","type":"chat","session_id":"s1"}"#,
        )
        .unwrap();
        assert_eq!(snake, camel);
    }

    /// Старые app_config.json могли хранить `customTitle` — собственный ярлык
    /// вкладки, который затенял `title` сессии. Поле удалено из схемы, и serde
    /// обязана молча его игнорировать, а не ронять загрузку всего конфига.
    #[test]
    fn test_tab_state_ignores_retired_custom_title() {
        let legacy: TabState = serde_json::from_str(
            r#"{"id":"t1","type":"chat","sessionId":"s1","customTitle":"Мой ярлык"}"#,
        )
        .unwrap();
        assert_eq!(legacy.session_id.as_deref(), Some("s1"));
        assert_eq!(legacy.title.as_deref(), None);
        assert_eq!(serde_json::to_value(&legacy).unwrap().get("customTitle"), None);
    }

    /// Старые app_config.json хранили `two_phase_default` — двухфазный режим
    /// генерации удалён из приложения (см. tasks/17.10.26). serde обязана
    /// молча игнорировать неизвестный ключ, а не ронять загрузку конфига.
    #[test]
    fn test_app_config_ignores_retired_two_phase_default() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"models": [], "last_model": null, "two_phase_default": false, "theme": "dark"}"#,
        )
        .unwrap();
        assert_eq!(cfg.theme, "dark");
        assert_eq!(
            serde_json::to_value(&cfg).unwrap().get("two_phase_default"),
            None
        );
    }

    #[test]
    fn test_save_config_file_preserves_external_fields() {
        let tmp = std::env::temp_dir().join(format!("king_orch_cfg_test_{}.json", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let initial_json = r#"{"cloud_routers":{"9router":{"dir":"D:\\custom\\9router","port":20128}}}"#;
        fs::write(&tmp, initial_json).unwrap();

        let cfg = AppConfig::default();
        save_config_file(&tmp, &cfg).unwrap();

        let saved = fs::read_to_string(&tmp).unwrap();
        let val: serde_json::Value = serde_json::from_str(&saved).unwrap();
        let _ = fs::remove_file(&tmp);

        assert_eq!(
            val.get("cloud_routers")
                .and_then(|cr| cr.get("9router"))
                .and_then(|nr| nr.get("dir"))
                .and_then(|d| d.as_str()),
            Some("D:\\custom\\9router")
        );
    }

    /// Движковые ключи владеются общим файлом плагина: сохранение хостового
    /// конфига обязано вырезать их (в том числе остатки старой схемы, уже
    /// лежащие в файле), иначе в хостовом файле снова появится вторая копия
    /// правды, расходящаяся с панелью моделей (инцидент 25.10.26).
    #[test]
    fn test_save_config_file_strips_engine_keys() {
        let tmp = std::env::temp_dir().join(format!("king_orch_cfg_strip_{}.json", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        // Остатки старой схемы: движковые ключи в хостовом файле.
        let initial_json = r#"{"models":["D:\\old\\stale.gguf"],"last_model":"D:\\old\\stale.gguf","theme":"dark"}"#;
        fs::write(&tmp, initial_json).unwrap();

        let mut cfg = AppConfig::default();
        cfg.theme = "light".to_string();
        cfg.models = vec!["D:\\new\\model.gguf".to_string()];
        save_config_file(&tmp, &cfg).unwrap();

        let saved = fs::read_to_string(&tmp).unwrap();
        let val: serde_json::Value = serde_json::from_str(&saved).unwrap();
        let _ = fs::remove_file(&tmp);

        for key in ENGINE_KEYS {
            assert!(
                val.get(*key).is_none(),
                "движковый ключ «{key}» не должен попадать в хостовый файл, а он есть: {}",
                val.get(*key).unwrap()
            );
        }
        assert_eq!(val.get("theme").and_then(|t| t.as_str()), Some("light"));
    }

    /// Оверлей движковых ключей: файл плагина — SSOT, хостовые (устаревшие)
    /// значения проигрывают по всем девяти ключам.
    #[test]
    fn test_apply_engine_keys_overrides_host_values() {
        let mut cfg = AppConfig::default();
        cfg.models = vec!["D:\\host\\stale.gguf".to_string()];
        cfg.last_model = Some("D:\\host\\stale.gguf".to_string());
        cfg.engine_variant = Some("cpu".to_string());

        let params = ModelParams::default();
        let mut eng = EngineConfig::default();
        eng.models = vec!["D:\\plugin\\fresh.gguf".to_string()];
        eng.last_model = Some("D:\\plugin\\fresh.gguf".to_string());
        eng.models_dir = Some("D:\\KingOrchData\\models".to_string());
        eng.model_params.insert("D:\\plugin\\fresh.gguf".to_string(), params);
        eng.mmproj_files.insert(
            "D:\\plugin\\fresh.gguf".to_string(),
            "D:\\plugin\\fresh.mmproj.gguf".to_string(),
        );
        eng.model_meta.insert(
            "D:\\plugin\\fresh.gguf".to_string(),
            ModelMeta { uncen: true, vision: true, audio: false },
        );
        eng.llamacpp_dir = Some("D:\\KingOrchData\\llamacpp".to_string());
        eng.engine_source = Some("ggml-org".to_string());
        eng.engine_variant = Some("cuda-13.3".to_string());

        apply_engine_keys(&mut cfg, &eng);

        assert_eq!(cfg.models, eng.models);
        assert_eq!(cfg.last_model, eng.last_model);
        assert_eq!(cfg.models_dir, eng.models_dir);
        assert_eq!(cfg.model_params.len(), 1);
        assert!(cfg.model_params.contains_key("D:\\plugin\\fresh.gguf"));
        assert_eq!(
            cfg.mmproj_files.get("D:\\plugin\\fresh.gguf").map(String::as_str),
            Some("D:\\plugin\\fresh.mmproj.gguf")
        );
        assert_eq!(
            serde_json::to_value(cfg.model_meta.get("D:\\plugin\\fresh.gguf")).unwrap(),
            serde_json::to_value(ModelMeta { uncen: true, vision: true, audio: false }).unwrap()
        );
        assert_eq!(cfg.llamacpp_dir, eng.llamacpp_dir);
        assert_eq!(cfg.engine_source, eng.engine_source);
        assert_eq!(cfg.engine_variant.as_deref(), Some("cuda-13.3"));
    }

    /// Регрессия конфиг-ресета (26.10.26): `save_config_file` вырезает engine-ключи
    /// (`models`, `last_model`, …) из хостового файла — это задумано (они живут в
    /// SSOT-файле плагина). Десериализатор `AppConfig` обязан пережить их отсутствие:
    /// иначе `serde_json::from_str` падает с `missing field models`, `parse_config`
    /// возвращает `AppConfig::default()` и теряются вкладки + `agent_visibility`.
    /// См. tasks/config-reset-after-engine-keys-strip.md.
    #[test]
    fn test_save_config_file_output_reparses_without_engine_keys() {
        let tmp = std::env::temp_dir().join(format!(
            "king_orch_rt_reparse_{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cfg: AppConfig = serde_json::from_str(
            r#"{"tabs":[{"id":"keep_main","type":"main","title":"Главная"}],
                "agent_visibility":["basic","soma_translator"]}"#,
        )
        .unwrap();
        save_config_file(&tmp, &cfg).unwrap();

        let saved = fs::read_to_string(&tmp).unwrap();
        let reloaded: AppConfig = serde_json::from_str(&saved)
            .expect("host-файл без engine-ключей должен парситься в AppConfig");
        let _ = fs::remove_file(&tmp);

        assert_eq!(reloaded.tabs.len(), 1, "табка должна выжить save→read round-trip");
        assert_eq!(reloaded.tabs[0].id.as_str(), "keep_main");
        assert_eq!(reloaded.agent_visibility.len(), 2);
        assert_eq!(reloaded.agent_visibility[0].as_str(), "basic");
        assert_eq!(reloaded.agent_visibility[1].as_str(), "soma_translator");
    }
}