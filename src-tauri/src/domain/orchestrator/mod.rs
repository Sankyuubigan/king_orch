pub(crate) mod consts;
pub(crate) mod dispatch;
pub(crate) use dispatch::*;
pub mod stream;
pub(crate) mod signal_prompts;
pub(crate) mod text;
pub(crate) mod invocation;
pub(crate) mod grammar;
pub(crate) mod spill;
pub(crate) mod compaction;
pub(crate) mod prompt_log;
pub(crate) mod todo;
pub(crate) mod result;
pub(crate) use consts::*;
pub use stream::*;
pub(crate) use signal_prompts::*;
pub(crate) use text::*;
pub(crate) use invocation::*;
pub(crate) use grammar::*;
pub(crate) use spill::*;
pub(crate) use compaction::*;
pub(crate) use prompt_log::*;
pub(crate) use todo::*;
pub(crate) use result::*;

pub mod prompt;
mod runtime;

pub use runtime::builtin_tools;

use crate::domain::agent_manager::{load_agents, AgentProfile};
use crate::domain::signals::{SignalContract, build_signal_envelope_schema, load_signal_contract};
use crate::domain::workflow_engine::{
    find_workflow_by_stem, load_workflows, WorkflowContext, WorkflowRunner, NodeType, WorkflowDef,
};
use crate::infra::{ChatMessage, ChatAttachment, LlamaEngine, ModelParams, SubCall, LlmMessage, extract_model_filename, llm_history, GrammarSpec};
use crate::domain::parsers::{
    clean_thought_tags, extract_think_content, extract_thought_from_partial_json,
    has_incomplete_json_action, is_thinking_truncated, needs_cutoff_continuation,
    parse_orchestrator_response, parse_tool_call, split_thinking_and_answer,
};
use prompt::build_system_prompt;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// True, если текст — сообщение об ошибке агента (workflow должен остановиться fail-fast).
pub(crate) fn is_agent_error(text: &str) -> bool {
    text.trim_start().starts_with(AGENT_ERROR_PREFIX)
}

// ── Параметры «продолжай-цикла» для обрыва генерации по лимиту токенов ──
// Вместо «СГЕНЕРИРУЙ ЗАНОВО» (модель по кругу переписывает те же размышления и
// снова упирается в max_gen) — продолжаем РОВНО с места обрыва (cache_prompt=true
// позволяет движку переиспользовать KV префикса). Накопившиеся размышления при
// перерастании лимита сжимаем отдельным малым LLM-вызовом в тезисы (~300 токенов).
// Повторные попытки «ответа с начала»: модель завершила докачку, но начала
// финальный ответ с многоточия (продолжила оборванный думатель, начало ответа
// потеряно). Не перегенерируем всё заново — хвост остаётся в истории, просим
// написать начало; при исчерпании попыток принимаем ответ как есть.
// ── Умное завершение докачек: серия итераций без прогресса (видимого текста нет,
// размышления перестали расти) = модель зациклилась в думателе — докачку прекращаем
// и переходим к перегенерации с хинтом-запретом думателей. ──
// Предел повторов второго (сигнального) вызова emit_signal. Если модель не вернула
// распознаваемый JSON-конверт — ретраим с корректирующим хинтом, затем (не теряя
// отчёт агента!) пропускаем сигнал с логом, а не подставляем JSON вместо ответа.

/// Один шаг «докачного» цикла: генерация оборвалась по лимиту токенов → продолжаем
/// РОВНО с места обрыва (не регенерируем). При перерастании накопленных размышлений
/// сжимаем их отдельным малым LLM-вызовом в тезисы, чтобы не раздувать KV.
///
/// Возвращает `Ok(true)` если лимит продолжений исчерпан (вызывающий формирует ошибку),
/// `Ok(false)` — продолжай (вызывающий делает `continue` со следующей итерацией).
#[allow(clippy::too_many_arguments)]
fn push_continuation_for_cutoff(
    log_cb: &dyn Fn(String),
    agent_id: &str,
    engine: &LlamaEngine,
    model_params: &ModelParams,
    format_type: &str,
    cancel_flag: Arc<AtomicBool>,
    ctx_label: &str,
    stream_meta: Arc<Mutex<StreamMeta>>,
    combined: &str,
    parse_target: &str,
    raw_response: &str,
    llm_messages: &mut Vec<LlmMessage>,
    continuation_raw: &mut String,
    continuation_mark: &mut Option<usize>,
    continuation_count: &mut usize,
) -> Result<bool, String> {
    *continuation_count += 1;
    if *continuation_count >= MAX_CONTINUATIONS {
        return Ok(true);
    }

    if continuation_mark.is_none() {
        *continuation_mark = Some(llm_messages.len());
    }
    // Накопливаем сырой текст — он нужен для финального вырезания ответа из размышлений
    *continuation_raw = if continuation_raw.is_empty() {
        raw_response.to_string()
    } else {
        format!("{}\n{}", continuation_raw, raw_response)
    };

    // ── Сжатие накопленных размышлений в тезисы (малый отдельный LLM-вызов) ──
    if let Some(mark) = *continuation_mark {
        // Текущий оборванный кусок ещё НЕ запушен в llm_messages (пуш в конце
        // функции), поэтому учитываем и его — иначе компакт всегда запаздывает
        // на одну докачку и думатель успевает раздуться (потеря начала ответа,
        // раздутый KV-кэш).
        let acc_chars: usize = llm_messages[mark..]
            .iter()
            .filter(|m| m.role == "assistant")
            .map(|m| m.content.chars().count())
            .sum::<usize>()
            + continuation_raw.chars().count();
        if acc_chars > COMPACT_THRESHOLD_CHARS {
            let thinking = llm_messages[mark..]
                .iter()
                .filter(|m| m.role == "assistant")
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            log_cb(format!(
                "🧠 Размышления разрослись ({} символов) — сжатие в тезисы ({} токенов)...",
                thinking.chars().count(),
                COMPACT_MAX_TOKENS
            ));

            let saved_kind = stream_meta.lock().map(|m| m.kind.clone()).unwrap_or_default();
            if let Ok(mut m) = stream_meta.lock() {
                m.kind = String::new(); // внутренняя генерация — не стримим в UI
            }
            let summary = engine.generate_chat(
                &[
                    LlmMessage {
                        role: "system".to_string(),
                        content: "Ты — инструмент сжатия внутренних размышлений агента. Сожми приложенные размышления до 10-14 коротких тезисов. Сохрани ВСЕ факты, термины, цифры и выводы. Пиши на языке исходного текста. Только тезисы, без вступлений.".to_string(),
                    },
                    LlmMessage {
                        role: "user".to_string(),
                        content: thinking,
                    },
                ],
                COMPACT_MAX_TOKENS,
                model_params,
                format_type,
                cancel_flag.clone(),
                &format!("{}#compact", ctx_label),
                |_, _| {},
                log_cb,
            );
            if let Ok(mut m) = stream_meta.lock() {
                m.kind = saved_kind;
            }

            match summary {
                Ok(g) => {
                    let summary_text = tail_chars(&clean_thought_tags(&g.text), 2500);
                    llm_messages.truncate(mark);
                    llm_messages.push(LlmMessage {
                        role: "assistant".to_string(),
                        content: summary_text,
                    });
                    *continuation_mark = Some(llm_messages.len());
                    continuation_raw.clear();
                    log_cb("🧠 Размышления сжаты в тезисы, продолжаем с места обрыва.".to_string());
                }
                Err(e) => log_cb(format!(
                    "⚠️ Не удалось сжать размышления ({}), продолжаю без сжатия.",
                    e
                )),
            }
        }
    }

    // Точная точка обрыва: передаём модели последний сырой кусок + указание закончить
    let in_json = parse_target.contains("{") || parse_target.contains("    \"target\"");
    let hint = if in_json {
        "❌ Твой ответ оборвался в ПОЛОВИНЕ JSON-объекта из-за лимита токенов. Продолжи РОВНО с места обрыва и ЗАКРОЙ начатый JSON (все кавычки и скобки). НЕ повторяй уже написанное и не начинай заново."
    } else if is_thinking_truncated(combined) {
        "❌ Твои размышления оборваны лимитом токенов. Продолжи РОВНО с места обрыва, без повторений: кратко заверши мысль и СРАЗУ напиши финальный ответ простым текстом (без JSON)."
    } else {
        "❌ Твой ответ оборвался из-за лимита токенов. Продолжи РОВНО с места обрыва, без повторений, и заверши ответ."
    };
    llm_messages.push(LlmMessage {
        role: "assistant".to_string(),
        content: raw_response.to_string(),
    });
    llm_messages.push(LlmMessage {
        role: "user".to_string(),
        content: hint.to_string(),
    });
    log_cb(format!(
        "⏩ [{}] докача размышлений после обрыва (продолжение #{})",
        agent_id, continuation_count
    ));
    Ok(false)
}

/// Режим графа: системный промпт самого «тяжёлого» агента графа (worst-case).
/// Пиковая VRAM определяется одним LLM-вызовом (движок работает последовательно),
/// поэтому берём агента с самым длинным системным промптом. Sub-workflow узлы
/// намеренно НЕ раскрываются — считаем только текущий граф.
///
/// Собирается ТАК ЖЕ, как в run_agent_node (SSOT): инструменты (builtin + агентские)
/// и [КРИТИЧЕСКОЕ ОГРАНИЧЕНИЕ]. Схемы MCP-инструментов доступны только после
/// запуска серверов (рантайм), поэтому для них оценка — нижняя граница; точная
/// подгонка истории — после старта движка через /tokenize (цикл обрезки).
pub fn build_worst_agent_prompt(
    agents: &[AgentProfile],
    wf: &WorkflowDef,
    history: &[ChatMessage],
) -> (String, bool) {
    let worst = wf.nodes.iter()
        .filter(|n| n.node_type == NodeType::LlmWorker)
        .filter_map(|n| n.agent.as_deref())
        .filter_map(|aid| agents.iter().find(|a| a.id == aid))
        .map(|agent| {
            let mut tools = runtime::builtin_tools();
            tools.extend(runtime::agent_code_tool_schemas(agent));
            let has_tools = !agent.tools.is_empty() || !agent.mcp_servers.is_empty();
            let mut sp = build_system_prompt(agent, history, has_tools, &tools, 2048);
            sp.push_str("\n\n");
            sp.push_str(prompt::CRITICAL_LIMIT_BLOCK);
            (sp, has_tools)
        })
        .max_by_key(|(sp, _)| sp.chars().count());

    // Граф без llm_worker-узлов → пустой системный промпт (посчитаются только история + сообщение)
    worst.unwrap_or_else(|| (String::new(), false))
}

fn estimate_chars_per_token(worst_system_prompt: &str, history_text: &str, user_text: &str) -> usize {
    let total = worst_system_prompt.chars().count() + history_text.chars().count() + user_text.chars().count();
    if total == 0 {
        return 3;
    }
    let cyrillic = worst_system_prompt
        .chars()
        .chain(history_text.chars())
        .chain(user_text.chars())
        .filter(|c| ('\u{0400}'..='\u{04FF}').contains(c))
        .count();
    if cyrillic * 10 >= total * 3 {
        2
    } else {
        3
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_chat<L, S, C, ST>(
    log_cb: L, status_cb: S, subcall_cb: C, stream_cb: ST,
    agents_dir: std::path::PathBuf, mcp_servers_dir: std::path::PathBuf, bins_dir: std::path::PathBuf,
    engine_dir: std::path::PathBuf,
    model_path: String, agent_id: String, user_text: String, history: Vec<ChatMessage>,
    attachments: Vec<ChatAttachment>,
    context_size: u32, max_gen_tokens: u32, kv_quant_keys: bool, kv_quant_values: bool, reasoning_budget: u32,     model_params: ModelParams, format_type: String,
    mmproj_path: Option<String>,     cancel_flag: Arc<AtomicBool>,
    stream_meta: Arc<Mutex<StreamMeta>>,
    prompt_log: Option<std::path::PathBuf>,
    session_id: String,
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
    // (делитель зависит от языка промпта: ~3 симв/токен для латиницы, ~2 для
    // кириллицы) + запас на спецтокены/JSON и токены изображений. Точная подгонка
    // остаётся за циклом обрезки истории в run_agent_node (по точным /tokenize).
    let (worst_system_prompt, worst_has_tools) = match &workflow_match {
        Some(wf) => build_worst_agent_prompt(&agents, wf, &history),
        None => agents.iter().find(|a| a.id == agent_id)
            .map(|agent| {
                let mut tools = runtime::builtin_tools();
                tools.extend(runtime::agent_code_tool_schemas(agent));
                let has_tools = !agent.tools.is_empty() || !agent.mcp_servers.is_empty();
                (build_system_prompt(agent, &history, has_tools, &tools, max_gen_usize), has_tools)
            })
            .unwrap_or_else(|| (String::new(), false)),
    };
    let history_text: String = llm_history(&history).iter().map(|m| m.content.as_str()).collect();
    let history_chars = history_text.chars().count();
    let total_chars = worst_system_prompt.chars().count() + history_chars + user_text.chars().count();
    let image_tokens = attachments.len() as u32 * 2048;
    let chars_per_token = estimate_chars_per_token(&worst_system_prompt, &history_text, &user_text);
    let tool_budget = if worst_has_tools { TOOL_WORKING_BUDGET } else { 0 };
    let estimated_tokens = (total_chars / chars_per_token) as u32 + image_tokens + TOKEN_ESTIMATE_RESERVE + tool_budget;
    let engine_ctx_limit = (estimated_tokens + max_gen_tokens + 128).min(context_size).max(2048);
    log_cb(format!(
        "📐 Стартовый контекст движка: {} токенов (worst-case промпт ~{} символов{}, история ~{} симв., изображения ~{} токенов, резерв JSON {}, бюджет инструментов {}, max_gen {})",
        engine_ctx_limit, worst_system_prompt.chars().count(),
        if worst_has_tools { " с инструментами" } else { "" },
        history_chars, image_tokens, TOKEN_ESTIMATE_RESERVE, tool_budget, max_gen_tokens
    ));

    let engine = if mmproj_path.is_some() {
        LlamaEngine::new_with_mmproj(&engine_dir, &model_path, mmproj_path.as_deref(), engine_ctx_limit, kv_quant_keys, kv_quant_values, reasoning_budget, log_cb.clone(), stream_cb)?
    } else {
        LlamaEngine::new(&engine_dir, &model_path, engine_ctx_limit, kv_quant_keys, kv_quant_values, reasoning_budget, log_cb.clone(), stream_cb)?
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

    // Per-agent GBNF-грамматики лежат рядом с агентами: agents/<папка>/grammars/
    let grammars_dir = resolve_grammars_dir(&agents_dir, workflow_match.as_deref());
    log_cb(format!("🎯 Директория грамматик: {}", grammars_dir.display()));

    // Загружаем пресеты параметров LLM из sampling_presets.json (рядом с agents/)
    let project_dir = agents_dir.parent().unwrap_or(&agents_dir);
    let sampling_presets = crate::infra::load_sampling_presets(project_dir);

    // 🔐 Корень проекта для инструментов кодинга (запись внутри — авто,
    // проверяется тулами через ctx.workspace_root). Сбрасываем гранты сессии.
    crate::infra::global_approver().reset_session(&session_id);

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
            grammars_dir: &grammars_dir,
            all_sub_calls: &mut all_sub_calls,
            msg_counter: &mut msg_counter,
            stream_meta: stream_meta.clone(),
            sampling_presets: &sampling_presets,
            prompt_log: prompt_log.clone(),
            session_id: session_id.clone(),
            workspace_root: project_dir.to_path_buf(),
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

        let mcp_pool: crate::infra::mcp_client::McpPool = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::HashMap::<String, crate::infra::mcp_client::SharedMcpClient>::new(),
        ));

        let final_res = run_agent_node(
            log_cb.clone(), status_cb, subcall_cb,
            &engine, primary_agent, &agents, user_text, recent_history,
            &attachments,
            max_gen_usize, &model_params, &format_type,
            cancel_flag, 0, &mut all_sub_calls, None, &mcp_servers_dir, &bins_dir,
            &grammars_dir, mcp_pool,
            &mut messages_store, &mut msg_counter,
            String::new(),
            stream_meta.clone(), true,
            prompt_log.clone(),
            session_id.clone(),
            project_dir.to_path_buf(),
        )?;

        // Fail-fast: если primary-агент вернул ошибку — не сохраняем её как ответ
        if is_agent_error(&final_res) {
            log_cb(format!("❌ Основной агент '{}' вернул ошибку: {}", primary_agent.id, final_res));
            return Err(final_res);
        }

        let sub_calls_opt = if all_sub_calls.is_empty() { None } else { Some(all_sub_calls.clone()) };
            messages_store.push(ChatMessage {
                id: Some(format!("msg_{}", msg_counter)),
                msg_type: "message".to_string(),
                content: final_res.clone(),
                sub_calls: sub_calls_opt,
                author: Some(primary_agent.id.clone()),
                model: Some(extract_model_filename(&engine.model_path)),
                attachments: None,
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

/// Результаты инструментов длиннее этого порога сохраняются в spill-файл,
/// а модели отдаётся выжимка (head + tail) с локатором. Лечит раздувание
/// контекста: раньше модель видела полный вывод инструмента (mod.rs:~1122),
/// что убивало контекст на длинных выдачах (поиск, чтение файлов, логи).

/// Директория spills рядом с исполняемым файлом (правило: не писать в cwd).
pub(crate) fn spill_root_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|pp| pp.join("spill")))
        .unwrap_or_else(|| std::path::PathBuf::from("spill"))
}

pub(crate) fn run_agent_node<L, S, C>(
    log_cb: L, status_cb: S, subcall_cb: C,
    engine: &LlamaEngine, agent: &AgentProfile, agents: &[AgentProfile],
    user_text: String, _history: Vec<ChatMessage>,
    attachments: &[ChatAttachment],
    max_gen_tokens: usize, model_params: &ModelParams, format_type: &str,
    cancel_flag: Arc<AtomicBool>, depth: usize,
    all_sub_calls: &mut Vec<SubCall>, caller_name: Option<String>,
    mcp_servers_dir: &Path, bins_dir: &Path,
    grammars_dir: &Path,
    mcp_pool: crate::infra::mcp_client::McpPool,
    messages: &mut Vec<ChatMessage>, msg_counter: &mut u32,
    injected_reports: String,
    stream_meta: Arc<Mutex<StreamMeta>>,
    allow_stream: bool,
    prompt_log: Option<std::path::PathBuf>,
    session_id: String,
    workspace_root: std::path::PathBuf,
) -> Result<String, String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    if depth > 5 { return Err("Превышена максимальная глубина вложенности сабагентов".into()); }
    log_cb(format!("▶ Запуск агента: {} (глубина: {})", agent.name, depth));

    // 4.2: публикуем событие старта в in-process шину (статус агента для UI/логов).
    crate::infra::event_bus::global_bus().publish(crate::infra::event_bus::AgentEvent::Spawned {
        agent: agent.id.clone(),
        namespace: caller_name.as_deref().unwrap_or("main").to_string(),
    });

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

    let mut all_tools: Vec<(String, String, serde_json::Value)> = Vec::new();
    runtime::load_mcp_servers(&log_cb, mcp_servers_dir, bins_dir, &agent.mcp_servers, &mcp_pool, &mut all_tools);

    // 🛠 Capability кодинга: `tools: ["code_read"]` — только чтение; `tools: ["code_write"]` —
    // чтение + мутаторы (внутри корня авто, вне — плашка). SSOT — infra::tools.
    all_tools.extend(runtime::agent_code_tool_schemas(agent));

    let has_real_tools = !all_tools.is_empty() || !agent.tools.is_empty();

    all_tools.extend(runtime::builtin_tools());

    // 4.1: todo-инструмент — ОБЫЧНЫЙ opt-in тул, НЕ built-in для всех. Включается
    // только для агентов папок `coder`/`research` (и при явном `tools: ["todo"]` в .md),
    // чтобы не тратить токены и не путать агентов, которым чек-лист не нужен
    // (психотерапевт и прочие — без туду).
    let todo_enabled = agent.tools.iter().any(|t| t == "todo")
        || agent.folder.as_deref() == Some("coder")
        || agent.folder.as_deref() == Some("research");
    if todo_enabled {
        all_tools.extend(runtime::todo_tool_schemas());
    }

    let has_tools_for_prompt = has_real_tools;
    let mut system_prompt = build_system_prompt(agent, messages, has_tools_for_prompt, &all_tools, max_gen_tokens);
    // 4.5: плагин-слой — точка расширения системного промпта (pass-through, если плагинов нет).
    crate::infra::plugins::global_plugins().on_system_prompt(&agent.id, &mut system_prompt);
    if !injected_reports.is_empty() {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&injected_reports);
    }
    
    // Глобальное правило для всех мыслящих моделей, чтобы не пробивали лимит 2048 токенов
    system_prompt.push_str("\n\n[КРИТИЧЕСКОЕ ОГРАНИЧЕНИЕ]\n");
    system_prompt.push_str(prompt::CRITICAL_LIMIT_BLOCK);

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

    // Единый pipeline компакции: при давлении по токенам сворачиваем крупные
    // результаты и старую историю, чтобы не пробивать context_size (fallback —
    // жёсткая обрезка). Бюджет в символах: (global_ctx_limit − max_gen)·2
    // (консервативно для кириллицы).
    let budget_chars = (engine
        .global_ctx_limit
        .saturating_sub(max_gen_tokens as u32) as usize)
        .saturating_mul(2)
        .max(5000);
    let compaction = compact_llm_messages(&mut llm_messages, budget_chars);
    // ОБЯЗАТЕЛЬНО логируем любую потерю контекста (тихих операций с данными нет).
    if compaction.tool_results_pruned > 0
        || compaction.history_compressed
        || compaction.old_messages_dropped > 0
    {
        log_cb(format!(
            "🗜️ Компакция контекста агента '{}': свёрнуто результатов инструментов {}, сжата история {}, жёстко удалено старых сообщений {}.",
            agent.id, compaction.tool_results_pruned, compaction.history_compressed, compaction.old_messages_dropped
        ));
    }

    // Invocation-дамп для subcall: только то, что различается между вызовами
    // (task + отчёты коллег). Полный системный промпт агента живёт в .md файле
    // (SSOT) — его копия в сессии раздувала бы JSON на 5-11KB за каждый вызов.
    let invocation_dump = build_invocation_dump(&user_text, &injected_reports);

let start_time = Instant::now();
    // Метка режима для лога пиков памяти: llm_worker графа зовёт run_agent_node
    // с caller_name == "workflow_engine", всё остальное — legacy (.md) режим.
    let mem_mode = if caller_name.as_deref() == Some("workflow_engine") { "graph" } else { "legacy" };

    // ── Per-agent грамматика: agents/<...>/grammars/<agent_id>.gbnf ──
    // Задаётся для ПЕРВОГО вызова LLM агента (consume-and-clear в движке),
    // докачки/компакты/результаты инструментов идут уже с базовой грамматикой.
    // Сигнальные агенты (есть контракт в signals/root.schema.json) НЕ ограничены
    // GBNF: сигнал эмитится ВТОРЫМ вызовом под json_schema конверта.
    let signal_contract: Option<SignalContract> = {
        let signals_dir = grammars_dir.parent().unwrap_or(grammars_dir).join("signals");
        load_signal_contract(&signals_dir, &agent.id)
    };
    let agent_grammar = if signal_contract.is_none() {
        load_agent_grammar(grammars_dir, &agent.id)
    } else {
        None
    };
    if let Some(gbnf) = &agent_grammar {
        engine.set_grammar(Some(GrammarSpec { gbnf: Some(gbnf.clone()), json_schema: None }));
        log_cb(format!("🎯 Агент '{}': применена грамматика {} символов", agent.id, gbnf.len()));
    } else {
        engine.set_grammar(None);
        log_cb(format!("⚠️ Грамматика не найдена для агента '{}' (искал в {})", agent.id, grammars_dir.display()));
    }

    // ── Контекст цикла (dispatch.rs): всё разделяемое мутабельное состояние
    // упаковано в RunContext; блоки инструментов и сабагентов вынесены в методы
    // execute_tool_call / handle_subagent_call. Иммутабельные ссылки/Arc продублированы
    // локально для читаемости тела цикла (Clone/Copy, без рассинхронизации).
    let mut ctx = RunContext {
        engine,
        agent,
        agents,
        model_params,
        format_type,
        cancel_flag: cancel_flag.clone(),
        stream_meta: stream_meta.clone(),
        prompt_log: prompt_log.clone(),
        depth,
        has_tools_for_prompt,
        all_tools,
        mcp_clients: mcp_pool,
        log_cb: log_cb.clone(),
        status_cb: status_cb.clone(),
        subcall_cb: subcall_cb.clone(),
        mcp_servers_dir,
        bins_dir,
        grammars_dir,
        session_id: session_id.clone(),
        workspace_root: workspace_root.clone(),
        approver: crate::infra::global_approver(),
        llm_messages,
        messages,
        msg_counter,
        all_sub_calls,
        final_response: String::new(),
        tool_calls: Vec::new(),
        consecutive_failed_tools: 0,
        spill_idx: 0,
        consecutive_incomplete: 0,
        thinking_no_answer: 0,
        consecutive_invalid_targets: 0,
        last_thinking_len: 0,
        stalled_continuations: 0,
        signal_attempted: false,
        signal_saved: false,
        signal_retries: 0,
        signal_analysis: String::new(),
        continuation_count: 0,
        continuation_restarts: 0,
        continuation_raw: String::new(),
        continuation_mark: None,
        action_found: false,
        thought_logged: false,
    };

    for iter in 1..=30 {
        if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }

        let mut ideal_ctx;
        loop {
            let current_tokens = engine.get_tokens_count(&ctx.llm_messages, format_type).unwrap_or(0);
            ideal_ctx = (current_tokens as u32 + max_gen_tokens as u32 + 128).min(engine.global_ctx_limit);

            if current_tokens + max_gen_tokens <= ideal_ctx as usize || ctx.llm_messages.len() <= 2 {
                log_cb(format!("📊 Память: выделен KV-кэш на {} токенов (Промпт: {}, Резерв: {})", ideal_ctx, current_tokens, max_gen_tokens));
                break;
            }
            if ctx.llm_messages.len() > 2 {
                let removed = &ctx.llm_messages[1];
                let chars = removed.content.chars().count();
                let snippet: String = removed.content.chars().take(120).collect();
                log_cb(format!(
                    "⚠️ Превышен лимит контекста: промпт {} + генерация {} > лимита {}. Удалено самое старое сообщение [{}], {} симв.: {}",
                    current_tokens, max_gen_tokens, ideal_ctx, removed.role, chars,
                    if chars > 120 { format!("{}…", snippet) } else { snippet }
                ));
                ctx.llm_messages.remove(1);
            } else {
                break;
            }
        }

        // ── Снимок точного входа модели (правило «модель видит только записанное») ──
        if let Some(ref pl) = prompt_log {
            let logged_tokens = engine.get_tokens_count(&ctx.llm_messages, format_type).unwrap_or(0);
            write_prompt_log(pl, &agent.name, iter, logged_tokens, &ctx.llm_messages);
        }

        let gen_start = Instant::now();
        log_cb(format!(">>> [{}] LLM вызов #{}, msgs={}, max_gen={}, глубина={}", agent.name, iter, ctx.llm_messages.len(), max_gen_tokens, depth));
        let ctx_label = format!("{}:{}#{}", mem_mode, agent.name, iter);
        let gen = if !attachments.is_empty() && engine.is_multimodal() {
            engine.generate_chat_multimodal(
                &ctx.llm_messages, &attachments, max_gen_tokens, model_params, format_type, cancel_flag.clone(),
                &ctx_label,
                |p, _| { status_cb(format!("{} обрабатывает медиа (Шаг {})...", agent.name, iter), 20 + (p * 0.1) as u8); },
                log_cb.clone(),
            )?
        } else {
            engine.generate_chat(
                &ctx.llm_messages, max_gen_tokens, model_params, format_type, cancel_flag.clone(),
                &ctx_label,
                |p, _| { status_cb(format!("{} думает (Шаг {})...", agent.name, iter), 20 + (p * 0.1) as u8); },
                log_cb.clone(),
            )?
        };
        let raw_response = gen.text.clone();
        let reasoning = gen.reasoning.clone();
        let stop_reason = gen.stop_reason.clone();

        log_cb(format!("<<< [{}] LLM за {:.1}с, ответ {} символов", agent.name, gen_start.elapsed().as_secs_f32(), raw_response.len()));

        let response = clean_thought_tags(&raw_response);
        ctx.action_found = false;
        ctx.thought_logged = false;

        // ── Режим продолжения: парсим весь накопленный текст, а не последний кусок
        // (JSON/ответ может быть разорван между итерациями докачки) ──
        let is_continuation = ctx.continuation_mark.is_some();
        let combined = if is_continuation {
            format!("{}\n{}", ctx.continuation_raw, raw_response)
        } else {
            raw_response.clone()
        };
        let parse_target = if is_continuation {
            clean_thought_tags(&combined)
        } else {
            response.clone()
        };

        if response.trim().is_empty() {
            // Обрыв ВНУТРИ размышлений (thinking-модель): продолжаем с места обрыва,
            // а не считаем «пустой попыткой» и не просим генерировать ЗАНОВО.
            // Критерий входа — СОДЕРЖИМОЕ ответа (незакрытый думатель), а не причина
            // остановки: обрыв случается и по лимиту токенов, и по стоп-слову, и по EOS.
            if needs_cutoff_continuation(&combined, &stop_reason) {
                // Умное завершение: серия докачек без прогресса (видимого текста нет,
                // думание перестало расти) = модель зациклилась в думателе — докачку
                // прекращаем и уходим в перегенерацию с хинтом-запретом думателей.
                if ctx.stalled_continuations >= MAX_STALLED_CONTINUATIONS {
                    log_cb(format!(
                        "🛑 Докачка забуксовала: {} итераций подряд без роста размышлений и без видимого ответа — переключаемся на перегенерацию.",
                        ctx.stalled_continuations
                    ));
                    ctx.stalled_continuations = 0;
                    ctx.last_thinking_len = 0;
                } else {
                    // Новая серия докачек (стейт «докачки» сброшен) — стартуем с чистых метрик
                    if ctx.continuation_mark.is_none() {
                        ctx.last_thinking_len = 0;
                        ctx.stalled_continuations = 0;
                    }
                    let thinking_len = combined.chars().count() as isize;
                    let grew = thinking_len - ctx.last_thinking_len;
                    let raw_before = ctx.continuation_raw.len();
                    let exhausted = push_continuation_for_cutoff(
                        &log_cb, &agent.id, engine, model_params, format_type, cancel_flag.clone(),
                        &ctx_label, stream_meta.clone(), &combined, &parse_target, &raw_response,
                        &mut ctx.llm_messages, &mut ctx.continuation_raw, &mut ctx.continuation_mark, &mut ctx.continuation_count,
                    )?;
                    if exhausted {
                        ctx.final_response = format!("{} Агент '{}' не смог завершить размышления после {} докачек (модель упирается в лимит токенов). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id, MAX_CONTINUATIONS);
                        break;
                    }
                    ctx.last_thinking_len = thinking_len;
                    // Компакт размышлений — это прогресс (факты резюмируются в тезисы),
                    // серию «застоя» в этом случае сбрасываем, а не считаем застой.
                    let compacted = ctx.continuation_raw.len() < raw_before;
                    if !compacted && grew < MIN_THINKING_GROWTH_CHARS {
                        ctx.stalled_continuations += 1;
                        log_cb(format!(
                            "⚠️ Докачка #{}: размышления не растут (+{} симв.), видимого ответа нет — застой {}/{}",
                            ctx.continuation_count, grew, ctx.stalled_continuations, MAX_STALLED_CONTINUATIONS
                        ));
                    } else {
                        ctx.stalled_continuations = 0;
                    }
                    continue;
                }
            }
            if stop_reason == "STOP_WORD" || stop_reason == "MAX_TOKENS" {
                // Думатель без ответа: размышления НЕ попадают в историю (они уже
                // выброшены движком в отдельное поле), докачки бессмысленны — модель
                // исчерпала лимит на думатель. Повторный вызов с требованием ответа.
                if !reasoning.trim().is_empty() {
                    ctx.thinking_no_answer += 1;
                    if ctx.thinking_no_answer >= 3 {
                        ctx.final_response = format!("{} Агент '{}' 3 раза подряд сгенерировал только размышления без ответа (думатель исчерпал лимит токенов). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                        break;
                    }
                    ctx.continuation_raw.clear();
                    ctx.continuation_mark = None;
                    log_cb(format!(
                        "🧠 Агент '{}' выдал только думатель ({} симв.) без ответа ({}) — повторный вызов с требованием отвечать сразу ({}/3).",
                        agent.id,
                        reasoning.chars().count(),
                        stop_reason,
                        ctx.thinking_no_answer
                    ));
                    ctx.llm_messages.push(LlmMessage { role: "user".to_string(), content: "Ты потратил весь лимит токенов на внутренние размышления и не дал видимого ответа. На этот раз отвечай СРАЗУ, БЕЗ внутренних размышлений: только итоговый результат.".to_string() });
                    continue;
                }
                ctx.consecutive_incomplete += 1;
                if ctx.consecutive_incomplete >= 3 {
                    ctx.final_response = format!("{} Агент '{}' не смог сформировать ответ (3 пустых попытки: стоп-слово/лимит токенов). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                    break;
                }
                let hint = if stop_reason == "MAX_TOKENS" {
                    "Твои размышления прерваны из-за лимита токенов. Сгенерируй ответ ЗАНОВО с самого начала. СИЛЬНО СОКРАТИ свои внутренние размышления (максимум 2-3 вывода) и сразу переходи к финальному результату."
                } else {
                    "Ты прервал генерацию. ЗАПРЕЩЕНО начинать с размышлений в тегах (<think, 思考, thinking, <|channel>thought) — они запрещены. Сразу пиши финальный ответ ОБЫЧНЫМ ТЕКСТОМ без JSON."
                };
                ctx.llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                // Хинт требует ответ ЗАНОВО — стейт незавершённой докачки сбрасываем,
                // иначе следующий вызов парсил бы склеенный combined устаревших кусков.
                ctx.continuation_raw.clear();
                ctx.continuation_mark = None;
                ctx.llm_messages.push(LlmMessage { role: "user".to_string(), content: hint.to_string() });
                continue;
            }
        }

        // Внедряем архитектурную проверку на обрыв (MAX_TOKENS)
        let mut is_valid_json = false;
        if parse_target.contains('{') {
            is_valid_json = crate::domain::parsers::is_valid_json_action(&parse_target);
        }

        let resp_trim = parse_target.trim();
        let is_final_text = resp_trim.ends_with('.') || resp_trim.ends_with('!') || resp_trim.ends_with('?') || resp_trim.ends_with('"') || resp_trim.ends_with('\'') || resp_trim.ends_with('`');

        if stop_reason == "MAX_TOKENS" && !is_valid_json && !is_final_text {
            // Обрыв по лимиту: докачиваем с места обрыва вместо перегенерации
            if push_continuation_for_cutoff(
                &log_cb, &agent.id, engine, model_params, format_type, cancel_flag.clone(),
                &ctx_label, stream_meta.clone(), &combined, &parse_target, &raw_response,
                &mut ctx.llm_messages, &mut ctx.continuation_raw, &mut ctx.continuation_mark, &mut ctx.continuation_count,
            )? {
                ctx.final_response = format!("{} Агент '{}' не смог завершить ответ после {} докачек (модель упирается в лимит токенов). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id, MAX_CONTINUATIONS);
                break;
            }
            continue;
        }

        if let Some((tool_name, arguments, thought)) = parse_tool_call(&parse_target) {
            match ctx.execute_tool_call(&tool_name, &arguments, &thought, gen_start, &raw_response, &combined, is_continuation, &parse_target, &response)? {
                DispatchCtl::Continue => continue,
                DispatchCtl::Break => break,
                DispatchCtl::Return(v) => return Ok(v),
            }
        }

        if let Some(parsed) = parse_orchestrator_response(&parse_target) {
            ctx.action_found = true;
            ctx.consecutive_incomplete = 0;

            if parsed.target == "reply" || parsed.target == "user" {
                if parsed.content.is_empty() {
                    ctx.final_response = if is_continuation {
                        extract_answer_from_combined(&combined, &response)
                    } else {
                        response.clone()
                    };
                } else {
                    ctx.final_response = parsed.content;
                }

                // Ответ завершён обычным текстом — состояние «докачки» больше не нужно.
                // Без сброса следующий вызов (например, сигнальный emit_signal) парсил бы
                // склеенный combined и терял свежий JSON-конверт.
                ctx.continuation_raw.clear();
                ctx.continuation_mark = None;

                // ── Второй сигнальный вызов (та же логика, что в конце цикла):
                // агент ответил через reply, но сигнал по контракту не эмичен.
                if !ctx.signal_attempted && !ctx.signal_saved {
                    if let Some(contract) = &signal_contract {
                        ctx.signal_attempted = true;
                        ctx.signal_analysis = ctx.final_response.clone();
                        let schema = build_signal_envelope_schema(contract);
                        engine.set_grammar(Some(GrammarSpec { gbnf: None, json_schema: Some(schema) }));
                        log_cb(format!("📡 Второй сигнальный вызов агента '{}' (из reply): emit_signal('{}') под json_schema", agent.id, contract.key));
                        ctx.llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                        ctx.llm_messages.push(LlmMessage {
                            role: "user".to_string(),
                            content: signal_request_prompt(&contract.key),
                        });
                        continue;
                    }
                }

                // ── Сигнальная итерация: модель снова вернула reply, а не
                // JSON-конверт — ретраим с корректирующим хинтом. При исчерпании
                // попыток сигнал пропускаем (красная кнопка, core §2.2), отчёт сохраняем.
                if ctx.signal_attempted && !ctx.signal_saved {
                    ctx.signal_retries += 1;
                    if ctx.signal_retries <= MAX_SIGNAL_RETRIES {
                        if let Some(contract) = &signal_contract {
                            log_cb(format!(
                                "⚠️ [{}] ответ не распознан как emit_signal (попытка {}/{}): {}",
                                agent.id,
                                ctx.signal_retries,
                                MAX_SIGNAL_RETRIES,
                                safe_truncate(&ctx.final_response, 80)
                            ));
                            ctx.llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                            ctx.llm_messages.push(LlmMessage {
                                role: "user".to_string(),
                                content: signal_retry_hint(&contract.key),
                            });
                            continue;
                        }
                    }
                    log_cb(format!(
                        "⚠️ [{}] сигнал '{}' НЕ сохранён после {} попыток (JSON конверта не распознан). Отчёт агента сохранён, сигнал пропущен.",
                        agent.id,
                        signal_contract.as_ref().map(|c| c.key.as_str()).unwrap_or("?"),
                        MAX_SIGNAL_RETRIES
                    ));
                    if !ctx.signal_analysis.is_empty() {
                        ctx.final_response = ctx.signal_analysis.clone();
                    }
                }
                break;
            }

            // ── Блок сабагента / невалидного target (вынесен в dispatch::handle_subagent_call) ──
            match ctx.handle_subagent_call(&parsed, gen_start, &raw_response, &combined, is_continuation, max_gen_tokens)? {
                DispatchCtl::Continue => continue,
                DispatchCtl::Break => break,
                DispatchCtl::Return(v) => return Ok(v),
            }
        }

        if !ctx.thought_logged && !response.is_empty() {
            // В режиме докачки мысли ищем в накопленном сыром тексте
            let thought_source = if is_continuation { &combined } else { &raw_response };
            let extracted = extract_think_content(thought_source);
            for t in &extracted {
                let stored = safe_truncate(t, THOUGHT_STORE_MAX_CHARS);
                log_cb(format!("💭 Мысль {} [d={}] (размышление) [⏱{:.1}с]: {}", agent.name, depth, gen_start.elapsed().as_secs_f32(), stored));
                ctx.messages.push(ChatMessage {
                    id: Some(format!("msg_{}", ctx.msg_counter)),
                    msg_type: "thought".to_string(),
                    content: stored,
                    sub_calls: None,
                    author: Some(agent.id.clone()),
                    model: Some(extract_model_filename(&engine.model_path)),
                    attachments: None,
                });
                *ctx.msg_counter += 1;
            }
            if extracted.is_empty() && !thought_source.contains("<think") {
                if let Some(t) = extract_thought_from_partial_json(thought_source) {
                    let stored = safe_truncate(&t, THOUGHT_STORE_MAX_CHARS);
                    log_cb(format!("💭 Мысль {} [d={}] (размышление) [⏱{:.1}с]: {}", agent.name, depth, gen_start.elapsed().as_secs_f32(), stored));
                    ctx.messages.push(ChatMessage {
                        id: Some(format!("msg_{}", ctx.msg_counter)),
                        msg_type: "thought".to_string(),
                        content: stored,
                        sub_calls: None,
                        author: Some(agent.id.clone()),
                        model: Some(extract_model_filename(&engine.model_path)),
                        attachments: None,
                    });
                    *ctx.msg_counter += 1;
                }
            }
        }

        if !ctx.action_found && response.trim().is_empty() {
            if !reasoning.trim().is_empty() {
                // Только думатель без ответа (движок вернул его отдельным полем):
                // в историю не кладём, докачки не делаем — повторный вызов с ответом.
                ctx.thinking_no_answer += 1;
                if ctx.thinking_no_answer >= 3 {
                    ctx.final_response = format!("{} Агент '{}' 3 раза подряд сгенерировал только размышления без ответа (думатель исчерпал лимит токенов). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                    break;
                }
                ctx.continuation_raw.clear();
                ctx.continuation_mark = None;
                log_cb(format!(
                    "🧠 Агент '{}' выдал только думатель ({} симв.) без ответа ({}) — повторный вызов с требованием отвечать сразу ({}/3).",
                    agent.id,
                    reasoning.chars().count(),
                    stop_reason,
                    ctx.thinking_no_answer
                ));
                ctx.llm_messages.push(LlmMessage { role: "user".to_string(), content: "Ты потратил весь лимит токенов на внутренние размышления и не дал видимого ответа. На этот раз отвечай СРАЗУ, БЕЗ внутренних размышлений: только итоговый результат.".to_string() });
                continue;
            }
            ctx.consecutive_incomplete += 1;
            if ctx.consecutive_incomplete >= 5 {
                ctx.final_response = format!("{} Агент '{}' не смог сформировать ответ (5 пустых попыток). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                break;
            }
            let hint = if stop_reason == "MAX_TOKENS" || raw_response.contains("<think") {
                "Твои размышления прерваны из-за лимита токенов. Сгенерируй ответ ЗАНОВО с самого начала. СИЛЬНО СОКРАТИ свои внутренние размышления (максимум 2-3 вывода) и сразу переходи к финальному результату."
            } else {
                "Ты прервал генерацию. Продолжи ответ ОБЫЧНЫМ ТЕКСТОМ."
            };
            ctx.llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.clone() } else { raw_response.clone() } });
            ctx.continuation_raw.clear();
            ctx.continuation_mark = None;
            ctx.llm_messages.push(LlmMessage { role: "user".to_string(), content: hint.to_string() });
            continue;
        }

        if !ctx.action_found && ctx.has_tools_for_prompt {
            if has_incomplete_json_action(&parse_target) || has_json_thought_without_action(&parse_target) {
                ctx.consecutive_incomplete += 1;
                if ctx.consecutive_incomplete >= 5 {
                    ctx.final_response = format!("{} Агент '{}' не смог завершить действие (5 попыток). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                    break;
                }
                ctx.llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.clone() } else { raw_response.clone() } });
                ctx.continuation_raw.clear();
                ctx.continuation_mark = None;
                ctx.llm_messages.push(LlmMessage { role: "user".to_string(), content: "Ты начал размышлять в JSON, но не указал действие. Пиши кратко и СРАЗУ укажи \"target\" или \"tool\".".to_string() });
                continue;
            }
        }

        // ── Артефакт докачки: ответ начался с многоточия — модель продолжила
        // оборванный думатель вместо самостоятельного ответа, начало потеряно.
        // НЕ финализируем и НЕ перегенерируем всё с нуля: хвост уже в истории
        // (assistant), просим написать ответ С НАЧАЛА — результат не теряется.
        // Проверяем именно то, что станет финальным ответом (после вырезки
        // думателя), т.к. сырой текст может начинаться с маркеров размышлений.
        let (_, split_answer) = split_thinking_and_answer(&combined);
        if !ctx.action_found && starts_with_ellipsis(&split_answer)
            && ctx.continuation_restarts < MAX_CONTINUATION_RESTARTS && !response.trim().is_empty() {
            ctx.continuation_restarts += 1;
            log_cb(format!(
                "⚠️ [{}] ответ начался с обрыва размышлений («...») — перезапуск ответа с начала (#{}/{}), хвост сохранён в истории",
                agent.name, ctx.continuation_restarts, MAX_CONTINUATION_RESTARTS
            ));
            ctx.llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
            // Ответ будет писаться заново — состояние «докачки» сбрасываем,
            // чтобы следующий вызов не склеивал старый оборванный combined.
            ctx.continuation_raw.clear();
            ctx.continuation_mark = None;
            ctx.llm_messages.push(LlmMessage { role: "user".to_string(), content:
                "⚠️ Твой финальный ответ начался с многоточия — это продолжение оборванных размышлений, а не самостоятельный ответ. Напиши финальный ответ ЗАНОВО с самого начала: вступление и ВСЕ пункты по порядку. Твой текст после многоточия уже сохранён в истории — не повторяй и не продолжай его, не начинай с «...». Начни с полного первого пункта."
                .to_string()
            });
            continue;
        }

        let preview = safe_truncate(&response, 300).replace('\n', " ");
log_cb(format!("✅ Агент {} завершил ответом ({} символов): {}", agent.name, response.len(), preview));
        ctx.final_response = if is_continuation {
            extract_answer_from_combined(&combined, &response)
        } else {
            response
        };
        // Финальный ответ извлечён — состояние «докачки» завершено (иначе следующий
        // вызов парсил бы склеенный combined и терял свежий JSON-конверт).
        ctx.continuation_raw.clear();
        ctx.continuation_mark = None;

        // ── Сигнальная итерация: агент ответил, но конверт emit_signal так и не
        // распознан (модель вернула текст/невалидный JSON). JSON-конверт НЕ должен
        // стать финальным ответом — с single_report он затёр бы реальный отчёт.
        // Ретраим с корректирующим хинтом; при исчерпании — логируем (красная кнопка,
        // core §2.2) и возвращаем анализ агента, жертвуя сигналом, но не отчётом.
        if ctx.signal_attempted && !ctx.signal_saved {
            ctx.signal_retries += 1;
            if ctx.signal_retries <= MAX_SIGNAL_RETRIES {
                if let Some(contract) = &signal_contract {
                    log_cb(format!(
                        "⚠️ [{}] ответ не распознан как emit_signal (попытка {}/{}): {}",
                        agent.id, ctx.signal_retries, MAX_SIGNAL_RETRIES, preview
                    ));
                    ctx.llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                    ctx.llm_messages.push(LlmMessage {
                        role: "user".to_string(),
                        content: signal_retry_hint(&contract.key),
                    });
                    continue;
                }
            }
            // Ретраи исчерпаны: сигнал пропускаем, отчёт агента сохраняем.
            log_cb(format!(
                "⚠️ [{}] сигнал '{}' НЕ сохранён после {} попыток (JSON конверта не распознан). Отчёт агента сохранён, сигнал пропущен.",
                agent.id,
                signal_contract.as_ref().map(|c| c.key.as_str()).unwrap_or("?"),
                MAX_SIGNAL_RETRIES
            ));
            if !ctx.signal_analysis.is_empty() {
                ctx.final_response = ctx.signal_analysis.clone();
            }
        }

        // ── Второй сигнальный вызов: агент ответил свободно, но не вызвал
        // emit_signal. Если для него есть контракт сигнала — делаем ещё ОДИН
        // LLM-вызов СТРОГО под json_schema конверта (анализ остаётся первым).
        if !ctx.signal_attempted && !ctx.signal_saved {
            if let Some(contract) = &signal_contract {
                ctx.signal_attempted = true;
                ctx.signal_analysis = ctx.final_response.clone();
                let schema = build_signal_envelope_schema(contract);
                engine.set_grammar(Some(GrammarSpec { gbnf: None, json_schema: Some(schema) }));
                log_cb(format!("📡 Второй сигнальный вызов агента '{}': emit_signal('{}') под json_schema", agent.id, contract.key));
                ctx.llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                ctx.llm_messages.push(LlmMessage {
                    role: "user".to_string(),
                    content: signal_request_prompt(&contract.key),
                });
continue;
            }
        }
        break;
    }

    if ctx.depth > 0 {
        let subcall = SubCall { agent_name: ctx.agent.name.clone(), prompt: invocation_dump.clone(), response: ctx.final_response.clone(), time_sec: start_time.elapsed().as_secs_f32(), tool_calls: ctx.tool_calls };
        (ctx.subcall_cb)(&subcall);
        ctx.all_sub_calls.push(subcall);
    }

    // 4.2: публикуем событие завершения в шину (успех/ошибка + длительность).
    let err = if ctx.final_response.starts_with("⚠️") { Some(ctx.final_response.clone()) } else { None };
    crate::infra::event_bus::global_bus().publish(crate::infra::event_bus::AgentEvent::Finished {
        agent: ctx.agent.id.clone(),
        namespace: caller_name.as_deref().unwrap_or("main").to_string(),
        ms: start_time.elapsed().as_millis(),
        error: err,
    });
    // 4.5: плагин-слой — уведомление о завершении агента.
    crate::infra::plugins::global_plugins().on_agent_finish(&ctx.agent.id, &ctx.final_response);

    Ok(ctx.final_response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

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
            current_date: false,
        }
    }

    fn parse_wf(yaml: &str) -> WorkflowDef {
        serde_yaml::from_str(yaml).expect("Не удалось распарсить тестовый workflow")
    }

    #[test]
    fn starts_with_ellipsis_detects_continuation_artifact() {
        assert!(starts_with_ellipsis("...принятие решений, требующих участия других"));
        assert!(starts_with_ellipsis("…продолжение мыслей после обрыва"));
        assert!(starts_with_ellipsis("   ... ответ с ведущими пробелами"));
        assert!(!starts_with_ellipsis("Начни ответ с первого пункта"));
        assert!(!starts_with_ellipsis("Согласно данным, у вас есть симптомы"));
        assert!(!starts_with_ellipsis(""));
    }

    #[test]
    fn estimate_chars_per_token_picks_by_cyrillic_share() {
        assert_eq!(estimate_chars_per_token("привет мир", "", ""), 2);
        assert_eq!(estimate_chars_per_token("hello world", "", ""), 3);
        assert_eq!(estimate_chars_per_token("hello world", "привет", ""), 2);
        assert_eq!(estimate_chars_per_token("hello world", "abcdef", "й"), 3);
        assert_eq!(estimate_chars_per_token("", "", ""), 3);
    }

    #[test]
    fn spill_if_large_keeps_small_output_unchanged() {
        let small = "короткий результат инструмента";
        let (out, spilled) = spill_if_large(small, "agent1", 0);
        assert!(!spilled.is_some());
        assert_eq!(out, small);
    }

    #[test]
    fn spill_if_large_writes_file_and_condenses() {
        // ASCII: 9000 байт < лимит read_spill (16000 байт), но > SPILL_THRESHOLD (8000 символов).
        let big = "A".repeat(9000);
        let (out, spilled) = spill_if_large(&big, "agent1", 7);
        let path = spilled.expect("большой вывод должен быть сохранён в spill");
        assert!(path.exists(), "spill-файл должен существовать");
        // Модель видит выжимку, а не 9000 символов
        assert!(out.len() < big.len());
        assert!(out.contains("сохранён в файл spills"));
        // read_spill возвращает полное содержимое
        let restored = read_spill_file(&path.to_string_lossy()).expect("чтение spill");
        assert_eq!(restored, big);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn read_spill_file_rejects_paths_outside_spill_dir() {
        // Попытка прочитать файл вне директории spills должна упасть
        let outside = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "src-tauri/src/main.rs".to_string());
        let res = read_spill_file(&outside);
        assert!(res.is_err(), "чтение вне spill-директории запрещено");
    }

    #[test]
    fn compact_llm_messages_prunes_big_results_and_fits_budget() {
        let mut msgs = vec![
            LlmMessage { role: "system".to_string(), content: "SYS".to_string() },
            LlmMessage { role: "user".to_string(), content: "[РЕЗУЛЬТАТ ИНСТРУМЕНТА big_tool]:\n".to_string() + &"x".repeat(5000) },
            LlmMessage { role: "assistant".to_string(), content: "A".repeat(4000) },
            LlmMessage { role: "user".to_string(), content: "B".repeat(4000) },
            LlmMessage { role: "assistant".to_string(), content: "C".repeat(4000) },
        ];
        // бюджет чуть больше суммы после сворачивания крупного результата
        // (≈12100), чтобы сработала стратегия 1, но не жёсткое удаление.
        compact_llm_messages(&mut msgs, 13000);
        let total: usize = msgs[1..].iter().map(|m| m.content.chars().count()).sum();
        assert!(total <= 13000, "должны влезть в бюджет, получено {}", total);
        // system-промпт сохранён целиком
        assert_eq!(msgs[0].content, "SYS");
        // крупный результат инструмента свёрнут (стратегия 1 сработала)
        assert!(msgs.iter().any(|m| m.content.contains("крупный результат свёрнут")));
    }

    #[test]
    fn compact_llm_messages_keeps_small_conversations_untouched() {
        let mut msgs = vec![
            LlmMessage { role: "system".to_string(), content: "SYS".to_string() },
            LlmMessage { role: "user".to_string(), content: "привет".to_string() },
            LlmMessage { role: "assistant".to_string(), content: "здравствуй".to_string() },
        ];
        compact_llm_messages(&mut msgs, 6000);
        assert_eq!(msgs.len(), 3, "маленький диалог не трогаем");
    }

    #[test]
    fn todo_tool_add_list_done_persists_in_session() {
        let mut msgs: Vec<ChatMessage> = Vec::new();
        let r = run_todo_tool(
            "todo_write",
            &serde_json::json!({"action": "add", "title": "написать тест"}),
            &mut msgs,
            "coder_x",
        );
        assert!(r.contains("Добавлена задача"), "добавление должно подтвердиться");

        let l = run_todo_tool(
            "todo_write",
            &serde_json::json!({"action": "list"}),
            &mut msgs,
            "coder_x",
        );
        assert!(l.contains("написать тест"));
        assert!(l.contains("[ ]"), "новая задача невыполнена");

        let d = run_todo_tool(
            "todo_write",
            &serde_json::json!({"action": "done", "index": 1}),
            &mut msgs,
            "coder_x",
        );
        assert!(d.contains("отмечена выполненной"));

        // Состояние должно пережить вызов и лежать в сессии (thought todo::coder_x).
        let persisted = read_todos(&msgs, "coder_x");
        assert_eq!(persisted.len(), 1);
        assert!(persisted[0].1, "задача отмечена выполненной в сессии");
    }

    #[test]
    fn todo_tool_clear_empties_list() {
        let mut msgs: Vec<ChatMessage> = Vec::new();
        run_todo_tool(
            "todo_write",
            &serde_json::json!({"action": "add", "title": "задача"}),
            &mut msgs,
            "research_x",
        );
        run_todo_tool(
            "todo_write",
            &serde_json::json!({"action": "clear"}),
            &mut msgs,
            "research_x",
        );
        assert!(read_todos(&msgs, "research_x").is_empty(), "после clear список пуст");
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

        let (prompt, has_tools) = build_worst_agent_prompt(&agents, &wf, &[]);
        assert!(prompt.contains("очень длинный системный промпт"), "должен выбраться самый длинный агент");
        assert!(!prompt.contains("коротко"), "короткий агент не должен попасть в результат");
        assert!(!has_tools, "у тестовых агентов нет инструментов");
    }

    #[test]
    fn worst_agent_prompt_ignores_sub_workflow_and_non_worker_nodes() {
        let agents = vec![make_agent("worker_a", "промпт воркера А")];
        let wf = parse_wf(
            "name: test\nnodes:\n  - id: sub\n    type: sub_workflow\n    workflow: other_graph\n  - id: w\n    type: llm_worker\n    agent: worker_a\nedges: []\n",
        );

        let (prompt, _) = build_worst_agent_prompt(&agents, &wf, &[]);
        assert!(prompt.contains("промпт воркера А"));
    }

    #[test]
    fn worst_agent_prompt_empty_when_no_workers() {
        let agents: Vec<AgentProfile> = vec![];
        let wf = parse_wf(
            "name: test\nnodes:\n  - id: r\n    type: return\nedges: []\n",
        );

        let (prompt, has_tools) = build_worst_agent_prompt(&agents, &wf, &[]);
        assert_eq!(prompt, "");
        assert!(!has_tools, "без llm_worker-узлов инструментов нет");
    }

    #[test]
    fn legacy_agent_with_mcp_servers_gets_tools_in_prompt() {
        let mut agent = make_agent("search", "ты поисковик");
        agent.mcp_servers = vec!["web_search".to_string()];
        let tools = runtime::builtin_tools();
        let sp = build_system_prompt(&agent, &[], true, &tools, 2048);
        assert!(sp.contains("[ДОСТУПНЫЕ ИНСТРУМЕНТЫ]"), "legacy-ветка: агент с mcp_servers обязан получать список инструментов");
        assert!(sp.contains("emit_signal"));
        assert!(sp.contains("[ПРАВИЛА ВЫЗОВА ИНСТРУМЕНТОВ]"));
    }

    #[test]
    fn legacy_agent_without_tools_has_no_tools_section() {
        let agent = make_agent("plain", "просто агент");
        let sp = build_system_prompt(&agent, &[], false, &[], 2048);
        assert!(!sp.contains("[ДОСТУПНЫЕ ИНСТРУМЕНТЫ]"));
        assert!(!sp.contains("[ПРАВИЛА ВЫЗОВА ИНСТРУМЕНТОВ]"));
    }

    #[test]
    fn agent_with_current_date_flag_gets_date_block() {
        let mut agent = make_agent("dated", "поисковый агент");
        agent.current_date = true;
        let sp = build_system_prompt(&agent, &[], false, &[], 2048);
        assert!(sp.contains("[ТЕКУЩАЯ ДАТА]"), "агент с current_date: true обязан получать блок даты");
        assert!(sp.starts_with("[ТЕКУЩАЯ ДАТА]"), "блок даты должен быть в начале промпта");
        assert!(sp.contains("Сегодня"), "блок обязан содержать слово «Сегодня»");
        assert!(sp.contains("ЕДИНСТВЕННЫЙ источник истины"));
        assert!(sp.contains("поисковый агент"), "тело агента сохраняется после блока даты");
    }

    #[test]
    fn agent_without_current_date_flag_has_no_date_block() {
        let agent = make_agent("plain", "обычный агент");
        let sp = build_system_prompt(&agent, &[], false, &[], 2048);
        assert!(!sp.contains("[ТЕКУЩАЯ ДАТА]"), "агент без флага не должен получать блок даты");
    }

    #[test]
    fn worst_agent_prompt_marks_tools_when_worker_has_mcp_servers() {
        let mut agent = make_agent("search", "промпт поисковика");
        agent.mcp_servers = vec!["web_search".to_string(), "docs_fetcher".to_string()];
        let wf = parse_wf(
            "name: test\nnodes:\n  - id: n1\n    type: llm_worker\n    agent: search\nedges: []\n",
        );

        let (prompt, has_tools) = build_worst_agent_prompt(&[agent], &wf, &[]);
        assert!(has_tools, "агент с mcp_servers даёт has_tools=true → run_chat добавит TOOL_WORKING_BUDGET к оценке контекста");
        assert!(prompt.contains("[ДОСТУПНЫЕ ИНСТРУМЕНТЫ]"));
        assert!(prompt.contains("emit_signal"));
    }

    // ─────────────────────────── Верификация research-агентов ───────────────────────────
    // Регрессия 14.08.26: docs_researcher/web_researcher отвечали «по памяти» и не вызывали
    // WebFetch. Эти тесты фиксируют контракт: инструменты верификации обязаны доходить до модели.

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .to_path_buf()
    }

    fn real_agent(id: &str) -> AgentProfile {
        let agents_dir = workspace_root().join("agents");
        let agents = load_agents(&agents_dir).expect("агенты должны загрузиться из agents/");
        agents
            .into_iter()
            .find(|a| a.id == id)
            .unwrap_or_else(|| panic!("агент '{id}' не найден в agents/"))
    }

    /// Имена инструментов, зарегистрированных MCP-сервером (парсинг tools-блока .ts-файла).
    fn tools_in_server(server_name: &str) -> Vec<String> {
        let path = workspace_root()
            .join("src-tauri/mcp_servers")
            .join(format!("{server_name}.ts"));
        let src = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut out: Vec<String> = Vec::new();
        for line in src.lines() {
            if let Some(name) = line.trim().strip_prefix("name:").and_then(|s| s.trim().strip_prefix('"')) {
                if let Some(end) = name.find('"') {
                    let t = name[..end].to_string();
                    if t.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
                        out.push(t);
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    fn tools_of_server_as_all_tools(server_name: &str) -> Vec<(String, String, serde_json::Value)> {
        tools_in_server(server_name)
            .into_iter()
            .map(|t| (format!("{server_name}:{t}"), t, serde_json::Value::Null))
            .collect()
    }

    #[test]
    fn docs_researcher_prompt_guarantees_webfetch_reaches_the_model() {
        let agent = real_agent("docs_researcher");
        assert!(
            agent.mcp_servers.iter().any(|s| s == "docs_fetcher"),
            "docs_researcher обязан иметь mcp-сервер docs_fetcher, есть: {:?}",
            agent.mcp_servers
        );
        assert!(agent.current_date, "docs_researcher должен иметь current_date: true (протокол актуальности)");

        let df_tools = tools_in_server("docs_fetcher");
        for t in ["WebFetch", "FetchArticle", "FetchGithubReadme"] {
            assert!(
                df_tools.iter().any(|x| x == t),
                "docs_fetcher.ts не регистрирует инструмент {t}, есть: {df_tools:?}"
            );
        }

        let mut all_tools = tools_of_server_as_all_tools("docs_fetcher");
        all_tools.extend(tools_of_server_as_all_tools("web_search"));
        all_tools.extend(runtime::builtin_tools());
        let sp = build_system_prompt(&agent, &[], true, &all_tools, 2048);

        for t in ["WebFetch", "FetchArticle", "FetchGithubReadme", "WebSearch", "emit_signal"] {
            assert!(sp.contains(t), "промпт docs_researcher не содержит '{t}'");
        }
        assert!(sp.contains("[ДОСТУПНЫЕ ИНСТРУМЕНТЫ]"));
        assert!(
            sp.contains("ЗАПРЕЩЕНО") && sp.contains("WebFetch"),
            "промпт обязан содержать обязательный протокол верификации"
        );
    }

    #[test]
    fn web_researcher_prompt_guarantees_webfetch_reaches_the_model() {
        let agent = real_agent("web_researcher");
        assert!(
            agent.mcp_servers.iter().any(|s| s == "docs_fetcher"),
            "web_researcher обязан иметь mcp-сервер docs_fetcher, есть: {:?}",
            agent.mcp_servers
        );
        let mut all_tools = tools_of_server_as_all_tools("docs_fetcher");
        all_tools.extend(tools_of_server_as_all_tools("web_search"));
        all_tools.extend(runtime::builtin_tools());
        let sp = build_system_prompt(&agent, &[], true, &all_tools, 2048);
        assert!(sp.contains("WebFetch"), "промпт web_researcher не содержит WebFetch");
        assert!(sp.contains("WebSearch"));
    }

    /// Гипотеза 14.08.26: модель сама не вызывает WebFetch даже когда промпт требует
    /// верификации (Q2: serde 1.0.215 вместо реальной 1.0.229). Реальная модель, реальный
    /// промпт: pass = вызов инструмента, fail = ответ текстом по памяти.
    /// Запуск: TEST_MODEL_PATH=... test.bat "docs_researcher_calls_tool_instead_of_guessing_version -- --ignored"
    #[test]
    #[ignore]
    fn docs_researcher_calls_tool_instead_of_guessing_version() {
        let model_path = std::env::var("TEST_MODEL_PATH").expect("Set TEST_MODEL_PATH to a GGUF file path");
        let agent = real_agent("docs_researcher");
        let mut all_tools = tools_of_server_as_all_tools("docs_fetcher");
        all_tools.extend(tools_of_server_as_all_tools("web_search"));
        all_tools.extend(runtime::builtin_tools());
        let mut system_prompt = build_system_prompt(&agent, &[], true, &all_tools, 2048);
        system_prompt.push_str("\n\n[КРИТИЧЕСКОЕ ОГРАНИЧЕНИЕ]\n");
        system_prompt.push_str(prompt::CRITICAL_LIMIT_BLOCK);

        let user_text = "Узнай и сообщи: какая сейчас последняя версия крейта serde? НЕ называй версию по памяти — прочитай официальную страницу через WebFetch и назови версию из неё.";

        let engine_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .map(|d| crate::infra::llamacpp_installer::default_dir(&d))
            .unwrap_or_else(std::path::PathBuf::new);
        // В тестовой сборке движок не лежит рядом с exe — берём директорию из конфига приложения.
        let engine_dir = if engine_dir.join("backends").exists() {
            engine_dir
        } else {
            std::env::var("APPDATA")
                .ok()
                .map(|a| Path::new(&a).join("com.kingorch.app").join("app_config.json"))
                .and_then(|cfg| fs::read_to_string(cfg).ok())
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .and_then(|v| v.get("llamacpp_dir").and_then(|d| d.as_str()).map(PathBuf::from))
                .filter(|p| p.join("backends").exists())
                .unwrap_or(engine_dir)
        };
        let engine = LlamaEngine::new(&engine_dir, &model_path, 8192, false, false, 0, &|_| {}, |_| {}).unwrap();
        let mut params = ModelParams::default();
        params.temperature = 0.8;

        let msgs = vec![
            LlmMessage { role: "system".to_string(), content: system_prompt },
            LlmMessage { role: "user".to_string(), content: user_text.to_string() },
        ];
        let cancel = Arc::new(AtomicBool::new(false));
        let gen = engine
            .generate_chat(&msgs, 1024, &params, "Auto", cancel, "test:docs_researcher_tool", |_, _| {}, |_| {})
            .unwrap();
        let response = gen.text;
        println!("=== RAW RESPONSE ===\n{}\n=== END ===", response);

        match parse_tool_call(&response) {
            Some((tool, args, _)) => {
                println!("TOOL CALL: {tool} {args}");
                assert!(
                    tool == "WebFetch" || tool == "WebSearch" || tool == "FetchArticle",
                    "инструмент '{tool}' не является инструментом верификации"
                );
            }
            None => {
                let preview: String = response.chars().take(500).collect();
                panic!("Модель ответила текстом, не вызвав инструмент верификации (гипотеза подтверждена). Ответ: {preview}");
            }
        }
    }
}
