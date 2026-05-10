use crate::llm::{LlamaEngine, ChatMessage, SubCall};
use crate::agent_manager::{load_agents, build_l0_manifest, AgentProfile};
use crate::processor::{emit_log, emit_status};
use crate::parsers::{parse_tool_call, parse_orchestrator_response};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use std::time::Instant;

const TRUTH_PROTOCOL: &str = "YOU SHOULD: Tell the truth; never guess or speculate. Say 'I don't know' or 'I cannot confirm this' when something cannot be verified. Prioritize accuracy over speed. YOU MUST AVOID: Fabricating facts, making confident statements without proof, answering if unsure without disclosing uncertainty.";

pub fn run_chat(
    app: AppHandle,
    model_path: String,
    agent_id: String,
    user_text: String,
    history: Vec<ChatMessage>,
    context_size: u32,
    kv_quantization: bool,
    temperature: f32,
    format_type: String,
    conf_threshold: f32,
    cancel_flag: Arc<AtomicBool>,
) -> Result<(String, Vec<SubCall>), String> {
    emit_status(&app, "Загрузка модели в память...", 10);
    let engine = LlamaEngine::new(&model_path, context_size, kv_quantization)?;
    let agents = load_agents(&app);

    let max_gen_tokens = (context_size as usize).saturating_sub(2048).max(1024);
    emit_log(&app, &format!("⚙️ Динамический контекст: выделено {} токенов для генерации ответа", max_gen_tokens));

    let mut recent_history = history.clone();
    if recent_history.len() > 8 {
        emit_status(&app, "Сжатие памяти чата (SimpleMem)...", 10);
        let compress_tokens = (context_size as usize / 4).max(1024);
        match compress_memory(&app, &engine, &recent_history, compress_tokens, &format_type, cancel_flag.clone()) {
            Ok(summary) => {
                let mut new_history = vec![ChatMessage {
                    role: "system".to_string(),
                    content: format!("[СИСТЕМНАЯ ПАМЯТЬ ПРОШЛЫХ СООБЩЕНИЙ]:\n{}", summary),
                    sub_calls: None,
                }];
                new_history.extend_from_slice(&recent_history[recent_history.len() - 2..]);
                recent_history = new_history;
            },
            Err(_) => {
                recent_history = recent_history[recent_history.len() - 8..].to_vec();
            }
        }
    }

    if let Some(id) = agent_id.strip_prefix("agent_") {
        if let Some(agent) = agents.iter().find(|a| a.id == id) {
            if agent.mode == "primary" {
                run_primary_agent_loop(&app, &engine, user_text, recent_history, agent, &agents, temperature, max_gen_tokens, &format_type, conf_threshold, cancel_flag)
            } else {
                let res = execute_agent_with_tools(&app, &engine, agent, user_text, recent_history, max_gen_tokens, &format_type, cancel_flag)?;
                Ok((res, vec![]))
            }
        } else {
            Err(format!("Агент с ID '{}' не найден", id))
        }
    } else {
        Err("Неизвестный тип агента".to_string())
    }
}

fn compress_memory(
    app: &AppHandle,
    engine: &LlamaEngine,
    history: &[ChatMessage],
    max_tokens: usize,
    format_type: &str,
    cancel_flag: Arc<AtomicBool>
) -> Result<String, String> {
    let mut text_to_summarize = String::new();
    for msg in history.iter().take(history.len() - 2) {
        text_to_summarize.push_str(&format!("{}: {}\n", msg.role, msg.content));
    }

    let prompt = format!(
        "Сделай краткую выжимку (summary) прошлой переписки. Сохрани ключевые факты.\nПЕРЕПИСКА:\n{}", text_to_summarize
    );

    let messages = vec![
        ChatMessage { role: "system".to_string(), content: "Ты — модуль памяти.".to_string(), sub_calls: None },
        ChatMessage { role: "user".to_string(), content: prompt, sub_calls: None }
    ];

    let app_clone = app.clone();
    engine.generate_chat(&messages, max_tokens, format_type, cancel_flag, move |p, _| {
        emit_status(&app_clone, "Сжатие памяти чата...", 10 + (p * 0.1) as u8);
    })
}

fn execute_agent_with_tools(
    app: &AppHandle,
    engine: &LlamaEngine,
    agent: &AgentProfile,
    user_text: String,
    history: Vec<ChatMessage>,
    max_gen_tokens: usize,
    format_type: &str,
    cancel_flag: Arc<AtomicBool>,
) -> Result<String, String> {
    emit_log(app, &format!("Работает агент: {}", agent.name));
    
    let mut system_prompt = agent.system_prompt.clone();
    system_prompt.push_str("\n\n[ПРОТОКОЛ ЧЕСТНОСТИ]\n");
    system_prompt.push_str(TRUTH_PROTOCOL);
    
    let tools_list = crate::tool_executor::get_available_tools(app);
    if !tools_list.is_empty() {
        system_prompt.push_str(&format!(
            "\n\n[ИНСТРУМЕНТЫ]\nДоступны скрипты: {}\nДля вызова верни JSON: {{\"tool\": \"название\", \"arg\": \"аргумент\"}}",
            tools_list.join(", ")
        ));
    } else {
        system_prompt.push_str("\n\n[ИНСТРУМЕНТЫ ОТСУТСТВУЮТ]\nУ тебя нет инструментов. ЗАПРЕЩЕНО выдумывать содержимое сайтов или файлов.");
    }

    let mut messages = vec![ChatMessage {
        role: "system".to_string(),
        content: system_prompt,
        sub_calls: None,
    }];
    messages.extend(history);
    messages.push(ChatMessage { role: "user".to_string(), content: user_text, sub_calls: None });

    let mut final_response = String::new();
    let max_tool_iterations = 5;

    for _ in 1..=max_tool_iterations {
        if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }

        let app_clone = app.clone();
        let response = engine.generate_chat(&messages, max_gen_tokens, format_type, cancel_flag.clone(), move |p, msg| {
            emit_status(&app_clone, msg, p as u8);
        })?;

        if let Some((tool, arg)) = parse_tool_call(&response) {
            emit_status(app, &format!("Выполнение {}...", tool), 60);
            let tool_output = crate::tool_executor::execute_tool(app, &tool, &arg).unwrap_or_else(|e| format!("Ошибка: {}", e));
            
            let truncated_output = if tool_output.chars().count() > 10000 {
                format!("{}...\n[ОБРЕЗАНО]", tool_output.chars().take(10000).collect::<String>())
            } else { tool_output };

            messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), sub_calls: None });
            messages.push(ChatMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ {}]:\n{}\nДай финальный ответ.", tool, truncated_output), sub_calls: None });
        } else {
            final_response = response;
            break;
        }
    }

    Ok(if final_response.is_empty() { "Ошибка: зацикливание инструментов".to_string() } else { final_response })
}

fn run_primary_agent_loop(
    app: &AppHandle,
    engine: &LlamaEngine,
    user_text: String,
    history: Vec<ChatMessage>,
    primary_agent: &AgentProfile,
    agents: &[AgentProfile],
    _temperature: f32,
    max_gen_tokens: usize,
    format_type: &str,
    conf_threshold: f32,
    cancel_flag: Arc<AtomicBool>,
) -> Result<(String, Vec<SubCall>), String> {
    emit_log(app, &format!("Запуск цикла Лид-Агента: {}...", primary_agent.name));

    let filtered_agents: Vec<AgentProfile> = agents.iter()
        .filter(|a| a.id != primary_agent.id && a.mode != "primary")
        .cloned().collect();
        
    let l0_manifest = build_l0_manifest(&filtered_agents);
    
    let system_prompt = format!(
        "{}\n\n\
        {}\n\n[ПРОТОКОЛ ЧЕСТНОСТИ]\n{}\n\n\
        ИНСТРУКЦИЯ ДЛЯ МАРШРУТИЗАЦИИ:\n\
        Отвечай СТРОГО в формате JSON. Выдай ровно ОДИН JSON с полями:\n\
        {{\n\
          \"confidence_score\": 0.95,\n\
          \"target\": \"ID_АГЕНТА или user\",\n\
          \"task_or_response\": \"Запрос для сабагента ИЛИ финальный ответ пользователю\"\n\
        }}\n\
        ЗАПРЕЩЕНО галлюцинировать. Если уверенность ниже {:.2}, система запросит уточнение у пользователя.",
        primary_agent.system_prompt, l0_manifest, TRUTH_PROTOCOL, conf_threshold
    );

    let mut orchestrator_messages = vec![ChatMessage { role: "system".to_string(), content: system_prompt, sub_calls: None }];
    orchestrator_messages.extend(history);
    orchestrator_messages.push(ChatMessage { role: "user".to_string(), content: user_text, sub_calls: None });

    let mut all_sub_calls = Vec::new();
    let max_iterations = 8; 
    
    for i in 1..=max_iterations {
        if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }

        let app_clone = app.clone();
        let orch_response = engine.generate_chat(&orchestrator_messages, max_gen_tokens, format_type, cancel_flag.clone(), move |p, _| {
            emit_status(&app_clone, &format!("Лид-Агенту нужно подумать (Шаг {})...", i), 20 + (p * 0.1) as u8);
        })?;

        emit_log(app, &format!("Сырой ответ Лид-Агента: {}", orch_response));

        let (confidence, target, content) = parse_orchestrator_response(&orch_response)
            .unwrap_or((1.0, "user".to_string(), "Ошибка формата JSON Лид-Агента. Повторите запрос.".to_string()));

        if confidence < conf_threshold {
            let interactive_msg = format!("⚠️ **Уверенность системы слишком мала ({:.2}).**\nПожалуйста, уточните запрос или дайте больше контекста.\n\n_Сомнения системы:_ {}", confidence, content);
            return Ok((interactive_msg, all_sub_calls));
        }

        if target == "user" {
            emit_status(app, "Ответ готов!", 100);
            return Ok((content, all_sub_calls));
        } else {
            if let Some(agent) = agents.iter().find(|a| a.id == target) {
                let start_time = Instant::now();
                
                // Вызываем сабагента (внутри он выполняет только 1 шаг и использует инструменты, если нужно)
                let sub_response = execute_agent_with_tools(
                    app, engine, agent, content.clone(), vec![], max_gen_tokens, format_type, cancel_flag.clone()
                )?;
                
                let elapsed = start_time.elapsed().as_secs_f32();

                let sub_call = SubCall {
                    agent_name: agent.name.clone(),
                    prompt: content.clone(),
                    response: sub_response.clone(),
                    time_sec: elapsed,
                };
                
                // Отправляем событие в UI, чтобы отчет появился в чате сразу же
                let _ = app.emit("subcall_done", &sub_call);
                all_sub_calls.push(sub_call);

                // Добавляем ответ сабагента в историю Лид-Агента
                orchestrator_messages.push(ChatMessage { role: "assistant".to_string(), content: orch_response.clone(), sub_calls: None });
                orchestrator_messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: format!("Результат работы сабагента {}:\n{}\n\nПроанализируй результат. Если нужны еще данные от других сабагентов для завершения пайплайна — вызови следующего (target: \"ID_АГЕНТА\"). Если пайплайн завершен ИЛИ если сабагент в своем ответе просит задать уточняющий вопрос пользователю — ответь пользователю (target: \"user\"). Обязательно в формате JSON.", agent.name, sub_response),
                    sub_calls: None,
                });
            } else {
                orchestrator_messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: format!("Ошибка: Агент '{}' не найден. Выбери существующего из списка или ответь user.", target),
                    sub_calls: None,
                });
            }
        }
    }

    Err("Превышен лимит шагов Лид-Агента (Зацикливание). Проверьте промпты.".to_string())
}