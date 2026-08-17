use super::*;
use crate::domain::parsers::*; // parse_orchestrator_response, parse_tool_call, strip_tool_call, ParsedOrchestratorResponse
use crate::domain::agent_manager::AgentProfile;
use crate::infra::*; // LlamaEngine, ChatMessage, LlmMessage, SubCall, ToolCallInfo, ModelParams, GrammarSpec, extract_model_filename, push_report
use std::sync::{Arc, atomic::AtomicBool};
use std::sync::Mutex;
use std::time::Instant;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use serde_json::Value;
use crate::domain::orchestrator::prompt::build_system_prompt;

/// Управляющий сигнал цикла `run_agent_node`: как продолжить после
/// вызова `execute_tool_call` / `handle_subagent_call`.
pub(crate) enum DispatchCtl {
    /// Продолжить цикл со следующей итерацией (бывший `continue`).
    Continue,
    /// Прервать цикл и вернуть `ctx.final_response` (бывший `break`).
    Break,
    /// Немедленно вернуть значение из `run_agent_node`.
    #[allow(dead_code)]
    Return(String),
}

/// Всё разделяемое состояние цикла `run_agent_node` (иммутабельные ссылки +
/// мутабельные поля). Позволяет выносить блоки цикла в отдельные методы без
/// передачи десятков аргументов.
pub(crate) struct RunContext<'a, L, S, C>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    // ── иммутабельные (ссылки / Arc / значения) ──
    pub(crate) engine: &'a LlamaEngine,
    pub(crate) agent: &'a AgentProfile,
    pub(crate) agents: &'a [AgentProfile],
    pub(crate) model_params: &'a ModelParams,
    pub(crate) format_type: &'a str,
    pub(crate) cancel_flag: Arc<AtomicBool>,
    pub(crate) stream_meta: Arc<Mutex<StreamMeta>>,
    pub(crate) prompt_log: Option<PathBuf>,
    pub(crate) depth: usize,
    pub(crate) has_tools_for_prompt: bool,
    pub(crate) all_tools: Vec<(String, String, Value)>,
    pub(crate) mcp_clients: HashMap<String, McpClient>,
    pub(crate) log_cb: L,
    pub(crate) status_cb: S,
    pub(crate) subcall_cb: C,
    pub(crate) mcp_servers_dir: &'a Path,
    pub(crate) bins_dir: &'a Path,
    pub(crate) grammars_dir: &'a Path,
    // ── мутабельные (владение / &mut-ссылки) ──
    pub(crate) llm_messages: Vec<LlmMessage>,
    pub(crate) messages: &'a mut Vec<ChatMessage>,
    pub(crate) msg_counter: &'a mut u32,
    pub(crate) all_sub_calls: &'a mut Vec<SubCall>,
    pub(crate) final_response: String,
    pub(crate) tool_calls: Vec<ToolCallInfo>,
    pub(crate) consecutive_failed_tools: usize,
    pub(crate) spill_idx: u32,
    pub(crate) consecutive_incomplete: usize,
    pub(crate) consecutive_invalid_targets: usize,
    pub(crate) last_thinking_len: isize,
    pub(crate) stalled_continuations: usize,
    pub(crate) signal_attempted: bool,
    pub(crate) signal_saved: bool,
    pub(crate) signal_retries: usize,
    pub(crate) signal_analysis: String,
    pub(crate) continuation_count: usize,
    pub(crate) continuation_restarts: usize,
    pub(crate) continuation_raw: String,
    pub(crate) continuation_mark: Option<usize>,
    pub(crate) action_found: bool,
    pub(crate) thought_logged: bool,
}

impl<'a, L, S, C> RunContext<'a, L, S, C>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    /// Блок диспетчеризации инструментов: разбор `parse_tool_call` результата
    /// LLM, исполнение built-in / MCP-инструментов, обработка ошибок.
    ///
    /// Возвращает `DispatchCtl` (Continue = «продолжить цикл», Break = «вернуть
    /// final_response»). `Err` пробрасывается `?` — означает `return Err(..)`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_tool_call(
        &mut self,
        tool_name: &str,
        arguments: &Value,
        thought: &str,
        gen_start: Instant,
        raw_response: &str,
        combined: &str,
        is_continuation: bool,
        parse_target: &str,
        response: &str,
    ) -> Result<DispatchCtl, String> {
        self.action_found = true;
        self.consecutive_incomplete = 0;
        log_agent_thought(&self.log_cb, self.agent, "инструмент", tool_name, thought, gen_start.elapsed().as_secs_f32(), self.depth);
        self.thought_logged = true;

        (self.status_cb)(format!("Выполнение {}...", tool_name), 60);
        let args_str = arguments.to_string();
        (self.log_cb)(format!("🔧 Агент '{}' вызвал инструмент {}: {}", self.agent.name, tool_name, safe_truncate(&args_str, 200)));
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
                self.consecutive_failed_tools = 0;
                self.signal_saved = true;
                let signal_msg = ChatMessage {
                    id: Some(format!("msg_{}", self.msg_counter)),
                    msg_type: "signal".to_string(),
                    content: serde_json::json!({key: value}).to_string(),
                    sub_calls: None,
                    author: Some(self.agent.id.clone()),
                    model: None,
                    attachments: None,
                };
                self.messages.push(signal_msg);
                *self.msg_counter += 1;

                // Результат (анализ) агента в messages[] сохраняет вызывающий
                // (узел workflow / legacy-коллер), иначе получается клон: один
                // и тот же ответ дважды. Здесь остаётся только сигнал.
                let (analysis, _) = strip_tool_call(parse_target);
                let analysis = if analysis.trim().is_empty() {
                    if thought.is_empty() { response.to_string() } else { thought.to_string() }
                } else {
                    analysis
                };

                (self.log_cb)(format!("💭 Мысль {} [d={}] (сигнал + анализ) [⏱{:.1}с]: {}", self.agent.name, self.depth, gen_start.elapsed().as_secs_f32(), safe_truncate(&analysis, 500)));
                self.tool_calls.push(ToolCallInfo {
                    tool_name: "emit_signal".to_string(),
                    arguments: args_str.clone(),
                    result: format!("✅ Сигнал '{}' сохранён", key),
                });
                // На втором (сигнальном) вызове пользовательский ответ агента —
                // это результат ПЕРВОГО вызова: возвращаем его, а не голый JSON.
                self.final_response = if self.signal_analysis.is_empty() {
                    analysis
                } else {
                    self.signal_analysis.clone()
                };
                return Ok(DispatchCtl::Break);
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
            let result = run_todo_tool(tool_name, arguments, &mut *self.messages, &self.agent.id);
            tool_output = Some(result);
        } else if let Some((mcp_name, _, _)) = self.all_tools.iter().find(|(_, name, _)| name == &tool_name) {
            if let Some(client) = self.mcp_clients.get_mut(mcp_name) {
                tool_found = true;
                match client.call_tool(tool_name, arguments.clone()) {
                    Ok(res) => {
                        tool_output = Some(res);
                        self.consecutive_failed_tools = 0;
                        crate::infra::event_bus::global_bus().publish(
                            crate::infra::event_bus::AgentEvent::ToolCall {
                                agent: self.agent.id.clone(),
                                tool: tool_name.to_string(),
                            },
                        );
                    }
                    Err(e) => { tool_output = Some(format!("Ошибка '{}': {}", tool_name, e)); }
                }
            }
        }
        if !tool_found {
            if self.agents.iter().any(|a| a.id == tool_name && a.id != self.agent.id) {
                (self.log_cb)(format!("🔄 Синтаксическая ошибка: '{}' использовал 'tool' для вызова сабагента '{}' вместо 'target'.", self.agent.name, tool_name));
                if self.consecutive_failed_tools >= 3 {
                    self.final_response = format!("{} Синтаксическая ошибка (3 попытки): агент '{}' продолжает использовать 'tool' вместо 'target'. Невозможно продолжить.", AGENT_ERROR_PREFIX, self.agent.id);
                    return Ok(DispatchCtl::Break);
                }
                self.llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.to_string() } else { raw_response.to_string() } });
                self.continuation_raw.clear();
                self.continuation_mark = None;
                self.llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("⚠️ ОШИБКА_СИНТАКСИСА: ты использовал 'tool' для вызова сабагента '{}'. Это сабагент, а не инструмент. Исправь: используй 'target'. Пример: {{\"thought\": \"...\", \"target\": \"{}\", \"task_or_response\": \"...\"}}.", tool_name, tool_name) });
                return Ok(DispatchCtl::Continue);
            }
        }
        let mut output = tool_output.unwrap_or_else(|| format!("Ошибка: Инструмент '{}' не найден.", tool_name));
        // 4.5: плагин-слой — точка расширения результата инструмента (pass-through по умолчанию).
        crate::infra::plugins::global_plugins().on_tool_result(&self.agent.id, tool_name, &mut output);
        (self.log_cb)(format!("🔧 Инструмент '{}' (агент '{}') вернул результат ({} символов): {}", tool_name, self.agent.name, output.chars().count(), safe_truncate(&output, 300)));

        if self.depth == 0 && tool_found && tool_name != "emit_signal" {
            let stored = safe_truncate(&output, THOUGHT_STORE_MAX_CHARS);
            self.messages.push(ChatMessage {
                id: Some(format!("msg_{}", self.msg_counter)),
                msg_type: "thought".to_string(),
                content: format!("🔧 Вызван инструмент {}: {}\nРезультат: {}", tool_name, safe_truncate(&args_str, 200), stored),
                sub_calls: None,
                author: Some(self.agent.id.clone()),
                model: Some(extract_model_filename(&self.engine.model_path)),
                attachments: None,
            });
            *self.msg_counter += 1;
        }

        if !tool_found || output.starts_with("Ошибка") {
            self.consecutive_failed_tools += 1;
            if self.consecutive_failed_tools >= 3 {
                self.final_response = format!("{} Лимит неудачных вызовов инструмента ({}). Агент: '{}'. Инструмент: '{}'. Невозможно продолжить.", AGENT_ERROR_PREFIX, self.consecutive_failed_tools, self.agent.id, tool_name);
                return Ok(DispatchCtl::Break);
            }
            self.tool_calls.push(ToolCallInfo { tool_name: tool_name.to_string(), arguments: args_str, result: output.clone() });
            self.llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.to_string() } else { raw_response.to_string() } });
            self.continuation_raw.clear();
            self.continuation_mark = None;
            self.llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\n⚠️ Инструмент вернул ошибку. Используй другой инструмент или заверши через {{\"target\": \"reply\"}}.", tool_name, output) });
            return Ok(DispatchCtl::Continue);
        }
        self.consecutive_failed_tools = 0;
        self.tool_calls.push(ToolCallInfo { tool_name: tool_name.to_string(), arguments: args_str, result: output.clone() });
        // Большие результаты — в spill-файл, модели отдаём выжимку (лечит
        // раздувание контекста). Счётчик spill_idx уникален в рамках вызова.
        let (model_output, _spilled) = spill_if_large(&output, &self.agent.id, self.spill_idx);
        self.spill_idx += 1;
        self.llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.to_string() } else { raw_response.to_string() } });
        self.continuation_raw.clear();
        self.continuation_mark = None;
        self.llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\nЕсли задача выполнена — ответь ОБЫЧНЫМ ТЕКСТОМ.", tool_name, model_output) });
        Ok(DispatchCtl::Continue)
    }

    /// Блок вызова сабагента / обработки невалидного `target`.
    ///
    /// Рекурсивно вызывает `run_agent_node` для найденного сабагента и
    /// сохраняет отчёт; при невалидном target инкрементирует счётчик и
    /// подмешивает корректирующее сообщение. Возвращает `DispatchCtl`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_subagent_call(
        &mut self,
        parsed: &ParsedOrchestratorResponse,
        gen_start: Instant,
        raw_response: &str,
        combined: &str,
        is_continuation: bool,
        max_gen_tokens: usize,
    ) -> Result<DispatchCtl, String> {
        // Деструктуризация на непересекающиеся поля: снимает конфликт
        // «&mut self + передача &mut messages в рекурсивный run_agent_node».
        let RunContext {
            engine, agent, agents, messages, msg_counter, all_sub_calls,
            llm_messages, final_response, consecutive_invalid_targets,
            thought_logged, log_cb, status_cb, subcall_cb, stream_meta,
            prompt_log, mcp_servers_dir, bins_dir, grammars_dir, model_params,
            format_type, cancel_flag, depth, has_tools_for_prompt, all_tools,
            continuation_raw, continuation_mark, ..
        } = self;

        if let Some(subagent) = (*agents).iter().find(|a| a.id == parsed.target) {
            *consecutive_invalid_targets = 0;
            log_agent_thought(&*log_cb, *agent, "вызов", &parsed.target, &parsed.thought, gen_start.elapsed().as_secs_f32(), *depth);
            *thought_logged = true;

            (*log_cb)(format!("📞 {} вызывает сабагента: {}", (*agent).name, subagent.name));

            let start_len = (**all_sub_calls).len();
            let sub_result = run_agent_node(
                (*log_cb).clone(), (*status_cb).clone(), (*subcall_cb).clone(),
                *engine, subagent, *agents, parsed.content.clone(), vec![],
                &[],
                max_gen_tokens, *model_params, *format_type,
                cancel_flag.clone(), *depth + 1, &mut **all_sub_calls, Some((*agent).name.clone()), *mcp_servers_dir, *bins_dir,
                *grammars_dir,
                &mut **messages, &mut **msg_counter,
                String::new(),
                stream_meta.clone(), false,
                prompt_log.clone(),
            )?;
            let end_len = (**all_sub_calls).len();
            let node_sub_calls = if start_len < end_len {
                Some((**all_sub_calls)[start_len..end_len].to_vec())
            } else {
                None
            };

            if sub_result.starts_with(AGENT_ERROR_PREFIX) {
                (*log_cb)(format!("❌ Сабагент '{}' вернул ошибку — fold: {}", subagent.id, sub_result));
                let err_msg = ChatMessage {
                    id: Some(format!("msg_{}", **msg_counter)),
                    msg_type: "thought".to_string(),
                    content: sub_result.clone(),
                    sub_calls: node_sub_calls.clone(),
                    author: Some(subagent.id.clone()),
                    model: Some(extract_model_filename(&(*engine).model_path)),
                    attachments: None,
                };
                push_report(&mut **messages, err_msg, subagent.single_report);
                **msg_counter += 1;
                *final_response = sub_result;
                return Ok(DispatchCtl::Break);
            }

            let msg = ChatMessage {
                id: Some(format!("msg_{}", **msg_counter)),
                msg_type: "thought".to_string(),
                content: sub_result.clone(),
                sub_calls: node_sub_calls.clone(),
                author: Some(subagent.id.clone()),
                model: Some(extract_model_filename(&(*engine).model_path)),
                attachments: None,
            };
            push_report(&mut **messages, msg, subagent.single_report);
            **msg_counter += 1;

            let new_sys = build_system_prompt(*agent, &**messages, *has_tools_for_prompt, all_tools, max_gen_tokens);
            if let Some(f) = llm_messages.first_mut() { if f.role == "system" { f.content = new_sys; } }
            llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.to_string() } else { raw_response.to_string() } });
            continuation_raw.clear();
            *continuation_mark = None;
            llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("Отчет от {}:\n{}\n\nЕсли достаточно — ответь ОБЫЧНЫМ ТЕКСТОМ.", subagent.name, truncate_result(&sub_result, 2000)) });
            Ok(DispatchCtl::Continue)
        } else {
            *consecutive_invalid_targets += 1;
            if *consecutive_invalid_targets >= 3 {
                (*log_cb)(format!("❌ {} превысил лимит неверных target-вызовов (3).", (*agent).name));
                *final_response = format!("{} Агент '{}' вызывает несуществующего сабагента '{}'. Невозможно продолжить.", AGENT_ERROR_PREFIX, (*agent).id, parsed.target);
                return Ok(DispatchCtl::Break);
            }
            llm_messages.push(LlmMessage { role: "assistant".to_string(), content: if is_continuation { combined.to_string() } else { raw_response.to_string() } });
            continuation_raw.clear();
            *continuation_mark = None;
            let valid_ids = valid_agent_ids(*agents, &(*agent).id, "primary");
            let error_msg = if valid_ids.is_empty() {
                format!("Ошибка: Агент '{}' не найден.", parsed.target)
            } else {
                format!("Ошибка: Агент '{}' не найден. Доступные агенты: {}. Ответь JSON с одним из них.", parsed.target, valid_ids.join(", "))
            };
            llm_messages.push(LlmMessage { role: "user".to_string(), content: error_msg });
            Ok(DispatchCtl::Continue)
        }
    }
}