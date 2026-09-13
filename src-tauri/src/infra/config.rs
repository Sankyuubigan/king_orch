use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};

#[derive(Serialize, Deserialize, Clone)]
pub struct ModelParams {
    pub temperature: f32,
    pub top_k: u32,
    pub top_p: f32,
    pub min_p: f32,
    pub repetition_penalty: f32,
    pub presence_penalty: f32,
    #[serde(default)]
    pub dry_multiplier: f32,
    #[serde(default = "default_dry_base")]
    pub dry_base: f32,
    #[serde(default = "default_dry_allowed_length")]
    pub dry_allowed_length: i32,
    #[serde(default)]
    pub dry_penalty_last_n: i32,
    #[serde(default)]
    pub xtc_probability: f32,
    #[serde(default = "default_xtc_threshold")]
    pub xtc_threshold: f32,
}

fn default_dry_base() -> f32 { 1.75 }
fn default_dry_allowed_length() -> i32 { 2 }
fn default_xtc_threshold() -> f32 { 0.1 }

impl Default for ModelParams {
    fn default() -> Self {
        Self {
            temperature: 0.5,
            top_k: 40,
            top_p: 0.95,
            min_p: 0.1,
            repetition_penalty: 1.15,
            presence_penalty: 0.0,
            dry_multiplier: 0.8,
            dry_base: 1.75,
            dry_allowed_length: 2,
            dry_penalty_last_n: 256,
            xtc_probability: 0.0,
            xtc_threshold: 0.1,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AppConfig {
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
    #[serde(default = "default_kv_quant_keys")]
    pub kv_quant_keys: bool,
    #[serde(default = "default_kv_quant_values")]
    pub kv_quant_values: bool,
    #[serde(default = "default_theme")]
    pub theme: String,
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

/// Ищет запись каталога по имени установленного файла модели.
/// Сопоставление идёт по stem имени файла модели с именем файла из
/// `download_url` или `mmproj_url` записи каталога (как в backfill_model_meta).
pub fn find_catalog_entry_for_model<'a>(
    catalog: &'a [CatalogEntry],
    model_path: &str,
) -> Option<&'a CatalogEntry> {
    let stem = std::path::Path::new(model_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())?;
    for entry in catalog {
        let dl_stem = entry
            .download_url
            .split('/')
            .last()
            .and_then(|s| s.split('?').next())
            .and_then(|f| std::path::Path::new(f).file_stem())
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase());
        if dl_stem.as_deref() == Some(stem.as_str()) {
            return Some(entry);
        }
        let mmp_stem = entry
            .mmproj_url
            .as_ref()
            .map(|u| {
                u.split('/')
                    .last()
                    .and_then(|s| s.split('?').next())
                    .and_then(|f| std::path::Path::new(f).file_stem())
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default()
            });
        if mmp_stem.as_deref() == Some(stem.as_str()) {
            return Some(entry);
        }
    }
    None
}

pub fn auto_detect_mmproj(model_path: &str) -> Option<String> {
    let dir = Path::new(model_path).parent()?;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.to_lowercase().contains("mmproj") && name.ends_with(".gguf") {
                return Some(entry.path().to_string_lossy().to_string());
            }
        }
    }
    None
}

/// Проверяет, является ли файл mmproj (мультимодальный проектор), а НЕ языковой
/// моделью. Проектор нельзя запускать как LLM: llama-server либо упадёт, либо
/// выдаст мусор/краши при промпт-процессинге (см. баг с mmproj в списке моделей).
/// Критерии: имя содержит "mmproj" (нижний регистр) ИЛИ GGUF `general.architecture`
/// == `clip`. Чтение GGUF — только fallback: имя решает без чтения файла, что
/// работает и для несуществующих/повреждённых файлов.
pub fn is_mmproj_file(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if name.to_lowercase().contains("mmproj") {
        return true;
    }
    if let Some(arch) = crate::infra::llm_gguf::extract_string_from_gguf(path, "general.architecture") {
        if arch.trim().eq_ignore_ascii_case("clip") {
            return true;
        }
    }
    false
}

/// Для mmproj-файла ищет «братскую» LLM, которую юзер на самом деле хотел
/// добавить. Приоритет:
/// 1) точное имя LLM из записи каталога (`download_url`), если такой файл лежит
///    рядом с mmproj;
/// 2) единственный не-mmproj `.gguf` в той же папке.
/// Возвращает None, если кандидат неоднозначен (0 или >1 кандидатов) или не
/// существует — в этом случае юзеру нужна ошибка с пояснением, а не автоподмена.
pub fn find_sibling_llm_for_mmproj(model_path: &str, catalog: &[CatalogEntry]) -> Option<String> {
    let dir = Path::new(model_path).parent()?;

    // 1) Точное имя LLM из каталога.
    if let Some(entry) = find_catalog_entry_for_model(catalog, model_path) {
        let dl_name = entry
            .download_url
            .split('/')
            .last()
            .and_then(|s| s.split('?').next());
        if let Some(dl_name) = dl_name {
            let candidate = dir.join(dl_name);
            if candidate.exists() && !is_mmproj_file(&candidate.to_string_lossy()) {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    // 2) Единственный не-mmproj .gguf в папке.
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().map_or(false, |e| e == "gguf")
                && !is_mmproj_file(&p.to_string_lossy())
            {
                candidates.push(p.to_string_lossy().to_string());
            }
        }
    }
    if candidates.len() == 1 {
        return candidates.pop();
    }
    None
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
/// Tauri-приложения, когда AppHandle ещё недоступен).
pub const APP_DATA_DIR_NAME: &str = "com.kingorch.app";

/// Папка данных приложения без AppHandle (APPDATA/<APP_DATA_DIR_NAME>).
fn app_data_dir_early() -> PathBuf {
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
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        AppConfig::default()
    }
}

pub fn load_config(app: &AppHandle) -> AppConfig {
    if let Ok(data) = fs::read_to_string(get_config_path(app)) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        AppConfig::default()
    }
}

pub fn save_config(app: &AppHandle, config: &AppConfig) {
    if let Ok(data) = serde_json::to_string_pretty(config) {
        let _ = fs::write(get_config_path(app), data);
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ModelMeta {
    #[serde(default)]
    pub uncen: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub audio: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CatalogEntry {
    pub name: String,
    pub download_url: String,
    #[serde(default)]
    pub size_gb: Option<String>,
    #[serde(default)]
    pub tokenizer_id: Option<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub mmproj_url: Option<String>,
    #[serde(default)]
    pub uncen: Option<bool>,
    #[serde(default)]
    pub vision: Option<bool>,
    #[serde(default)]
    pub audio: Option<bool>,
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

pub fn load_catalog(app: &AppHandle) -> Vec<CatalogEntry> {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
    
    let possible_paths = vec![
        exe_dir.join("models_catalog.json"),
        resource_dir.join("models_catalog.json"),
        PathBuf::from("models_catalog.json"),
        exe_dir.join("..").join("..").join("models_catalog.json"),
    ];

    for path in possible_paths {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(catalog) = serde_json::from_str(&data) {
                return catalog;
            }
        }
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_catalog() -> Vec<CatalogEntry> {
        vec![
            CatalogEntry {
                name: "gemma-4-8b".to_string(),
                download_url:
                    "https://hf.co/x/resolve/main/gemma-4-E4B-it-Q4_K_XL.gguf?download=true".to_string(),
                size_gb: Some("4".to_string()),
                tokenizer_id: None,
                is_default: true,
                mmproj_url: Some(
                    "https://hf.co/x/resolve/main/mmproj-gemma-4-E4B-it-BF16.gguf?download=true"
                        .to_string(),
                ),
                uncen: Some(true),
                vision: Some(true),
                audio: Some(true),
            },
            CatalogEntry {
                name: "plain".to_string(),
                download_url: "https://hf.co/y/resolve/main/plain-model-Q4.gguf".to_string(),
                size_gb: None,
                tokenizer_id: None,
                is_default: false,
                mmproj_url: None,
                uncen: None,
                vision: None,
                audio: None,
            },
        ]
    }

    #[test]
    fn catalog_matches_by_download_stem() {
        let cat = sample_catalog();
        let hit = find_catalog_entry_for_model(&cat, "D:\\models\\gemma-4-E4B-it-Q4_K_XL.gguf");
        assert_eq!(hit.map(|e| e.name.as_str()), Some("gemma-4-8b"));
    }

    #[test]
    fn catalog_matches_by_mmproj_stem() {
        let cat = sample_catalog();
        let hit = find_catalog_entry_for_model(&cat, "D:\\models\\mmproj-gemma-4-E4B-it-BF16.gguf");
        assert_eq!(hit.map(|e| e.name.as_str()), Some("gemma-4-8b"));
    }

    #[test]
    fn catalog_unknown_model_returns_none() {
        let cat = sample_catalog();
        assert!(find_catalog_entry_for_model(&cat, "D:\\models\\other.gguf").is_none());
    }

    /// Минимальный GGUF, читаемый extract_string_from_gguf: сигнатура + 1 строковый KV.
    fn mini_gguf(arch: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"GGUF");
        out.extend_from_slice(&3u32.to_le_bytes());   // version
        out.extend_from_slice(&0u64.to_le_bytes());   // n_tensors
        out.extend_from_slice(&1u64.to_le_bytes());   // n_kv = 1
        let key = "general.architecture";
        out.extend_from_slice(&(key.len() as u64).to_le_bytes());
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(&8u32.to_le_bytes());   // type 8 = GGUF_TYPE_STRING
        out.extend_from_slice(&(arch.len() as u64).to_le_bytes());
        out.extend_from_slice(arch.as_bytes());
        out
    }

    fn ko_temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ko_mmproj_{}_{}", std::process::id(), name));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn ko_cleanup(dir: &std::path::Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mmproj_detected_by_filename() {
        assert!(is_mmproj_file("D:\\models\\mmproj-gemma-4-E4B-it-BF16.gguf"));
        assert!(is_mmproj_file("D:\\models\\MMPROJ-foo-Q8_k.gguf"));
        // Имя решает даже без существующего файла (легаси-конфиг с битым путём).
        assert!(is_mmproj_file("C:\\missing\\mmproj-x.gguf"));
    }

    #[test]
    fn mmproj_detected_by_architecture() {
        let dir = ko_temp_dir("arch");
        let p = dir.join("projector-foo.gguf");
        std::fs::write(&p, mini_gguf("clip")).unwrap();
        assert!(is_mmproj_file(&p.to_string_lossy()));
        ko_cleanup(&dir);
    }

    #[test]
    fn regular_llm_not_mmproj() {
        let dir = ko_temp_dir("llm");
        let p = dir.join("gemma-4-E4B-it-Q4_K_XL.gguf");
        std::fs::write(&p, mini_gguf("gemma4")).unwrap();
        assert!(!is_mmproj_file(&p.to_string_lossy()));
        ko_cleanup(&dir);
    }

    #[test]
    fn sibling_llm_found_via_catalog() {
        let dir = ko_temp_dir("cat");
        let llm = dir.join("gemma-4-E4B-it-Q4_K_XL.gguf");
        let mmp = dir.join("mmproj-gemma-4-E4B-it-BF16.gguf");
        std::fs::write(&llm, mini_gguf("gemma4")).unwrap();
        std::fs::write(&mmp, mini_gguf("clip")).unwrap();
        let found = find_sibling_llm_for_mmproj(&mmp.to_string_lossy(), &sample_catalog());
        assert_eq!(found.as_deref(), Some(llm.to_string_lossy().as_ref()));
        ko_cleanup(&dir);
    }

    #[test]
    fn sibling_llm_found_by_single_gguf_without_catalog() {
        let dir = ko_temp_dir("single");
        let llm = dir.join("mystery-model-Q4.gguf");
        let mmp = dir.join("mmproj-mystery-Q8.gguf");
        std::fs::write(&llm, mini_gguf("llama")).unwrap();
        std::fs::write(&mmp, mini_gguf("clip")).unwrap();
        let found = find_sibling_llm_for_mmproj(&mmp.to_string_lossy(), &[]);
        assert_eq!(found.as_deref(), Some(llm.to_string_lossy().as_ref()));
        ko_cleanup(&dir);
    }

    #[test]
    fn ambiguous_siblings_return_none() {
        let dir = ko_temp_dir("ambig");
        let mmp = dir.join("mmproj-ambig.gguf");
        std::fs::write(&mmp, mini_gguf("clip")).unwrap();
        std::fs::write(dir.join("a-model.gguf"), mini_gguf("llama")).unwrap();
        std::fs::write(dir.join("b-model.gguf"), mini_gguf("llama")).unwrap();
        assert!(find_sibling_llm_for_mmproj(&mmp.to_string_lossy(), &[]).is_none());
        ko_cleanup(&dir);
    }
}