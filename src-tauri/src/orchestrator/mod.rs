mod prompt;
mod runtime;

use crate::agent_manager::{load_agents, AgentProfile};
use crate::llm::{ChatMessage, LlamaEngine, SubCall};
use crate::parsers::{clean_thought_tags, has_incomplete_json_action, parse_orchestrator_response, parse_tool_call};
use crate::config::ModelParams;
use prompt::build_system_prompt;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[allow(clippy::too_many_arguments)]
pub fn run_chat<L, S, C>(
    log_cb: L, status_cb: S, subcall_cb: C,
    agents_dir: std::path::PathBuf, mcp_servers_dir: std::path::PathBuf,
    model_path: String, agent_id: String, user_text: String, history: Vec<ChatMessage>,
    context_size: u32, kv_quantization: bool, model_params: ModelParams, format_type: String,
    _conf_threshold: f32, cancel_flag: Arc<AtomicBool>, dossier: HashMap<String, String>,
) -> Result<(String, Vec<SubCall>, HashMap<String, String>), String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    status_cb("Загрузка модели в память...".to_string(), 10);
    let engine = LlamaEngine::new(&model_path, context_size, kv_quantization)?;
    let agents = load_agents(&agents_dir)?;
    let max_gen_tokens = (context_size as usize).saturating_sub(2048).max(1024);
    let mut recent_history = history.clone();
    if recent_history.len() > 8 { recent_history = recent_history[recent_history.len() - 8..].to_vec(); }
    let mut initial_session_state = dossier.clone();
    if !initial_session_state.contains_key("user_query") {
        initial_session_state.insert("user_query".to_string(), user_text.clone());
    }
    if let Some(id) = agent_id.strip_prefix("agent_") {
        if let Some(primary_agent) = agents.iter().find(|a| a.id == id) {
            let mut all_sub_calls = Vec::new();
            let (final_res, final_dossier) = run_agent_node(
                log_cb, status_cb, subcall_cb,
                &engine, primary_agent, &agents, user_text, recent_history,
                initial_session_state, max_gen_tokens, &model_params, &format_type,
                cancel_flag, 0, &mut all_sub_calls, None, &mcp_servers_dir,
            )?;
            Ok((final_res, all_sub_calls, final_dossier))
        } else { Err(format!("Агент с ID '{}' не найден", id)) }
    } else { Err("Неизвестный тип агента".to_string()) }
}

#[allow(clippy::too_many_arguments)]
fn run_agent_node<L, S, C>(
    log_cb: L, status_cb: S, subcall_cb: C,
    engine: &LlamaEngine, agent: &AgentProfile, agents: &[AgentProfile],
    user_text: String, history: Vec<ChatMessage>, mut current_dossier: HashMap<String, String>,
    max_gen_tokens: usize, model_params: &ModelParams, format_type: &str,
    cancel_flag: Arc<AtomicBool>, depth: usize,
    all_sub_calls: &mut Vec<SubCall>, caller_name: Option<String>,
    mcp_servers_dir: &Path,
) -> Result<(String, HashMap<String, String>), String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    if depth > 5 { return Err("Превышена максимальная глубина вложенности сабагентов".into()); }
    log_cb(format!("▶ Запуск агента: {} (mode: {}, глубина: {})", agent.name, agent.mode, depth));

    if agent.mode == "router" && !current_dossier.contains_key("user_query") {
        current_dossier.insert("user_query".to_string(), user_text.clone());
    }

    let allows_all = agent.subagents.iter().any(|s| s == "*");
    let filtered_agents: Vec<AgentProfile> = agents.iter()
        .filter(|a| a.id != agent.id && a.mode != "primary")
        .filter(|a| allows_all || agent.subagents.contains(&a.id))
        .cloned().collect();

    let mut mcp_clients = std::collections::HashMap::new();
    let mut all_tools: Vec<(String, String, serde_json::Value)> = Vec::new();
    runtime::load_mcp_servers(&log_cb, mcp_servers_dir, &agent.mcp_servers, &mut mcp_clients, &mut all_tools);

    let has_subagents = !filtered_agents.is_empty();
    let has_tools = !all_tools.is_empty();
    let system_prompt = build_system_prompt(agent, &current_dossier, has_subagents, has_tools, &filtered_agents, &all_tools);

    let mut messages = vec![ChatMessage { role: "system".to_string(), content: system_prompt.clone(), sub_calls: None, agent_name: None }];
    match agent.mode.as_str() {
        "router" | "worker" => {
            messages.push(ChatMessage { role: "user".to_string(), content: user_text.clone(), sub_calls: None, agent_name: caller_name.clone() });
        }
        _ => { messages.extend(history); messages.push(ChatMessage { role: "user".to_string(), content: user_text.clone(), sub_calls: None, agent_name: caller_name.clone() }); }
    }
    if (agent.mode == "router" || agent.mode == "primary") && (has_subagents || has_tools) {
        if let Some(last_msg) = messages.last_mut() { if last_msg.role == "user" { last_msg.content.push_str("\n\n[ВАЖНО]: Если нужен инструмент — ответь JSON."); } }
    }

    let initial_context_dump = format!("### [MODE: {}]\n### [SESSION_STATE]\n{}\n\n### [SYSTEM PROMPT]\n{}",
        agent.mode,
        prompt::render_session_state_full(&current_dossier),
        agent.system_prompt
    );

    let mut final_response = String::new();
    let mut tool_calls = Vec::new();
    let start_time = Instant::now();
    let mut consecutive_failed_tools = 0;

    for iter in 1..=30 {
        if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }

        let raw_response = engine.generate_chat(
            &messages, max_gen_tokens, model_params, format_type, cancel_flag.clone(),
            |p, _| { status_cb(format!("{} думает (Шаг {})...", agent.name, iter), 20 + (p * 0.1) as u8); },
            log_cb.clone(),
        )?;
        let response = clean_thought_tags(&raw_response);

        if let Some((tool_name, arguments, thought)) = parse_tool_call(&response) {
            if !thought.is_empty() { log_cb(format!("💭 Мысль {} (инструмент {}): {}", agent.name, tool_name, thought)); }
            status_cb(format!("Выполнение {}...", tool_name), 60);
            let args_str = arguments.to_string();
            let mut tool_output = None;
            let mut tool_found = false;
            if let Some((mcp_name, _, _)) = all_tools.iter().find(|(_, name, _)| name == &tool_name) {
                if let Some(client) = mcp_clients.get_mut(mcp_name) {
                    tool_found = true;
                    match client.call_tool(&tool_name, arguments) {
                        Ok(res) => { tool_output = Some(res); consecutive_failed_tools = 0; }
                        Err(e) => { tool_output = Some(format!("Ошибка '{}': {}", tool_name, e)); consecutive_failed_tools += 1; }
                    }
                }
            }
            if !tool_found { consecutive_failed_tools += 1; }
            if consecutive_failed_tools >= 3 { final_response = format!("⚠️ Лимит неудачных вызовов ({}).", consecutive_failed_tools); break; }
            let output = tool_output.unwrap_or_else(|| format!("Ошибка: Инструмент '{}' не найден.", tool_name));
            tool_calls.push(crate::llm::ToolCallInfo { tool_name: tool_name.clone(), arguments: args_str, result: output.clone() });
            messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
            messages.push(ChatMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\nЕсли задача выполнена — ответь ОБЫЧНЫМ ТЕКСТОМ.", tool_name, output), sub_calls: None, agent_name: None });
            continue;
        }

        if let Some((_conf, target, content, thought)) = parse_orchestrator_response(&response) {
            if !thought.is_empty() { log_cb(format!("💭 Мысль {} (вызов {}): {}", agent.name, target, thought)); }

            if target == "reply" || target == "user" { final_response = content; break; }

            if let Some(subagent) = agents.iter().find(|a| a.id == target) {
                if subagent.mode == "worker" && current_dossier.contains_key(&subagent.id) && !current_dossier[&subagent.id].is_empty() {
                    messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                    messages.push(ChatMessage { role: "user".to_string(), content: format!("ОШИБКА: Сабагент '{}' уже был вызван.", subagent.id), sub_calls: None, agent_name: None });
                    continue;
                }

                log_cb(format!("📞 {} вызывает сабагента: {}", agent.name, subagent.name));
                let (sub_result, sub_dossier) = run_agent_node(
                    log_cb.clone(), status_cb.clone(), subcall_cb.clone(),
                    engine, subagent, agents, content.clone(), vec![],
                    current_dossier.clone(), max_gen_tokens, model_params, format_type,
                    cancel_flag.clone(), depth + 1, all_sub_calls, Some(agent.name.clone()), mcp_servers_dir,
                )?;

                if subagent.mode == "worker" { current_dossier.insert(subagent.id.clone(), sub_result.clone()); }
                if subagent.mode == "router" { for (key, value) in sub_dossier { if !key.is_empty() { current_dossier.insert(key, value); } } }

                if agent.mode == "primary" && subagent.mode == "router" {
                    let new_system = build_system_prompt(agent, &current_dossier, has_subagents, has_tools, &filtered_agents, &all_tools);
                    if let Some(first_msg) = messages.first_mut() { if first_msg.role == "system" { first_msg.content = new_system; } }
                    messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                    messages.push(ChatMessage { role: "user".to_string(), content: format!("Сабагент {} завершил. Прочитай [СОСТОЯНИЕ СЕССИИ]. ОБЫЧНЫЙ ТЕКСТ.", subagent.name), sub_calls: None, agent_name: Some(subagent.name.clone()) });
                } else if agent.mode == "router" {
                    let new_system = build_system_prompt(agent, &current_dossier, has_subagents, has_tools, &filtered_agents, &all_tools);
                    messages = vec![
                        ChatMessage { role: "system".to_string(), content: new_system, sub_calls: None, agent_name: None },
                        ChatMessage { role: "user".to_string(), content: "Состояние обновлено. Кого вызвать следующим?".to_string(), sub_calls: None, agent_name: None }
                    ];
                } else {
                    messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                    messages.push(ChatMessage { role: "user".to_string(), content: format!("Отчет от {}:\n{}\n\nЕсли достаточно — ответь ОБЫЧНЫМ ТЕКСТОМ.", subagent.name, sub_result), sub_calls: None, agent_name: Some(subagent.name.clone()) });
                }
                continue;
            } else {
                messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                messages.push(ChatMessage { role: "user".to_string(), content: format!("Ошибка: Агент '{}' не найден.", target), sub_calls: None, agent_name: None });
                continue;
            }
        }

        if (has_subagents || has_tools) && response.trim().is_empty() {
            log_cb(format!("⚠️ {} вернул пустой ответ, перезапрашиваем (итерация {})", agent.name, iter));
            messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
            messages.push(ChatMessage { role: "user".to_string(), content: "Ты не выдал ответ. Ответь текстом или вызови инструмент.".to_string(), sub_calls: None, agent_name: None });
            continue;
        }

        if (has_subagents || has_tools) && has_incomplete_json_action(&response) {
            log_cb(format!("⚠️ {} вернул неполный JSON (thought без target/tool), перезапрашиваем (итерация {})", agent.name, iter));
            messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
            messages.push(ChatMessage { role: "user".to_string(), content: "Ты начал размышлять в JSON, но не указал действие. Верни ПОЛНЫЙ JSON с полем \"target\" (или \"tool\"), либо ответь ОБЫЧНЫМ ТЕКСТОМ без JSON.".to_string(), sub_calls: None, agent_name: None });
            continue;
        }

        log_cb(format!("✅ Агент {} завершил ответом ({} символов)", agent.name, response.len()));
        final_response = response;
        break;
    }

    if depth > 0 {
        let subcall = SubCall {
            agent_name: agent.name.clone(),
            prompt: initial_context_dump.trim().to_string(),
            response: final_response.clone(),
            time_sec: start_time.elapsed().as_secs_f32(),
            tool_calls,
        };
        subcall_cb(&subcall);
        all_sub_calls.push(subcall);
    }

    Ok((final_response, current_dossier))
}