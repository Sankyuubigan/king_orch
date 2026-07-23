use serde::Serialize;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State, Emitter, Manager};

use crate::domain;
use crate::infra::{self, ChatMessage, ChatAttachment, ModelParams, SubCall, LlmMessage};
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
            let m = meta_for_cb.lock().expect("stream_meta lock poisoned");
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

/// Режим графа: строит системный промпт самого «тяжёлого» агента графа.
/// Пиковая VRAM определяется одним LLM-вызовом (движок работает последовательно),
/// поэтому берём агента с самым длинным системным промптом (worst-case).
/// Sub-workflow узлы намеренно НЕ раскрываются — считаем только текущий граф.
fn build_worst_agent_prompt(
    agents: &[domain::AgentProfile],
    wf: &domain::WorkflowDef,
    history: &[ChatMessage],
) -> String {
    let worst_prompt = wf.nodes.iter()
        .filter(|n| n.node_type == domain::NodeType::LlmWorker)
        .filter_map(|n| n.agent.as_deref())
        .filter_map(|aid| agents.iter().find(|a| a.id == aid))
        .map(|agent| domain::build_system_prompt(agent, history, false, &[], 2048))
        .max_by_key(|sp| sp.chars().count());

    // Граф без llm_worker-узлов → пустой системный промпт (посчитаются только история + сообщение)
    worst_prompt.unwrap_or_default()
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

    // Системный промпт: либо конкретного .md-агента, либо — в режиме графа —
    // самого «тяжёлого» агента графа (worst-case для оценки VRAM).
    let system_prompt = match agents.iter().find(|a| a.id == agent_id) {
        Some(agent) => crate::domain::build_system_prompt(agent, &history, false, &[], 2048),
        None => {
            let workflows = crate::domain::load_workflows(&agents_dir)?;
            let wf = crate::domain::find_workflow_by_stem(&workflows, &agent_id)
                .ok_or("Entry point не найден: нет ни .md агента, ни workflow с таким ID")?;
            build_worst_agent_prompt(&agents, wf, &history)
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

    let pf = crate::infra::llm::PromptFormat::detect_from_path(&model_path);
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
    Ok(crate::infra::llm::estimate_vram_mb(&model_path, effective_ctx, kv_quant_keys, kv_quant_values))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_agent(id: &str, system_prompt: &str) -> domain::AgentProfile {
        domain::AgentProfile {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            system_prompt: system_prompt.to_string(),
            is_hidden: false,
            mode: "worker".to_string(),
            mcp_servers: Vec::new(),
            subagents: Vec::new(),
            folder: None,
            single_report: false,
        }
    }

    fn parse_wf(yaml: &str) -> domain::WorkflowDef {
        serde_yaml::from_str(yaml).expect("Не удалось распарсить тестовый workflow")
    }

    #[test]
    fn worst_agent_prompt_picks_longest_system_prompt() {
        let agents = vec![
            make_agent("short", "коротко"),
            make_agent("long", &"очень длинный системный промпт ".repeat(20)),
            make_agent("medium", &"средний ".repeat(5)),
        ];
        let wf = parse_wf(
            "name: test\nnodes:\n  - id: n1\n    type: llm_worker\n    agent: short\n  - id: n2\n    type: llm_worker\n    agent: long\n  - id: n3\n    type: llm_worker\n    agent: medium\nedges: []\n",
        );

        let prompt = build_worst_agent_prompt(&agents, &wf, &[]);
        assert!(prompt.contains("очень длинный системный промпт"), "должен выбраться самый длинный агент");
        assert!(!prompt.contains("коротко"), "короткий агент не должен попасть в результат");
    }

    #[test]
    fn worst_agent_prompt_ignores_sub_workflow_and_non_worker_nodes() {
        let agents = vec![make_agent("worker_a", "промпт воркера А")];
        let wf = parse_wf(
            "name: test\nnodes:\n  - id: sub\n    type: sub_workflow\n    workflow: other_graph\n  - id: w\n    type: llm_worker\n    agent: worker_a\nedges: []\n",
        );

        let prompt = build_worst_agent_prompt(&agents, &wf, &[]);
        assert!(prompt.contains("промпт воркера А"));
    }

    #[test]
    fn worst_agent_prompt_empty_when_no_workers() {
        let agents: Vec<domain::AgentProfile> = vec![];
        let wf = parse_wf(
            "name: test\nnodes:\n  - id: r\n    type: return\nedges: []\n",
        );

        let prompt = build_worst_agent_prompt(&agents, &wf, &[]);
        assert_eq!(prompt, "");
    }
}