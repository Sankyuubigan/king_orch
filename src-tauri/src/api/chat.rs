use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State, Emitter, Manager};

use crate::domain;
use crate::infra::{self, ChatMessage, ChatAttachment, ModelParams, SubCall, LlmMessage, llm_history};
use crate::api::AppState;

// ─── Лог-файл ───
// В release логи пишутся в king_orch.log РЯДОМ С EXE (infra::startup_log) —
// чтобы юзер мог прислать лог, даже если приложение падает на старте.
// В dev-комплекте (в рабочем каталоге есть папка test/) startup_log сам
// дублирует ВСЕ записи (включая ERROR) в test/last_logs.txt — см.
// infra::startup_log::init_dev_log(). Проверка runtime (а не cfg(debug_assertions)):
// релизные сборки запускаются из каталога проекта, где test/ лежит рядом,
// и логи нужны для диагностики даже без debug-профиля.

pub fn init_log_file() {
    if !infra::startup_log::is_initialized() {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                infra::startup_log::init(exe_dir);
            }
        }
    }
    // Dev-зеркало (test/last_logs.txt) инициализируется внутри startup_log;
    // проверка «каталог test/ существует в CWD» живёт там же.
    infra::startup_log::init_dev_log();
}

fn append_log(msg: &str) {
    infra::startup_log::append("LLM", msg);
}

/// Отмены пользователем (Stop) — не сбои: они не попадают в телеметрию.
fn is_user_cancel(msg: &str) -> bool {
    msg.contains("Отменено") || msg.contains("Прервано")
}

#[derive(Serialize)]
pub struct ChatResponse {
    text: String,
    sub_calls: Vec<SubCall>,
    messages: Vec<ChatMessage>,
    /// "gpu" / "cpu" — как реально работала модель в этом запросе
    engine_mode: String,
    /// Скорость последней генерации (tok/s)
    engine_tok_per_sec: f64,
    /// Причина CPU-режима (пусто, если GPU)
    engine_mode_detail: String,
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

#[derive(Serialize, Clone)]
pub struct ToolCallEvent {
    author: String,
    tool: String,
    /// Есть только в событии старта вызова (аргументы JSON)
    args: Option<String>,
    /// Есть только в событии результата (ужатый вывод инструмента)
    result: Option<String>,
}

/// Парсинг логов вида:
///   "🔧 Агент 'X' вызвал инструмент Y: args"           — старт вызова
///   "🔧 Инструмент 'Y' (агент 'X') вернул результат ..." — результат
fn parse_tool_from_log(msg: &str) -> Option<ToolCallEvent> {
    let rest = msg.strip_prefix("🔧 ")?;

    // Старт вызова: "Агент 'X' вызвал инструмент Y: args"
    if let Some(pos) = rest.find(" вызвал инструмент ") {
        let before = &rest[..pos];
        let agent_name = before
            .strip_prefix("Агент ")
            .and_then(|s| s.strip_prefix('\''))
            .and_then(|s| s.split_once('\'').map(|(n, _)| n.to_string()))
            .unwrap_or_else(|| before.trim_matches('\'').to_string());
        let after = &rest[pos + " вызвал инструмент ".len()..];
        let (tool, args) = match after.split_once(": ") {
            Some((t, a)) => (t.to_string(), Some(a.to_string())),
            None => (after.to_string(), None),
        };
        return Some(ToolCallEvent { author: agent_name, tool, args, result: None });
    }

    // Результат: "Инструмент 'Y' (агент 'X') вернул результат (N символов): out"
    if let Some(pos) = rest.find(" вернул результат (") {
        let before = &rest[..pos]; // "Инструмент 'Y' (агент 'X')"
        let q1 = before.find('\'')?;
        let q2 = before[q1 + 1..].find('\'')? + q1 + 1;
        let tool = before[q1 + 1..q2].to_string();
        let agent_start = before.find("(агент '")?;
        let agent_name = before[agent_start + "(агент '".len()..]
            .trim_end_matches(')')
            .trim_end_matches('\'')
            .to_string();
        let tail = &rest[pos + " вернул результат (".len()..];
        let (_, output) = tail.split_once(" символов): ")?;
        return Some(ToolCallEvent { author: agent_name, tool, args: None, result: Some(output.to_string()) });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{parse_tool_from_log, parse_thought_from_log};

    #[test]
    fn parses_tool_call_start() {
        let evt = parse_tool_from_log("🔧 Агент 'search-specialist' вызвал инструмент WebSearch: {\"query\":\"курс доллара\"}")
            .expect("должно распарсить старт вызова");
        assert_eq!(evt.author, "search-specialist");
        assert_eq!(evt.tool, "WebSearch");
        assert_eq!(evt.args.as_deref(), Some("{\"query\":\"курс доллара\"}"));
        assert!(evt.result.is_none());
    }

    #[test]
    fn parses_tool_call_result_with_cyrillic_agent() {
        let evt = parse_tool_from_log("🔧 Инструмент 'WebSearch' (агент 'search-specialist') вернул результат (5 символов): тест")
            .expect("должно распарсить результат вызова");
        assert_eq!(evt.author, "search-specialist");
        assert_eq!(evt.tool, "WebSearch");
        assert_eq!(evt.result.as_deref(), Some("тест"));
        assert!(evt.args.is_none());
    }

    #[test]
    fn parses_tool_call_result_with_cyrillic_agent_and_rus_tool() {
        let evt = parse_tool_from_log("🔧 Инструмент 'batch_get_agent_report' (агент 'soma_translator') вернул результат (11 символов): отчёт готов")
            .expect("должно распарсить результат с кириллическим агентом");
        assert_eq!(evt.author, "soma_translator");
        assert_eq!(evt.tool, "batch_get_agent_report");
        assert_eq!(evt.result.as_deref(), Some("отчёт готов"));
    }

    #[test]
    fn parses_tool_call_result_no_agent_part() {
        assert!(parse_tool_from_log("🔧 Что-то другое").is_none());
    }

    #[test]
    fn parses_thought_log() {
        let (agent, thought, time) = parse_thought_from_log("💭 Мысль search-specialist [d=0] (инструмент WebSearch) [⏱2.6с]: Найду курс доллара")
            .expect("должно распарсить мысль");
        assert_eq!(agent, "search-specialist");
        assert_eq!(thought, "Найду курс доллара");
        assert!((time - 2.6).abs() < 0.01);
    }

    #[test]
    fn parses_thought_log_ignores_subagents() {
        assert!(parse_thought_from_log("💭 Мысль worker [d=1] (инструмент WebSearch) [⏱1.0с]: что-то").is_none());
    }
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
    reasoning_budget: Option<u32>,
    kv_quant_keys: bool,
    kv_quant_values: bool,
    model_params: ModelParams,
    attachments: Vec<ChatAttachment>,
    mmproj_path: Option<String>,
) -> Result<ChatResponse, String> {
    let mut cfg = infra::load_config(&app);
    cfg.context_size = context_size;
    cfg.max_gen_tokens = max_gen_tokens;
    if let Some(rb) = reasoning_budget {
        cfg.reasoning_budget = rb;
    }
    cfg.kv_quant_keys = kv_quant_keys;
    cfg.kv_quant_values = kv_quant_values;
    infra::save_config(&app, &cfg);
    let reasoning_budget = cfg.reasoning_budget;

    // ── Проверка установки движка llama.cpp (llama-server) ──
    // Новая архитектура: движок — ОТДЕЛЬНЫЙ процесс, инференс возможен ТОЛЬКО
    // через него (нет встроенного CPU-фолбэка). Если движка нет — понятная ошибка.
    let engine_dir = crate::api::llamacpp::get_engine_dir(&app);
    if !infra::llamacpp_installer::has_any_installed(&engine_dir) {
        let msg = "Движок llama.cpp не установлен (нет llama-server.exe).\n\
             Откройте Настройки → «Движок запуска нейромоделей» и нажмите «Установить движок»."
            .to_string();
        infra::startup_log::append("WARN", &msg);
        return Err(msg);
    }

    let format_type = cfg.prompt_format.clone();
    state.cancel_flag.store(false, Ordering::SeqCst);
    let cancel_flag = state.cancel_flag.clone();

    // ── Автодокачка mmproj ──
    // Если фронтенд не успел докачать проектор (модель добавлена вручную), а в
    // запросе есть вложения — докачиваем по каталогу до запуска движка.
    let mmproj_path = match mmproj_path {
        Some(p) => Some(p),
        None if !attachments.is_empty() => {
            match infra::ensure_mmproj_for_model(&app, &model_path).await {
                Ok(Some(p)) => Some(p),
                Ok(None) => {
                    infra::startup_log::append(
                        "WARN",
                        "mmproj для модели не найден в каталоге — мультимодальный режим недоступен",
                    );
                    None
                }
                Err(e) => {
                    infra::startup_log::append("WARN", &format!("Не удалось докачать mmproj: {}", e));
                    None
                }
            }
        }
        None => None,
    };

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
        if let Some(evt) = parse_tool_from_log(&msg) {
            let _ = app_log.emit("agent_tool_call", evt);
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
    // Prompt-log: снимок точного входа модели (правило «модель видит только записанное»).
    // Путь best-effort по timestamp; если директория недоступна — лог просто не пишется.
    let prompt_log_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let prompt_log = app.path().app_data_dir().ok().map(|d| {
        d.join("prompt_logs").join(format!("{}_{}.prompt_log.jsonl", agent_id, prompt_log_ts))
    });
    let run_result = tokio::task::spawn_blocking(move || {
        // ── Контроль утечек: RSS приложения до и после запроса ──
        // Рост между последовательными запросами = утечка в приложении.
        let app_rss_before = crate::infra::current_process_rss();

        let run_result = domain::run_chat(
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
            reasoning_budget,
            model_params,
            format_type,
            mmproj_path,
            cancel_flag,
            stream_meta,
            prompt_log,
        );
        let app_rss_after = crate::infra::current_process_rss();
        match (app_rss_before, app_rss_after) {
            (Some(b), Some(a)) => log_cb(format!(
                "📊 RSS приложения: до={} МБ, после={} МБ (дельта {} МБ)",
                b / 1048576,
                a / 1048576,
                (a as i64 - b as i64) / 1048576
            )),
            _ => log_cb("⚠️ Не удалось измерить RSS приложения (sysinfo)".to_string()),
        }
        run_result
    })
    .await;

    // Ошибки запроса ОБЯЗАНЫ попасть в лог и телеметрию (не только в UI).
    // Отмены пользователем — не сбои, их не трекаем.
    let result = match run_result {
        Err(join_err) => {
            let m = format!("chat_request (spawn_blocking): {}", join_err);
            infra::startup_log::append("ERROR", &m);
            return Err(m);
        }
        Ok(Err(e)) => {
            if !is_user_cancel(&e) {
                infra::startup_log::append("ERROR", &format!("chat_request (run_chat): {}", e));
            }
            return Err(e);
        }
        Ok(Ok(r)) => r,
    };
    if result.messages.is_empty() {
        infra::startup_log::append(
            "WARN",
            "chat_request: run_chat вернул Ok, но messages[] пуст (фронтенд получит пустой ответ)",
        );
    }

    log_cb_for_result(format!("DEBUG chat_request: result.messages.len={}, types_authors={:?}", result.messages.len(), result.messages.iter().map(|m| (m.msg_type.clone(), m.author.clone())).collect::<Vec<_>>()));

    // Событие для UI-индикатора «GPU/CPU» в шапке чата
    let _ = app.emit(
        "engine_mode",
        serde_json::json!({
            "mode": result.engine_mode,
            "tok_per_sec": result.engine_tok_per_sec,
            "detail": result.engine_mode_detail,
        }),
    );

    Ok(ChatResponse {
        text: result.text,
        sub_calls: result.sub_calls,
        messages: result.messages,
        engine_mode: result.engine_mode,
        engine_tok_per_sec: result.engine_tok_per_sec,
        engine_mode_detail: result.engine_mode_detail,
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
        Some(agent) => {
            let tools = crate::domain::builtin_tools();
            let has_tools = !agent.tools.is_empty() || !agent.mcp_servers.is_empty();
            crate::domain::build_system_prompt(agent, &history, has_tools, &tools, 2048)
        }
        None => {
            let workflows = crate::domain::load_workflows(&agents_dir)?;
            let wf = crate::domain::find_workflow_by_stem(&workflows, &agent_id)
                .ok_or("Entry point не найден: нет ни .md агента, ни workflow с таким ID")?;
            let (system_prompt, _) = crate::domain::build_worst_agent_prompt(&agents, wf, &history);
            system_prompt
        }
    };

    let mut llm_messages: Vec<LlmMessage> = vec![LlmMessage {
        role: "system".to_string(),
        content: system_prompt,
    }];
    
    for msg in llm_history(&history) {
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