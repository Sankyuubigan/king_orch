use super::*;
use crate::domain::agent_manager::AgentProfile;
use crate::domain::orchestrator::prompt::build_system_prompt;
use crate::domain::parsers::*; // parse_orchestrator_response, parse_tool_call, strip_tool_call, ParsedOrchestratorResponse
use crate::domain::signals::{validate_signal_value, SignalContract};
use crate::infra::*; // LlamaEngine, ChatMessage, LlmMessage, SubCall, ToolCallInfo, ModelParams, GrammarSpec, extract_model_filename, push_report
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Instant;

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
    pub(crate) mcp_clients: McpPool,
    pub(crate) log_cb: L,
    pub(crate) status_cb: S,
    pub(crate) subcall_cb: C,
    pub(crate) mcp_servers_dir: &'a Path,
    pub(crate) bins_dir: &'a Path,
    pub(crate) grammars_dir: &'a Path,
    pub(crate) session_id: String,
    pub(crate) workspace_root: PathBuf,
    /// Авто-зона записи пайплайна (обычно == workspace_root; для аналитического —
    /// <workspace_root>/.agents_workspace).
    pub(crate) write_root: PathBuf,
    /// Поведение при записи вне write_root (Prompt | Deny).
    pub(crate) write_outside: crate::infra::WriteOutside,
    pub(crate) approver: Arc<crate::infra::PermissionApprover>,
    // ── мутабельные (владение / &mut-ссылки) ──
    pub(crate) llm_messages: Vec<LlmMessage>,
    pub(crate) messages: &'a mut Vec<ChatMessage>,
    pub(crate) msg_counter: &'a mut u32,
    pub(crate) all_sub_calls: &'a mut Vec<SubCall>,
    pub(crate) final_response: String,
    /// Полный текст размышлений Phase 1 агента — попадает в SubCall.thinking
    /// (отчёт сабагента в GUI). Не режется лимитом THOUGHT_STORE_MAX_CHARS.
    pub(crate) phase1_thinking: Option<String>,
    pub(crate) tool_calls: Vec<ToolCallInfo>,
    pub(crate) consecutive_failed_tools: usize,
    pub(crate) spill_idx: u32,
    pub(crate) consecutive_incomplete: usize,
    /// Сколько раз подряд модель выдала ТОЛЬКО думатель (reasoning) без видимого
    /// ответа. Защита от зацикливания: после лимита — ошибка агента.
    pub(crate) thinking_no_answer: usize,
    pub(crate) consecutive_invalid_targets: usize,
    pub(crate) last_thinking_len: isize,
    pub(crate) stalled_continuations: usize,
    pub(crate) signal_saved: bool,
    pub(crate) signal_analysis: String,
    /// Буфер для сигнала: сохраняется здесь вместо messages[], чтобы
    /// caller мог сохранить [thought, signal] в правильном порядке.
    pub(crate) pending_signal: Option<ChatMessage>,
    /// Контракт сигнала агента (signals/root.schema.json), если агент — emit-агент.
    /// Используется для ЧЕСТНОЙ валидации формы сигнала (см. docs/SIGNAL_CONTRACTS.md):
    /// если модель прислала неверное/пустое обязательное поле — возвращаем ей ошибку
    /// на retry вместо тихого обрыва маршрутизации.
    pub(crate) signal_contract: Option<SignalContract>,
    pub(crate) continuation_count: usize,
    pub(crate) continuation_restarts: usize,
    pub(crate) continuation_raw: String,
    pub(crate) continuation_mark: Option<usize>,
    pub(crate) action_found: bool,
    pub(crate) thought_logged: bool,
    /// Per-agent GBNF-грамматика (загружена из agents/*/grammars/<agent_id>.gbnf).
    /// Хранится для повторного применения после каждого tool call
    /// (take_pending_grammar() consume-and-clear сбрасывает грамматику).
    pub(crate) agent_grammar: Option<String>,
    /// Активная грамматика для текущего агента (signal envelope или per-agent GBNF).
    /// Используется restore_grammar() для единообразного восстановления после
    /// каждого generate_chat(), tool call, retry. Единая точка вместо 7+ ручных вызовов.
    pub(crate) active_grammar: Option<GrammarSpec>,
    /// Инструменты (OpenAI tools[]) для нативного tool-calling. Some для native
    /// агентов (папки coder/research с реальными тулами); None — legacy/чистые.
    /// Устанавливаются в движок через restore_tools() (consume-and-clear).
    pub(crate) native_tools: Option<Vec<ToolDefinition>>,
    /// Хотя бы один нативный tool-call реально выполнен (для GUARD «финал без тулов»).
    pub(crate) native_used: bool,
    /// Сколько раз GUARD уже попросил модель вызвать инструмент вместо финального текста
    /// (макс. 2 — потом финальный текст принимается как есть).
    pub(crate) native_final_retries: usize,
    /// Per-agent GBNF (строка) для ГИБРИДНЫХ агентов (qa_diagnost/arch_reviewer):
    /// применяется ТОЛЬКО в финальном вердиктном grammar-пассе после цикла.
    /// В цикле strict-GBNF не применяется (блокирует нативные tool_calls).
    pub(crate) hybrid_gbnf: Option<String>,
}

impl<'a, L, S, C> RunContext<'a, L, S, C>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    /// Восстанавливает активную грамматику в движке после generate_chat()
    /// (consume-and-clear), tool call, retry или continuation.
    /// Единая точка вместо 7+ ручных вызовов set_grammar().
    pub(crate) fn restore_grammar(&self) {
        if let Some(ref spec) = self.active_grammar {
            self.engine.set_grammar(Some(spec.clone()));
        }
    }

    /// Восстанавливает нативные инструменты (OpenAI tools[]) для следующего
    /// generate_chat() (consume-and-clear). Вызывается вместе с restore_grammar()
    /// на каждой итерации цикла native-агентов.
    pub(crate) fn restore_tools(&self) {
        if let Some(tools) = &self.native_tools {
            if !tools.is_empty() {
                self.engine.set_tools(Some(tools.clone()));
            }
        }
    }

    /// GUARD нативного tool-calling: агент с реальными тулами выдал ФИНАЛЬНЫЙ
    /// текст, НИ РАЗУ не вызвав инструмент. Возвращает true, пока не исчерпаны
    /// 2 ретрая — цикл должен повторить вызов с требованием использовать тул.
    pub(crate) fn guard_native_final_without_tools(&self) -> bool {
        self.native_tools.as_ref().is_some_and(|t| !t.is_empty())
            && !self.native_used
            && self.native_final_retries < NATIVE_FINAL_RETRIES_MAX
    }

    /// App-level GUARD нативного tool-calling: финальный текст без единого тула.
    /// Кладёт ответ в историю + хинт «вызови инструмент» и возвращает true, если
    /// цикл должен повторить вызов (ретраи ограничены NATIVE_FINAL_RETRIES_MAX).
    /// После исчерпания ретраев вернёт false — финальный текст принимается как есть
    /// (fallback): агент не выродится в бесконечный цикл жёстких требований.
    pub(crate) fn retry_native_final_without_tools(&mut self, raw_response: &str) -> bool {
        if !self.guard_native_final_without_tools() {
            return false;
        }
        self.native_final_retries += 1;
        self.action_found = true;
        self.llm_messages.push(LlmMessage {
            role: "assistant".to_string(),
            content: raw_response.to_string(),
            ..Default::default()
        });
        self.continuation_raw.clear();
        self.continuation_mark = None;
        self.llm_messages.push(LlmMessage {
            role: "user".to_string(),
            content: "⚠️ Ты не использовал НИ ОДИН доступный инструмент, а задача требует их применения. Изучи задачу и вызови ПОДХОДЯЩИЙ ИНСТРУМЕНТ из списка (аргументы — валидный JSON). Сначала фактический результат инструмента, потом итоговый ответ.".to_string(),
         ..Default::default()});
        (self.log_cb)(format!("🧤 [{}] GUARD: финал без единого вызова инструмента ({}/{}) — ретрай с требованием вызвать тул", self.agent.name, self.native_final_retries, NATIVE_FINAL_RETRIES_MAX));
        true
    }

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
        log_agent_thought(
            &self.log_cb,
            self.agent,
            "инструмент",
            tool_name,
            thought,
            gen_start.elapsed().as_secs_f32(),
            self.depth,
        );
        self.thought_logged = true;

        (self.status_cb)(format!("Выполнение {}...", tool_name), 60);
        let args_str = arguments.to_string();
        (self.log_cb)(format!(
            "🔧 Агент '{}' вызвал инструмент {}: {}",
            self.agent.name,
            tool_name,
            safe_truncate(&args_str, 200)
        ));
        // emit_signal — особый сигнальный инструмент (legacy-конверт): требует полного
        // цикла сигнала (валидация контракта, сохранение в сессию). Нативным агентам
        // (coder/research) не выдаётся — их инструменты фильтруются в mod.rs.
        if tool_name == "emit_signal" {
            return self.handle_emit_signal(
                arguments,
                thought,
                gen_start,
                raw_response,
                combined,
                parse_target,
                response,
            );
        }

        let (mut output, tool_found) = self.run_tool_core(tool_name, arguments);

        if !tool_found {
            if self
                .agents
                .iter()
                .any(|a| a.id == tool_name && a.id != self.agent.id)
            {
                (self.log_cb)(format!("🔄 Синтаксическая ошибка: '{}' использовал 'tool' для вызова сабагента '{}' вместо 'target'.", self.agent.name, tool_name));
                if self.consecutive_failed_tools >= 3 {
                    self.final_response = format!("{} Синтаксическая ошибка (3 попытки): агент '{}' продолжает использовать 'tool' вместо 'target'. Невозможно продолжить.", AGENT_ERROR_PREFIX, self.agent.id);
                    return Ok(DispatchCtl::Break);
                }
                self.llm_messages.push(LlmMessage {
                    role: "assistant".to_string(),
                    content: if is_continuation {
                        combined.to_string()
                    } else {
                        raw_response.to_string()
                    },
                    ..Default::default()
                });
                self.continuation_raw.clear();
                self.continuation_mark = None;
                self.llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("⚠️ ОШИБКА_СИНТАКСИСА: ты использовал 'tool' для вызова сабагента '{}'. Это сабагент, а не инструмент. Исправь: используй 'target'. Пример: {{\"thought\": \"...\", \"target\": \"{}\", \"task_or_response\": \"...\"}}.", tool_name, tool_name) , ..Default::default()});
                return Ok(DispatchCtl::Continue);
            }
        }

        if self.depth == 0 && tool_found && tool_name != "emit_signal" {
            let stored = safe_truncate(&output, THOUGHT_STORE_MAX_CHARS);
            self.messages.push(ChatMessage {
                id: Some(format!("msg_{}", self.msg_counter)),
                msg_type: "thought".to_string(),
                content: format!(
                    "🔧 Вызван инструмент {}: {}\nРезультат: {}",
                    tool_name,
                    safe_truncate(&args_str, 200),
                    stored
                ),
                sub_calls: None,
                author: Some(self.agent.id.clone()),
                model: Some(extract_model_filename(&self.engine.model_path)),
                time_sec: None,
                attachments: None,
                phase: None,
            });
            *self.msg_counter += 1;
        }

        if !tool_found || output.starts_with("Ошибка") {
            self.consecutive_failed_tools += 1;
            if self.consecutive_failed_tools >= 3 {
                self.final_response = format!("{} Лимит неудачных вызовов инструмента ({}). Агент: '{}'. Инструмент: '{}'. Невозможно продолжить.", AGENT_ERROR_PREFIX, self.consecutive_failed_tools, self.agent.id, tool_name);
                return Ok(DispatchCtl::Break);
            }
            self.tool_calls.push(ToolCallInfo {
                tool_name: tool_name.to_string(),
                arguments: args_str,
                result: output.clone(),
            });
            self.llm_messages.push(LlmMessage {
                role: "assistant".to_string(),
                content: if is_continuation {
                    combined.to_string()
                } else {
                    raw_response.to_string()
                },
                ..Default::default()
            });
            self.continuation_raw.clear();
            self.continuation_mark = None;
            self.llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\n⚠️ Инструмент вернул ошибку. Проверь аргументы и вызови инструмент СНОВА с исправленными данными.", tool_name, output) , ..Default::default()});
            return Ok(DispatchCtl::Continue);
        }
        self.consecutive_failed_tools = 0;
        self.tool_calls.push(ToolCallInfo {
            tool_name: tool_name.to_string(),
            arguments: args_str,
            result: output.clone(),
        });
        // Большие результаты — в spill-файл, модели отдаём выжимку (лечит
        // раздувание контекста). Счётчик spill_idx уникален в рамках вызова.
        let (model_output, _spilled) = spill_if_large(&output, &self.agent.id, self.spill_idx);
        self.spill_idx += 1;
        self.llm_messages.push(LlmMessage {
            role: "assistant".to_string(),
            content: if is_continuation {
                combined.to_string()
            } else {
                raw_response.to_string()
            },
            ..Default::default()
        });
        self.continuation_raw.clear();
        self.continuation_mark = None;
        self.llm_messages.push(LlmMessage { role: "user".to_string(), content: format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\nЕсли задача выполнена — ответь ОБЫЧНЫМ ТЕКСТОМ.", tool_name, model_output) , ..Default::default()});
        Ok(DispatchCtl::Continue)
    }

    /// Чистое исполнение ОДНОГО инструмента (код-тулы/MCP/todo/read_spill) БЕЗ
    /// изменений истории сообщений и счётчиков. Единое ядро для legacy-конверта
    /// (execute_tool_call) и нативного OpenAI tools[] (execute_native_tool_calls).
    /// Применяет плагин-слой on_tool_result и пишет диагностический лог результата.
    /// Возвращает (output, found).
    fn run_tool_core(&mut self, tool_name: &str, arguments: &Value) -> (String, bool) {
        let mut tool_output = None;
        let mut tool_found = false;
        if tool_name == "read_spill" {
            // Встроенный инструмент дочитки больших результатов инструментов.
            tool_found = true;
            let p = arguments
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            match read_spill_file(&p) {
                Ok(content) => {
                    tool_output = Some(content);
                }
                Err(e) => {
                    tool_output = Some(format!("Ошибка read_spill: {}", e));
                }
            }
        } else if tool_name == "todo_write" || tool_name == "todo_list" {
            // 4.1: opt-in чек-лист задач (доступен только агентам coder/research).
            tool_found = true;
            let result = run_todo_tool(tool_name, arguments, &mut *self.messages, &self.agent.id);
            tool_output = Some(result);
        } else if crate::infra::tools::is_code_tool_reference(tool_name) {
            // 🛠 Инструменты кодинга (SSOT в infra::tools::all_tools): read/grep/glob/
            // list_directory (read-only, авто) + write/edit/bash (мутаторы: внутри
            // корня — авто, вне — плашка пользователя). Перед MCP-поиском.
            tool_found = true;
            // Capability-проверка (дефенс-ин-депс): агент может вызвать ТОЛЬКО
            // тулы, которые ему выданы в промпт (all_tools). code_read-агент не
            // должен получить write_file/edit_file/bash даже если модель
            // «угадала» имя — пустой промпт не должен вскрывать capability.
            let granted = crate::infra::tools::is_tool_granted(&self.all_tools, tool_name);
            if !granted {
                tool_output = Some(format!(
                    "Ошибка '{}': инструмент недоступен агенту '{}' (не выдан в capabilities; вероятно, у агента только code_read, а {} — мутатор).",
                    tool_name, self.agent.id, tool_name
                ));
            } else {
                let code_ctx = crate::infra::ToolCtx {
                    workspace_root: &self.workspace_root,
                    write_root: &self.write_root,
                    write_outside: self.write_outside,
                    session_id: &self.session_id,
                    approver: &self.approver,
                    agent_id: &self.agent.id,
                    bins_dir: self.bins_dir,
                };
                match crate::infra::tools::execute_tool(tool_name, arguments, &code_ctx) {
                    Ok(res) => {
                        tool_output = Some(res);
                        crate::infra::event_bus::global_bus().publish(
                            crate::infra::event_bus::AgentEvent::ToolCall {
                                agent: self.agent.id.clone(),
                                tool: tool_name.to_string(),
                            },
                        );
                    }
                    Err(e) => {
                        tool_output = Some(format!("Ошибка '{}': {}", tool_name, e));
                    }
                }
            }
        } else if let Some((mcp_name, _, _)) = self
            .all_tools
            .iter()
            .find(|(_, name, _)| name == &tool_name)
        {
            if let Some(shared) = self.mcp_clients.lock().unwrap().get(mcp_name).cloned() {
                tool_found = true;
                match shared
                    .lock()
                    .unwrap()
                    .call_tool(tool_name, arguments.clone())
                {
                    Ok(res) => {
                        tool_output = Some(res);
                        crate::infra::event_bus::global_bus().publish(
                            crate::infra::event_bus::AgentEvent::ToolCall {
                                agent: self.agent.id.clone(),
                                tool: tool_name.to_string(),
                            },
                        );
                    }
                    Err(e) => {
                        tool_output = Some(format!("Ошибка '{}': {}", tool_name, e));
                    }
                }
            }
        }
        let mut output =
            tool_output.unwrap_or_else(|| format!("Ошибка: Инструмент '{}' не найден.", tool_name));
        // 4.5: плагин-слой — точка расширения результата инструмента (pass-through по умолчанию).
        crate::infra::plugins::global_plugins().on_tool_result(
            &self.agent.id,
            tool_name,
            &mut output,
        );
        (self.log_cb)(format!(
            "🔧 Инструмент '{}' (агент '{}') вернул результат ({} символов): {}",
            tool_name,
            self.agent.name,
            output.chars().count(),
            safe_truncate(&output, 300)
        ));
        (output, tool_found)
    }

    /// Нативный OpenAI tools[]: исполняет ВСЕ tool_calls из одного ответа модели
    /// (finish_reason="tool_calls"). История ведётся в правильном OpenAI-формате:
    /// assistant-сообщение с полем `tool_calls` + отдельные сообщения роли "tool"
    /// с tool_call_id на каждый результат. Первый успешный вызов поднимает
    /// native_used (GUARD «финал без единого тула» его отслеживает).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_native_tool_calls(
        &mut self,
        calls: &[ToolCall],
        gen_start: Instant,
        assistant_text: &str,
    ) -> Result<DispatchCtl, String> {
        if calls.is_empty() {
            return Ok(DispatchCtl::Continue);
        }
        self.action_found = true;
        self.consecutive_incomplete = 0;
        self.thought_logged = true;

        let tool_calls_json: Vec<Value> = calls
            .iter()
            .map(|c| {
                serde_json::json!({
                    "id": c.id,
                    "type": "function",
                    "function": { "name": c.name, "arguments": c.arguments }
                })
            })
            .collect();

        let mut results: Vec<(String, String)> = Vec::new();
        let mut any_success = false;
        for call in calls {
            (self.status_cb)(format!("Выполнение {}...", call.name), 60);
            let args_str = call.arguments.clone();
            let args: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
            (self.log_cb)(format!(
                "🔧 Агент '{}' вызвал инструмент {}: {}",
                self.agent.name,
                call.name,
                safe_truncate(&args_str, 200)
            ));
            if self
                .agents
                .iter()
                .any(|a| a.id == call.name && a.id != self.agent.id)
            {
                (self.log_cb)(format!("⚠️ Нативный вызов: '{}' использовал tool '{}' для сабагента — это ошибка синтаксиса (нужен target).", self.agent.name, call.name));
            }
            let (output, found) = self.run_tool_core(&call.name, &args);
            if found && !output.starts_with("Ошибка") {
                any_success = true;
            }
            self.tool_calls.push(ToolCallInfo {
                tool_name: call.name.clone(),
                arguments: args_str.clone(),
                result: output.clone(),
            });
            // Большие результаты — в spill-файл, модели отдаём конденсированное содержание.
            let (model_output, _spilled) = spill_if_large(&output, &self.agent.id, self.spill_idx);
            self.spill_idx += 1;
            results.push((call.id.clone(), model_output));

            if self.depth == 0 {
                let stored = safe_truncate(&output, THOUGHT_STORE_MAX_CHARS);
                self.messages.push(ChatMessage {
                    id: Some(format!("msg_{}", self.msg_counter)),
                    msg_type: "thought".to_string(),
                    content: format!(
                        "🔧 Вызван инструмент {}: {}\nРезультат: {}",
                        call.name,
                        safe_truncate(&args_str, 200),
                        stored
                    ),
                    sub_calls: None,
                    author: Some(self.agent.id.clone()),
                    model: Some(extract_model_filename(&self.engine.model_path)),
                    time_sec: None,
                    attachments: None,
                    phase: None,
                });
                *self.msg_counter += 1;
            }
        }

        self.consecutive_failed_tools = if any_success {
            0
        } else {
            self.consecutive_failed_tools + 1
        };
        if self.consecutive_failed_tools >= 3 {
            self.final_response = format!(
                "{} Лимит неудачных вызовов инструмента ({}). Агент: '{}'. Невозможно продолжить.",
                AGENT_ERROR_PREFIX, self.consecutive_failed_tools, self.agent.id
            );
            return Ok(DispatchCtl::Break);
        }
        if any_success {
            self.native_used = true;
        }

        // OpenAI-формат: assistant-сообщение с tool_calls + результаты роли "tool".
        // Текст боком с вызовом сохраняем в content (пустой → null в сериализации).
        let assistant_msg = LlmMessage {
            role: "assistant".to_string(),
            content: assistant_text.to_string(),
            ..Default::default()
        }
        .with_tool_calls(tool_calls_json);
        self.llm_messages.push(assistant_msg);
        for (id, content) in results {
            self.llm_messages
                .push(LlmMessage::default().as_tool_result(id, content));
        }
        self.continuation_raw.clear();
        self.continuation_mark = None;
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
            engine,
            agent,
            agents,
            messages,
            msg_counter,
            all_sub_calls,
            llm_messages,
            final_response,
            consecutive_invalid_targets,
            thought_logged,
            log_cb,
            status_cb,
            subcall_cb,
            stream_meta,
            prompt_log,
            mcp_servers_dir,
            bins_dir,
            grammars_dir,
            mcp_clients: mcp_pool,
            model_params,
            format_type,
            cancel_flag,
            depth,
            has_tools_for_prompt,
            all_tools,
            continuation_raw,
            continuation_mark,
            session_id,
            workspace_root,
            write_root,
            write_outside,
            agent_grammar,
            active_grammar,
            ..
        } = self;

        if let Some(subagent) = (*agents).iter().find(|a| a.id == parsed.target) {
            *consecutive_invalid_targets = 0;
            log_agent_thought(
                &*log_cb,
                *agent,
                "вызов",
                &parsed.target,
                &parsed.thought,
                gen_start.elapsed().as_secs_f32(),
                *depth,
            );
            *thought_logged = true;

            (*log_cb)(format!(
                "📞 {} вызывает сабагента: {}",
                (*agent).name,
                subagent.name
            ));

            let start_len = (**all_sub_calls).len();
            let mut sub_pending_signal = None;
            let sub_result = run_agent_node(
                (*log_cb).clone(),
                (*status_cb).clone(),
                (*subcall_cb).clone(),
                *engine,
                subagent,
                *agents,
                parsed.content.clone(),
                vec![],
                &[],
                max_gen_tokens,
                *model_params,
                *format_type,
                cancel_flag.clone(),
                *depth + 1,
                &mut **all_sub_calls,
                Some((*agent).name.clone()),
                *mcp_servers_dir,
                *bins_dir,
                *grammars_dir,
                mcp_pool.clone(),
                &mut **messages,
                &mut **msg_counter,
                String::new(),
                stream_meta.clone(),
                false,
                prompt_log.clone(),
                session_id.clone(),
                workspace_root.clone(),
                write_root.clone(),
                *write_outside,
                &mut sub_pending_signal,
                false, // two_phase_thinking — только для signal-агентов из workflow
            )?;
            let end_len = (**all_sub_calls).len();
            let node_sub_calls = if start_len < end_len {
                Some((**all_sub_calls)[start_len..end_len].to_vec())
            } else {
                None
            };

            if sub_result.starts_with(AGENT_ERROR_PREFIX) {
                (*log_cb)(format!(
                    "❌ Сабагент '{}' вернул ошибку — fold: {}",
                    subagent.id, sub_result
                ));
                let err_msg = ChatMessage {
                    id: Some(format!("msg_{}", **msg_counter)),
                    msg_type: "thought".to_string(),
                    content: sub_result.clone(),
                    sub_calls: node_sub_calls.clone(),
                    author: Some(subagent.id.clone()),
                    model: Some(extract_model_filename(&(*engine).model_path)),
                    time_sec: None,
                    attachments: None,
                    phase: Some(2),
                };
                push_report(&mut **messages, err_msg, subagent.replace_report, Some(2));
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
                time_sec: None,
                attachments: None,
                phase: Some(2),
            };
            push_report(&mut **messages, msg, subagent.replace_report, Some(2));
            **msg_counter += 1;
            // Сигнал сабагента сохраняется ПОСЛЕ thought.
            if let Some(signal) = sub_pending_signal.take() {
                push_report(&mut **messages, signal, false, None);
                **msg_counter += 1;
            }

            let uses_method_3 = self.signal_contract.is_some();
            let mut new_sys = build_system_prompt(
                *agent,
                &**messages,
                *has_tools_for_prompt,
                all_tools,
                max_gen_tokens,
                uses_method_3,
                self.native_tools.is_some(),
                self.hybrid_gbnf.is_some(),
            );
            if let Some(f) = llm_messages.first_mut() {
                if f.role == "system" {
                    f.content = new_sys;
                }
            }
            llm_messages.push(LlmMessage {
                role: "assistant".to_string(),
                content: if is_continuation {
                    combined.to_string()
                } else {
                    raw_response.to_string()
                },
                ..Default::default()
            });
            continuation_raw.clear();
            *continuation_mark = None;
            llm_messages.push(LlmMessage {
                role: "user".to_string(),
                content: format!(
                    "Отчет от {}:\n{}\n\nЕсли достаточно — ответь ОБЫЧНЫМ ТЕКСТОМ.",
                    subagent.name,
                    truncate_result(&sub_result, 2000)
                ),
                ..Default::default()
            });
            Ok(DispatchCtl::Continue)
        } else {
            *consecutive_invalid_targets += 1;
            if *consecutive_invalid_targets >= 3 {
                (*log_cb)(format!(
                    "❌ {} превысил лимит неверных target-вызовов (3).",
                    (*agent).name
                ));
                *final_response = format!(
                    "{} Агент '{}' вызывает несуществующего сабагента '{}'. Невозможно продолжить.",
                    AGENT_ERROR_PREFIX,
                    (*agent).id,
                    parsed.target
                );
                return Ok(DispatchCtl::Break);
            }
            llm_messages.push(LlmMessage {
                role: "assistant".to_string(),
                content: if is_continuation {
                    combined.to_string()
                } else {
                    raw_response.to_string()
                },
                ..Default::default()
            });
            continuation_raw.clear();
            *continuation_mark = None;
            let valid_ids = valid_agent_ids(*agents, &(*agent).id, "primary");
            let error_msg = if valid_ids.is_empty() {
                format!("Ошибка: Агент '{}' не найден.", parsed.target)
            } else {
                format!("Ошибка: Агент '{}' не найден. Доступные агенты: {}. Ответь JSON с одним из них.", parsed.target, valid_ids.join(", "))
            };
            llm_messages.push(LlmMessage {
                role: "user".to_string(),
                content: error_msg,
                ..Default::default()
            });
            Ok(DispatchCtl::Continue)
        }
    }

    /// Обработка emit_signal — вынесена из execute_tool_call для переиспользования
    /// в Phase 2 (JSON-only fallback). Возвращает Break (сигнал сохранён) или
    /// Continue (ошибка — модель должна исправить конверт).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_emit_signal(
        &mut self,
        arguments: &Value,
        thought: &str,
        gen_start: Instant,
        raw_response: &str,
        combined: &str,
        parse_target: &str,
        _response: &str,
    ) -> Result<DispatchCtl, String> {
        let mut key_val = arguments.get("key");
        let mut val_val = arguments.get("value");
        if key_val.is_none() && val_val.is_none() {
            if let Some(props) = arguments.get("properties") {
                key_val = props.get("key");
                val_val = props.get("value");
            }
        }
        let key = key_val.and_then(|v| v.as_str()).filter(|s| !s.is_empty());
        let value = val_val.filter(|v| !v.is_null());

        if let (Some(key), Some(value)) = (key, value) {
            // ЧЕСТНАЯ валидация формы сигнала против контракта (SSOT).
            if let Some(contract) = &self.signal_contract {
                if let Err(e) = validate_signal_value(contract, value) {
                    self.consecutive_failed_tools += 1;
                    (self.log_cb)(format!("⚠️ emit_signal: '{}' валидация: {}", key, e));
                    return Ok(DispatchCtl::Continue);
                }
            }
            self.consecutive_failed_tools = 0;
            self.signal_saved = true;
            let signal_content = serde_json::json!({key: value}).to_string();
            let signal_msg = ChatMessage {
                id: Some(format!("msg_{}", self.msg_counter)),
                msg_type: "signal".to_string(),
                content: signal_content.clone(),
                sub_calls: None,
                author: Some(self.agent.id.clone()),
                model: None,
                time_sec: None,
                attachments: None,
                phase: Some(2),
            };
            self.pending_signal = Some(signal_msg);
            *self.msg_counter += 1;
            (self.log_cb)(format!(
                "📡 emit_signal: '{}' = {} (в messages[], signal bus подхватит после узла)",
                key,
                safe_truncate(&signal_content, 200)
            ));

            // Результат (анализ) агента в messages[] сохраняет вызывающий.
            let (analysis, _) = strip_tool_call(parse_target);
            let analysis = if analysis.trim().is_empty() {
                let think_contents = extract_think_content(raw_response);
                if !think_contents.is_empty() {
                    think_contents.join("\n\n")
                } else if thought.is_empty() {
                    // Конверт сигнала больше не несёт поле thought (двухфазный режим:
                    // размышления агента уже сохранены как отдельное thought-сообщение
                    // в Phase 1). Не подставляем сюда сырой JSON-конверт — иначе он
                    // попадёт в сообщение агента в GUI.
                    String::new()
                } else {
                    thought.to_string()
                }
            } else {
                analysis
            };

            (self.log_cb)(format!(
                "💭 Мысль {} [d={}] (сигнал + анализ) [⏱{:.1}с]: {}",
                self.agent.name,
                self.depth,
                gen_start.elapsed().as_secs_f32(),
                safe_truncate(&analysis, 500)
            ));
            self.tool_calls.push(ToolCallInfo {
                tool_name: "emit_signal".to_string(),
                arguments: arguments.to_string(),
                result: format!("✅ Сигнал '{}' сохранён", key),
            });
            self.final_response = if self.signal_analysis.is_empty() {
                analysis
            } else {
                self.signal_analysis.clone()
            };
            return Ok(DispatchCtl::Break);
        } else {
            let key_str = arguments
                .get("key")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "отсутствует".to_string());
            let val_str = arguments
                .get("value")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "отсутствует".to_string());
            (self.log_cb)(format!("❌ emit_signal: невалидный конверт (key={}, value={}). Требуется {{\"key\":\"...\",\"value\":...}}", key_str, val_str));
            self.consecutive_failed_tools += 1;
            self.tool_calls.push(ToolCallInfo {
                tool_name: "emit_signal".to_string(),
                arguments: arguments.to_string(),
                result: format!("❌ Невалидный конверт: key={}, value={}", key_str, val_str),
            });
            self.llm_messages.push(LlmMessage {
                role: "assistant".to_string(),
                content: if combined.is_empty() {
                    raw_response.to_string()
                } else {
                    combined.to_string()
                },
                ..Default::default()
            });
            self.continuation_raw.clear();
            self.continuation_mark = None;
            self.llm_messages.push(LlmMessage { role: "user".to_string(), content: format!(
                "Ошибка: emit_signal требует 'key' (строка) и 'value' (объект). Получено: key={}, value={}. Исправь и вызови СНОВА.",
                key_str, val_str
            ) , ..Default::default()});
            return Ok(DispatchCtl::Continue);
        }
    }
}
