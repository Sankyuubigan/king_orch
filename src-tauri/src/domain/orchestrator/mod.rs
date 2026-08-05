pub mod prompt;
mod runtime;

use crate::domain::agent_manager::{load_agents, AgentProfile};
use crate::domain::workflow_engine::{
    find_workflow_by_stem, load_workflows, WorkflowContext, WorkflowRunner, NodeType, WorkflowDef,
};
use crate::infra::{ChatMessage, ChatAttachment, LlamaEngine, ModelParams, SubCall, ToolCallInfo, push_report, LlmMessage, extract_model_filename, llm_history};
use crate::domain::parsers::{
    clean_thought_tags, extract_think_content, extract_thought_from_partial_json,
    has_incomplete_json_action, parse_orchestrator_response, parse_tool_call, strip_tool_call,
};
use prompt::build_system_prompt;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Метаданные текущего стрима: куда выводить токены.
/// `kind == "message"` → печатать в основной чат юзеру.
/// `kind == "thought"` → печатать в блок «Мысли агентов».
/// Пустой `kind` → не стримить вообще (внутренние вызовы: fact-extractor и т.п.).
#[derive(Clone, Default)]
pub struct StreamMeta {
    pub kind: String,
    pub author: String,
    /// Накопленный сырой текст текущего стрима. Нужен, чтобы фильтровать
    /// служебные теги LLM (`<|channel>...`, `<|turn>`) по ПОЛНОМУ тексту,
    /// а не по отдельному чанку (тег может быть разорван между чанками).
    pub buffer: String,
    /// Флаг: `<think>` уже закрылся. Используется в stream_cb для
    /// динамического переключения kind с "thought" на "message".
    pub thinking_done: bool,
}

/// Восстанавливает предыдущее значение `StreamMeta` при выходе из узла/агента,
/// чтобы вложенные сабагенты не оставляли флаг включённым навсегда.
struct StreamGuard {
    meta: Arc<Mutex<StreamMeta>>,
    prev: StreamMeta,
}
impl Drop for StreamGuard {
    fn drop(&mut self) {
        if let Ok(mut m) = self.meta.lock() {
            *m = self.prev.clone();
        }
    }
}

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

/// Компактное описание вызова агента для subcall (вместо полной копии системного
/// промпта): только task и injected_reports — то, что реально различается между
/// вызовами. Источник промпта — сам .md файл агента (SSOT).
fn build_invocation_dump(task: &str, injected_reports: &str) -> String {
    let mut dump = String::new();
    if !task.trim().is_empty() {
        dump.push_str(&format!("### [ЗАДАЧА]\n{}\n\n", task.trim()));
    }
    if !injected_reports.trim().is_empty() {
        dump.push_str(&format!("### [ОТЧЕТЫ КОЛЛЕГ]\n{}\n\n", injected_reports.trim()));
    }
    if dump.is_empty() {
        dump.push_str("(вызов без задачи)");
    }
    dump.trim().to_string()
}

fn log_agent_thought(log_cb: &dyn Fn(String), agent: &AgentProfile, action_type: &str, target: &str, thought: &str, thinking_sec: f32, depth: usize) {
    if thought.is_empty() { return; }
    if thinking_sec > 0.0 {
        log_cb(format!("💭 Мысль {} [d={}] ({} {}) [⏱{:.1}с]: {}", agent.name, depth, action_type, target, thinking_sec, thought));
    } else {
        log_cb(format!("💭 Мысль {} [d={}] ({} {}): {}", agent.name, depth, action_type, target, thought));
    }
}

fn valid_agent_ids(agents: &[AgentProfile], exclude_id: &str, exclude_mode: &str) -> Vec<String> {
    agents.iter()
        .filter(|a| a.id != exclude_id && a.mode != exclude_mode)
        .map(|a| a.id.clone())
        .collect()
}

/// Режим графа: системный промпт самого «тяжёлого» агента графа (worst-case).
/// Пиковая VRAM определяется одним LLM-вызовом (движок работает последовательно),
/// поэтому берём агента с самым длинным системным промптом. Sub-workflow узлы
/// намеренно НЕ раскрываются — считаем только текущий граф.
pub fn build_worst_agent_prompt(
    agents: &[AgentProfile],
    wf: &WorkflowDef,
    history: &[ChatMessage],
) -> String {
    let worst_prompt = wf.nodes.iter()
        .filter(|n| n.node_type == NodeType::LlmWorker)
        .filter_map(|n| n.agent.as_deref())
        .filter_map(|aid| agents.iter().find(|a| a.id == aid))
        .map(|agent| build_system_prompt(agent, history, false, &[], 2048))
        .max_by_key(|sp| sp.chars().count());

    // Граф без llm_worker-узлов → пустой системный промпт (посчитаются только история + сообщение)
    worst_prompt.unwrap_or_default()
}

/// Результат чата: текст ответа + собранные sub-calls + обновлённый массив сообщений
/// + диагностика движка (режим GPU/CPU и скорость) для UI-индикатора.
#[derive(Debug, Clone)]
pub struct ChatRunResult {
    pub text: String,
    pub sub_calls: Vec<SubCall>,
    pub messages: Vec<ChatMessage>,
    /// "gpu" / "cpu" — как реально работала модель в этом запросе
    pub engine_mode: String,
    /// Скорость последней генерации (tok/s)
    pub engine_tok_per_sec: f64,
    /// Причина CPU-режима (пусто, если GPU)
    pub engine_mode_detail: String,
}

#[allow(clippy::too_many_arguments)]
pub fn run_chat<L, S, C, ST>(
    log_cb: L, status_cb: S, subcall_cb: C, stream_cb: ST,
    agents_dir: std::path::PathBuf, mcp_servers_dir: std::path::PathBuf, bins_dir: std::path::PathBuf,
    engine_dir: std::path::PathBuf,
    model_path: String, agent_id: String, user_text: String, history: Vec<ChatMessage>,
    attachments: Vec<ChatAttachment>,
    context_size: u32, max_gen_tokens: u32, kv_quant_keys: bool, kv_quant_values: bool,     model_params: ModelParams, format_type: String,
    mmproj_path: Option<String>, cancel_flag: Arc<AtomicBool>,
    stream_meta: Arc<Mutex<StreamMeta>>,
) -> Result<ChatRunResult, String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
    ST: Fn(String) + Clone + Send + Sync + 'static,
{
    status_cb("Загрузка модели в память...".to_string(), 10);
    let agents = load_agents(&agents_dir)?;
    let max_gen_usize = max_gen_tokens as usize;
    let recent_history: Vec<ChatMessage> = history.iter()
        .filter(|m| m.msg_type != "thought")
        .cloned()
        .collect();
    let mut recent_history = recent_history;
    if recent_history.len() > 8 { recent_history = recent_history[recent_history.len() - 8..].to_vec(); }

    let workflows = load_workflows(&agents_dir).unwrap_or_default();
    let workflow_match = find_workflow_by_stem(&workflows, &agent_id).filter(|wf| wf.visible);

    // ── Worst-case оценка стартового контекста движка ──
    // Токенизатор живёт ВНУТРИ движка (llama-server) и недоступен до его старта,
    // а --ctx-size фиксируется при старте процесса. Оцениваем токены эвристикой
    // (~3 символа на токен) + запас на токены изображений. Точная подгонка
    // остаётся за циклом обрезки истории в run_agent_node (по точным /tokenize).
    let worst_system_prompt = match &workflow_match {
        Some(wf) => build_worst_agent_prompt(&agents, wf, &history),
        None => agents.iter().find(|a| a.id == agent_id)
            .map(|agent| build_system_prompt(agent, &history, false, &[], max_gen_usize))
            .unwrap_or_default(),
    };
    let history_chars: usize = llm_history(&history).iter().map(|m| m.content.chars().count()).sum();
    let total_chars = worst_system_prompt.chars().count() + history_chars + user_text.chars().count();
    let image_tokens = attachments.len() as u32 * 2048;
    let estimated_tokens = (total_chars / 3) as u32 + image_tokens;
    let engine_ctx_limit = (estimated_tokens + max_gen_tokens + 128).min(context_size).max(2048);
    log_cb(format!(
        "📐 Стартовый контекст движка: {} токенов (worst-case промпт ~{} символов, история ~{}, изображения ~{} токенов)",
        engine_ctx_limit, worst_system_prompt.chars().count(), history_chars, image_tokens
    ));

    let engine = if mmproj_path.is_some() {
        LlamaEngine::new_with_mmproj(&engine_dir, &model_path, mmproj_path.as_deref(), engine_ctx_limit, kv_quant_keys, kv_quant_values, &log_cb, stream_cb)?
    } else {
        LlamaEngine::new(&engine_dir, &model_path, engine_ctx_limit, kv_quant_keys, kv_quant_values, &log_cb, stream_cb)?
    };
    let mut messages_store = history.clone();
    for (i, msg) in messages_store.iter_mut().enumerate() {
        if msg.id.is_none() {
            msg.id = Some(format!("msg_{}", i));
        }
    }
    let mut msg_counter = messages_store.len() as u32;

    let actual_user_text = if user_text.is_empty() {
        history.iter()
            .rev()
            .find(|m| m.author.as_deref() == Some("user") && m.msg_type == "message")
            .map(|m| m.content.clone())
            .unwrap_or_default()
    } else {
        user_text.clone()
    };

    let mut all_sub_calls = Vec::new();

    // Загружаем пресеты параметров LLM из sampling_presets.json (рядом с agents/)
    let project_dir = agents_dir.parent().unwrap_or(&agents_dir);
    let sampling_presets = crate::infra::load_sampling_presets(project_dir);

    if let Some(workflow) = workflow_match {
        log_cb(format!("▶ Запуск workflow '{}' (entry: {})", workflow.name, agent_id));
        let mut ctx = WorkflowContext::new(
            actual_user_text.clone(),
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
            max_gen_tokens: max_gen_usize,
            model_params: &model_params,
            format_type: &format_type,
            cancel_flag: cancel_flag.clone(),
            mcp_servers_dir: &mcp_servers_dir,
            bins_dir: &bins_dir,
            all_sub_calls: &mut all_sub_calls,
            msg_counter: &mut msg_counter,
            stream_meta: stream_meta.clone(),
            sampling_presets: &sampling_presets,
        };
        crate::domain::workflow_engine::run_workflow(
            workflow, &mut ctx, &mut runner,
        )?;
        return Ok(ChatRunResult {
            text: String::new(),
            sub_calls: all_sub_calls,
            messages: ctx.messages,
            engine_mode: engine.engine_mode().to_string(),
            engine_tok_per_sec: engine.tok_per_sec(),
            engine_mode_detail: engine.engine_mode_detail().to_string(),
        });
    }

    if let Some(primary_agent) = agents.iter().find(|a| a.id == agent_id) {
        log_cb(format!("▶ Запуск агента: {}", primary_agent.name));
        log_cb(format!("DEBUG run_chat: history.len={}, msg_0_author={:?}", history.len(), history.first().map(|m| m.author.clone())));

        let final_res = run_agent_node(
            log_cb, status_cb, subcall_cb,
            &engine, primary_agent, &agents, user_text, recent_history,
            &attachments,
            max_gen_usize, &model_params, &format_type,
            cancel_flag, 0, &mut all_sub_calls, None, &mcp_servers_dir, &bins_dir,
            &mut messages_store, &mut msg_counter,
            String::new(),
            stream_meta.clone(), true,
        )?;
        
        let sub_calls_opt = if all_sub_calls.is_empty() { None } else { Some(all_sub_calls.clone()) };
            messages_store.push(ChatMessage {
                id: Some(format!("msg_{}", msg_counter)),
                msg_type: "message".to_string(),
                content: final_res.clone(),
                sub_calls: sub_calls_opt,
                author: Some(primary_agent.id.clone()),
                model: Some(extract_model_filename(&engine.model_path)),
            });
            Ok(ChatRunResult {
                text: final_res,
                sub_calls: all_sub_calls,
                messages: messages_store,
                engine_mode: engine.engine_mode().to_string(),
                engine_tok_per_sec: engine.tok_per_sec(),
                engine_mode_detail: engine.engine_mode_detail().to_string(),
            })
    } else {
        Err(format!("Entry point '{}' не найден: нет ни workflow, ни .md агента с таким ID", agent_id))
    }
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
    max_gen_tokens: usize, model_params: &ModelParams, format_type: &str,
    cancel_flag: Arc<AtomicBool>, depth: usize,
    all_sub_calls: &mut Vec<SubCall>, caller_name: Option<String>,
    mcp_servers_dir: &Path, bins_dir: &Path,
    messages: &mut Vec<ChatMessage>, msg_counter: &mut u32,
    injected_reports: String,
    stream_meta: Arc<Mutex<StreamMeta>>,
    allow_stream: bool,
) -> Result<String, String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    if depth > 5 { return Err("Превышена максимальная глубина вложенности сабагентов".into()); }
    log_cb(format!("▶ Запуск агента: {} (глубина: {})", agent.name, depth));

    // ── Маркер стрима: куда выводить токены этого агента ──
    let prev_meta = stream_meta.lock().map(|m| m.clone()).unwrap_or_default();
    {
        let mut m = stream_meta.lock().expect("stream_meta lock poisoned");
        m.kind = if allow_stream { "message" } else { "thought" }.to_string();
        m.author = agent.name.clone();
        m.thinking_done = !allow_stream;
        m.buffer.clear();
    }
    let _stream_guard = StreamGuard { meta: stream_meta.clone(), prev: prev_meta };

    let mut mcp_clients = HashMap::new();
    let mut all_tools: Vec<(String, String, serde_json::Value)> = Vec::new();
    runtime::load_mcp_servers(&log_cb, mcp_servers_dir, bins_dir, &agent.mcp_servers, &mut mcp_clients, &mut all_tools);

    let has_real_tools = !all_tools.is_empty() || !agent.tools.is_empty();

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

    let has_tools_for_prompt = has_real_tools;
    let mut system_prompt = build_system_prompt(agent, messages, has_tools_for_prompt, &all_tools, max_gen_tokens);
    if !injected_reports.is_empty() {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&injected_reports);
    }
    
    // Глобальное правило для всех мыслящих моделей, чтобы не пробивали лимит 2048 токенов
    system_prompt.push_str("\n\n[КРИТИЧЕСКОЕ ОГРАНИЧЕНИЕ]\nТвой максимальный лимит генерации строго ограничен. Твои внутренние размышления должны состоять максимум из 3-4 предложений, после чего сразу должен идти финальный ответ или вызов.");

    let mut llm_messages: Vec<LlmMessage> = vec![LlmMessage { role: "system".to_string(), content: system_prompt.clone() }];

    // История для LLM — единое правило (llm_history): не-thought сообщения, только content.
    for msg in llm_history(messages) {
        
        let actual_author = msg.author.as_deref().unwrap_or("user");
        let role;
        let mut content = msg.content.clone();

        if actual_author == "user" {
            role = "user";
        } else if actual_author == "system" {
            role = "system";
        } else if actual_author == agent.id || actual_author == agent.name || actual_author == "assistant" {
            role = "assistant";
        } else {
            role = "user";
            content = format!("[Контекст из чата. Предыдущий ответ от агента '{}']:\n{}", actual_author, content);
        }

        llm_messages.push(LlmMessage { role: role.to_string(), content });
    }

    let user_text_dup = llm_messages.last()
        .map(|m| m.role == "user" && m.content == user_text)
        .unwrap_or(false);
    if !user_text_dup && !user_text.is_empty() {
        llm_messages.push(LlmMessage { role: "user".to_string(), content: user_text.clone() });
    }

    // Invocation-дамп для subcall: только то, что различается между вызовами
    // (task + отчёты коллег). Полный системный промпт агента живёт в .md файле
    // (SSOT) — его копия в сессии раздувала бы JSON на 5-11KB за каждый вызов.
    let invocation_dump = build_invocation_dump(&user_text, &injected_reports);

    let mut final_response = String::new();
    let mut tool_calls = Vec::new();
    let start_time = Instant::now();
    let mut consecutive_failed_tools = 0;
    let mut consecutive_incomplete = 0;
    let mut consecutive_invalid_targets = 0;
    // Метка режима для лога пиков памяти: llm_worker графа зовёт run_agent_node
    // с caller_name == "workflow_engine", всё остальное — legacy (.md) режим.
    let mem_mode = if caller_name.as_deref() == Some("workflow_engine") { "graph" } else { "legacy" };

    for iter in 1..=30 {
        if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }

        let mut ideal_ctx;
        loop {
            let current_tokens = engine.get_tokens_count(&llm_messages, format_type).unwrap_or(0);
            ideal_ctx = (current_tokens as u32 + max_gen_tokens as u32 + 128).min(engine.global_ctx_limit);

            if current_tokens + max_gen_tokens <= ideal_ctx as usize || llm_messages.len() <= 2 {
                log_cb(format!("📊 Память: выделен KV-кэш на {} токенов (Промпт: {}, Резерв: {})", ideal_ctx, current_tokens, max_gen_tokens));
                break;
            }
            if llm_messages.len() > 2 {
                llm_messages.remove(1);
                log_cb("⚠️ Превышен лимит контекста! Удалено самое старое сообщение из памяти LLM.".to_string());
            } else {
                break;
            }
        }

        let gen_start = Instant::now();
        log_cb(format!(">>> [{}] LLM вызов #{}, msgs={}, max_gen={}, глубина={}", agent.name, iter, llm_messages.len(), max_gen_tokens, depth));
        let ctx_label = format!("{}:{}#{}", mem_mode, agent.name, iter);
        let gen = if !attachments.is_empty() && engine.is_multimodal() {
            engine.generate_chat_multimodal(
                &llm_messages, &attachments, max_gen_tokens, model_params, format_type, cancel_flag.clone(),
                &ctx_label,
                |p, _| { status_cb(format!("{} обрабатывает медиа (Шаг {})...", agent.name, iter), 20 + (p * 0.1) as u8); },
                log_cb.clone(),
            )?
        } else {
            engine.generate_chat(
                &llm_messages, max_gen_tokens, model_params, format_type, cancel_flag.clone(),
                &ctx_label,
                |p, _| { status_cb(format!("{} думает (Шаг {})...", agent.name, iter), 20 + (p * 0.1) as u8); },
                log_cb.clone(),
            )?
        };
        let raw_response = gen.text.clone();
        let stop_reason = gen.stop_reason.clone();

        log_cb(format!("<<< [{}] LLM за {:.1}с, ответ {} символов", agent.name, gen_start.elapsed().as_secs_f32(), raw_response.len()));

        let response = clean_thought_tags(&raw_response);
        let mut action_found = false;
        let mut thought_logged = false;

        if response.trim().is_empty() && (stop_reason == "STOP_WORD" || stop_reason == "MAX_TOKENS") {
            consecutive_incomplete += 1;
            if consecutive_incomplete >= 3 {
                final_response = format!("{} Агент '{}' не смог сформировать ответ (3 пустых попытки: стоп-слово/лимит токенов). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                break;
            }
            let hint = if stop_reason == "MAX_TOKENS" {
                "Твои размышления прерваны из-за лимита токенов. Сгенерируй ответ ЗАНОВО с самого начала. СИЛЬНО СОКРАТИ свои внутренние размышления (максимум 2-3 вывода) и сразу переходи к финальному результату."
            } else {
                "Ты прервал генерацию. Продолжи ответ ОБЫЧНЫМ ТЕКСТОМ без JSON."
            };
            llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
            llm_messages.push(LlmMessage { role: "user".to_string(), content: hint.to_string() });
            continue;
        }

        // Внедряем архитектурную проверку на обрыв (MAX_TOKENS)
        let mut is_valid_json = false;
        if response.contains('{') {
            is_valid_json = crate::domain::parsers::is_valid_json_action(&response);
        }
        
        let resp_trim = response.trim();
        let is_final_text = resp_trim.ends_with('.') || resp_trim.ends_with('!') || resp_trim.ends_with('?') || resp_trim.ends_with('"') || resp_trim.ends_with('\'') || resp_trim.ends_with('`');

        if stop_reason == "MAX_TOKENS" && !is_valid_json && !is_final_text {
            consecutive_incomplete += 1;
            if consecutive_incomplete >= 3 {
                final_response = format!("{} Агент '{}' не смог сформировать ответ (3 попытки оборваны по лимиту токенов). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                break;
            }
            llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
            llm_messages.push(LlmMessage { role: "user".to_string(), content: "Твой ответ прервался из-за лимита токенов. Сгенерируй его ЗАНОВО с самого начала. СИЛЬНО СОКРАТИ свои внутренние размышления (максимум 2-3 вывода) и сразу переходи к финальному результату.".to_string() });
            continue;
        }

        if let Some((tool_name, arguments, thought)) = parse_tool_call(&response) {
            action_found = true;
            consecutive_incomplete = 0;
            log_agent_thought(&log_cb, agent, "инструмент", &tool_name, &thought, gen_start.elapsed().as_secs_f32(), depth);
            thought_logged = true;

            status_cb(format!("Выполнение {}...", tool_name), 60);
            let args_str = arguments.to_string();
            let mut tool_output = None;
            let mut tool_found = false;

            if tool_name == "emit_signal" {
                tool_found = true;
                let mut key_val = arguments.get("key");
                let mut val_val = arguments.get("value");
                if key_val.is_none() && val_val.is_none() {
                    if let Some(props) = arguments.get("properties") {
                        key_val = props.get("key");
                        val_val = props.get("value");
                    }
                }
                let key = key_val
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty());
                let value = val_val
                    .filter(|v| !v.is_null());

                if let (Some(key), Some(value)) = (key, value) {
                    consecutive_failed_tools = 0;
                    let signal_msg = ChatMessage {
                        id: Some(format!("msg_{}", msg_counter)),
                        msg_type: "signal".to_string(),
                        content: serde_json::json!({key: value}).to_string(),
                        sub_calls: None,
                        author: Some(agent.id.clone()),
                        model: None,
                    };
                    messages.push(signal_msg);
                    *msg_counter += 1;

                    // Результат (анализ) агента в messages[] сохраняет вызывающий
                    // (узел workflow / legacy-коллер), иначе получается клон: один
                    // и тот же ответ дважды. Здесь остаётся только сигнал.
                    let (analysis, _) = strip_tool_call(&response);
                    let analysis = if analysis.trim().is_empty() {
                        if thought.is_empty() { response.clone() } else { thought.clone() }
                    } else {
                        analysis
                    };

                    log_cb(format!("💭 Мысль {} [d={}] (сигнал + анализ) [⏱{:.1}с]: {}", agent.name, depth, gen_start.elapsed().as_secs_f32(), safe_truncate(&analysis, 500)));
                    tool_calls.push(ToolCallInfo {
                        tool_name: "emit_signal".to_string(),
                        arguments: args_str.clone(),
                        result: format!("✅ Сигнал '{}' сохранён", key),
                    });
                    final_response = analysis;
                    break;
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
            if !tool_found {
                if agents.iter().any(|a| a.id == tool_name && a.id != agent.id) {
                    consecutive_failed_tools += 1;
                    log_cb(format!("🔄 Синтаксическая ошибка: '{}' использовал 'tool' для вызова сабагента '{}' вместо 'target'.", agent.name, tool_name));
                    if consecutive_failed_tools >= 3 {
                        final_response = format!("{} Синтаксическая ошибка (3 попытки): агент '{}' продолжает использовать 'tool' вместо 'target'. Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                        break;
                    }
                    llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                    llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("⚠️ ОШИБКА_СИНТАКСИСА: ты использовал 'tool' для вызова сабагента '{}'. Это сабагент, а не инструмент. Исправь: используй 'target'. Пример: {{\"thought\": \"...\", \"target\": \"{}\", \"task_or_response\": \"...\"}}.", tool_name, tool_name) });
                    continue;
                }
            }
            let output = tool_output.unwrap_or_else(|| format!("Ошибка: Инструмент '{}' не найден.", tool_name));
            if !tool_found || output.starts_with("Ошибка") {
                consecutive_failed_tools += 1;
                if consecutive_failed_tools >= 3 {
                    final_response = format!("{} Лимит неудачных вызовов инструмента ({}). Агент: '{}'. Инструмент: '{}'. Невозможно продолжить.", AGENT_ERROR_PREFIX, consecutive_failed_tools, agent.id, tool_name);
                    break;
                }
                tool_calls.push(ToolCallInfo { tool_name: tool_name.clone(), arguments: args_str, result: output.clone() });
                llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\n⚠️ Инструмент вернул ошибку. Используй другой инструмент или заверши через {{\"target\": \"reply\"}}.", tool_name, output) });
                continue;
            }
            consecutive_failed_tools = 0;
            tool_calls.push(ToolCallInfo { tool_name: tool_name.clone(), arguments: args_str, result: output.clone() });
            llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
            llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\nЕсли задача выполнена — ответь ОБЫЧНЫМ ТЕКСТОМ.", tool_name, output) });
            continue;
        }

        if let Some(parsed) = parse_orchestrator_response(&response) {
            action_found = true;
            consecutive_incomplete = 0;

            if parsed.target == "reply" || parsed.target == "user" {
                if parsed.content.is_empty() {
                    final_response = response.clone();
                } else {
                    final_response = parsed.content;
                }
                break;
            }

            if let Some(subagent) = agents.iter().find(|a| a.id == parsed.target) {
                consecutive_invalid_targets = 0;
                log_agent_thought(&log_cb, agent, "вызов", &parsed.target, &parsed.thought, gen_start.elapsed().as_secs_f32(), depth);
                thought_logged = true;

                log_cb(format!("📞 {} вызывает сабагента: {}", agent.name, subagent.name));

                let start_len = all_sub_calls.len();
                let sub_result = run_agent_node(
                    log_cb.clone(), status_cb.clone(), subcall_cb.clone(),
                    engine, subagent, agents, parsed.content.clone(), vec![],
                    &[],
                    max_gen_tokens, model_params, format_type,
                    cancel_flag.clone(), depth + 1, all_sub_calls, Some(agent.name.clone()), mcp_servers_dir, bins_dir,
                    messages, msg_counter,
                    String::new(),
                    stream_meta.clone(), false,
                )?;
                let end_len = all_sub_calls.len();
                let node_sub_calls = if start_len < end_len {
                    Some(all_sub_calls[start_len..end_len].to_vec())
                } else {
                    None
                };

                if sub_result.starts_with(AGENT_ERROR_PREFIX) {
                    log_cb(format!("❌ Сабагент '{}' вернул ошибку — fold: {}", subagent.id, sub_result));
                    let err_msg = ChatMessage {
                        id: Some(format!("msg_{}", msg_counter)),
                        msg_type: "thought".to_string(),
                        content: sub_result.clone(),
                        sub_calls: node_sub_calls.clone(),
                        author: Some(subagent.id.clone()),
                        model: Some(extract_model_filename(&engine.model_path)),
                    };
                    push_report(messages, err_msg, subagent.single_report);
                    *msg_counter += 1;
                    final_response = sub_result;
                    break;
                }

                let msg = ChatMessage {
                    id: Some(format!("msg_{}", msg_counter)),
                    msg_type: "thought".to_string(),
                    content: sub_result.clone(),
                    sub_calls: node_sub_calls.clone(),
                    author: Some(subagent.id.clone()),
                    model: Some(extract_model_filename(&engine.model_path)),
                };
                push_report(messages, msg, subagent.single_report);
                *msg_counter += 1;

                let new_sys = build_system_prompt(agent, messages, has_tools_for_prompt, &all_tools, max_gen_tokens);
                if let Some(f) = llm_messages.first_mut() { if f.role == "system" { f.content = new_sys; } }
                llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("Отчет от {}:\n{}\n\nЕсли достаточно — ответь ОБЫЧНЫМ ТЕКСТОМ.", subagent.name, truncate_result(&sub_result, 2000)) });
                continue;
            } else {
                consecutive_invalid_targets += 1;
                if consecutive_invalid_targets >= 3 {
                    log_cb(format!("❌ {} превысил лимит неверных target-вызовов (3).", agent.name));
                    final_response = format!("{} Агент '{}' вызывает несуществующего сабагента '{}'. Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id, parsed.target);
                    break;
                }
                llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                let valid_ids = valid_agent_ids(agents, &agent.id, "primary");
                let error_msg = if valid_ids.is_empty() {
                    format!("Ошибка: Агент '{}' не найден.", parsed.target)
                } else {
                    format!("Ошибка: Агент '{}' не найден. Доступные агенты: {}. Ответь JSON с одним из них.", parsed.target, valid_ids.join(", "))
                };
                llm_messages.push(LlmMessage { role: "user".to_string(), content: error_msg });
                continue;
            }
        }

        if !thought_logged && !response.is_empty() {
            let extracted = extract_think_content(&raw_response);
            for t in &extracted {
                log_cb(format!("💭 Мысль {} [d={}] (размышление) [⏱{:.1}с]: {}", agent.name, depth, gen_start.elapsed().as_secs_f32(), t));
                messages.push(ChatMessage {
                    id: Some(format!("msg_{}", msg_counter)),
                    msg_type: "thought".to_string(),
                    content: t.clone(),
                    sub_calls: None,
                    author: Some(agent.id.clone()),
                    model: Some(extract_model_filename(&engine.model_path)),
                });
                *msg_counter += 1;
            }
            if extracted.is_empty() && !raw_response.contains("<think") {
                if let Some(t) = extract_thought_from_partial_json(&raw_response) {
                    log_cb(format!("💭 Мысль {} [d={}] (размышление) [⏱{:.1}с]: {}", agent.name, depth, gen_start.elapsed().as_secs_f32(), t));
                    messages.push(ChatMessage {
                        id: Some(format!("msg_{}", msg_counter)),
                        msg_type: "thought".to_string(),
                        content: t.clone(),
                        sub_calls: None,
                        author: Some(agent.id.clone()),
                        model: Some(extract_model_filename(&engine.model_path)),
                    });
                    *msg_counter += 1;
                }
            }
        }

        if !action_found && response.trim().is_empty() {
            consecutive_incomplete += 1;
            if consecutive_incomplete >= 5 {
                final_response = format!("{} Агент '{}' не смог сформировать ответ (5 пустых попыток). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                break;
            }
            let hint = if stop_reason == "MAX_TOKENS" || raw_response.contains("<think") {
                "Твои размышления прерваны из-за лимита токенов. Сгенерируй ответ ЗАНОВО с самого начала. СИЛЬНО СОКРАТИ свои внутренние размышления (максимум 2-3 вывода) и сразу переходи к финальному результату."
            } else {
                "Ты прервал генерацию. Продолжи ответ ОБЫЧНЫМ ТЕКСТОМ."
            };
            llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
            llm_messages.push(LlmMessage { role: "user".to_string(), content: hint.to_string() });
            continue;
        }

        if !action_found && has_tools_for_prompt {
            if has_incomplete_json_action(&response) || has_json_thought_without_action(&response) {
                consecutive_incomplete += 1;
                if consecutive_incomplete >= 5 {
                    final_response = format!("{} Агент '{}' не смог завершить действие (5 попыток). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                    break;
                }
                llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                llm_messages.push(LlmMessage { role: "user".to_string(), content: "Ты начал размышлять в JSON, но не указал действие. Пиши кратко и СРАЗУ укажи \"target\" или \"tool\".".to_string() });
                continue;
            }
        }

        let preview = safe_truncate(&response, 300).replace('\n', " ");
        log_cb(format!("✅ Агент {} завершил ответом ({} символов): {}", agent.name, response.len(), preview));
        final_response = response;
        break;
    }

    if depth > 0 {
        let subcall = SubCall { agent_name: agent.name.clone(), prompt: invocation_dump.clone(), response: final_response.clone(), time_sec: start_time.elapsed().as_secs_f32(), tool_calls };
        subcall_cb(&subcall);
        all_sub_calls.push(subcall);
    }

    Ok(final_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_agent(id: &str, system_prompt: &str) -> AgentProfile {
        AgentProfile {
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
            tools: Vec::new(),
        }
    }

    fn parse_wf(yaml: &str) -> WorkflowDef {
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
        let agents: Vec<AgentProfile> = vec![];
        let wf = parse_wf(
            "name: test\nnodes:\n  - id: r\n    type: return\nedges: []\n",
        );

        let prompt = build_worst_agent_prompt(&agents, &wf, &[]);
        assert_eq!(prompt, "");
    }
}