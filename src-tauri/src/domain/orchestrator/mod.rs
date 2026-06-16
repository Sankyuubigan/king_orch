pub mod prompt;
mod runtime;

use crate::domain::agent_manager::{load_agents, AgentProfile};
use crate::domain::workflow_engine::{
    find_workflow_by_stem, load_workflows, WorkflowContext, WorkflowRunner,
};
use crate::infra::{ChatMessage, ChatAttachment, LlamaEngine, ModelParams, SubCall, ToolCallInfo};
use crate::domain::parsers::{
    clean_thought_tags, extract_think_content, extract_thought_from_partial_json,
    has_incomplete_json_action, parse_orchestrator_response, parse_tool_call,
};
use prompt::build_system_prompt;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

const AGENT_ERROR_PREFIX: &str = "⚠️ ОШИБКА_АГЕНТА:";

fn safe_truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len { return s.to_string(); }
    let end = s.char_indices()
        .take_while(|(i, _)| *i < max_len)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(max_len.min(s.len()));
    format!("{}...", &s[..end])
}

// ─── Helper: централизованный вывод мысли ───
fn log_agent_thought(log_cb: &dyn Fn(String), agent: &AgentProfile, action_type: &str, target: &str, thought: &str, thinking_sec: f32, depth: usize) {
    if thought.is_empty() { return; }
    if thinking_sec > 0.0 {
        log_cb(format!("💭 Мысль {} [d={}] ({} {}) [⏱{:.1}с]: {}", agent.name, depth, action_type, target, thinking_sec, thought));
    } else {
        log_cb(format!("💭 Мысль {} [d={}] ({} {}): {}", agent.name, depth, action_type, target, thought));
    }
}

// ─── Helper: список валидных ID сабагентов для ошибок ───
fn valid_agent_ids(agents: &[AgentProfile], exclude_id: &str, exclude_mode: &str) -> Vec<String> {
    agents.iter()
        .filter(|a| a.id != exclude_id && a.mode != exclude_mode)
        .map(|a| a.id.clone())
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn run_chat<L, S, C>(
    log_cb: L, status_cb: S, subcall_cb: C,
    agents_dir: std::path::PathBuf, mcp_servers_dir: std::path::PathBuf,
    model_path: String, agent_id: String, user_text: String, history: Vec<ChatMessage>,
    attachments: Vec<ChatAttachment>,
    context_size: u32, kv_quantization: bool, model_params: ModelParams, format_type: String,
    mmproj_path: Option<String>, cancel_flag: Arc<AtomicBool>,
) -> Result<(String, Vec<SubCall>, Vec<ChatMessage>), String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    status_cb("Загрузка модели в память...".to_string(), 10);
    let engine = if mmproj_path.is_some() {
        LlamaEngine::new_with_mmproj(&model_path, mmproj_path.as_deref(), context_size, kv_quantization, &log_cb)?
    } else {
        LlamaEngine::new(&model_path, context_size, kv_quantization, &log_cb)?
    };
    let agents = load_agents(&agents_dir)?;
    let max_gen_tokens = ((context_size as usize).saturating_sub(2048).max(1024)).min(4096);
    let recent_history: Vec<ChatMessage> = history.iter()
        .filter(|m| m.msg_type != "thought")
        .cloned()
        .collect();
    let mut recent_history = recent_history;
    if recent_history.len() > 8 { recent_history = recent_history[recent_history.len() - 8..].to_vec(); }

    let mut messages_store = history.clone();
    for (i, msg) in messages_store.iter_mut().enumerate() {
        if msg.id.is_none() {
            msg.id = Some(format!("msg_{}", i));
        }
    }
    let mut msg_counter = messages_store.len() as u32;

    // Определяем entry type: если agent_id совпадает с file_stem YAML workflow — запускаем граф
    let mut all_sub_calls = Vec::new();
    let workflows = load_workflows(&agents_dir).unwrap_or_default();
    let workflow_match = find_workflow_by_stem(&workflows, &agent_id).filter(|wf| wf.visible);

    if let Some(workflow) = workflow_match {
        log_cb(format!("▶ Запуск workflow '{}' (entry: {})", workflow.name, agent_id));
        let mut ctx = WorkflowContext::new(
            user_text.clone(),
            "main".to_string(),
            messages_store.clone(),
            recent_history.clone(),
        );
        let mut runner = WorkflowRunner {
            engine: &engine,
            agents: &agents,
            workflows: &workflows,
            log_cb: log_cb.clone(),
            status_cb: status_cb.clone(),
            subcall_cb: subcall_cb.clone(),
            max_gen_tokens,
            model_params: &model_params,
            format_type: &format_type,
            cancel_flag: cancel_flag.clone(),
            mcp_servers_dir: &mcp_servers_dir,
            all_sub_calls: &mut all_sub_calls,
            msg_counter: &mut msg_counter,
        };
        crate::domain::workflow_engine::run_workflow(
            workflow, &mut ctx, &mut runner,
        )?;
        return Ok((String::new(), all_sub_calls, ctx.messages));
    }

    // Fallback: запуск .md агента
    if let Some(primary_agent) = agents.iter().find(|a| a.id == agent_id) {
        log_cb(format!("▶ Запуск агента: {} (ns: main)", primary_agent.name));
        let final_res = run_agent_node(
            log_cb, status_cb, subcall_cb,
            &engine, primary_agent, &agents, user_text, recent_history,
            &attachments,
            "main", max_gen_tokens, &model_params, &format_type,
            cancel_flag, 0, &mut all_sub_calls, None, &mcp_servers_dir,
            &mut messages_store, &mut msg_counter,
        )?;
        let sub_calls_opt = if all_sub_calls.is_empty() { None } else { Some(all_sub_calls.clone()) };
        messages_store.push(ChatMessage {
            id: Some(format!("msg_{}", msg_counter)),
            msg_type: "message".to_string(),
            content: final_res.clone(),
            namespace: None,
            sub_calls: sub_calls_opt,
            author: Some("assistant".to_string()),
        });
        msg_counter += 1;
        Ok((final_res, all_sub_calls, messages_store))
    } else {
        Err(format!("Entry point '{}' не найден: нет ни workflow, ни .md агента с таким ID", agent_id))
    }
}

fn get_agent_report_from_messages(messages: &[ChatMessage], author: &str, namespace: &str) -> Option<String> {
    if author == "user" {
        return messages.iter().rev().find(|m| {
            m.msg_type == "message" && m.author.as_deref() == Some("user")
        }).map(|m| m.content.clone());
    }
    messages.iter().rev().find(|m| {
        m.author.as_deref() == Some(author) && m.namespace.as_deref() == Some(namespace)
    }).map(|m| m.content.clone())
}

fn truncate_result(text: &str, max_len: usize) -> String {
    if text.len() <= max_len { text.to_string() }
    else {
        let cut = text.char_indices().take_while(|(i, _)| *i < max_len).last()
            .map(|(i, c)| i + c.len_utf8()).unwrap_or(max_len.min(text.len()));
        format!("{}...\n(обрезано)", &text[..cut])
    }
}

fn has_json_thought_without_action(text: &str) -> bool {
    let json_str = if let Some(start) = text.find("```json") {
        let cs = start + 7;
        if let Some(end) = text[cs..].find("```") {
            Some(text[cs..cs + end].trim().to_string())
        } else {
            text[cs..].find('{').and_then(|brace_start| {
                text[cs + brace_start..].rfind('}').map(|brace_end| {
                    text[cs + brace_start..cs + brace_start + brace_end + 1].trim().to_string()
                })
            })
        }
    } else if text.contains('{') {
        text.find('{').and_then(|start| {
            text.rfind('}').map(|end| text[start..=end].trim().to_string())
        })
    } else {
        None
    };

    if let Some(json) = json_str {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json)
            .or_else(|_| serde_json::from_str(&json.replace('\n', " ").replace('\r', "")))
        {
            let has_thought = val.get("thought").is_some();
            let has_target = val.get("target").is_some();
            let has_tool = val.get("tool").is_some();
            return has_thought && !has_target && !has_tool;
        }
        let has_thought_re = regex::Regex::new(r#""thought"\s*:"#).ok().map(|re| re.is_match(&json)).unwrap_or(false);
        let has_target_re = regex::Regex::new(r#""target"\s*:"#).ok().map(|re| re.is_match(&json)).unwrap_or(false);
        let has_tool_re = regex::Regex::new(r#""tool"\s*:"#).ok().map(|re| re.is_match(&json)).unwrap_or(false);
        return has_thought_re && !has_target_re && !has_tool_re;
    }
    false
}

#[allow(clippy::too_many_arguments)]
#[allow(unused_assignments)]
pub(crate) fn run_agent_node<L, S, C>(
    log_cb: L, status_cb: S, subcall_cb: C,
    engine: &LlamaEngine, agent: &AgentProfile, agents: &[AgentProfile],
    user_text: String, _history: Vec<ChatMessage>,
    attachments: &[ChatAttachment],
    current_namespace: &str,
    max_gen_tokens: usize, model_params: &ModelParams, format_type: &str,
    cancel_flag: Arc<AtomicBool>, depth: usize,
    all_sub_calls: &mut Vec<SubCall>, _caller_name: Option<String>,
    mcp_servers_dir: &Path,
    messages: &mut Vec<ChatMessage>, msg_counter: &mut u32,
) -> Result<String, String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    // ═══════════════════════════════════════
    //  PHASE 1: SETUP
    // ═══════════════════════════════════════
    if depth > 5 { return Err("Превышена максимальная глубина вложенности сабагентов".into()); }
    log_cb(format!("▶ Запуск агента: {} (ns: {}, глубина: {})", agent.name, current_namespace, depth));

    let mut mcp_clients = HashMap::new();
    let mut all_tools: Vec<(String, String, serde_json::Value)> = Vec::new();
    runtime::load_mcp_servers(&log_cb, mcp_servers_dir, &agent.mcp_servers, &mut mcp_clients, &mut all_tools);

    // Built-in batch_get_agent_report tool
    all_tools.push(("_builtin".to_string(), "batch_get_agent_report".to_string(), serde_json::json!({
        "name": "batch_get_agent_report",
        "description": "Получить несколько отчетов за один вызов. Принимает массив запросов {author, namespace}. author='user' для запроса пользователя.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "queries": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "author": {"type": "string", "description": "ID агента или 'user'"},
                            "namespace": {"type": "string", "description": "Неймспейс"}
                        },
                        "required": ["author", "namespace"]
                    }
                }
            },
            "required": ["queries"]
        }
    })));

    // Built-in emit_signal tool
    all_tools.push(("_builtin".to_string(), "emit_signal".to_string(), serde_json::json!({
        "name": "emit_signal",
        "description": "Сохранить сигнал/маркер в сессию. Другие агенты, экстрактор и phase_router увидят его. Принимает key (имя сигнала) и value (произвольный JSON-объект с данными).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "key": {"type": "string", "description": "Имя сигнала, например 'validator_report' или 'phase'"},
                "value": {"type": "object", "description": "Произвольный JSON с данными сигнала"}
            },
            "required": ["key", "value"]
        }
    })));

    let has_tools = !all_tools.is_empty();
    let system_prompt = build_system_prompt(agent, messages, current_namespace, has_tools, &all_tools);

    let mut llm_messages = vec![ChatMessage { id: None, msg_type: "message".to_string(), content: system_prompt.clone(), namespace: None, sub_calls: None, author: Some("system".to_string()) }];

    for msg in messages.iter() {
        if msg.msg_type == "thought" { continue; }
        let role = match msg.author.as_deref() {
            Some("user") => "user",
            Some("system") => "system",
            _ => "assistant",
        };
        llm_messages.push(ChatMessage {
            id: None,
            msg_type: "message".to_string(),
            content: msg.content.clone(),
            namespace: None,
            sub_calls: None,
            author: Some(role.to_string()),
        });
    }

    llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: user_text.clone(), namespace: None, sub_calls: None, author: Some("user".to_string()) });

    let initial_dump = format!("### [NS: {}]\n### [SYSTEM PROMPT]\n{}",
        current_namespace, agent.system_prompt);

    // ═══════════════════════════════════════
    //  PHASE 2: LOOP STATE
    // ═══════════════════════════════════════
    let mut final_response = String::new();
    let mut tool_calls = Vec::new();
    let start_time = Instant::now();
    let mut consecutive_failed_tools = 0;
    let mut consecutive_incomplete = 0;
    let mut consecutive_invalid_targets = 0;

    // ═══════════════════════════════════════
    //  PHASE 3: MAIN GENERATION LOOP
    // ═══════════════════════════════════════
    for iter in 1..=30 {
        if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }

        // ── 3a. LLM GENERATION ──
        let gen_start = Instant::now();
        log_cb(format!(">>> [{}] LLM вызов #{}, msgs={}, max_gen={}, глубина={}", agent.name, iter, llm_messages.len(), max_gen_tokens, depth));
        let raw_response = if !attachments.is_empty() && engine.is_multimodal() {
            engine.generate_chat_multimodal(
                &llm_messages, &attachments, max_gen_tokens, model_params, format_type, cancel_flag.clone(),
                |p, _| { status_cb(format!("{} обрабатывает медиа (Шаг {})...", agent.name, iter), 20 + (p * 0.1) as u8); },
                log_cb.clone(),
            )?
        } else {
            engine.generate_chat(
                &llm_messages, max_gen_tokens, model_params, format_type, cancel_flag.clone(),
                |p, _| { status_cb(format!("{} думает (Шаг {})...", agent.name, iter), 20 + (p * 0.1) as u8); },
                log_cb.clone(),
            )?
        };

        log_cb(format!("<<< [{}] LLM за {:.1}с, ответ {} символов", agent.name, gen_start.elapsed().as_secs_f32(), raw_response.len()));

        let response = clean_thought_tags(&raw_response);
        let cleaned_len = response.len();
        if cleaned_len != raw_response.len() {
            log_cb(format!("🧹 clean_thought_tags удалил {} символов (было {}, стало {})", raw_response.len() - cleaned_len, raw_response.len(), cleaned_len));
        }
        let mut action_found = false;
        let mut thought_logged = false;

        // ── 3b. RESPONSE DISPATCH: Tool call ──
        if let Some((tool_name, arguments, thought)) = parse_tool_call(&response) {
            action_found = true;
            consecutive_incomplete = 0;
            log_agent_thought(&log_cb, agent, "инструмент", &tool_name, &thought, gen_start.elapsed().as_secs_f32(), depth);
            thought_logged = true;

            status_cb(format!("Выполнение {}...", tool_name), 60);
            let args_str = arguments.to_string();
            let mut tool_output = None;
            let mut tool_found = false;

            if tool_name == "batch_get_agent_report" {
                tool_found = true;
                consecutive_failed_tools = 0;
                let mut results = Vec::new();
                let mut found_list: Vec<String> = Vec::new();
                let mut not_found_list: Vec<String> = Vec::new();
                if let Some(queries) = arguments.get("queries").and_then(|v| v.as_array()) {
                    for q in queries {
                        let author = q.get("author").and_then(|v| v.as_str()).unwrap_or("");
                        let namespace = q.get("namespace").and_then(|v| v.as_str()).unwrap_or("");
                        let report = get_agent_report_from_messages(messages, author, namespace)
                            .unwrap_or_else(|| format!("Отчёт не найден: '{}' в неймспейсе '{}'", author, namespace));
                        let found = !report.starts_with("Отчёт не найден");
                        let label = format!("{}/{}", author, namespace);
                        if found { found_list.push(label); } else { not_found_list.push(label); }
                        results.push(serde_json::json!({
                            "author": author,
                            "namespace": namespace,
                            "found": found,
                            "report": report
                        }));
                    }
                }
                let summary = format!(
                    "[РЕЗУЛЬТАТ batch_get_agent_report]\n✅ Найдено ({}): {}\n❌ Не найдено ({}): {}\n\n[ДЕТАЛИ]\n{}",
                    found_list.len(),
                    if found_list.is_empty() { "—".to_string() } else { found_list.join(", ") },
                    not_found_list.len(),
                    if not_found_list.is_empty() { "—".to_string() } else { not_found_list.join(", ") },
                    serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string())
                );
                tool_output = Some(summary);
            } else if tool_name == "emit_signal" {
                tool_found = true;
                let key = arguments.get("key")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty());
                let value = arguments.get("value")
                    .filter(|v| !v.is_null());

                if let (Some(key), Some(value)) = (key, value) {
                    consecutive_failed_tools = 0;
                    let signal_msg = ChatMessage {
                        id: Some(format!("msg_{}", msg_counter)),
                        msg_type: "signal".to_string(),
                        content: serde_json::json!({key: value}).to_string(),
                        namespace: Some(current_namespace.to_string()),
                        sub_calls: None,
                        author: Some(agent.id.clone()),
                    };
                    messages.push(signal_msg);
                    *msg_counter += 1;
                    tool_output = Some(format!(
                        "[РЕЗУЛЬТАТ emit_signal]\n✅ Сигнал '{}' сохранён в сессию (ns: {}) от агента '{}'.\n\nЕсли задача выполнена — ответь ОБЫЧНЫМ ТЕКСТОМ.",
                        key, current_namespace, agent.id
                    ));
                } else {
                    let key_str = arguments.get("key").map(|v| v.to_string()).unwrap_or_else(|| "отсутствует".to_string());
                    let val_str = arguments.get("value").map(|v| v.to_string()).unwrap_or_else(|| "отсутствует".to_string());
                    tool_output = Some(format!(
                        "Ошибка: emit_signal требует 'key' (строка) и 'value' (объект). Получено: key={}, value={}. Исправь и вызови СНОВА.",
                        key_str, val_str
                    ));
                }
            } else if let Some((mcp_name, _, _)) = all_tools.iter().find(|(_, name, _)| name == &tool_name) {
                if let Some(client) = mcp_clients.get_mut(mcp_name) {
                    tool_found = true;
                    match client.call_tool(&tool_name, arguments) {
                        Ok(res) => { tool_output = Some(res); consecutive_failed_tools = 0; }
                        Err(e) => { tool_output = Some(format!("Ошибка '{}': {}", tool_name, e)); consecutive_failed_tools += 1; }
                    }
                }
            }
            // ─── FALLBACK: tool_name совпадает с ID сабагента → синтаксическая ошибка, reprompt ───
            if !tool_found {
                if agents.iter().any(|a| a.id == tool_name && a.id != agent.id) {
                    consecutive_failed_tools += 1;
                    log_cb(format!("🔄 Синтаксическая ошибка: '{}' использовал 'tool' для вызова сабагента '{}' вместо 'target'.", agent.name, tool_name));
                    if consecutive_failed_tools >= 3 {
                        final_response = format!("{} Синтаксическая ошибка (3 попытки): агент '{}' продолжает использовать 'tool' вместо 'target'. Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                        break;
                    }
                    llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: response.clone(), namespace: None, sub_calls: None, author: Some("assistant".to_string()) });
                    llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: format!("⚠️ ОШИБКА_СИНТАКСИСА: ты использовал 'tool' для вызова сабагента '{}'. Это сабагент, а не инструмент. Исправь: используй 'target'. Пример: {{\"thought\": \"...\", \"target\": \"{}\", \"task_or_response\": \"...\"}}.", tool_name, tool_name), namespace: None, sub_calls: None, author: Some("user".to_string()) });
                    continue;
                }
            }
            let output = tool_output.unwrap_or_else(|| format!("Ошибка: Инструмент '{}' не найден.", tool_name));
            // ─── FOLD: ошибка инструмента — мгновенный проброс наверх ───
            if !tool_found || output.starts_with("Ошибка") {
                consecutive_failed_tools += 1;
                if consecutive_failed_tools >= 3 {
                    final_response = format!("{} Лимит неудачных вызовов инструмента ({}). Агент: '{}'. Инструмент: '{}'. Невозможно продолжить.", AGENT_ERROR_PREFIX, consecutive_failed_tools, agent.id, tool_name);
                    break;
                }
                tool_calls.push(ToolCallInfo { tool_name: tool_name.clone(), arguments: args_str, result: output.clone() });
                llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: response.clone(), namespace: None, sub_calls: None, author: Some("assistant".to_string()) });
                llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\n⚠️ Инструмент вернул ошибку. Используй другой инструмент или заверши через {{\"target\": \"reply\"}}.", tool_name, output), namespace: None, sub_calls: None, author: None });
                continue;
            }
            consecutive_failed_tools = 0;
            tool_calls.push(ToolCallInfo { tool_name: tool_name.clone(), arguments: args_str, result: output.clone() });
            llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: response.clone(), namespace: None, sub_calls: None, author: Some("assistant".to_string()) });
            llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\nЕсли задача выполнена — ответь ОБЫЧНЫМ ТЕКСТОМ.", tool_name, output), namespace: None, sub_calls: None, author: None });
            continue;
        }

        // ── 3c. RESPONSE DISPATCH: Orchestrator JSON (target) ──
        if let Some(parsed) = parse_orchestrator_response(&response) {
            action_found = true;
            consecutive_incomplete = 0;

            if parsed.target == "reply" || parsed.target == "user" {
                if parsed.content.is_empty() {
                    let fallback = get_agent_report_from_messages(messages, "visualizer_agent", "main")
                        .or_else(|| {
                            messages.iter().rev().find(|m| m.msg_type == "thought" && !m.content.is_empty() && m.content.len() > 10)
                                .map(|m| m.content.clone())
                        })
                        .unwrap_or_else(|| format!("{} Анализ завершен, но данных для ответа нет.", AGENT_ERROR_PREFIX));
                    final_response = fallback;
                } else {
                    final_response = parsed.content;
                }
                break;
            }

            // FIX A: Validate target BEFORE logging thought to UI
            if let Some(subagent) = agents.iter().find(|a| a.id == parsed.target) {
                consecutive_invalid_targets = 0;
                log_agent_thought(&log_cb, agent, "вызов", &parsed.target, &parsed.thought, gen_start.elapsed().as_secs_f32(), depth);
                thought_logged = true;

                // Skip if already called in this namespace
                let ens = parsed.namespace.as_deref().unwrap_or(current_namespace);
                if get_agent_report_from_messages(messages, &subagent.id, ens).is_some() {
                    llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: response.clone(), namespace: None, sub_calls: None, author: Some("assistant".to_string()) });
                    llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: format!(
                        "ВНИМАНИЕ: Сабагент '{}' уже вызывался в '{}'. НЕ вызывай повторно! Вызови другого или заверши через {{\"target\": \"reply\"}}.", subagent.id, ens), namespace: None, sub_calls: None, author: Some("user".to_string()) });
                    continue;
                }

                // Recurse into subagent
                let child_ns = parsed.namespace.as_deref().unwrap_or(current_namespace);
                log_cb(format!("📞 {} вызывает сабагента: {} (ns: {})", agent.name, subagent.name, child_ns));

                let start_len = all_sub_calls.len();
                let sub_result = run_agent_node(
                    log_cb.clone(), status_cb.clone(), subcall_cb.clone(),
                    engine, subagent, agents, parsed.content.clone(), vec![],
                    &[],
                    child_ns,
                    max_gen_tokens, model_params, format_type,
                    cancel_flag.clone(), depth + 1, all_sub_calls, Some(agent.name.clone()), mcp_servers_dir,
                    messages, msg_counter,
                )?;
                let end_len = all_sub_calls.len();
                let node_sub_calls = if start_len < end_len {
                    Some(all_sub_calls[start_len..end_len].to_vec())
                } else {
                    None
                };

                // ─── FOLD: ошибка сабагента — мгновенный проброс наверх ───
                if sub_result.starts_with(AGENT_ERROR_PREFIX) {
                    log_cb(format!("❌ Сабагент '{}' вернул ошибку — fold: {}", subagent.id, sub_result));
                    // Сохраняем ошибку в messages перед fold
                    let err_msg = ChatMessage {
                        id: Some(format!("msg_{}", msg_counter)),
                        msg_type: "thought".to_string(),
                        content: sub_result.clone(),
                        namespace: Some(child_ns.to_string()),
                        sub_calls: node_sub_calls.clone(),
                        author: Some(subagent.id.clone()),
                    };
                    messages.push(err_msg);
                    *msg_counter += 1;
                    final_response = sub_result;
                    break;
                }

                let msg = ChatMessage {
                    id: Some(format!("msg_{}", msg_counter)),
                    msg_type: "thought".to_string(),
                    content: sub_result.clone(),
                    namespace: Some(child_ns.to_string()),
                    sub_calls: node_sub_calls.clone(),
                    author: Some(subagent.id.clone()),
                };
                messages.push(msg);
                *msg_counter += 1;

                let new_sys = build_system_prompt(agent, messages, current_namespace, has_tools, &all_tools);
                if let Some(f) = llm_messages.first_mut() { if f.msg_type == "message" && f.author.as_deref() == Some("system") { f.content = new_sys; } }
                llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: response.clone(), namespace: None, sub_calls: None, author: Some("assistant".to_string()) });
                llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: format!("Отчет от {} (ns: {}):\n{}\n\nЕсли достаточно — ответь ОБЫЧНЫМ ТЕКСТОМ.", subagent.name, child_ns, truncate_result(&sub_result, 2000)), namespace: None, sub_calls: None, author: Some("user".to_string()) });
                continue;
            } else {
                // FIX A + D: Invalid target — DON'T log thought, limit retries, list valid agents
                consecutive_invalid_targets += 1;
                if consecutive_invalid_targets >= 3 {
                    log_cb(format!("❌ {} превысил лимит неверных target-вызовов (3).", agent.name));
                    final_response = format!("{} Агент '{}' вызывает несуществующего сабагента '{}'. Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id, parsed.target);
                    break;
                }
                llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: response.clone(), namespace: None, sub_calls: None, author: Some("assistant".to_string()) });
                let valid_ids = valid_agent_ids(agents, &agent.id, "primary");
                let error_msg = if valid_ids.is_empty() {
                    format!("Ошибка: Агент '{}' не найден.", parsed.target)
                } else {
                    format!("Ошибка: Агент '{}' не найден. Доступные агенты: {}. Ответь JSON с одним из них.", parsed.target, valid_ids.join(", "))
                };
                llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: error_msg, namespace: None, sub_calls: None, author: Some("user".to_string()) });
                continue;
            }
        }

        // ── 3d. THOUGHT LOGGING (think tags, partial JSON without action) ──
        if !thought_logged {
            let extracted = extract_think_content(&raw_response);
            for t in &extracted {
                log_cb(format!("💭 Мысль {} [d={}] (размышление) [⏱{:.1}с]: {}", agent.name, depth, gen_start.elapsed().as_secs_f32(), t));
            }
            if extracted.is_empty() && !raw_response.contains("<think") {
                if let Some(t) = extract_thought_from_partial_json(&raw_response) {
                log_cb(format!("💭 Мысль {} [d={}] (размышление) [⏱{:.1}с]: {}", agent.name, depth, gen_start.elapsed().as_secs_f32(), t));
                }
            }
        }

        // ── 3e. HANDLE NO ACTION ──
        if !action_found && has_tools {
            if response.trim().is_empty() {
                consecutive_incomplete += 1;
                if consecutive_incomplete >= 5 {
                    final_response = format!("{} Агент '{}' не смог сформировать ответ (5 пустых попыток). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                    break;
                }
                llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: String::new(), namespace: None, sub_calls: None, author: Some("assistant".to_string()) });
                llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: "Ты размышлял, но не выдал ответ. ПРЕКРАТИ размышлять. Вызови сабагента через JSON или ответь ОБЫЧНЫМ ТЕКСТОМ.".to_string(), namespace: None, sub_calls: None, author: Some("user".to_string()) });
                continue;
            }

            if has_incomplete_json_action(&response) || has_json_thought_without_action(&response) {
                consecutive_incomplete += 1;
                if consecutive_incomplete >= 5 {
                    final_response = format!("{} Агент '{}' не смог завершить действие (5 попыток). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                    break;
                }
                llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: response.clone(), namespace: None, sub_calls: None, author: Some("assistant".to_string()) });
                llm_messages.push(ChatMessage { id: None, msg_type: "message".to_string(), content: "Ты начал размышлять в JSON, но не указал действие. Пиши кратко и СРАЗУ укажи \"target\" или \"tool\".".to_string(), namespace: None, sub_calls: None, author: Some("user".to_string()) });
                continue;
            }

        }

        // ── 3f. COMPLETION (worker/primary with plain text) ──
        let preview = safe_truncate(&response, 300).replace('\n', " ");
        log_cb(format!("✅ Агент {} завершил ответом ({} символов): {}", agent.name, response.len(), preview));
        final_response = response;
        break;
    }

    // ═══════════════════════════════════════
    //  PHASE 4: TEARDOWN
    // ═══════════════════════════════════════
    if depth > 0 {
        let subcall = SubCall { agent_name: agent.name.clone(), prompt: initial_dump.trim().to_string(), response: final_response.clone(), time_sec: start_time.elapsed().as_secs_f32(), tool_calls };
        subcall_cb(&subcall);
        all_sub_calls.push(subcall);
    }

    Ok(final_response)
}