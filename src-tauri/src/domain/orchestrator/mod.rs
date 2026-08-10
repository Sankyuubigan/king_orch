pub mod prompt;
mod runtime;

pub use runtime::builtin_tools;

use crate::domain::agent_manager::{load_agents, AgentProfile};
use crate::domain::signals::{SignalContract, build_signal_envelope_schema, load_signal_contract};
use crate::domain::workflow_engine::{
    find_workflow_by_stem, load_workflows, WorkflowContext, WorkflowRunner, NodeType, WorkflowDef,
};
use crate::infra::{ChatMessage, ChatAttachment, LlamaEngine, ModelParams, SubCall, ToolCallInfo, push_report, LlmMessage, extract_model_filename, llm_history, GrammarSpec};
use crate::domain::parsers::{
    clean_thought_tags, extract_think_content, extract_thought_from_partial_json,
    has_incomplete_json_action, is_thinking_truncated, parse_orchestrator_response, parse_tool_call,
    split_thinking_and_answer, strip_tool_call,
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

/// True, если текст — сообщение об ошибке агента (workflow должен остановиться fail-fast).
pub(crate) fn is_agent_error(text: &str) -> bool {
    text.trim_start().starts_with(AGENT_ERROR_PREFIX)
}

// ── Параметры «продолжай-цикла» для обрыва генерации по лимиту токенов ──
// Вместо «СГЕНЕРИРУЙ ЗАНОВО» (модель по кругу переписывает те же размышления и
// снова упирается в max_gen) — продолжаем РОВНО с места обрыва (cache_prompt=true
// позволяет движку переиспользовать KV префикса). Накопившиеся размышления при
// перерастании лимита сжимаем отдельным малым LLM-вызовом в тезисы (~300 токенов).
const MAX_CONTINUATIONS: usize = 12;          // предел «докачек» после обрыва
const COMPACT_THRESHOLD_CHARS: usize = 9000;  // накопленных размышлений → сжать в тезисы
const COMPACT_MAX_TOKENS: usize = 300;
const THOUGHT_STORE_MAX_CHARS: usize = 2100;  // в сессию сохраняем мысль срезом (приоритет ответу)

/// Хвост строки длиной ≤ `n` символов (без разрыва UTF-8).
fn tail_chars(s: &str, n: usize) -> String {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    if chars.len() <= n {
        return s.to_string();
    }
    let idx = chars[chars.len() - n].0;
    s[idx..].to_string()
}

/// Извлекает финальный ответ из накопленного текста (размышления вырезаны).
fn extract_answer_from_combined(combined: &str, fallback: &str) -> String {
    let (_, answer) = split_thinking_and_answer(combined);
    let cleaned = clean_thought_tags(&answer);
    if !cleaned.trim().is_empty() {
        return cleaned;
    }
    // Нет распознанных маркеров размышлений → обычная чистка всего текста
    let full = clean_thought_tags(combined);
    if !full.trim().is_empty() { full } else { fallback.to_string() }
}

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
        let acc_chars: usize = llm_messages[mark..]
            .iter()
            .filter(|m| m.role == "assistant")
            .map(|m| m.content.chars().count())
            .sum();
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

/// Загружает per-agent GBNF-грамматику из `grammars_dir/<agent_id>.gbnf`.
/// Если файла нет — агент работает без per-agent грамматики (только база движка).
fn load_agent_grammar(grammars_dir: &Path, agent_id: &str) -> Option<String> {
    let path = grammars_dir.join(format!("{}.gbnf", agent_id));
    match std::fs::read_to_string(&path) {
        Ok(content) if !content.trim().is_empty() => Some(content),
        Ok(_) => None,
        Err(e) => {
            eprintln!("[grammar] {}: {}", path.display(), e);
            None
        }
    }
}

/// Ищет директорию с per-agent GBNF-грамматиками.
/// Структура: `agents/<набор агентов>/grammars/*.gbnf` (напр. `agents/psychotherapist/grammars/`).
/// Приоритет: 1) рядом с workflow (`workflow.parent_dir` = `.../transitions` → `.../grammars`);
/// 2) первая найденная подпапка `<agents_dir>/<папка>/grammars`; 3) fallback `<agents_dir>/grammars`.
pub fn resolve_grammars_dir(agents_dir: &Path, workflow: Option<&WorkflowDef>) -> std::path::PathBuf {
    if let Some(wf) = workflow {
        let candidate = Path::new(&wf.parent_dir)
            .parent()
            .unwrap_or(agents_dir)
            .join("grammars");
        if candidate.is_dir() {
            return candidate;
        }
    }
    if let Ok(entries) = std::fs::read_dir(agents_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let g = entry.path().join("grammars");
                if g.is_dir() {
                    return g;
                }
            }
        }
    }
    agents_dir.join("grammars")
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
            let tools = runtime::builtin_tools();
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

/// Запас на спецтокены и JSON-разметку инструментов при оценке стартового контекста.
const TOKEN_ESTIMATE_RESERVE: u32 = 512;

/// Резерв на рабочий цикл инструментов (JSON tool call + результат(ы) за вызов).
/// Для поисковых агентов выдача движка ~870 токенов: без этого резерва реальный
/// промпт после tool call переполняет оценку, и цикл деградации выбрасывает из
/// контекста вопрос пользователя и сам tool call (см. баг с выдуманной погодой).
const TOOL_WORKING_BUDGET: u32 = 1024;

/// Делитель «символы → токены» для эвристики стартового контекста.
/// Латиница токенизируется ~3 симв/токен, кириллица — плотнее (~2 симв/токен):
/// эвристика /3 для русских промптов занижает оценку, и движок стартует с
/// маленьким контекстом, а обрезка истории срабатывает преждевременно.
/// Делитель выбирается по доле кириллицы во всём будущем промпте.
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
    // (делитель зависит от языка промпта: ~3 симв/токен для латиницы, ~2 для
    // кириллицы) + запас на спецтокены/JSON и токены изображений. Точная подгонка
    // остаётся за циклом обрезки истории в run_agent_node (по точным /tokenize).
    let (worst_system_prompt, worst_has_tools) = match &workflow_match {
        Some(wf) => build_worst_agent_prompt(&agents, wf, &history),
        None => agents.iter().find(|a| a.id == agent_id)
            .map(|agent| {
                let has_tools = !agent.tools.is_empty() || !agent.mcp_servers.is_empty();
                let tools = if has_tools { runtime::builtin_tools() } else { Vec::new() };
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
        LlamaEngine::new_with_mmproj(&engine_dir, &model_path, mmproj_path.as_deref(), engine_ctx_limit, kv_quant_keys, kv_quant_values, log_cb.clone(), stream_cb)?
    } else {
        LlamaEngine::new(&engine_dir, &model_path, engine_ctx_limit, kv_quant_keys, kv_quant_values, log_cb.clone(), stream_cb)?
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
            log_cb.clone(), status_cb, subcall_cb,
            &engine, primary_agent, &agents, user_text, recent_history,
            &attachments,
            max_gen_usize, &model_params, &format_type,
            cancel_flag, 0, &mut all_sub_calls, None, &mcp_servers_dir, &bins_dir,
            &grammars_dir,
            &mut messages_store, &mut msg_counter,
            String::new(),
            stream_meta.clone(), true,
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
    grammars_dir: &Path,
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

    all_tools.extend(runtime::builtin_tools());

    let has_tools_for_prompt = has_real_tools;
    let mut system_prompt = build_system_prompt(agent, messages, has_tools_for_prompt, &all_tools, max_gen_tokens);
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
    // ── Второй сигнальный вызов: агент сначала отвечает текстом (свободно),
    // затем оркестратор ВТОРОЙ итерацией просит emit_signal под json_schema.
    let mut signal_attempted = false;   // второй вызов уже сделан (не зацикливаться)
    let mut signal_saved = false;       // сигнал успешно сохранён
    let mut signal_analysis = String::new(); // текст первого (пользовательского) ответа
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

    // ── Состояние «докачки» после обрыва генерации по лимиту токенов ──
    let mut continuation_count = 0usize;      // сколько раз докачивали оборванную генерацию
    let mut continuation_raw = String::new(); // накопленный сырой текст (для вырезания ответа)
    let mut continuation_mark: Option<usize> = None; // граница сообщений, добавленных при докачке

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
                let removed = &llm_messages[1];
                let chars = removed.content.chars().count();
                let snippet: String = removed.content.chars().take(120).collect();
                log_cb(format!(
                    "⚠️ Превышен лимит контекста: промпт {} + генерация {} > лимита {}. Удалено самое старое сообщение [{}], {} симв.: {}",
                    current_tokens, max_gen_tokens, ideal_ctx, removed.role, chars,
                    if chars > 120 { format!("{}…", snippet) } else { snippet }
                ));
                llm_messages.remove(1);
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

        // ── Режим продолжения: парсим весь накопленный текст, а не последний кусок
        // (JSON/ответ может быть разорван между итерациями докачки) ──
        let is_continuation = continuation_mark.is_some();
        let combined = if is_continuation {
            format!("{}\n{}", continuation_raw, raw_response)
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
            if stop_reason == "MAX_TOKENS" && is_thinking_truncated(&combined) {
                if push_continuation_for_cutoff(
                    &log_cb, &agent.id, engine, model_params, format_type, cancel_flag.clone(),
                    &ctx_label, stream_meta.clone(), &combined, &parse_target, &raw_response,
                    &mut llm_messages, &mut continuation_raw, &mut continuation_mark, &mut continuation_count,
                )? {
                    final_response = format!("{} Агент '{}' не смог завершить размышления после {} докачек (модель упирается в лимит токенов). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id, MAX_CONTINUATIONS);
                    break;
                }
                continue;
            }
            if stop_reason == "STOP_WORD" || stop_reason == "MAX_TOKENS" {
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
                &mut llm_messages, &mut continuation_raw, &mut continuation_mark, &mut continuation_count,
            )? {
                final_response = format!("{} Агент '{}' не смог завершить ответ после {} докачек (модель упирается в лимит токенов). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id, MAX_CONTINUATIONS);
                break;
            }
            continue;
        }

        if let Some((tool_name, arguments, thought)) = parse_tool_call(&parse_target) {
            action_found = true;
            consecutive_incomplete = 0;
            log_agent_thought(&log_cb, agent, "инструмент", &tool_name, &thought, gen_start.elapsed().as_secs_f32(), depth);
            thought_logged = true;

            status_cb(format!("Выполнение {}...", tool_name), 60);
            let args_str = arguments.to_string();
            log_cb(format!("🔧 Агент '{}' вызвал инструмент {}: {}", agent.name, tool_name, safe_truncate(&args_str, 200)));
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
                    signal_saved = true;
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
                    let (analysis, _) = strip_tool_call(&parse_target);
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
                    // На втором (сигнальном) вызове пользовательский ответ агента —
                    // это результат ПЕРВОГО вызова: возвращаем его, а не голый JSON.
                    final_response = if signal_analysis.is_empty() {
                        analysis
                    } else {
                        signal_analysis.clone()
                    };
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
                    llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.clone() } else { raw_response.clone() } });
                    continuation_raw.clear();
                    continuation_mark = None;
                    llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("⚠️ ОШИБКА_СИНТАКСИСА: ты использовал 'tool' для вызова сабагента '{}'. Это сабагент, а не инструмент. Исправь: используй 'target'. Пример: {{\"thought\": \"...\", \"target\": \"{}\", \"task_or_response\": \"...\"}}.", tool_name, tool_name) });
                    continue;
                }
            }
            let output = tool_output.unwrap_or_else(|| format!("Ошибка: Инструмент '{}' не найден.", tool_name));
            log_cb(format!("🔧 Инструмент '{}' (агент '{}') вернул результат ({} символов): {}", tool_name, agent.name, output.chars().count(), safe_truncate(&output, 300)));

            if depth == 0 && tool_found && tool_name != "emit_signal" {
                let stored = safe_truncate(&output, THOUGHT_STORE_MAX_CHARS);
                messages.push(ChatMessage {
                    id: Some(format!("msg_{}", msg_counter)),
                    msg_type: "thought".to_string(),
                    content: format!("🔧 Вызван инструмент {}: {}\nРезультат: {}", tool_name, safe_truncate(&args_str, 200), stored),
                    sub_calls: None,
                    author: Some(agent.id.clone()),
                    model: Some(extract_model_filename(&engine.model_path)),
                });
                *msg_counter += 1;
            }

            if !tool_found || output.starts_with("Ошибка") {
                consecutive_failed_tools += 1;
                if consecutive_failed_tools >= 3 {
                    final_response = format!("{} Лимит неудачных вызовов инструмента ({}). Агент: '{}'. Инструмент: '{}'. Невозможно продолжить.", AGENT_ERROR_PREFIX, consecutive_failed_tools, agent.id, tool_name);
                    break;
                }
                tool_calls.push(ToolCallInfo { tool_name: tool_name.clone(), arguments: args_str, result: output.clone() });
                llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.clone() } else { raw_response.clone() } });
                continuation_raw.clear();
                continuation_mark = None;
                llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\n⚠️ Инструмент вернул ошибку. Используй другой инструмент или заверши через {{\"target\": \"reply\"}}.", tool_name, output) });
                continue;
            }
            consecutive_failed_tools = 0;
            tool_calls.push(ToolCallInfo { tool_name: tool_name.clone(), arguments: args_str, result: output.clone() });
            llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.clone() } else { raw_response.clone() } });
            continuation_raw.clear();
            continuation_mark = None;
            llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\nЕсли задача выполнена — ответь ОБЫЧНЫМ ТЕКСТОМ.", tool_name, output) });
            continue;
        }

        if let Some(parsed) = parse_orchestrator_response(&parse_target) {
            action_found = true;
            consecutive_incomplete = 0;

            if parsed.target == "reply" || parsed.target == "user" {
                if parsed.content.is_empty() {
                    final_response = if is_continuation {
                        extract_answer_from_combined(&combined, &response)
                    } else {
                        response.clone()
                    };
                } else {
                    final_response = parsed.content;
                }

                // ── Второй сигнальный вызов (та же логика, что в конце цикла):
                // агент ответил через reply, но сигнал по контракту не эмичен.
                if !signal_attempted && !signal_saved {
                    if let Some(contract) = &signal_contract {
                        signal_attempted = true;
                        signal_analysis = final_response.clone();
                        let schema = build_signal_envelope_schema(contract);
                        engine.set_grammar(Some(GrammarSpec { gbnf: None, json_schema: Some(schema) }));
                        log_cb(format!("📡 Второй сигнальный вызов агента '{}' (из reply): emit_signal('{}') под json_schema", agent.id, contract.key));
                        llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                        llm_messages.push(LlmMessage {
                            role: "user".to_string(),
                            content: format!(
                                "Отлично. Теперь сохрани результат анализа как сигнал: вызови инструмент emit_signal с key=\"{}\" и value по контракту (точно той структуры, как описано в системном промпте). Ответь ТОЛЬКО JSON с вызовом эмиссии — без пояснений.",
                                contract.key
                            ),
                        });
                        continue;
                    }
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
                    grammars_dir,
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
                llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.clone() } else { raw_response.clone() } });
                continuation_raw.clear();
                continuation_mark = None;
                llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("Отчет от {}:\n{}\n\nЕсли достаточно — ответь ОБЫЧНЫМ ТЕКСТОМ.", subagent.name, truncate_result(&sub_result, 2000)) });
                continue;
            } else {
                consecutive_invalid_targets += 1;
                if consecutive_invalid_targets >= 3 {
                    log_cb(format!("❌ {} превысил лимит неверных target-вызовов (3).", agent.name));
                    final_response = format!("{} Агент '{}' вызывает несуществующего сабагента '{}'. Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id, parsed.target);
                    break;
                }
                llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.clone() } else { raw_response.clone() } });
                continuation_raw.clear();
                continuation_mark = None;
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
            // В режиме докачки мысли ищем в накопленном сыром тексте
            let thought_source = if is_continuation { &combined } else { &raw_response };
            let extracted = extract_think_content(thought_source);
            for t in &extracted {
                let stored = safe_truncate(t, THOUGHT_STORE_MAX_CHARS);
                log_cb(format!("💭 Мысль {} [d={}] (размышление) [⏱{:.1}с]: {}", agent.name, depth, gen_start.elapsed().as_secs_f32(), stored));
                messages.push(ChatMessage {
                    id: Some(format!("msg_{}", msg_counter)),
                    msg_type: "thought".to_string(),
                    content: stored,
                    sub_calls: None,
                    author: Some(agent.id.clone()),
                    model: Some(extract_model_filename(&engine.model_path)),
                });
                *msg_counter += 1;
            }
            if extracted.is_empty() && !thought_source.contains("<think") {
                if let Some(t) = extract_thought_from_partial_json(thought_source) {
                    let stored = safe_truncate(&t, THOUGHT_STORE_MAX_CHARS);
                    log_cb(format!("💭 Мысль {} [d={}] (размышление) [⏱{:.1}с]: {}", agent.name, depth, gen_start.elapsed().as_secs_f32(), stored));
                    messages.push(ChatMessage {
                        id: Some(format!("msg_{}", msg_counter)),
                        msg_type: "thought".to_string(),
                        content: stored,
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
            llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.clone() } else { raw_response.clone() } });
            continuation_raw.clear();
            continuation_mark = None;
            llm_messages.push(LlmMessage { role: "user".to_string(), content: hint.to_string() });
            continue;
        }

        if !action_found && has_tools_for_prompt {
            if has_incomplete_json_action(&parse_target) || has_json_thought_without_action(&parse_target) {
                consecutive_incomplete += 1;
                if consecutive_incomplete >= 5 {
                    final_response = format!("{} Агент '{}' не смог завершить действие (5 попыток). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id);
                    break;
                }
                llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.clone() } else { raw_response.clone() } });
                continuation_raw.clear();
                continuation_mark = None;
                llm_messages.push(LlmMessage { role: "user".to_string(), content: "Ты начал размышлять в JSON, но не указал действие. Пиши кратко и СРАЗУ укажи \"target\" или \"tool\".".to_string() });
                continue;
            }
        }

        let preview = safe_truncate(&response, 300).replace('\n', " ");
        log_cb(format!("✅ Агент {} завершил ответом ({} символов): {}", agent.name, response.len(), preview));
        final_response = if is_continuation {
            extract_answer_from_combined(&combined, &response)
        } else {
            response
        };

        // ── Второй сигнальный вызов: агент ответил свободно, но не вызвал
        // emit_signal. Если для него есть контракт сигнала — делаем ещё ОДИН
        // LLM-вызов СТРОГО под json_schema конверта (анализ остаётся первым).
        if !signal_attempted && !signal_saved {
            if let Some(contract) = &signal_contract {
                signal_attempted = true;
                signal_analysis = final_response.clone();
                let schema = build_signal_envelope_schema(contract);
                engine.set_grammar(Some(GrammarSpec { gbnf: None, json_schema: Some(schema) }));
                log_cb(format!("📡 Второй сигнальный вызов агента '{}': emit_signal('{}') под json_schema", agent.id, contract.key));
                llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                llm_messages.push(LlmMessage {
                    role: "user".to_string(),
                    content: format!(
                        "Отлично. Теперь сохрани результат анализа как сигнал: вызови инструмент emit_signal с key=\"{}\" и value по контракту (точно той структуры, как описано в системном промпте). Ответь ТОЛЬКО JSON с вызовом эмиссии — без пояснений.",
                        contract.key
                    ),
                });
                continue;
            }
        }
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
            current_date: false,
        }
    }

    fn parse_wf(yaml: &str) -> WorkflowDef {
        serde_yaml::from_str(yaml).expect("Не удалось распарсить тестовый workflow")
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
}