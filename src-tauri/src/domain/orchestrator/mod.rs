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
    has_incomplete_json_action, is_thinking_truncated, needs_cutoff_continuation,
    parse_orchestrator_response, parse_tool_call, split_thinking_and_answer, strip_tool_call,
};
use prompt::build_system_prompt;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::io::Write;

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
// Повторные попытки «ответа с начала»: модель завершила докачку, но начала
// финальный ответ с многоточия (продолжила оборванный думатель, начало ответа
// потеряно). Не перегенерируем всё заново — хвост остаётся в истории, просим
// написать начало; при исчерпании попыток принимаем ответ как есть.
const MAX_CONTINUATION_RESTARTS: usize = 3;
const COMPACT_THRESHOLD_CHARS: usize = 6000;  // накопленных размышлений → сжать в тезисы
const COMPACT_MAX_TOKENS: usize = 300;
const THOUGHT_STORE_MAX_CHARS: usize = 2100;  // в сессию сохраняем мысль срезом (приоритет ответу)
// ── Умное завершение докачек: серия итераций без прогресса (видимого текста нет,
// размышления перестали расти) = модель зациклилась в думателе — докачку прекращаем
// и переходим к перегенерации с хинтом-запретом думателей. ──
const MAX_STALLED_CONTINUATIONS: usize = 3;   // докачек подряд без прогресса
const MIN_THINKING_GROWTH_CHARS: isize = 128; // порог «размышления растут» за одну докачку
// Предел повторов второго (сигнального) вызова emit_signal. Если модель не вернула
// распознаваемый JSON-конверт — ретраим с корректирующим хинтом, затем (не теряя
// отчёт агента!) пропускаем сигнал с логом, а не подставляем JSON вместо ответа.
const MAX_SIGNAL_RETRIES: usize = 3;

/// Промпт для сигнального LLM-вызова: сохранить результат анализа как сигнал.
fn signal_request_prompt(contract_key: &str) -> String {
    format!(
        "Отлично. Теперь сохрани результат анализа как сигнал: вызови инструмент emit_signal с key=\"{}\" и value по контракту (точно той структуры, как описано в системном промпте). Ответь ТОЛЬКО JSON с вызовом эмиссии — без пояснений.",
        contract_key
    )
}

/// Корректирующий хинт для ретрая сигнального вызова: модель должна вернуть
/// РОВНО один JSON-конверт emit_signal (без markdown и пояснений).
fn signal_retry_hint(contract_key: &str) -> String {
    format!(
        "⚠️ Твой ответ не был распознан как вызов emit_signal. Верни РОВНО ОДИН JSON без пояснений и без markdown: {{\"tool\": \"emit_signal\", \"arguments\": {{\"key\": \"{}\", \"value\": <значение по контракту из системного промпта>}}}}.",
        contract_key
    )
}

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

fn safe_truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len { return s.to_string(); }
    let end = s.char_indices()
        .take_while(|(i, _)| *i < max_len)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(max_len.min(s.len()));
    format!("{}...", &s[..end])
}

/// Артефакт докачки: финальный ответ начинается с многоточия — модель
/// продолжила оборванный думатель вместо того, чтобы начать ответ с начала.
fn starts_with_ellipsis(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with("...") || t.starts_with('…')
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
    mmproj_path: Option<String>,     cancel_flag: Arc<AtomicBool>,
    stream_meta: Arc<Mutex<StreamMeta>>,
    prompt_log: Option<std::path::PathBuf>,
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
            prompt_log: prompt_log.clone(),
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
            prompt_log.clone(),
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
/// Снимок ТОЧНОГО входа модели (`llm_messages`) перед каждым вызовом LLM.
///
/// Реализует правило «модель видит только то, что записано»: сам факт записи
/// всего отправленного делает вход воспроизводимым (база replay-тестов без модели)
/// и устраняет риск «тихо показать модели то, чего нет в логе».
/// Запись best-effort: ошибки НЕ фатальны (правило 2.2 — логируем, не падаем).
fn write_prompt_log(path: &Path, agent: &str, call: usize, tokens: usize, messages: &[LlmMessage]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let entry = serde_json::json!({
        "ts": ts,
        "agent": agent,
        "call": call,
        "tokens": tokens,
        "messages": messages,
    });
    let line = serde_json::to_string(&entry).unwrap_or_default();
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => { let _ = writeln!(f, "{}", line); }
        Err(e) => { eprintln!("[prompt_log] не удалось записать {}: {}", path.display(), e); }
    }
}

/// Результаты инструментов длиннее этого порога сохраняются в spill-файл,
/// а модели отдаётся выжимка (head + tail) с локатором. Лечит раздувание
/// контекста: раньше модель видела полный вывод инструмента (mod.rs:~1122),
/// что убивало контекст на длинных выдачах (поиск, чтение файлов, логи).
const SPILL_THRESHOLD: usize = 8000;

/// Директория spills рядом с исполняемым файлом (правило: не писать в cwd).
pub(crate) fn spill_root_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|pp| pp.join("spill")))
        .unwrap_or_else(|| std::path::PathBuf::from("spill"))
}

fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

/// Если вывод инструмента большой — пишет полный текст в spill-файл и
/// возвращает выжимку (head 2000 + tail 1000) с локатором для встроенного
/// инструмента `read_spill`. Иначе возвращает текст как есть, без spill.
fn spill_if_large(output: &str, agent_id: &str, idx: u32) -> (String, Option<std::path::PathBuf>) {
    if output.len() <= SPILL_THRESHOLD {
        return (output.to_string(), None);
    }
    let root = spill_root_dir();
    let _ = std::fs::create_dir_all(&root);
    let fname = format!("spill_{}_{}.txt", sanitize_name(agent_id), idx);
    let fpath = root.join(&fname);
    if std::fs::write(&fpath, output).is_err() {
        return (output.to_string(), None);
    }
    let head: String = output.chars().take(2000).collect();
    let mut tail_chars: Vec<char> = output.chars().rev().take(1000).collect();
    tail_chars.reverse();
    let tail: String = tail_chars.into_iter().collect();
    let display = format!(
        "[РЕЗУЛЬТАТ ИНСТРУМЕНТА сохранён в файл spills]\n{}\n\n... [полный результат {} символов: {}] ...\n\n{}\n\nЧтобы дочитать полностью, вызови инструмент read_spill с аргументом {{\"path\": \"{}\"}}.",
        head, output.len(), fpath.display(), tail, fpath.display()
    );
    (display, Some(fpath))
}

/// Встроенный инструмент `read_spill`: читает spill-файл (только внутри
/// директории spills) и возвращает содержимое, обрезанное до 16К символов.
pub(crate) fn read_spill_file(path: &str) -> Result<String, String> {
    let p = std::path::Path::new(path);
    // Канонизируем оба пути: на Windows canonicalize добавляет префикс \\?\,
    // поэтому сравнивать нужно канонизированные версии.
    let root_abs = spill_root_dir()
        .canonicalize()
        .unwrap_or_else(|_| spill_root_dir());
    let abs = p
        .canonicalize()
        .map_err(|e| format!("Невалидный путь spill: {}", e))?;
    if !abs.starts_with(&root_abs) {
        return Err("Чтение разрешено только внутри директории spills".to_string());
    }
    let content =
        std::fs::read_to_string(&abs).map_err(|e| format!("Ошибка чтения spill: {}", e))?;
    if content.len() > 16000 {
        Ok(format!(
            "{}...\n[обрезано до 16000 символов]",
            &content[..16000]
        ))
    } else {
        Ok(content)
    }
}

/// Единый pipeline компакции контекста перед генерацией. Работает ТОЛЬКО с
/// не-system сообщениями (system-промпт сохраняется целиком). Стратегии по
/// возрастанию агрессивности:
///   1) сворачиваем крупные (ещё не spilled) результаты инструментов в указатель;
///   2) сжимаем самые старые сообщения в одну выжимку (head-эксцерпты);
///   3) жёстко удаляем самые старые, пока не влезем в бюджет (fallback).
/// Бюджет в символах считается вызывающим (global_ctx_limit − max_gen_tokens)·2
/// (консервативно: для кириллицы 2 символа/токен — компактим раньше, чем нужно).
/// Отчёт о проделанной компакции — чтобы вызывающий ОБЯЗАТЕЛЬНО записал в лог,
/// что из контекста было удалено/свёрнуто (правило: тихих операций с данными нет).
pub(crate) struct CompactionReport {
    pub tool_results_pruned: usize,
    pub history_compressed: bool,
    pub old_messages_dropped: usize,
}

/// Ключ хранения чек-листа задач агента в сессии (как `thought`-сообщение).
fn todo_store_key(agent_id: &str) -> String {
    format!("todo::{}", agent_id)
}

/// Прочитать чек-лист задач агента из сессии (или пустой список).
fn read_todos(messages: &[ChatMessage], agent_id: &str) -> Vec<(String, bool)> {
    let key = todo_store_key(agent_id);
    for m in messages {
        if m.msg_type == "thought" && m.author.as_deref() == Some(&key) {
            if let Ok(v) = serde_json::from_str::<Vec<(String, bool)>>(&m.content) {
                return v;
            }
        }
    }
    Vec::new()
}

/// Записать/обновить чек-лист задач агента в сессии (персистится, переживает компакцию).
fn write_todos(messages: &mut Vec<ChatMessage>, agent_id: &str, todos: &[(String, bool)]) {
    let key = todo_store_key(agent_id);
    let content = serde_json::to_string(todos).unwrap_or_default();
    for m in messages.iter_mut() {
        if m.msg_type == "thought" && m.author.as_deref() == Some(&key) {
            m.content = content;
            return;
        }
    }
    messages.push(ChatMessage {
        id: None,
        msg_type: "thought".to_string(),
        content,
        sub_calls: None,
        author: Some(key),
        model: None,
    });
}

/// Исполнение туду-инструментов (`todo_write` / `todo_list`).
fn run_todo_tool(
    tool_name: &str,
    arguments: &serde_json::Value,
    messages: &mut Vec<ChatMessage>,
    agent_id: &str,
) -> String {
    let mut todos = read_todos(messages, agent_id);
    match tool_name {
        "todo_list" => {
            if todos.is_empty() {
                return "📋 Список задач пуст.".to_string();
            }
            let mut s = String::from("📋 Список задач:\n");
            for (i, (t, done)) in todos.iter().enumerate() {
                s.push_str(&format!("{}. [{}] {}\n", i + 1, if *done { "x" } else { " " }, t));
            }
            s
        }
        "todo_write" => {
            let action = arguments
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("add");
            match action {
                "list" => run_todo_tool("todo_list", arguments, messages, agent_id),
                "add" => {
                    let title = arguments
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if title.trim().is_empty() {
                        return "❌ Ошибка: для добавления нужен 'title' (текст задачи).".to_string();
                    }
                    todos.push((title.clone(), false));
                    write_todos(messages, agent_id, &todos);
                    format!("✅ Добавлена задача '{}'. Всего задач: {}.", title, todos.len())
                }
                "done" | "remove" => {
                    let idx = resolve_todo_index(arguments, &todos);
                    match idx {
                        Some(i) => {
                            if action == "done" {
                                let t = todos[i].0.clone();
                                todos[i].1 = true;
                                write_todos(messages, agent_id, &todos);
                                format!("✅ Задача '{}' отмечена выполненной.", t)
                            } else {
                                let t = todos.remove(i).0;
                                write_todos(messages, agent_id, &todos);
                                format!("🗑 Удалена задача '{}'.", t)
                            }
                        }
                        None => "❌ Ошибка: укажи 'index' (номер задачи) или 'title'.".to_string(),
                    }
                }
                "clear" => {
                    write_todos(messages, agent_id, &[]);
                    "🗑 Список задач очищен.".to_string()
                }
                _ => "❌ Ошибка: неизвестное действие. Используй add/done/remove/clear/list.".to_string(),
            }
        }
        _ => "❌ Неизвестный todo-инструмент.".to_string(),
    }
}

/// Найти индекс задачи по `index` (1-based) или по `title` (подстрока).
fn resolve_todo_index(
    arguments: &serde_json::Value,
    todos: &[(String, bool)],
) -> Option<usize> {
    if let Some(i) = arguments.get("index").and_then(|v| v.as_u64()) {
        let i = i as usize;
        if i >= 1 && i <= todos.len() {
            return Some(i - 1);
        }
    }
    if let Some(t) = arguments.get("title").and_then(|v| v.as_str()) {
        let t = t.trim().to_lowercase();
        return todos.iter().position(|(title, _)| title.to_lowercase().contains(&t));
    }
    None
}

fn compact_llm_messages(messages: &mut Vec<LlmMessage>, budget_chars: usize) -> CompactionReport {
    let mut report = CompactionReport {
        tool_results_pruned: 0,
        history_compressed: false,
        old_messages_dropped: 0,
    };
    if messages.len() <= 1 {
        return report;
    }
    let total_chars = |msgs: &[LlmMessage]| -> usize {
        msgs[1..].iter().map(|m| m.content.chars().count()).sum()
    };

    if total_chars(messages) <= budget_chars {
        return report;
    }

    // Стратегия 1: сворачиваем крупные результаты инструментов (кроме уже
    // spilled — у них важен локатор пути для read_spill).
    for m in messages.iter_mut().skip(1) {
        if m.content.contains("[РЕЗУЛЬТАТ ИНСТРУМЕНТА")
            && !m.content.contains("сохранён в файл spills")
            && m.content.chars().count() > 1500
        {
            let tool = m
                .content
                .lines()
                .next()
                .map(|l| {
                    l.trim_start_matches("[РЕЗУЛЬТАТ ИНСТРУМЕНТА ")
                        .trim_end_matches("]:")
                        .trim()
                        .to_string()
                })
                .unwrap_or_else(|| "инструмент".to_string());
            m.content = format!(
                "[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}] — крупный результат свёрнут для экономии контекста (полный текст в истории сессии).]",
                tool
            );
            report.tool_results_pruned += 1;
        }
    }

    if total_chars(messages) <= budget_chars {
        return report;
    }

    // Стратегия 2: сжимаем самые старые сообщения в одну выжимку, сохраняя
    // system-промпт и `keep_recent` последних сообщений.
    let keep_recent = 4usize;
    let n = messages.len();
    if n > keep_recent + 2 {
        let compress_end = n - keep_recent;
        let mut summary = String::from("[СЖАТАЯ ИСТОРИЯ]\n");
        for m in messages.iter().take(compress_end).skip(1) {
            let excerpt: String = m.content.chars().take(200).collect();
            summary.push_str(&format!("({}) {}…\n", m.role, excerpt));
        }
        let dropped = compress_end - 1; // сколько старых сообщений ушло в выжимку
        messages.drain(1..compress_end);
        messages.insert(
            1,
            LlmMessage { role: "system".to_string(), content: summary },
        );
        report.old_messages_dropped += dropped;
        report.history_compressed = true;
    }

    // Стратегия 3: жёстко удаляем самые старые, пока не влезем.
    while messages.len() > 2 {
        if total_chars(messages) <= budget_chars {
            break;
        }
        messages.remove(1);
        report.old_messages_dropped += 1;
    }

    report
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
    messages: &mut Vec<ChatMessage>, msg_counter: &mut u32,
    injected_reports: String,
    stream_meta: Arc<Mutex<StreamMeta>>,
    allow_stream: bool,
    prompt_log: Option<std::path::PathBuf>,
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

    let mut mcp_clients = HashMap::new();
    let mut all_tools: Vec<(String, String, serde_json::Value)> = Vec::new();
    runtime::load_mcp_servers(&log_cb, mcp_servers_dir, bins_dir, &agent.mcp_servers, &mut mcp_clients, &mut all_tools);

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

let mut final_response = String::new();
    let mut tool_calls = Vec::new();
    let start_time = Instant::now();
    let mut consecutive_failed_tools = 0;
    let mut spill_idx: u32 = 0;
    let mut consecutive_incomplete = 0;
    let mut consecutive_invalid_targets = 0;
    // Докачка: длина накопленных размышлений на прошлой докачке и серия
    // «застойных» докачек без прогресса (для умного завершения).
    let mut last_thinking_len: isize = 0;
    let mut stalled_continuations = 0usize;
    // ── Второй сигнальный вызов: агент сначала отвечает текстом (свободно),
    // затем оркестратор ВТОРОЙ итерацией просит emit_signal под json_schema.
    let mut signal_attempted = false;   // второй вызов уже сделан (не зацикливаться)
    let mut signal_saved = false;       // сигнал успешно сохранён
    let mut signal_retries = 0usize;    // неудачных попыток распознать JSON-конверт
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
    let mut continuation_restarts = 0usize;   // ретраев «ответа с начала» (артефакт «...»)
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

        // ── Снимок точного входа модели (правило «модель видит только записанное») ──
        if let Some(ref pl) = prompt_log {
            let logged_tokens = engine.get_tokens_count(&llm_messages, format_type).unwrap_or(0);
            write_prompt_log(pl, &agent.name, iter, logged_tokens, &llm_messages);
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
            // Критерий входа — СОДЕРЖИМОЕ ответа (незакрытый думатель), а не причина
            // остановки: обрыв случается и по лимиту токенов, и по стоп-слову, и по EOS.
            if needs_cutoff_continuation(&combined, &stop_reason) {
                // Умное завершение: серия докачек без прогресса (видимого текста нет,
                // думание перестало расти) = модель зациклилась в думателе — докачку
                // прекращаем и уходим в перегенерацию с хинтом-запретом думателей.
                if stalled_continuations >= MAX_STALLED_CONTINUATIONS {
                    log_cb(format!(
                        "🛑 Докачка забуксовала: {} итераций подряд без роста размышлений и без видимого ответа — переключаемся на перегенерацию.",
                        stalled_continuations
                    ));
                    stalled_continuations = 0;
                    last_thinking_len = 0;
                } else {
                    // Новая серия докачек (стейт «докачки» сброшен) — стартуем с чистых метрик
                    if continuation_mark.is_none() {
                        last_thinking_len = 0;
                        stalled_continuations = 0;
                    }
                    let thinking_len = combined.chars().count() as isize;
                    let grew = thinking_len - last_thinking_len;
                    let raw_before = continuation_raw.len();
                    let exhausted = push_continuation_for_cutoff(
                        &log_cb, &agent.id, engine, model_params, format_type, cancel_flag.clone(),
                        &ctx_label, stream_meta.clone(), &combined, &parse_target, &raw_response,
                        &mut llm_messages, &mut continuation_raw, &mut continuation_mark, &mut continuation_count,
                    )?;
                    if exhausted {
                        final_response = format!("{} Агент '{}' не смог завершить размышления после {} докачек (модель упирается в лимит токенов). Невозможно продолжить.", AGENT_ERROR_PREFIX, agent.id, MAX_CONTINUATIONS);
                        break;
                    }
                    last_thinking_len = thinking_len;
                    // Компакт размышлений — это прогресс (факты резюмируются в тезисы),
                    // серию «застоя» в этом случае сбрасываем, а не считаем застой.
                    let compacted = continuation_raw.len() < raw_before;
                    if !compacted && grew < MIN_THINKING_GROWTH_CHARS {
                        stalled_continuations += 1;
                        log_cb(format!(
                            "⚠️ Докачка #{}: размышления не растут (+{} симв.), видимого ответа нет — застой {}/{}",
                            continuation_count, grew, stalled_continuations, MAX_STALLED_CONTINUATIONS
                        ));
                    } else {
                        stalled_continuations = 0;
                    }
                    continue;
                }
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
                    "Ты прервал генерацию. ЗАПРЕЩЕНО начинать с размышлений в тегах (<think, 思考, thinking, <|channel>thought) — они запрещены. Сразу пиши финальный ответ ОБЫЧНЫМ ТЕКСТОМ без JSON."
                };
                llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                // Хинт требует ответ ЗАНОВО — стейт незавершённой докачки сбрасываем,
                // иначе следующий вызов парсил бы склеенный combined устаревших кусков.
                continuation_raw.clear();
                continuation_mark = None;
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
            } else if tool_name == "read_spill" {
                // Встроенный инструмент дочитки больших результатов инструментов.
                tool_found = true;
                let p = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                match read_spill_file(&p) {
                    Ok(content) => { tool_output = Some(content); }
                    Err(e) => { tool_output = Some(format!("Ошибка read_spill: {}", e)); }
                }
            } else if tool_name == "todo_write" || tool_name == "todo_list" {
                // 4.1: opt-in чек-лист задач (доступен только агентам coder/research).
                tool_found = true;
                let result = run_todo_tool(&tool_name, &arguments, messages, &agent.id);
                tool_output = Some(result);
            } else if let Some((mcp_name, _, _)) = all_tools.iter().find(|(_, name, _)| name == &tool_name) {
                if let Some(client) = mcp_clients.get_mut(mcp_name) {
                    tool_found = true;
                    match client.call_tool(&tool_name, arguments) {
                        Ok(res) => {
                            tool_output = Some(res);
                            consecutive_failed_tools = 0;
                            crate::infra::event_bus::global_bus().publish(
                                crate::infra::event_bus::AgentEvent::ToolCall {
                                    agent: agent.id.clone(),
                                    tool: tool_name.clone(),
                                },
                            );
                        }
                        Err(e) => { tool_output = Some(format!("Ошибка '{}': {}", tool_name, e)); }
                    }
                }
            }
            if !tool_found {
                if agents.iter().any(|a| a.id == tool_name && a.id != agent.id) {
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
            let mut output = tool_output.unwrap_or_else(|| format!("Ошибка: Инструмент '{}' не найден.", tool_name));
            // 4.5: плагин-слой — точка расширения результата инструмента (pass-through по умолчанию).
            crate::infra::plugins::global_plugins().on_tool_result(&agent.id, &tool_name, &mut output);
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
            // Большие результаты — в spill-файл, модели отдаём выжимку (лечит
            // раздувание контекста). Счётчик spill_idx уникален в рамках вызова.
            let (model_output, _spilled) = spill_if_large(&output, &agent.id, spill_idx);
            spill_idx += 1;
            llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.clone() } else { raw_response.clone() } });
            continuation_raw.clear();
            continuation_mark = None;
            llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\nЕсли задача выполнена — ответь ОБЫЧНЫМ ТЕКСТОМ.", tool_name, model_output) });
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

                // Ответ завершён обычным текстом — состояние «докачки» больше не нужно.
                // Без сброса следующий вызов (например, сигнальный emit_signal) парсил бы
                // склеенный combined и терял свежий JSON-конверт.
                continuation_raw.clear();
                continuation_mark = None;

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
                            content: signal_request_prompt(&contract.key),
                        });
                        continue;
                    }
                }

                // ── Сигнальная итерация: модель снова вернула reply, а не
                // JSON-конверт — ретраим с корректирующим хинтом. При исчерпании
                // попыток сигнал пропускаем (красная кнопка, core §2.2), отчёт сохраняем.
                if signal_attempted && !signal_saved {
                    signal_retries += 1;
                    if signal_retries <= MAX_SIGNAL_RETRIES {
                        if let Some(contract) = &signal_contract {
                            log_cb(format!(
                                "⚠️ [{}] ответ не распознан как emit_signal (попытка {}/{}): {}",
                                agent.id,
                                signal_retries,
                                MAX_SIGNAL_RETRIES,
                                safe_truncate(&final_response, 80)
                            ));
                            llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                            llm_messages.push(LlmMessage {
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
                    if !signal_analysis.is_empty() {
                        final_response = signal_analysis.clone();
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
                    prompt_log.clone(),
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

        // ── Артефакт докачки: ответ начался с многоточия — модель продолжила
        // оборванный думатель вместо самостоятельного ответа, начало потеряно.
        // НЕ финализируем и НЕ перегенерируем всё с нуля: хвост уже в истории
        // (assistant), просим написать ответ С НАЧАЛА — результат не теряется.
        // Проверяем именно то, что станет финальным ответом (после вырезки
        // думателя), т.к. сырой текст может начинаться с маркеров размышлений.
        let (_, split_answer) = split_thinking_and_answer(&combined);
        if !action_found && starts_with_ellipsis(&split_answer)
            && continuation_restarts < MAX_CONTINUATION_RESTARTS && !response.trim().is_empty() {
            continuation_restarts += 1;
            log_cb(format!(
                "⚠️ [{}] ответ начался с обрыва размышлений («...») — перезапуск ответа с начала (#{}/{}), хвост сохранён в истории",
                agent.name, continuation_restarts, MAX_CONTINUATION_RESTARTS
            ));
            llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
            // Ответ будет писаться заново — состояние «докачки» сбрасываем,
            // чтобы следующий вызов не склеивал старый оборванный combined.
            continuation_raw.clear();
            continuation_mark = None;
            llm_messages.push(LlmMessage { role: "user".to_string(), content:
                "⚠️ Твой финальный ответ начался с многоточия — это продолжение оборванных размышлений, а не самостоятельный ответ. Напиши финальный ответ ЗАНОВО с самого начала: вступление и ВСЕ пункты по порядку. Твой текст после многоточия уже сохранён в истории — не повторяй и не продолжай его, не начинай с «...». Начни с полного первого пункта."
                .to_string()
            });
            continue;
        }

        let preview = safe_truncate(&response, 300).replace('\n', " ");
log_cb(format!("✅ Агент {} завершил ответом ({} символов): {}", agent.name, response.len(), preview));
        final_response = if is_continuation {
            extract_answer_from_combined(&combined, &response)
        } else {
            response
        };
        // Финальный ответ извлечён — состояние «докачки» завершено (иначе следующий
        // вызов парсил бы склеенный combined и терял свежий JSON-конверт).
        continuation_raw.clear();
        continuation_mark = None;

        // ── Сигнальная итерация: агент ответил, но конверт emit_signal так и не
        // распознан (модель вернула текст/невалидный JSON). JSON-конверт НЕ должен
        // стать финальным ответом — с single_report он затёр бы реальный отчёт.
        // Ретраим с корректирующим хинтом; при исчерпании — логируем (красная кнопка,
        // core §2.2) и возвращаем анализ агента, жертвуя сигналом, но не отчётом.
        if signal_attempted && !signal_saved {
            signal_retries += 1;
            if signal_retries <= MAX_SIGNAL_RETRIES {
                if let Some(contract) = &signal_contract {
                    log_cb(format!(
                        "⚠️ [{}] ответ не распознан как emit_signal (попытка {}/{}): {}",
                        agent.id, signal_retries, MAX_SIGNAL_RETRIES, preview
                    ));
                    llm_messages.push(LlmMessage { role: "assistant".to_string(), content: raw_response.clone() });
                    llm_messages.push(LlmMessage {
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
            if !signal_analysis.is_empty() {
                final_response = signal_analysis.clone();
            }
        }

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
                    content: signal_request_prompt(&contract.key),
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

    // 4.2: публикуем событие завершения в шину (успех/ошибка + длительность).
    let err = if final_response.starts_with("⚠️") { Some(final_response.clone()) } else { None };
    crate::infra::event_bus::global_bus().publish(crate::infra::event_bus::AgentEvent::Finished {
        agent: agent.id.clone(),
        namespace: caller_name.as_deref().unwrap_or("main").to_string(),
        ms: start_time.elapsed().as_millis(),
        error: err,
    });
    // 4.5: плагин-слой — уведомление о завершении агента.
    crate::infra::plugins::global_plugins().on_agent_finish(&agent.id, &final_response);

    Ok(final_response)
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
        let engine = LlamaEngine::new(&engine_dir, &model_path, 8192, false, false, &|_| {}, |_| {}).unwrap();
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