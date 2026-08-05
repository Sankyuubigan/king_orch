use serde::Serialize;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State, Emitter, Manager};

use crate::domain;
use crate::infra::{self, ChatMessage, ChatAttachment, ModelParams, SubCall, LlmMessage};
use crate::api::AppState;

// ─── Лог-файл ───
// В release логи пишутся в king_orch.log РЯДОМ С EXE (infra::startup_log) —
// чтобы юзер мог прислать лог, даже если приложение падает на старте.
// В dev (debug_assertions) дополнительно дублируем в test/last_logs.txt.
static DEV_LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn init_log_file() {
    if !infra::startup_log::is_initialized() {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                infra::startup_log::init(exe_dir);
            }
        }
    }
    #[cfg(debug_assertions)]
    {
        let path = std::path::PathBuf::from("test").join("last_logs.txt");
        let _ = std::fs::create_dir_all("test");
        if let Ok(file) = std::fs::File::create(&path) {
            if let Ok(mut guard) = DEV_LOG_FILE.lock() {
                *guard = Some(file);
            }
        }
    }
}

fn append_log(msg: &str) {
    infra::startup_log::append("LLM", msg);
    #[cfg(debug_assertions)]
    if let Ok(mut guard) = DEV_LOG_FILE.lock() {
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
    max_gen_tokens: u32,
    kv_quant_keys: bool,
    kv_quant_values: bool,
    model_params: ModelParams,
    attachments: Vec<ChatAttachment>,
    mmproj_path: Option<String>,
) -> Result<ChatResponse, String> {
    let mut cfg = infra::load_config(&app);
    cfg.context_size = context_size;
    cfg.max_gen_tokens = max_gen_tokens;
    cfg.kv_quant_keys = kv_quant_keys;
    cfg.kv_quant_values = kv_quant_values;
    infra::save_config(&app, &cfg);

    // ── Проверка установки движка llama.cpp (llama-server) ──
    // Новая архитектура: движок — ОТДЕЛЬНЫЙ процесс, инференс возможен ТОЛЬКО
    // через него (нет встроенного CPU-фолбэка). Если движка нет — понятная ошибка.
    let engine_dir = crate::api::llamacpp::get_engine_dir(&app);
    if !infra::llamacpp_installer::is_installed(&engine_dir) {
        return Err(
            "Движок llama.cpp не установлен (нет llama-server.exe).\n\
             Откройте Настройки → «Движок запуска нейромоделей» и нажмите «Установить движок»."
                .to_string(),
        );
    }

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
    
    let app_stream = app.clone();
    let stream_meta = Arc::new(Mutex::new(domain::StreamMeta::default()));
    let meta_for_cb = stream_meta.clone();
    let stream_cb = move |chunk: String| {
        let (kind, author) = {
            let mut m = meta_for_cb.lock().expect("stream_meta lock poisoned");
            // Динамическое переключение: пока LLM генерирует  блок,
            // стримим как "thought". Когда  закрылся — переключаем
            // на "message" чтобы появилась болванка ответа.
            if m.kind == "message" && !m.thinking_done {
                m.buffer.push_str(&chunk);
                if m.buffer.contains("</think>") || m.buffer.contains("</think ") {
                    //  закрылся — переключаем на message
                    m.thinking_done = true;
                    m.buffer.clear();
                } else if m.buffer.contains("<think>") || m.buffer.contains("<think ") {
                    // Всё ещё внутри think блока — стримим как thought
                    let _ = app_stream.emit(
                        "stream_chunk",
                        serde_json::json!({ "kind": "thought", "author": m.author, "text": chunk }),
                    );
                    return;
                } else if !m.buffer.is_empty() {
                    // Нет  в буфере — значит LLM пишет обычный текст
                    // сразу (без think). Переключаем на message.
                    m.thinking_done = true;
                }
            }
            (m.kind.clone(), m.author.clone())
        };
        if kind.is_empty() {
            return;
        }
        let _ = app_stream.emit(
            "stream_chunk",
            serde_json::json!({ "kind": kind, "author": author, "text": chunk }),
        );
    };

    let bins_dir = crate::infra::bin_downloader::get_bins_dir(
        &app.path().app_data_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
    );
    let log_cb_for_result = log_cb.clone();
    let result = tokio::task::spawn_blocking(move || {
        domain::run_chat(
            log_cb.clone(),
            status_cb,
            subcall_cb,
            stream_cb,
            agents_dir,
            mcp_servers_dir,
            bins_dir,
            engine_dir,
            model_path,
            agent_id,
            message,
            history,
            attachments,
            context_size,
            max_gen_tokens,
            kv_quant_keys,
            kv_quant_values,
            model_params,
            format_type,
            mmproj_path,
            cancel_flag,
            stream_meta,
        )
    })
    .await
    .map_err(|e| e.to_string())??;

    log_cb_for_result(format!("DEBUG chat_request: result.messages.len={}, types_authors={:?}", result.2.len(), result.2.iter().map(|m| (m.msg_type.clone(), m.author.clone())).collect::<Vec<_>>()));

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

/// Режим графа: системный промпт самого «тяжёлого» агента графа — см.
/// domain::orchestrator::build_worst_agent_prompt (переехал в домен, SSOT).

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

    // Системный промпт: либо конкретного .md-агента, либо — в режиме графа —
    // самого «тяжёлого» агента графа (worst-case для оценки VRAM).
    let system_prompt = match agents.iter().find(|a| a.id == agent_id) {
        Some(agent) => crate::domain::build_system_prompt(agent, &history, false, &[], 2048),
        None => {
            let workflows = crate::domain::load_workflows(&agents_dir)?;
            let wf = crate::domain::find_workflow_by_stem(&workflows, &agent_id)
                .ok_or("Entry point не найден: нет ни .md агента, ни workflow с таким ID")?;
            crate::domain::build_worst_agent_prompt(&agents, wf, &history)
        }
    };

    let mut llm_messages: Vec<LlmMessage> = vec![LlmMessage {
        role: "system".to_string(),
        content: system_prompt,
    }];
    
    for msg in history.iter().filter(|m| m.msg_type != "thought") {
        llm_messages.push(msg.to_llm_message());
    }
    
    if !message.is_empty() {
        llm_messages.push(LlmMessage {
            role: "user".to_string(),
            content: message,
        });
    }

    let pf = crate::infra::PromptFormat::detect_from_path(&model_path);
    Ok(pf.format_messages(&llm_messages))
}

/// Для Live-превью: прогноз потребления VRAM (модель + KV-кэш) для заданного размера контекста.
#[tauri::command]
pub fn get_prompt_memory(
    model_path: String,
    context_size: u32,
    kv_quant_keys: bool,
    kv_quant_values: bool,
    prompt_tokens: u32,
    max_gen: u32,
) -> Result<f64, String> {
    // Движок выделяет KV-кэш не на весь лимит контекста, а на реально
    // необходимый объём: (промпт + запас на генерацию + 128).min(лимит).
    // Иначе оценка всегда завышена и не зависит от длины промпта.
    const CTX_RESERVE: u32 = 128;
    let effective_ctx = (prompt_tokens + max_gen + CTX_RESERVE).min(context_size);
    Ok(crate::infra::estimate_vram_mb(&model_path, effective_ctx, kv_quant_keys, kv_quant_values))
}