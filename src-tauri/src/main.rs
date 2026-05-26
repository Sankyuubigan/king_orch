#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod llm;
mod parsers;
mod agent_manager;
mod orchestrator;
mod session_manager;
mod mcp_client;
mod config;
mod downloader;

use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State, Emitter};
use agent_manager::AgentProfile;
use llm::{ChatMessage, SubCall};
use config::{AppConfig, ModelParams};

pub fn emit_log(app: &AppHandle, msg: &str) { let _ = app.emit("log", msg); }
pub fn emit_status(app: &AppHandle, msg: &str, progress: u8) { let _ = app.emit("status", msg); let _ = app.emit("progress", progress); }

pub struct AppState { pub cancel_flag: Arc<AtomicBool> }

fn find_agents_dir(app: &AppHandle) -> PathBuf {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
    let possible_dirs = vec![
        exe_dir.join("agents"),
        resource_dir.join("agents"),
        PathBuf::from("agents"),
        exe_dir.join("..").join("..").join("agents"),
    ];
    for dir in &possible_dirs { if dir.exists() { return dir.clone(); } }
    let default = exe_dir.join("agents");
    let _ = std::fs::create_dir_all(&default);
    default
}

fn find_mcp_servers_dir(app: &AppHandle) -> PathBuf {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
    let possible_dirs = vec![
        exe_dir.join("mcp_servers"),
        resource_dir.join("mcp_servers"),
        PathBuf::from("src-tauri").join("mcp_servers"),
        exe_dir.join("..").join("..").join("src-tauri").join("mcp_servers"),
    ];
    for dir in &possible_dirs { if dir.exists() { return dir.clone(); } }
    resource_dir.join("mcp_servers")
}

fn parse_thought_from_log(msg: &str) -> Option<(String, String)> {
    let rest = msg.strip_prefix("💭 Мысль ")?;
    let paren_pos = rest.find(" (")?;
    let agent_name = rest[..paren_pos].to_string();
    let colon_pos = rest.find("): ")?;
    let thought = rest[colon_pos + 3..].to_string();
    Some((agent_name, thought))
}

#[tauri::command]
fn get_config(app: AppHandle) -> AppConfig { config::load_config(&app) }

#[tauri::command]
fn set_config_value(app: AppHandle, key: String, value: serde_json::Value) {
    let mut cfg = config::load_config(&app);
    match key.as_str() { "confidence_threshold" => { if let Some(v) = value.as_f64() { cfg.confidence_threshold = v as f32; } }, _ => {} }
    config::save_config(&app, &cfg);
}

#[tauri::command]
fn get_agents(app: AppHandle) -> Vec<AgentProfile> {
    let agents_dir = find_agents_dir(&app);
    emit_log(&app, &format!("🔍 Поиск агентов в: {}", agents_dir.display()));
    match agent_manager::load_agents(&agents_dir) {
        Ok(agents) => { emit_log(&app, &format!("✅ Загружено агентов: {}", agents.len())); agents }
        Err(e) => { emit_log(&app, &format!("❌ Ошибка загрузки агентов: {}", e)); vec![] }
    }
}

#[tauri::command]
fn add_model(app: AppHandle, path: String) -> AppConfig {
    let mut cfg = config::load_config(&app); if !cfg.models.contains(&path) { cfg.models.push(path.clone()); } cfg.last_model = Some(path); config::save_config(&app, &cfg); cfg
}

#[tauri::command]
fn set_last_model(app: AppHandle, path: String) { let mut cfg = config::load_config(&app); cfg.last_model = Some(path); config::save_config(&app, &cfg); }
#[tauri::command]
fn set_theme(app: AppHandle, theme: String) { let mut cfg = config::load_config(&app); cfg.theme = theme; config::save_config(&app, &cfg); }
#[tauri::command]
fn set_prompt_format(app: AppHandle, format: String) { let mut cfg = config::load_config(&app); cfg.prompt_format = format; config::save_config(&app, &cfg); }

#[tauri::command]
fn get_sessions(app: AppHandle) -> Vec<session_manager::SessionMeta> { session_manager::get_sessions(&app) }
#[tauri::command]
fn load_session(app: AppHandle, id: String) -> Result<session_manager::ChatSession, String> { session_manager::get_session(&app, &id) }
#[tauri::command]
fn save_session(app: AppHandle, id: String, messages: Vec<ChatMessage>, dossier: HashMap<String, String>, draft: String) -> Result<(), String> {
    let title = messages.iter().find(|m| m.role == "user").map(|m| { let text = m.content.replace('\n', " "); if text.chars().count() > 35 { format!("{}...", text.chars().take(32).collect::<String>()) } else { text } }).unwrap_or_else(|| "Новая сессия".to_string());
    session_manager::save_session(&app, &id, &title, messages, dossier, draft)
}
#[tauri::command]
fn delete_session(app: AppHandle, id: String) -> Result<(), String> { session_manager::delete_session(&app, &id) }
#[tauri::command]
fn rename_session(app: AppHandle, id: String, new_title: String) -> Result<(), String> { session_manager::rename_session(&app, &id, &new_title) }
#[tauri::command]
fn open_session_folder(app: AppHandle, id: String) -> Result<(), String> { session_manager::open_session_folder(&app, &id) }
#[tauri::command]
fn get_models_catalog(app: AppHandle) -> Vec<config::CatalogEntry> { config::load_catalog(&app) }

#[tauri::command]
fn get_model_params(app: AppHandle, model_path: String) -> ModelParams {
    let mut cfg = config::load_config(&app);
    if let Some(params) = cfg.model_params.get(&model_path) { return params.clone(); }
    let catalog = config::load_catalog(&app);
    let file_name = std::path::Path::new(&model_path).file_name().unwrap_or_default().to_string_lossy();
    let mut params = ModelParams::default();
    let mut found = false;
    for entry in catalog { if file_name.contains(&entry.name) || entry.download_url.contains(&file_name.to_string()) { params = entry.default_params.clone(); found = true; break; } }
    if !found { if let Some(temp) = llm::extract_f32_from_gguf(&model_path, "tokenizer.ggml.temp") { params.temperature = temp; } if let Some(top_k) = llm::extract_u32_from_gguf(&model_path, "tokenizer.ggml.top_k") { params.top_k = top_k; } if let Some(top_p) = llm::extract_f32_from_gguf(&model_path, "tokenizer.ggml.top_p") { params.top_p = top_p; } if let Some(min_p) = llm::extract_f32_from_gguf(&model_path, "tokenizer.ggml.min_p") { params.min_p = min_p; } }
    cfg.model_params.insert(model_path.clone(), params.clone()); config::save_config(&app, &cfg); params
}

#[tauri::command]
fn set_model_params(app: AppHandle, model_path: String, params: ModelParams) { let mut cfg = config::load_config(&app); cfg.model_params.insert(model_path, params); config::save_config(&app, &cfg); }
#[tauri::command]
fn reset_model_params(app: AppHandle, model_path: String) -> ModelParams { let mut cfg = config::load_config(&app); cfg.model_params.remove(&model_path); config::save_config(&app, &cfg); get_model_params(app, model_path) }

#[derive(Serialize)]
struct ChatResponse { text: String, sub_calls: Vec<SubCall>, dossier: HashMap<String, String> }

#[tauri::command]
async fn chat_request(
    app: AppHandle, state: State<'_, AppState>, model_path: String, agent_id: String, message: String,
    history: Vec<ChatMessage>, context_size: u32, kv_quantization: bool, dossier: HashMap<String, String>, model_params: ModelParams,
) -> Result<ChatResponse, String> {
    let mut cfg = config::load_config(&app); cfg.context_size = context_size; cfg.kv_quantization = kv_quantization; config::save_config(&app, &cfg);
    let format_type = cfg.prompt_format.clone(); let conf_threshold = cfg.confidence_threshold;
    state.cancel_flag.store(false, Ordering::SeqCst); let cancel_flag = state.cancel_flag.clone();

    let agents_dir = find_agents_dir(&app);
    let mcp_servers_dir = find_mcp_servers_dir(&app);

    // Лог-коллбэк: эмитит "log" + "agent_thought" для мыслей
    let app_log = app.clone();
    let log_cb = move |msg: String| {
        let _ = app_log.emit("log", &msg);
        if let Some((agent_name, thought)) = parse_thought_from_log(&msg) {
            let _ = app_log.emit("agent_thought", serde_json::json!({
                "agent_name": agent_name,
                "thought": thought
            }));
        }
    };

    let app_status = app.clone();
    let status_cb = move |msg: String, progress: u8| { let _ = app_status.emit("status", &msg); let _ = app_status.emit("progress", progress); };

    // Коллбэк для отчётов сабагентов в реальном времени
    // Каждый завершённый сабагент сразу появляется в чате — не ждём финального ответа
    let app_subcall = app.clone();
    let subcall_cb = move |subcall: &SubCall| {
        let _ = app_subcall.emit("subcall_done", subcall.clone());
    };

    let result = tokio::task::spawn_blocking(move || {
        orchestrator::run_chat(
            log_cb, status_cb, subcall_cb,
            agents_dir, mcp_servers_dir, model_path, agent_id, message, history,
            context_size, kv_quantization, model_params, format_type, conf_threshold, cancel_flag, dossier
        )
    }).await.map_err(|e| e.to_string())??;

    Ok(ChatResponse { text: result.0, sub_calls: result.1, dossier: result.2 })
}

#[tauri::command]
async fn stop_processing(state: State<'_, AppState>) -> Result<(), String> { state.cancel_flag.store(true, Ordering::SeqCst); Ok(()) }

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init()).plugin(tauri_plugin_shell::init()).plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState { cancel_flag: Arc::new(AtomicBool::new(false)) })
        .invoke_handler(tauri::generate_handler![
            get_config, set_config_value, get_agents, add_model, set_last_model, set_theme, set_prompt_format,
            get_sessions, load_session, save_session, delete_session, rename_session, open_session_folder,
            get_models_catalog, get_model_params, set_model_params, reset_model_params, downloader::download_model, chat_request, stop_processing
        ])
        .run(tauri::generate_context!()).expect("error while running tauri application");
}