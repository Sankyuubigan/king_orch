use crate::agent_manager::{build_l0_manifest, load_agents, AgentProfile};
use crate::llm::{ChatMessage, LlamaEngine, SubCall, ToolCallInfo};
use crate::parsers::{extract_state_update, parse_orchestrator_response, parse_tool_call};
use crate::{emit_log, emit_status};
use crate::config::ModelParams;
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

fn get_mcp_server_path(app: &AppHandle, name: &str) -> Result<std::path::PathBuf, String> {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
    
    let possible_paths = vec![
        resource_dir.join("mcp_servers").join(format!("{}.js", name)),
        exe_dir.join("mcp_servers").join(format!("{}.js", name)),
        PathBuf::from("src-tauri").join("mcp_servers").join(format!("{}.js", name)),
        exe_dir.join("..").join("..").join("src-tauri").join("mcp_servers").join(format!("{}.js", name)),
    ];

    for path in possible_paths {
        if path.exists() {
            return Ok(path);
        }
    }
    Err(format!("MCP-сервер {} не найден", name))
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
    current_state: String,
) -> Result<(String, Vec<SubCall>, String), String> {
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

            let (final_res, final_state) = run_agent_node(
                &app,
                &engine,
                primary_agent,
                &agents,
                user_text,
                recent_history,
                current_state,
                max_gen_tokens,
                &model_params,
                &format_type,
                cancel_flag,
                0,
                &mut all_sub_calls,
            )?;

            Ok((final_res, all_sub_calls, final_state))
        } else {
            Err(format!("Агент с ID '{}' не найден", id))
        }
    } else {
        Err("Неизвестный тип агента".to_string())
    }
}

fn run_agent_node(
    app: &AppHandle,
    engine: &LlamaEngine,
    agent: &AgentProfile,
    agents: &[AgentProfile],
    user_text: String,
    history: Vec<ChatMessage>,
    mut current_state: String,
    max_gen_tokens: usize,
    model_params: &ModelParams,
    format_type: &str,
    cancel_flag: Arc<AtomicBool>,
    depth: usize,
    all_sub_calls: &mut Vec<SubCall>,
) -> Result<(String, String), String> {
    if depth > 5 {
        return Err("Превышена максимальная глубина вложенности сабагентов (Зацикливание)".into());
    }
    emit_log(
        app,
        &format!("▶ Запуск агента: {} (Глубина: {})", agent.name, depth),
    );

    let mut system_prompt = agent.system_prompt.clone();
    system_prompt.push_str("\n\n[ПРОТОКОЛ ЧЕСТНОСТИ]\n");
    system_prompt.push_str(TRUTH_PROTOCOL);

    system_prompt.push_str(&format!(
        "\n\n[ТЕКУЩЕЕ СОСТОЯНИЕ СЕССИИ]:\n{}\n",
        if current_state.is_empty() { "Пусто" } else { &current_state }
    ));

    let allows_all = agent.subagents.iter().any(|s| s == "*");
    let filtered_agents: Vec<AgentProfile> = agents
        .iter()
        .filter(|a| a.id != agent.id && a.mode != "primary")
        .filter(|a| allows_all || agent.subagents.contains(&a.id))
        .cloned()
        .collect();

    let mut mcp_clients = std::collections::HashMap::new();
    let mut all_tools = Vec::new();
    let mut tools_desc = String::new();
    
    for mcp_name in &agent.mcp_servers {
        if let Ok(script_path) = get_mcp_server_path(app, mcp_name) {
            let target = env!("TARGET");
            let dev_name = format!("node-{}.exe", target);
            let mut node_path = PathBuf::from("node");
            if let Ok(mut exe) = std::env::current_exe() {
                exe.pop();
                let possible_paths = vec![
                    exe.join("node.exe"),
                    exe.join(&dev_name),
                    exe.join("bin").join(&dev_name),
                    PathBuf::from("bin").join(&dev_name),
                ];
                for p in possible_paths {
                    if p.exists() { node_path = p; break; }
                }
            }

            if let Ok(mut client) = crate::mcp_client::McpClient::spawn(
                app,
                &node_path.to_string_lossy(),
                &[&script_path.to_string_lossy()],
            ) {
                if let Ok(tools) = client.list_tools() {
                    for tool in &tools {
                        if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                            all_tools.push((mcp_name.clone(), name.to_string(), tool.clone()));
                        }
                    }
                    mcp_clients.insert(mcp_name.clone(), client);
                }
            }
        }
    }

    let has_subagents = !filtered_agents.is_empty();
    let has_tools = !all_tools.is_empty();

    if has_subagents || has_tools {
        system_prompt.push_str("\n\n[КРИТИЧЕСКОЕ ПРАВИЛО ВЫЗОВА JSON]\n");
        system_prompt.push_str("Если ты хочешь делегировать задачу сабагенту или использовать инструмент, ты ДОЛЖЕН вернуть СТРОГО ОДИН валидный JSON-блок (внутри тегов ```json ... ```).\n");
        system_prompt.push_str("В JSON обязательно должно быть поле \"thought\" с кратким объяснением твоих действий.\n");
        
        if has_subagents {
            system_prompt.push_str("\nДля вызова сабагента используй формат:\n```json\n{\n  \"thought\": \"вижу проблему, передаю специалисту X\",\n  \"target\": \"ID_САБАГЕНТА\",\n  \"task_or_response\": \"ПОДРОБНАЯ ЗАДАЧА СО ВСЕМ КОНТЕКСТОМ\"\n}\n```\n");
            system_prompt.push_str("Для ответа пользователю напрямую (без сабагента) просто пиши обычный текст, или используй target: \"reply\".\n");
        }
    }

    if has_subagents {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&build_l0_manifest(&filtered_agents));
    }

    if has_tools {
        tools_desc.push_str("[ДОСТУПНЫЕ ИНСТРУМЕНТЫ]\nДля вызова инструмента используй формат:\n```json\n{\n  \"thought\": \"нужно найти информацию\",\n  \"tool\": \"ИМЯ_ИНСТРУМЕНТА\",\n  \"arguments\": {\"ключ\": \"значение\"}\n}\n```\n");
        for (_, name, tool) in &all_tools {
            let desc = tool.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let schema = tool.get("inputSchema").cloned().unwrap_or(serde_json::Value::Null);
            tools_desc.push_str(&format!(
                "- Имя: \"{}\"\n  Описание: {}\n  Параметры: {}\n\n",
                name, desc, serde_json::to_string(&schema).unwrap_or_default()
            ));
        }
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&tools_desc);
    }

    let mut messages = vec![ChatMessage {
        role: "system".to_string(), content: system_prompt, sub_calls: None, agent_name: None,
    }];
    messages.extend(history);
    messages.push(ChatMessage {
        role: "user".to_string(), content: user_text.clone(), sub_calls: None, agent_name: None,
    });

    let mut initial_context_dump = String::new();
    initial_context_dump.push_str("### [ПРОТОКОЛ ЧЕСТНОСТИ]\n");
    initial_context_dump.push_str(TRUTH_PROTOCOL);
    initial_context_dump.push_str("\n\n### [ТЕКУЩЕЕ СОСТОЯНИЕ СЕССИИ]\n");
    initial_context_dump.push_str(if current_state.is_empty() { "Пусто" } else { &current_state });
    if has_subagents {
        initial_context_dump.push_str("\n\n### [МАНИФЕСТ САБАГЕНТОВ]\n");
        initial_context_dump.push_str(&build_l0_manifest(&filtered_agents));
    }
    if has_tools {
        initial_context_dump.push_str("\n\n### ");
        initial_context_dump.push_str(tools_desc.trim());
    }
    for msg in &messages {
        if msg.role != "system" {
            initial_context_dump.push_str(&format!("\n\n### [{}]\n{}", msg.role.to_uppercase(), msg.content));
        }
    }

    let mut final_response = String::new();
    let mut tool_calls = Vec::new();
    let max_iterations = 30;
    let start_time = Instant::now();

    for iter in 1..=max_iterations {
        if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }

        let app_clone = app.clone();
        let agent_name = agent.name.clone();
        let response = engine.generate_chat(
            &messages,
            max_gen_tokens,
            model_params,
            format_type,
            cancel_flag.clone(),
            move |p, _| {
                emit_status(&app_clone, &format!("{} думает (Шаг {})...", agent_name, iter), 20 + (p * 0.1) as u8);
            },
        )?;

        let (new_state_opt, clean_response) = extract_state_update(&response);
        if let Some(new_state) = new_state_opt {
            if agent.can_update_state {
                current_state = new_state;
                emit_log(app, &format!("📝 Агент {} обновил глобальное состояние сессии.", agent.name));
            }
        }

        if let Some((tool_name, arguments, thought)) = parse_tool_call(&clean_response) {
            if !thought.is_empty() {
                emit_log(app, &format!("💭 Мысль агента {} (инструмент {}): {}", agent.name, tool_name, thought));
                let _ = app.emit("agent_thought", AgentThoughtPayload { agent_name: agent.name.clone(), thought: thought.clone() });
            }

            emit_status(app, &format!("Выполнение {}...", tool_name), 60);
            let args_str = arguments.to_string();
            let mut tool_output = None;
            if let Some((mcp_name, _, _)) = all_tools.iter().find(|(_, name, _)| name == &tool_name) {
                if let Some(client) = mcp_clients.get_mut(mcp_name) {
                    tool_output = Some(client.call_tool(&tool_name, arguments).unwrap_or_else(|e| format!("Ошибка: {}", e)));
                }
            }
            let truncated_output = tool_output.unwrap_or_else(|| format!("Ошибка: Инструмент '{}' не найден.", tool_name));

            tool_calls.push(ToolCallInfo { tool_name: tool_name.clone(), arguments: args_str, result: truncated_output.clone() });
            messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
            messages.push(ChatMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ {}]:\n{}\nПродолжай работу.", tool_name, truncated_output), sub_calls: None, agent_name: None });
            continue;
        }

        if let Some((_conf, target, content, thought)) = parse_orchestrator_response(&clean_response) {
            if !thought.is_empty() {
                emit_log(app, &format!("💭 Мысль агента {} (вызов {}): {}", agent.name, target, thought));
                let _ = app.emit("agent_thought", AgentThoughtPayload { agent_name: agent.name.clone(), thought: thought.clone() });
            }

            if target == "reply" || target == "user" {
                final_response = content;
                break;
            } else if let Some(subagent) = agents.iter().find(|a| a.id == target) {
                let (sub_result, updated_state) = run_agent_node(
                    app, engine, subagent, agents, content.clone(), vec![], current_state.clone(),
                    max_gen_tokens, model_params, format_type, cancel_flag.clone(), depth + 1, all_sub_calls,
                )?;

                current_state = updated_state;
                messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                messages.push(ChatMessage { role: "user".to_string(), content: format!("Отчет от {}:\n{}\n\nАнализируй и продолжай.", subagent.name, sub_result), sub_calls: None, agent_name: None });
                continue;
            } else {
                messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None, agent_name: None });
                messages.push(ChatMessage { role: "user".to_string(), content: format!("Ошибка: Агент '{}' не найден.", target), sub_calls: None, agent_name: None });
                continue;
            }
        }

        final_response = clean_response;
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

    Ok((final_response, current_state))
}