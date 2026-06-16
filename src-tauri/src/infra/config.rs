use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Serialize, Deserialize, Clone)]
pub struct ModelParams {
    pub temperature: f32,
    pub top_k: u32,
    pub top_p: f32,
    pub min_p: f32,
    pub repetition_penalty: f32,
    pub presence_penalty: f32,
}

impl Default for ModelParams {
    fn default() -> Self {
        Self {
            temperature: 0.8,
            top_k: 40,
            top_p: 0.95,
            min_p: 0.05,
            repetition_penalty: 1.1,
            presence_penalty: 0.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub models: Vec<String>,
    pub last_model: Option<String>,
    #[serde(default)]
    pub model_params: HashMap<String, ModelParams>,
    #[serde(default = "default_context_size")]
    pub context_size: u32,
    #[serde(default = "default_kv_quantization")]
    pub kv_quantization: bool,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_prompt_format")]
    pub prompt_format: String,
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f32,
    #[serde(default = "default_show_advanced_features")]
    pub show_advanced_features: bool,
    #[serde(default)]
    pub mmproj_files: HashMap<String, String>,
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

fn default_context_size() -> u32 { 24576 }
fn default_kv_quantization() -> bool { false }
fn default_theme() -> String { "dark".to_string() }
fn default_prompt_format() -> String { "Auto".to_string() }
fn default_confidence_threshold() -> f32 { 0.8 }
fn default_show_advanced_features() -> bool { false }

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            models: Vec::new(),
            last_model: None,
            model_params: HashMap::new(),
            context_size: default_context_size(),
            kv_quantization: default_kv_quantization(),
            theme: default_theme(),
            prompt_format: default_prompt_format(),
            confidence_threshold: default_confidence_threshold(),
            show_advanced_features: default_show_advanced_features(),
            mmproj_files: HashMap::new(),
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

#[derive(Serialize, Deserialize, Clone)]
pub struct CatalogEntry {
    pub name: String,
    pub download_url: String,
    pub default_params: ModelParams,
}

pub fn find_agents_dir(app: &AppHandle) -> PathBuf {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
    for dir in [
        exe_dir.join("agents"),
        resource_dir.join("agents"),
        PathBuf::from("agents"),
        exe_dir.join("..").join("..").join("agents"),
    ] {
        if dir.exists() {
            return dir;
        }
    }
    let default = exe_dir.join("agents");
    let _ = fs::create_dir_all(&default);
    default
}

pub fn find_mcp_servers_dir(app: &AppHandle) -> PathBuf {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
    for dir in [
        exe_dir.join("mcp_servers"),
        resource_dir.join("mcp_servers"),
        PathBuf::from("src-tauri").join("mcp_servers"),
        exe_dir.join("..").join("..").join("src-tauri").join("mcp_servers"),
    ] {
        if dir.exists() {
            return dir;
        }
    }
    resource_dir.join("mcp_servers")
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