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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, State, Emitter};
use agent_manager::AgentProfile;
use llm::{ChatMessage, SubCall};
use config::{AppConfig, ModelParams};

pub fn emit_log(app: &AppHandle, msg: &str) {
    let _ = app.emit("log", msg);
}

pub fn emit_status(app: &AppHandle, msg: &str, progress: u8) {
    let _ = app.emit("status", msg);
    let _ = app.emit("progress", progress);
}

pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>,
}

#[tauri::command]
fn get_config(app: tauri::AppHandle) -> AppConfig {
    config::load_config(&app)
}

#[tauri::command]
fn set_config_value(app: tauri::AppHandle, key: String, value: serde_json::Value) {
    let mut cfg = config::load_config(&app);
    match key.as_str() {
        "confidence_threshold" => {
            if let Some(v) = value.as_f64() {
                cfg.confidence_threshold = v as f32;
            }
        },
        _ => {}
    }
    config::save_config(&app, &cfg);
}

#[tauri::command]
fn get_agents(app: tauri::AppHandle) -> Vec<AgentProfile> {
    agent_manager::load_agents(&app)
}

#[tauri::command]
fn add_model(app: tauri::AppHandle, path: String) -> AppConfig {
    let mut cfg = config::load_config(&app);
    if !cfg.models.contains(&path) {
        cfg.models.push(path.clone());
    }
    cfg.last_model = Some(path);
    config::save_config(&app, &cfg);
    cfg
}

#[tauri::command]
fn set_last_model(app: tauri::AppHandle, path: String) {
    let mut cfg = config::load_config(&app);
    cfg.last_model = Some(path);
    config::save_config(&app, &cfg);
}

#[tauri::command]
fn set_theme(app: tauri::AppHandle, theme: String) {
    let mut cfg = config::load_config(&app);
    cfg.theme = theme;
    config::save_config(&app, &cfg);
}

#[tauri::command]
fn set_prompt_format(app: tauri::AppHandle, format: String) {
    let mut cfg = config::load_config(&app);
    cfg.prompt_format = format;
    config::save_config(&app, &cfg);
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
fn save_session(app: tauri::AppHandle, id: String, messages: Vec<ChatMessage>, state_markdown: String, draft: String) -> Result<(), String> {
    let title = messages.iter().find(|m| m.role == "user").map(|m| {
        let text = m.content.replace('\n', " ");
        if text.chars().count() > 35 {
            format!("{}...", text.chars().take(32).collect::<String>())
        } else {
            text
        }
    }).unwrap_or_else(|| "Новая сессия".to_string());
    
    session_manager::save_session(&app, &id, &title, messages, state_markdown, draft)
}

#[tauri::command]
fn delete_session(app: tauri::AppHandle, id: String) -> Result<(), String> {
    session_manager::delete_session(&app, &id)
}

#[tauri::command]
fn rename_session(app: tauri::AppHandle, id: String, new_title: String) -> Result<(), String> {
    session_manager::rename_session(&app, &id, &new_title)
}

#[tauri::command]
fn open_session_folder(app: tauri::AppHandle, id: String) -> Result<(), String> {
    session_manager::open_session_folder(&app, &id)
}

#[tauri::command]
fn get_models_catalog(app: tauri::AppHandle) -> Vec<config::CatalogEntry> {
    config::load_catalog(&app)
}

#[tauri::command]
fn get_model_params(app: tauri::AppHandle, model_path: String) -> ModelParams {
    let mut cfg = config::load_config(&app);
    if let Some(params) = cfg.model_params.get(&model_path) {
        return params.clone();
    }

    // Фоллбэк: ищем в JSON каталоге по имени файла
    let catalog = config::load_catalog(&app);
    let file_name = std::path::Path::new(&model_path).file_name().unwrap_or_default().to_string_lossy();
    
    let mut params = ModelParams::default();
    let mut found = false;

    for entry in catalog {
        if file_name.contains(&entry.name) || entry.download_url.contains(&file_name.to_string()) {
            params = entry.default_params.clone();
            found = true;
            break;
        }
    }

    // Если не нашли в JSON, вытаскиваем из GGUF метаданных
    if !found {
        if let Some(temp) = llm::extract_f32_from_gguf(&model_path, "tokenizer.ggml.temp") { params.temperature = temp; }
        if let Some(top_k) = llm::extract_u32_from_gguf(&model_path, "tokenizer.ggml.top_k") { params.top_k = top_k; }
        if let Some(top_p) = llm::extract_f32_from_gguf(&model_path, "tokenizer.ggml.top_p") { params.top_p = top_p; }
        if let Some(min_p) = llm::extract_f32_from_gguf(&model_path, "tokenizer.ggml.min_p") { params.min_p = min_p; }
    }

    cfg.model_params.insert(model_path.clone(), params.clone());
    config::save_config(&app, &cfg);
    params
}

#[tauri::command]
fn set_model_params(app: tauri::AppHandle, model_path: String, params: ModelParams) {
    let mut cfg = config::load_config(&app);
    cfg.model_params.insert(model_path, params);
    config::save_config(&app, &cfg);
}

#[tauri::command]
fn reset_model_params(app: tauri::AppHandle, model_path: String) -> ModelParams {
    let mut cfg = config::load_config(&app);
    cfg.model_params.remove(&model_path);
    config::save_config(&app, &cfg);
    get_model_params(app, model_path) // Перезапустит фоллбэк
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
    model_params: ModelParams, // Получаем параметры из UI
) -> Result<ChatResponse, String> {
    let mut cfg = config::load_config(&app);
    cfg.context_size = context_size;
    cfg.kv_quantization = kv_quantization;
    config::save_config(&app, &cfg);
    
    let format_type = cfg.prompt_format.clone();
    let conf_threshold = cfg.confidence_threshold;

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
            model_params,
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
            rename_session,
            open_session_folder,
            get_models_catalog,
            get_model_params,
            set_model_params,
            reset_model_params,
            downloader::download_model,
            chat_request,
            stop_processing
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}