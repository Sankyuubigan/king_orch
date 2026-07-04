use serde::Serialize;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::{AppHandle, State, Emitter};

use crate::domain;
use crate::infra::{self, ChatMessage, ChatAttachment, ModelParams, SubCall};
use crate::api::AppState;

// ─── Лог-файл последнего запуска ───
static LAST_LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn init_log_file() {
    let path = std::path::PathBuf::from("test").join("last_logs.txt");
    let _ = std::fs::create_dir_all("test");
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



fn parse_thought_from_log(msg: &str) -> Option<(String, String, f32)> {
    let rest = msg.strip_prefix("💭 Мысль ")?;

    // Extract depth marker: "Name [d=N] (action) [⏱time]: thought"
    let d_start = rest.find(" [d=")?;
    let agent_name = rest[..d_start].to_string();
    let after_d = &rest[d_start + 4..];
    let d_end = after_d.find(']')?;
    let depth: usize = after_d[..d_end].parse().ok()?;

    // Only primary agents (depth=0) emit agent_thought events
    if depth != 0 { return None; }

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
    attachments: Vec<ChatAttachment>,
    mmproj_path: Option<String>,
) -> Result<ChatResponse, String> {
    let mut cfg = infra::load_config(&app);
    cfg.context_size = context_size;
    cfg.kv_quantization = kv_quantization;
    infra::save_config(&app, &cfg);

    let format_type = cfg.prompt_format.clone();
    state.cancel_flag.store(false, Ordering::SeqCst);
    let cancel_flag = state.cancel_flag.clone();

    let agents_dir = infra::find_agents_dir(&app);
    let mcp_servers_dir = infra::find_mcp_servers_dir(&app);

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
            attachments,
            context_size,
            kv_quantization,
            model_params,
            format_type,
            mmproj_path,
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

/// Для Live-превью токенов: возвращает сырую строку промпта, как она будет выглядеть для LLM
#[tauri::command]
pub fn get_prompt_preview(
    app: AppHandle,
    model_path: String,
    agent_id: String,
    message: String,
    history: Vec<ChatMessage>,
) -> Result<String, String> {
    let agents_dir = crate::infra::find_agents_dir(&app);
    let agents = crate::domain::load_agents(&agents_dir)?;
    let agent = agents.iter().find(|a| a.id == agent_id).ok_or("Агент не найден")?;

    // Строим системный промпт (без загрузки реальных MCP инструментов для скорости)
    let system_prompt = crate::domain::build_system_prompt(agent, &history, false, &[]);

    let mut llm_messages = vec![ChatMessage {
        id: None,
        msg_type: "message".to_string(),
        content: system_prompt,
        sub_calls: None,
        author: Some("system".to_string()),
    }];
    
    for msg in history.iter().filter(|m| m.msg_type != "thought") {
        llm_messages.push(msg.clone());
    }
    
    if !message.is_empty() {
        llm_messages.push(ChatMessage {
            id: None,
            msg_type: "message".to_string(),
            content: message,
            sub_calls: None,
            author: Some("user".to_string()),
        });
    }

    let pf = crate::infra::llm::PromptFormat::detect_from_path(&model_path);
    Ok(pf.format_messages(&llm_messages))
}