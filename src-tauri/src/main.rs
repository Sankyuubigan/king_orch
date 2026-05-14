#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod llm;
mod parsers;
mod agent_manager;
mod orchestrator;
mod session_manager;
mod tool_executor;
mod mcp_client;

use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, State, Manager, Emitter}; // <-- Добавили Emitter сюда
use agent_manager::AgentProfile;
use llm::{ChatMessage, SubCall};

// Функция для логирования, которую раньше брали из процессора
pub fn emit_log(app: &AppHandle, msg: &str) {
    let _ = app.emit("log", msg);
}

pub fn emit_status(app: &AppHandle, msg: &str, progress: u8) {
    let _ = app.emit("status", msg);
    let _ = app.emit("progress", progress);
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AppConfig {
    models: Vec<String>,
    last_model: Option<String>,
    #[serde(default = "default_temperature")]
    temperature: f32,
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
}

fn default_temperature() -> f32 { 0.2 }
fn default_context_size() -> u32 { 24576 }
fn default_kv_quantization() -> bool { false }
fn default_theme() -> String { "dark".to_string() }
fn default_prompt_format() -> String { "Auto".to_string() }
fn default_confidence_threshold() -> f32 { 0.8 }

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            models: Vec::new(),
            last_model: None,
            temperature: default_temperature(),
            context_size: default_context_size(),
            kv_quantization: default_kv_quantization(),
            theme: default_theme(),
            prompt_format: default_prompt_format(),
            confidence_threshold: default_confidence_threshold(),
        }
    }
}

pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>,
}

fn get_config_path(app: &AppHandle) -> std::path::PathBuf {
    let base = app.path().app_data_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    if !base.exists() {
        let _ = fs::create_dir_all(&base);
    }
    base.join("app_config.json")
}

fn load_config(app: &AppHandle) -> AppConfig {
    if let Ok(data) = fs::read_to_string(get_config_path(app)) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        AppConfig::default()
    }
}

fn save_config(app: &AppHandle, config: &AppConfig) {
    if let Ok(data) = serde_json::to_string_pretty(config) {
        let _ = fs::write(get_config_path(app), data);
    }
}

#[tauri::command]
fn get_config(app: tauri::AppHandle) -> AppConfig {
    load_config(&app)
}

#[tauri::command]
fn set_config_value(app: tauri::AppHandle, key: String, value: serde_json::Value) {
    let mut config = load_config(&app);
    match key.as_str() {
        "confidence_threshold" => {
            if let Some(v) = value.as_f64() {
                config.confidence_threshold = v as f32;
            }
        },
        _ => {}
    }
    save_config(&app, &config);
}

#[tauri::command]
fn get_agents(app: tauri::AppHandle) -> Vec<AgentProfile> {
    agent_manager::load_agents(&app)
}

#[tauri::command]
fn add_model(app: tauri::AppHandle, path: String) -> AppConfig {
    let mut config = load_config(&app);
    if !config.models.contains(&path) {
        config.models.push(path.clone());
    }
    config.last_model = Some(path);
    save_config(&app, &config);
    config
}

#[tauri::command]
fn set_last_model(app: tauri::AppHandle, path: String) {
    let mut config = load_config(&app);
    config.last_model = Some(path);
    save_config(&app, &config);
}

#[tauri::command]
fn set_theme(app: tauri::AppHandle, theme: String) {
    let mut config = load_config(&app);
    config.theme = theme;
    save_config(&app, &config);
}

#[tauri::command]
fn set_prompt_format(app: tauri::AppHandle, format: String) {
    let mut config = load_config(&app);
    config.prompt_format = format;
    save_config(&app, &config);
}

#[tauri::command]
fn get_sessions(app: tauri::AppHandle) -> Vec<session_manager::SessionMeta> {
    session_manager::get_sessions(&app)
}

#[tauri::command]
fn load_session(app: tauri::AppHandle, id: String) -> Result<session_manager::ChatSession, String> {
    session_manager::get_session(&app, &id)
}

#[tauri::command]
fn save_session(app: tauri::AppHandle, id: String, messages: Vec<ChatMessage>, state_markdown: String) -> Result<(), String> {
    let title = messages.iter().find(|m| m.role == "user").map(|m| {
        let text = m.content.replace('\n', " ");
        if text.chars().count() > 35 {
            format!("{}...", text.chars().take(32).collect::<String>())
        } else {
            text
        }
    }).unwrap_or_else(|| "Новая сессия".to_string());
    
    session_manager::save_session(&app, &id, &title, messages, state_markdown)
}

#[tauri::command]
fn delete_session(app: tauri::AppHandle, id: String) -> Result<(), String> {
    session_manager::delete_session(&app, &id)
}

#[derive(Serialize)]
struct ChatResponse {
    text: String,
    sub_calls: Vec<SubCall>,
    new_state: String,
}

#[tauri::command]
async fn chat_request(
    app: AppHandle,
    state: State<'_, AppState>,
    model_path: String,
    agent_id: String,
    message: String,
    history: Vec<ChatMessage>,
    context_size: u32,
    kv_quantization: bool,
    current_state: String,
) -> Result<ChatResponse, String> {
    let mut config = load_config(&app);
    config.context_size = context_size;
    config.kv_quantization = kv_quantization;
    save_config(&app, &config);
    
    let format_type = config.prompt_format.clone();
    let conf_threshold = config.confidence_threshold;

    state.cancel_flag.store(false, Ordering::SeqCst);
    let cancel_flag = state.cancel_flag.clone();

    let result = tokio::task::spawn_blocking(move || {
        orchestrator::run_chat(
            app,
            model_path,
            agent_id,
            message,
            history,
            context_size,
            kv_quantization,
            config.temperature,
            format_type,
            conf_threshold,
            cancel_flag,
            current_state
        )
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(ChatResponse { text: result.0, sub_calls: result.1, new_state: result.2 })
}

#[tauri::command]
async fn stop_processing(state: State<'_, AppState>) -> Result<(), String> {
    state.cancel_flag.store(true, Ordering::SeqCst);
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_config_value,
            get_agents,
            add_model,
            set_last_model,
            set_theme,
            set_prompt_format,
            get_sessions,
            load_session,
            save_session,
            delete_session,
            chat_request,
            stop_processing
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}