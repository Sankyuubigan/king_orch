use crate::agent_manager::{build_l0_manifest, load_agents, AgentProfile};
use crate::llm::{ChatMessage, LlamaEngine, SubCall, ToolCallInfo};
use crate::parsers::{clean_thought_tags, parse_orchestrator_response, parse_tool_call};
use crate::{emit_log, emit_status};
use crate::config::ModelParams;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};

const TRUTH_PROTOCOL: &str = "YOU SHOULD: Tell the truth; never guess or speculate. Say 'I don't know' or 'I cannot confirm this' when something cannot be verified. Prioritize accuracy over speed. YOU MUST AVOID: Fabricating facts, making confident statements without proof, answering if unsure without disclosing uncertainty.";

#[derive(serde::Serialize, Clone)]
struct AgentThoughtPayload {
    agent_name: String,
    thought: String,
}

fn render_status_dossier(dossier: &HashMap<String, String>) -> String {
    if dossier.is_empty() {
        return "[СТАТУС ДОСЬЕ]: Пусто — данные ещё не собраны.".to_string();
    }
    let mut lines = vec!["[СТАТУС ДОСЬЕ]:".to_string()];
    for (key, value) in dossier {
        if value.is_empty() {
            lines.push(format!("- {}: ❌ Нет данных", key));
        } else {
            lines.push(format!("- {}: ✅ Данные собраны", key));
        }
    }
    lines.join("\n")
}

fn render_full_dossier(dossier: &HashMap<String, String>) -> String {
    if dossier.is_empty() {
        return "[ТЕКУЩЕЕ ДОСЬЕ]: Пусто.".to_string();
    }
    let mut lines = vec!["[ТЕКУЩЕЕ ДОСЬЕ]:".to_string()];
    for (key, value) in dossier {
        if value.is_empty() {
            lines.push(format!("--- {} ---\n(Нет данных)", key));
        } else {
            lines.push(format!("--- {} ---\n{}", key, value));
        }
    }
    lines.join("\n\n")
}

fn build_system_prompt(
    agent: &AgentProfile,
    dossier: &HashMap<String, String>,
    has_subagents: bool,
    has_tools: bool,
    filtered_agents: &[AgentProfile],
    all_tools: &[(String, String, serde_json::Value)],
) -> String {
    let mut sp = agent.system_prompt.clone();
    sp.push_str("\n\n[ПРОТОКОЛ ЧЕСТНОСТИ]\n");
    sp.push_str(TRUTH_PROTOCOL);

    match agent.mode.as_str() {
        "router" => sp.push_str(&format!("\n\n{}\n", render_status_dossier(dossier))),
        "worker" => sp.push_str(&format!("\n\n{}\n", render_full_dossier(dossier))),
        _ => sp.push_str(&format!("\n\n{}\n", render_full_dossier(dossier))),
    }

    if has_subagents || has_tools {
        sp.push_str("\n\n[ПРАВИЛА ВЫЗОВА]\n");
        sp.push_str("Если тебе нужно вызвать сабагента или инструмент — верни ОДИН JSON-блок (внутри ```json ... ```).\n");
        sp.push_str("В JSON обязательно должно быть поле \"thought\" с кратким объяснением.\n");
        sp.push_str("\n⚠️ ВАЖНО: Если задача ВЫПОЛНЕНА и ты хочешь просто ответить пользователю — пиши ОБЫЧНЫЙ ТЕКСТ без JSON! НЕ используй JSON с \"tool\": \"none\" — это вызовет ошибку!\n");
        
        if has_subagents {
            sp.push_str("\nДля вызова сабагента:\n```json\n{\"thought\": \"...\", \"target\": \"ID_САБАГЕНТА\", \"task_or_response\": \"ЗАДАЧА\"}\n```\n");
            sp.push_str("Для прямого ответа пользователю:\n```json\n{\"thought\": \"...\", \"target\": \"reply\", \"task_or_response\": \"ТВОЙ ОТВЕТ\"}\n```\n");
        }
        
        if agent.mode == "router" {
            sp.push_str("\n\n[АБСОЛЮТНОЕ ПРАВИЛО ДЛЯ МАРШРУТИЗАТОРА]\n");
            sp.push_str("Ты ВСЕГДА отвечаешь в формате JSON.\n");
            sp.push_str("НЕ используй форматы вроде <|channel>thought или <think/> — ТОЛЬКО JSON!\n");
        }
    }

    if has_subagents {
        sp.push_str("\n\n");
        sp.push_str(&build_l0_manifest(filtered_agents));
    }

    if has_tools {
        let mut td = String::from("[ДОСТУПНЫЕ ИНСТРУМЕНТЫ]\nДля вызова инструмента:\n```json\n{\"thought\": \"...\", \"tool\": \"ИМЯ\", \"arguments\": {\"ключ\": \"значение\"}}\n```\n\n");
        for (_, name, tool) in all_tools {
            let desc = tool.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let schema = tool.get("inputSchema").cloned().unwrap_or(serde_json::Value::Null);
            td.push_str(&format!("- \"{}\": {} | Параметры: {}\n\n", name, desc, serde_json::to_string(&schema).unwrap_or_default()));
        }
        sp.push_str("\n\n");
        sp.push_str(&td);
    }

    sp
}

fn get_mcp_server_path(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
    let possible_paths = vec![
        resource_dir.join("mcp_servers").join(format!("{}.cjs", name)),
        exe_dir.join("mcp_servers").join(format!("{}.cjs", name)),
        PathBuf::from("src-tauri").join("mcp_servers").join(format!("{}.cjs", name)),
        exe_dir.join("..").join("..").join("src-tauri").join("mcp_servers").join(format!("{}.cjs", name)),
        resource_dir.join("mcp_servers").join(format!("{}.js", name)),
        exe_dir.join("mcp_servers").join(format!("{}.js", name)),
        PathBuf::from("src-tauri").join("mcp_servers").join(format!("{}.js", name)),
        exe_dir.join("..").join("..").join("src-tauri").join("mcp_servers").join(format!("{}.js", name)),
    ];
    for path in possible_paths {
        if path.exists() { return Ok(path); }
    }
    Err(format!("MCP-сервер {}.cjs (и {}.js) не найден ни в одной директории", name, name))
}

pub fn run_chat(
    app: AppHandle,
    model_path: String,
    agent_id: String,
    user_text: String,
    history: Vec<ChatMessage>,
    context_size: u32,
    kv_quantization: bool,
    model_params: ModelParams,
    format_type: String,
    _conf_threshold: f32,
    cancel_flag: Arc<AtomicBool>,
    dossier: HashMap<String, String>,
) -> Result<(String, Vec<SubCall>, HashMap<String, String>), String> {
    emit_status(&app, "Загрузка модели в память...", 10);
    let engine = LlamaEngine::new(&model_path, context_size, kv_quantization)?;
    let agents = load_agents(&app);
    let max_gen_tokens = (context_size as usize).saturating_sub(2048).max(1024);

    let mut recent_history = history.clone();
    if recent_history.len() > 8 {
        recent_history = recent_history[recent_history.len() - 8..].to_vec();
    }

    if let Some(id) = agent_id.strip_prefix("agent_") {
        if let Some(primary_agent) = agents.iter().find(|a| a.id == id) {
            let mut all_sub_calls = Vec::new();
            let (final_res, final_dossier) = run_agent_node(
                &app, &engine, primary_agent, &agents, user_text, recent_history, dossier,
                max_gen_tokens, &model_params, &format_type, cancel_flag, 0, &mut all_sub_calls, None,
            )?;
            Ok((final_res, all_sub_calls, final_dossier))
        } else {
            Err(format!("Агент с ID '{}' не найден", id))
        }
    } else {
        Err("Неизвестный тип агента".to_string())
    }
}

#[allow(clippy::too_many_arguments)]
fn run_agent_node(
    app: &AppHandle,
    engine: &LlamaEngine,
    agent: &AgentProfile,
    agents: &[AgentProfile],
    user_text: String,
    history: Vec<ChatMessage>,
    mut current_dossier: HashMap<String, String>,
    max_gen_tokens: usize,
    model_params: &ModelParams,
    format_type: &str,
    cancel_flag: Arc<AtomicBool>,
    depth: usize,
    all_sub_calls: &mut Vec<SubCall>,
    caller_name: Option<String>,
) -> Result<(String, HashMap<String, String>), String> {
    if depth > 5 {
        return Err("Превышена максимальная глубина вложенности сабагентов".into());
    }
    emit_log(app, &format!("▶ Запуск агента: {} (mode: {}, глубина: {})", agent.name, agent.mode, depth));

    if agent.mode == "router" && !current_dossier.contains_key("user_query") {
        current_dossier.insert("user_query".to_string(), user_text.clone());
    }

    let allows_all = agent.subagents.iter().any(|s| s == "*");
    let filtered_agents: Vec<AgentProfile> = agents
        .iter()
        .filter(|a| a.id != agent.id && a.mode != "primary")
        .filter(|a| allows_all || agent.subagents.contains(&a.id))
        .cloned()
        .collect();

    let mut mcp_clients = std::collections::HashMap::new();
    let mut all_tools: Vec<(String, String, serde_json::Value)> = Vec::new();
    
    for mcp_name in &agent.mcp_servers {
        emit_log(app, &format!("⏳ Инициализация MCP сервера: {}", mcp_name));
        match get_mcp_server_path(app, mcp_name) {
            Ok(script_path) => {
                let target = env!("TARGET");
                let dev_name = format!("node-{}.exe", target);
                let mut node_path = PathBuf::from("node");
                if let Ok(mut exe) = std::env::current_exe() {
                    exe.pop();
                    for p in vec![exe.join("node.exe"), exe.join(&dev_name), exe.join("bin").join(&dev_name), PathBuf::from("bin").join(&dev_name)] {
                        if p.exists() { node_path = p; break; }
                    }
                }
                
                emit_log(app, &format!("   Путь к скрипту: {}", script_path.display()));
                
                match crate::mcp_client::McpClient::spawn(app, &node_path.to_string_lossy(), &[&script_path.to_string_lossy()]) {
                    Ok(mut client) => {
                        match client.list_tools() {
                            Ok(tools) => {
                                let mut loaded_tools = 0;
                                for tool in &tools {
                                    if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                                        all_tools.push((mcp_name.clone(), name.to_string(), tool.clone()));
                                        loaded_tools += 1;
                                    }
                                }
                                mcp_clients.insert(mcp_name.clone(), client);
                                emit_log(app, &format!("✅ MCP сервер '{}' запущен. Инструментов загружено: {}", mcp_name, loaded_tools));
                            }
                            Err(e) => {
                                emit_log(app, &format!("❌ Ошибка запроса списка инструментов у '{}': {}", mcp_name, e));
                            }
                        }
                    }
                    Err(e) => {
                        emit_log(app, &format!("❌ Критическая ошибка запуска MCP сервера '{}': {}", mcp_name, e));
                    }
                }
            }
            Err(e) => {
                emit_log(app, &format!("❌ Ошибка поиска файла сервера: {}", e));
            }
        }
    }

    let has_subagents = !filtered_agents.is_empty();
    let has_tools = !all_tools.is_empty();

    let system_prompt = build_system_prompt(agent, &current_dossier, has_subagents, has_tools, &filtered_agents, &all_tools);

    let mut messages = vec![ChatMessage { role: "system".to_string(), content: system_prompt.clone(), sub_calls: None, agent_name: None }];
    
    match agent.mode.as_str() {
        "router" | "worker" => {
            messages.push(ChatMessage { role: "user".to_string(), content: user_text.clone(), sub_calls: None, agent_name: caller_name.clone() });
        }
        _ => {
            messages.extend(history);
            messages.push(ChatMessage { role: "user".to_string(), content: user_text.clone(), sub_calls: None, agent_name: caller_name.clone() });
        }
    }

    // Добавляем инструкцию о JSON только к ПЕРВОМУ запросу (не после результатов инструментов)
    let needs_json_instruction = (agent.mode == "router" || agent.mode == "primary") && (has_subagents || has_tools);
    if needs_json_instruction {
        if let Some(last_msg) = messages.last_mut() {
            if last_msg.role == "user" {
                last_msg.content.push_str("\n\n[ВАЖНО]: Если нужно вызвать инструмент — ответь JSON. Если задача ясна — вызови инструмент сразу.");
            }
        }
    }

    let initial_context_dump = format!(
        "### [MODE: {}]\n### [DOSSIER]\n{}\n\n### [SYSTEM PROMPT]\n{}",
        agent.mode,
        if agent.mode == "router" { render_status_dossier(&current_dossier) } else { render_full_dossier(&current_dossier) },
        agent.system_prompt
    );

    let mut final_response = String::new();
    let mut tool_calls = Vec::new();
    let max_iterations = 30;
    let start_time = Instant::now();
    let mut consecutive_failed_tools = 0;
    const MAX_CONSECUTIVE_FAILED_TOOLS: usize = 3;

    for iter in 1..=max_iterations {
        if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }

        let app_clone = app.clone();
        let agent_name = agent.name.clone();
        let raw_response = engine.generate_chat(
            &messages, max_gen_tokens, model_params, format_type, cancel_flag.clone(),
            move |p, _| { emit_status(&app_clone, &format!("{} думает (Шаг {})...", agent_name, iter), 20 + (p * 0.1) as u8); },
        )?;

        let response = clean_thought_tags(&raw_response);

        // 1. Проверяем вызов инструмента
        if let Some((tool_name, arguments, thought)) = parse_tool_call(&response) {
            if !thought.is_empty() {
                emit_log(app, &format!("💭 Мысль {} (инструмент {}): {}", agent.name, tool_name, thought));
                let _ = app.emit("agent_thought", AgentThoughtPayload { agent_name: agent.name.clone(), thought });
            }
            emit_status(app, &format!("Выполнение {}...", tool_name), 60);
            let args_str = arguments.to_string();
            let mut tool_output = None;
            let mut tool_found = false;
            
            if let Some((mcp_name, _, _)) = all_tools.iter().find(|(_, name, _)| name == &tool_name) {
                if let Some(client) = mcp_clients.get_mut(mcp_name) {
                    tool_found = true;
                    emit_log(app, &format!("🔧 Передача параметров инструменту '{}'...", tool_name));
                    match client.call_tool(&tool_name, arguments) {
                        Ok(res) => {
                            emit_log(app, &format!("✅ Инструмент '{}' успешно отработал.", tool_name));
                            tool_output = Some(res);
                            consecutive_failed_tools = 0;
                        }
                        Err(e) => {
                            let err_msg = format!("Ошибка выполнения '{}': {}", tool_name, e);
                            emit_log(app, &format!("❌ {}", err_msg));
                            tool_output = Some(err_msg);
                            consecutive_failed_tools += 1;
                        }
                    }
                }
            }
            
            if !tool_found {
                consecutive_failed_tools += 1;
            }
            
            // Защита от бесконечного цикла неудачных вызовов
            if consecutive_failed_tools >= MAX_CONSECUTIVE_FAILED_TOOLS {
                let err = format!("⚠️ Превышен лимит неудачных вызовов инструментов ({}). Прекращаю попытки.", consecutive_failed_tools);
                emit_log(app, &err);
                final_response = err;
                break;
            }
            
            let output = tool_output.unwrap_or_else(|| {
                let err = format!("Ошибка: Инструмент '{}' не найден в загруженных MCP серверах.", tool_name);
                emit_log(app, &format!("❌ {}", err));
                err
            });
            
            tool_calls.push(ToolCallInfo { tool_name: tool_name.clone(), arguments: args_str, result: output.clone() });
            messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
            
            // КЛЮЧЕВОЕ ИЗМЕНЕНИЕ: После результата инструмента чётко говорим агенту что делать
            let tool_result_msg = format!(
                "[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\n{}\n{}", 
                tool_name, 
                output,
                if tool_found { "✅ Инструмент отработал." } else { "❌ Инструмент не найден или вызвал ошибку." },
                "Если задача выполнена — просто ответь ОБЫЧНЫМ ТЕКСТОМ без JSON. Если нужен другой инструмент — вызови его через JSON."
            );
            messages.push(ChatMessage { role: "user".to_string(), content: tool_result_msg, sub_calls: None, agent_name: None });
            continue;
        }

        // 2. Проверяем вызов сабагента / ответ reply
        if let Some((_conf, target, content, thought)) = parse_orchestrator_response(&response) {
            if !thought.is_empty() {
                emit_log(app, &format!("💭 Мысль {} (вызов {}): {}", agent.name, target, thought));
                let _ = app.emit("agent_thought", AgentThoughtPayload { agent_name: agent.name.clone(), thought });
            }

            if target == "reply" || target == "user" {
                final_response = content;
                break;
            } else if let Some(subagent) = agents.iter().find(|a| a.id == target) {
                if subagent.mode == "worker" && current_dossier.contains_key(&subagent.id) && !current_dossier[&subagent.id].is_empty() {
                    emit_log(app, &format!("🚫 Повторный вызов {} отклонён: данные уже в досье (✅).", subagent.id));
                    messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                    messages.push(ChatMessage {
                        role: "user".to_string(),
                        content: format!(
                            "ОШИБКА: Сабагент '{}' уже был вызван (✅). Вызови другого или верни ответ через {{\"target\": \"reply\"}}.",
                            subagent.id
                        ),
                        sub_calls: None,
                        agent_name: None,
                    });
                    continue;
                }

                let (sub_result, _sub_dossier) = run_agent_node(
                    app, engine, subagent, agents, content.clone(), vec![], current_dossier.clone(),
                    max_gen_tokens, model_params, format_type, cancel_flag.clone(), depth + 1, all_sub_calls,
                    Some(agent.name.clone()),
                )?;

                if subagent.mode == "worker" {
                    current_dossier.insert(subagent.id.clone(), sub_result.clone());
                    emit_log(app, &format!("📝 Досье обновлено: + {} (worker)", subagent.id));
                }
                if subagent.mode == "router" {
                    current_dossier.insert("pipeline_result".to_string(), sub_result.clone());
                    emit_log(app, &format!("📝 Досье обновлено: + pipeline_result (router)"));
                }

                if agent.mode == "primary" && subagent.mode == "router" {
                    let new_system = build_system_prompt(agent, &current_dossier, has_subagents, has_tools, &filtered_agents, &all_tools);
                    if let Some(first_msg) = messages.first_mut() {
                        if first_msg.role == "system" {
                            first_msg.content = new_system;
                        }
                    }
                    messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                    messages.push(ChatMessage { 
                        role: "user".to_string(), 
                        content: format!(
                            "Сабагент {} завершил работу. Прочитай досье и ответь клиенту простым языком. ОБЫЧНЫЙ ТЕКСТ без JSON.", 
                            subagent.name
                        ), 
                        sub_calls: None, 
                        agent_name: Some(subagent.name.clone()) 
                    });
                } else if agent.mode == "router" {
                    let new_system = build_system_prompt(agent, &current_dossier, has_subagents, has_tools, &filtered_agents, &all_tools);
                    messages = vec![
                        ChatMessage { role: "system".to_string(), content: new_system, sub_calls: None, agent_name: None },
                        ChatMessage {
                            role: "user".to_string(),
                            content: "Статус обновлен. Кого вызвать следующим? Или верни ответ через target: \"reply\".".to_string(),
                            sub_calls: None, agent_name: None,
                        },
                    ];
                } else {
                    messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                    messages.push(ChatMessage { role: "user".to_string(), content: format!("Отчет от {}:\n{}\n\nЕсли этого достаточно — ответь ОБЫЧНЫМ ТЕКСТОМ без JSON.", subagent.name, sub_result), sub_calls: None, agent_name: Some(subagent.name.clone()) });
                }
                continue;
            } else {
                messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                messages.push(ChatMessage { role: "user".to_string(), content: format!("Ошибка: Агент '{}' не найден. Ответь обычным текстом.", target), sub_calls: None, agent_name: None });
                continue;
            }
        }

        // 3. Пустой ответ
        if (has_subagents || has_tools) && response.trim().is_empty() {
            emit_log(app, &format!("⚠️ Агент {} выдал пустой ответ. Повторный запрос...", agent.name));
            messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
            messages.push(ChatMessage { role: "user".to_string(), content: "Ты не выдал ответ. Ответь обычным текстом или вызови инструмент.".to_string(), sub_calls: None, agent_name: None });
            continue;
        }

        // 4. Обычный текстовый ответ — ЗАВЕРШАЕМ ЦИКЛ
        final_response = response;
        break;
    }

    if depth > 0 {
        let sub_call = SubCall {
            agent_name: agent.name.clone(), prompt: initial_context_dump.trim().to_string(),
            response: final_response.clone(), time_sec: start_time.elapsed().as_secs_f32(), tool_calls,
        };
        let _ = app.emit("subcall_done", &sub_call);
        all_sub_calls.push(sub_call);
    }

    Ok((final_response, current_dossier))
}