mod prompt;
mod runtime;

use crate::domain::agent_manager::{load_agents, AgentProfile};
use crate::infra::{ChatMessage, LlamaEngine, ModelParams, SubCall, ToolCallInfo};
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

pub type Dossier = HashMap<String, HashMap<String, String>>;

fn safe_truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len { return s.to_string(); }
    let end = s.char_indices()
        .take_while(|(i, _)| *i < max_len)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(max_len.min(s.len()));
    format!("{}...", &s[..end])
}

fn clean_thought_for_log(thought: &str, max_len: usize) -> String {
    let trimmed = thought.trim();
    if trimmed.is_empty() { return String::new(); }
    let cleaned: String = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    safe_truncate(&cleaned, max_len)
}

#[allow(clippy::too_many_arguments)]
pub fn run_chat<L, S, C>(
    log_cb: L, status_cb: S, subcall_cb: C,
    agents_dir: std::path::PathBuf, mcp_servers_dir: std::path::PathBuf,
    model_path: String, agent_id: String, user_text: String, history: Vec<ChatMessage>,
    context_size: u32, kv_quantization: bool, model_params: ModelParams, format_type: String,
    _conf_threshold: f32, cancel_flag: Arc<AtomicBool>, dossier: Dossier,
) -> Result<(String, Vec<SubCall>, Dossier), String>
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
    let main_ns = initial_session_state.entry("main".to_string()).or_default();
    if !main_ns.contains_key("user_query") {
        let first_user_msg = history.iter()
            .find(|m| m.role == "user" && !m.content.trim().is_empty())
            .map(|m| m.content.clone())
            .unwrap_or_else(|| user_text.clone());
        main_ns.insert("user_query".to_string(), first_user_msg);
    }

    if let Some(id) = agent_id.strip_prefix("agent_") {
        if let Some(primary_agent) = agents.iter().find(|a| a.id == id) {
            let mut all_sub_calls = Vec::new();
            let (final_res, final_dossier) = run_agent_node(
                log_cb, status_cb, subcall_cb,
                &engine, primary_agent, &agents, user_text, recent_history,
                initial_session_state, "main", max_gen_tokens, &model_params, &format_type,
                cancel_flag, 0, &mut all_sub_calls, None, &mcp_servers_dir,
            )?;
            Ok((final_res, all_sub_calls, final_dossier))
        } else { Err(format!("Агент с ID '{}' не найден", id)) }
    } else { Err("Неизвестный тип агента".to_string()) }
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
fn run_agent_node<L, S, C>(
    log_cb: L, status_cb: S, subcall_cb: C,
    engine: &LlamaEngine, agent: &AgentProfile, agents: &[AgentProfile],
    user_text: String, history: Vec<ChatMessage>, mut current_dossier: Dossier,
    current_namespace: &str,
    max_gen_tokens: usize, model_params: &ModelParams, format_type: &str,
    cancel_flag: Arc<AtomicBool>, depth: usize,
    all_sub_calls: &mut Vec<SubCall>, caller_name: Option<String>,
    mcp_servers_dir: &Path,
) -> Result<(String, Dossier), String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    if depth > 5 { return Err("Превышена максимальная глубина вложенности сабагентов".into()); }
    log_cb(format!("▶ Запуск агента: {} (mode: {}, ns: {}, глубина: {})", agent.name, agent.mode, current_namespace, depth));

    if agent.mode == "router" {
        let main_ns = current_dossier.entry("main".to_string()).or_default();
        if !main_ns.contains_key("user_query") && !user_text.trim().is_empty() {
            main_ns.insert("user_query".to_string(), user_text.clone());
        }
    }

    let allows_all = agent.subagents.iter().any(|s| s == "*");
    let filtered_agents: Vec<AgentProfile> = agents.iter()
        .filter(|a| a.id != agent.id && a.mode != "primary")
        .filter(|a| allows_all || agent.subagents.contains(&a.id))
        .cloned().collect();

    let mut mcp_clients = HashMap::new();
    let mut all_tools: Vec<(String, String, serde_json::Value)> = Vec::new();
    runtime::load_mcp_servers(&log_cb, mcp_servers_dir, &agent.mcp_servers, &mut mcp_clients, &mut all_tools);

    let has_subagents = !filtered_agents.is_empty();
    let has_tools = !all_tools.is_empty();
    let system_prompt = build_system_prompt(agent, &current_dossier, current_namespace, has_subagents, has_tools, &filtered_agents, &all_tools);

    let mut messages = vec![ChatMessage { role: "system".to_string(), content: system_prompt.clone(), sub_calls: None, agent_name: None }];
    match agent.mode.as_str() {
        "router" | "worker" => { messages.push(ChatMessage { role: "user".to_string(), content: user_text.clone(), sub_calls: None, agent_name: caller_name.clone() }); }
        _ => { messages.extend(history); messages.push(ChatMessage { role: "user".to_string(), content: user_text.clone(), sub_calls: None, agent_name: caller_name.clone() }); }
    }
    if (agent.mode == "router" || agent.mode == "primary") && (has_subagents || has_tools) {
        if let Some(last) = messages.last_mut() { if last.role == "user" { last.content.push_str("\n\n[ВАЖНО]: Если нужен инструмент — ответь JSON."); } }
    }

    let initial_dump = format!("### [MODE: {} | NS: {}]\n### [SESSION_STATE]\n{}\n\n### [SYSTEM PROMPT]\n{}",
        agent.mode, current_namespace, prompt::render_session_state_full(&current_dossier, current_namespace), agent.system_prompt);

    let mut final_response = String::new();
    let mut tool_calls = Vec::new();
    let start_time = Instant::now();
    let mut consecutive_failed_tools = 0;
    let mut consecutive_incomplete = 0;

    for iter in 1..=30 {
        if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }

        let raw_response = engine.generate_chat(
            &messages, max_gen_tokens, model_params, format_type, cancel_flag.clone(),
            |p, _| { status_cb(format!("{} думает (Шаг {})...", agent.name, iter), 20 + (p * 0.1) as u8); },
            log_cb.clone(),
        )?;

        let response = clean_thought_tags(&raw_response);
        let mut action_found = false;
        let mut thought_logged = false;

        if let Some((tool_name, arguments, thought)) = parse_tool_call(&response) {
            action_found = true;
            if !thought.is_empty() {
                log_cb(format!("💭 Мысль {} (инструмент {}): {}", agent.name, tool_name, clean_thought_for_log(&thought, 500)));
                thought_logged = true;
            }
            consecutive_incomplete = 0;
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
            tool_calls.push(ToolCallInfo { tool_name: tool_name.clone(), arguments: args_str, result: output.clone() });
            messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
            messages.push(ChatMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\nЕсли задача выполнена — ответь ОБЫЧНЫМ ТЕКСТОМ.", tool_name, output), sub_calls: None, agent_name: None });
            continue;
        }

        if let Some(parsed) = parse_orchestrator_response(&response) {
            action_found = true;
            if !parsed.thought.is_empty() {
                log_cb(format!("💭 Мысль {} (вызов {}): {}", agent.name, parsed.target, clean_thought_for_log(&parsed.thought, 500)));
                thought_logged = true;
            }
            consecutive_incomplete = 0;
            if parsed.target == "reply" || parsed.target == "user" { final_response = parsed.content; break; }

            if let Some(subagent) = agents.iter().find(|a| a.id == parsed.target) {
                if subagent.mode == "worker" {
                    let ens = parsed.namespace.as_deref().unwrap_or(current_namespace);
                    if let Some(ns_data) = current_dossier.get(ens) {
                        if ns_data.contains_key(&subagent.id) && !ns_data[&subagent.id].is_empty() {
                            messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                            messages.push(ChatMessage { role: "user".to_string(), content: format!(
                                "ВНИМАНИЕ: Сабагент '{}' уже вызывался в '{}' (✅). НЕ вызывай повторно! Вызови другого или заверши через {{\"target\": \"reply\"}}.", subagent.id, ens), sub_calls: None, agent_name: None });
                            continue;
                        }
                    }
                }

                let child_ns = parsed.namespace.as_deref().unwrap_or(current_namespace);
                log_cb(format!("📞 {} вызывает сабагента: {} (ns: {})", agent.name, subagent.name, child_ns));

                let (sub_result, sub_dossier) = run_agent_node(
                    log_cb.clone(), status_cb.clone(), subcall_cb.clone(),
                    engine, subagent, agents, parsed.content.clone(), vec![],
                    current_dossier.clone(), child_ns,
                    max_gen_tokens, model_params, format_type,
                    cancel_flag.clone(), depth + 1, all_sub_calls, Some(agent.name.clone()), mcp_servers_dir,
                )?;

                if subagent.mode == "worker" {
                    current_dossier.entry(child_ns.to_string()).or_default().insert(subagent.id.clone(), sub_result.clone());
                }
                if subagent.mode == "router" {
                    for (ns, map) in sub_dossier { current_dossier.entry(ns).or_default().extend(map); }
                }

                // Обновляем системный промпт с новым досье для всех режимов
                let new_sys = build_system_prompt(agent, &current_dossier, current_namespace, has_subagents, has_tools, &filtered_agents, &all_tools);
                if let Some(f) = messages.first_mut() { if f.role == "system" { f.content = new_sys; } }

                if agent.mode == "primary" {
                    // Primary агент НЕ должен видеть сырые отчёты — он читает данные из досье
                    messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                    messages.push(ChatMessage { role: "user".to_string(), content: format!(
                        "Сабагент {} (ns: {}) завершил работу. [СОСТОЯНИЕ СЕССИИ] обновлено.\n\
                        ⚠️ НЕ копируй технические отчёты! Перечитай [СОСТОЯНИЕ СЕССИИ] и сформулируй ответ пользователю ОБЫЧНЫМ ТЕКСТОМ на основе данных из досье.",
                        subagent.name, child_ns), sub_calls: None, agent_name: None });
                } else if agent.mode == "router" {
                    messages = vec![
                        ChatMessage { role: "system".to_string(), content: messages.first().map(|m| m.content.clone()).unwrap_or_default(), sub_calls: None, agent_name: None },
                        ChatMessage { role: "user".to_string(), content: format!(
                            "Сабагент {} (ns: {}) завершил.\nПревью:\n{}\n\n⚠️ Перечитай [СОСТОЯНИЕ СЕССИИ]. Если есть незавершённые шаги — вызови следующего. Если всё готово — {{\"target\": \"reply\"}}.",
                            subagent.name, child_ns, truncate_result(&sub_result, 1500)), sub_calls: None, agent_name: None }
                    ];
                } else {
                    messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                    messages.push(ChatMessage { role: "user".to_string(), content: format!("Отчет от {} (ns: {}):\n{}\n\nЕсли достаточно — ответь ОБЫЧНЫМ ТЕКСТОМ.", subagent.name, child_ns, truncate_result(&sub_result, 2000)), sub_calls: None, agent_name: Some(subagent.name.clone()) });
                }
                continue;
            } else {
                messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                messages.push(ChatMessage { role: "user".to_string(), content: format!("Ошибка: Агент '{}' не найден.", parsed.target), sub_calls: None, agent_name: None });
                continue;
            }
        }

        if !thought_logged {
            for t in extract_think_content(&raw_response) {
                log_cb(format!("💭 Мысль {} (размышление): {}", agent.name, clean_thought_for_log(&t, 500)));
            }
            if !raw_response.contains("<think") {
                if let Some(t) = extract_thought_from_partial_json(&raw_response) {
                    log_cb(format!("💭 Мысль {} (размышление): {}", agent.name, clean_thought_for_log(&t, 500)));
                }
            }
        }

        if !action_found && (has_subagents || has_tools) {
            if response.trim().is_empty() {
                consecutive_incomplete += 1;
                if consecutive_incomplete >= 5 {
                    final_response = "⚠️ Агент не смог сформировать ответ. Попробуйте переформулировать запрос.".to_string();
                    break;
                }
                messages.push(ChatMessage { role: "assistant".to_string(), content: String::new(), sub_calls: None, agent_name: None });
                messages.push(ChatMessage { role: "user".to_string(), content: "Ты размышлял, но не выдал ответ. ПРЕКРАТИ размышлять. Вызови сабагента через JSON или ответь ОБЫЧНЫМ ТЕКСТОМ.".to_string(), sub_calls: None, agent_name: None });
                continue;
            }

            if has_incomplete_json_action(&response) || has_json_thought_without_action(&response) {
                consecutive_incomplete += 1;
                if consecutive_incomplete >= 5 {
                    final_response = "⚠️ Агент не смог завершить действие. Попробуйте переформулировать запрос.".to_string();
                    break;
                }
                messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                messages.push(ChatMessage { role: "user".to_string(), content: "Ты начал размышлять в JSON, но не указал действие. Пиши кратко и СРАЗУ укажи \"target\" или \"tool\".".to_string(), sub_calls: None, agent_name: None });
                continue;
            }
        }

        log_cb(format!("✅ Агент {} завершил ответом ({} символов)", agent.name, response.len()));
        final_response = response;
        break;
    }

    if depth > 0 {
        let subcall = SubCall { agent_name: agent.name.clone(), prompt: initial_dump.trim().to_string(), response: final_response.clone(), time_sec: start_time.elapsed().as_secs_f32(), tool_calls };
        subcall_cb(&subcall);
        all_sub_calls.push(subcall);
    }

    Ok((final_response, current_dossier))
}