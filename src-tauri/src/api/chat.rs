use serde::Serialize;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State, Emitter};

use crate::domain;
use crate::infra::{self, ChatMessage, ModelParams, SubCall};
use crate::api::AppState;

// ─── Лог-файл последнего запуска ───
static LAST_LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn init_log_file() {
    let path = std::path::PathBuf::from("temp").join("last_logs.txt");
    let _ = std::fs::create_dir_all("temp");
    if let Ok(file) = std::fs::File::create(&path) {
        if let Ok(mut guard) = LAST_LOG_FILE.lock() {
            *guard = Some(file);
        }
    }
}

fn append_log(msg: &str) {
    if let Ok(mut guard) = LAST_LOG_FILE.lock() {
        if let Some(ref mut file) = *guard {
            let _ = writeln!(file, "{}", msg);
        }
    }
}

#[derive(Serialize)]
pub struct ChatResponse {
    text: String,
    sub_calls: Vec<SubCall>,
    messages: Vec<ChatMessage>,
}

fn find_agents_dir(app: &AppHandle) -> std::path::PathBuf {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    for dir in [
        exe_dir.join("agents"),
        resource_dir.join("agents"),
        std::path::PathBuf::from("agents"),
        exe_dir.join("..").join("..").join("agents"),
    ] {
        if dir.exists() {
            return dir;
        }
    }
    let default = exe_dir.join("agents");
    let _ = std::fs::create_dir_all(&default);
    default
}

fn find_mcp_servers_dir(app: &AppHandle) -> std::path::PathBuf {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    for dir in [
        exe_dir.join("mcp_servers"),
        resource_dir.join("mcp_servers"),
        std::path::PathBuf::from("src-tauri").join("mcp_servers"),
        exe_dir.join("..").join("..").join("src-tauri").join("mcp_servers"),
    ] {
        if dir.exists() {
            return dir;
        }
    }
    resource_dir.join("mcp_servers")
}

fn parse_thought_from_log(msg: &str) -> Option<(String, String, f32)> {
    let rest = msg.strip_prefix("💭 Мысль ")?;
    let paren_pos = rest.find(" (")?;
    let agent_name = rest[..paren_pos].to_string();
    let time_sec = rest.rfind("[⏱").and_then(|start| {
        let after = &rest[start + 4..];
        let end = after.find("с]")?;
        after[..end].parse::<f32>().ok()
    }).unwrap_or(0.0);
    let colon_pos = rest.rfind("]: ").or_else(|| rest.rfind("): "));
    let thought = colon_pos.map(|p| rest[p + 3..].to_string()).unwrap_or_default();
    if thought.is_empty() { None } else { Some((agent_name, thought, time_sec)) }
}

#[tauri::command]
pub async fn chat_request(
    app: AppHandle,
    state: State<'_, AppState>,
    model_path: String,
    agent_id: String,
    message: String,
    history: Vec<ChatMessage>,
    context_size: u32,
    kv_quantization: bool,
    model_params: ModelParams,
) -> Result<ChatResponse, String> {
    let mut cfg = infra::load_config(&app);
    cfg.context_size = context_size;
    cfg.kv_quantization = kv_quantization;
    infra::save_config(&app, &cfg);

    let format_type = cfg.prompt_format.clone();
    let conf_threshold = cfg.confidence_threshold;
    state.cancel_flag.store(false, Ordering::SeqCst);
    let cancel_flag = state.cancel_flag.clone();

    let agents_dir = find_agents_dir(&app);
    let mcp_servers_dir = find_mcp_servers_dir(&app);

    let app_log = app.clone();
    let log_cb = move |msg: String| {
        append_log(&msg);
        let _ = app_log.emit("log", &msg);
        if let Some((agent_name, thought, time_sec)) = parse_thought_from_log(&msg) {
            let _ = app_log.emit(
                "agent_thought",
                serde_json::json!({ "author": agent_name, "thought": thought, "time_sec": time_sec }),
            );
        }
    };

    let app_status = app.clone();
    let status_cb = move |msg: String, progress: u8| {
        let _ = app_status.emit("status", &msg);
        let _ = app_status.emit("progress", progress);
    };

    let app_subcall = app.clone();
    let subcall_cb = move |subcall: &SubCall| {
        let _ = app_subcall.emit("subcall_done", subcall.clone());
    };

    let result = tokio::task::spawn_blocking(move || {
        domain::run_chat(
            log_cb,
            status_cb,
            subcall_cb,
            agents_dir,
            mcp_servers_dir,
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
        )
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(ChatResponse {
        text: result.0,
        sub_calls: result.1,
        messages: result.2,
    })
}

#[tauri::command]
pub async fn stop_processing(state: State<'_, AppState>) -> Result<(), String> {
    state.cancel_flag.store(true, Ordering::SeqCst);
    Ok(())
}