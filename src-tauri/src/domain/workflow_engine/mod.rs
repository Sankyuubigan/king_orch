//! 🚂 Workflow Engine — графовый движок маршрутизации
//!
//! Отвечает за выполнение YAML-графов (workflows).
//! Каждый workflow — это направленный граф узлов (nodes) и рёбер (edges).

pub mod context;
pub mod editor_fidelity;
pub mod fact_extractor;
pub mod groups;
pub mod nodes;
pub mod parser;
// `reports` — общий код боевой ноды (`nodes::execute_node`) и стендов: инжект
// отчётов собирается одной функцией, иначе стенд мерил бы не тот текст.
pub mod reports;
/// Диагностика «мёртвого signal-контракта» — страховка от пустого ответа
/// Method 3 агента, чей сигнал никто не потребляет (кейс synthesizer 17.10.26).
pub mod signal_consumer_check;
pub mod step_output;

// ─── Стенды замера валидатора 9 элементов ───
//
// Всё ниже обслуживает ОДНУ задачу — сравнение методов определения элементов
// (System-1 ONNX, System-2 LLM, трёхслойная воронка, плоская форма) на одном
// золоте. Движок о них не знает: все модули помечены `#[cfg(test)]`, поэтому в
// релизном бинаре измеритель места не занимает (AGENTS.md §1 «разделение
// бизнес-логики и движка»).
//
// Что где:
//   `probe_cases`      — загрузка кейсов из `test_cases/new_tests_for_validator/`
//   `probe_call`       — разбор конверта `emit_signal` (тот же путь, что в бою)
//                        и отказ с сырыми ответами слоёв
//   `mental_gate`      — маршрут «только соматика → слои не вызываются», из
//                        измеренного артефакта боевого гейта
//   `validator_layers` — состав воронки из `agents/.../validator_layers.yaml`
//   `verdict_contract` — общий счёт полей по контракту сигнала
//   `probe_report`     — форма отчёта, сводка по наборам, статистика маршрута
//   `probe_acceptance` — приёмка: пороги из `acceptance_gates.json`
//   `*_probe`          — сами стенды (форма = одна функция стенда)
//   `gold_rescore`     — пересчёт ВСЕХ систем на одном золоте без вызова модели
//
// Стенды ОТКЛОНЁННЫХ методов (Лайя 3 слоя, Лайя-консолидатор, старый плоский
// Орнит, гейт фактов) вынесены в `test/laya_probe/experiments/`: их вердикты
// зафиксированы в `docs/LAYA_MODEL.md` и `tasks/`, а сами прогоны больше не
// нужны — `gold_rescore` пересчитывает их результаты прямо из артефактов.
#[cfg(test)]
mod flat_validator_probe;
#[cfg(test)]
mod gold_rescore;
#[cfg(test)]
mod layered_validator_probe;
#[cfg(test)]
mod mental_gate;
#[cfg(test)]
mod probe_acceptance;
#[cfg(test)]
mod probe_call;
#[cfg(test)]
mod probe_cases;
#[cfg(test)]
mod probe_report;
#[cfg(test)]
mod validator_layers;
#[cfg(test)]
mod verdict_contract;

pub use context::WorkflowContext;
pub use editor_fidelity::{analyze_workflow_fidelity, GraphDiagnostic};
pub use parser::WorkflowConfig;
pub use parser::{
    find_workflow_by_stem, load_workflows, parse_workflow_file, resolve_entry,
    separate_top_level_fields, EdgeDef, FactsFile, GroupDef, NodeDef, NodeType, WorkflowDef,
};

use crate::domain::agent_manager::AgentProfile;
use crate::domain::orchestrator;
use crate::domain::orchestrator::RequestMedia;
use crate::domain::parsers::clean_thought_tags;
use crate::infra::{
    build_json_only_grammar, ChatMessage, GrammarSpec, LlmEngine, LlmMessage, ModelParams,
    SamplingPresets, SubCall,
};
use nodes::find_next_node;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Общие ресурсы для выполнения workflow — пробрасываются во все узлы.
pub struct WorkflowRunner<'a, L, S, C> {
    pub engine: &'a LlmEngine,
    pub agents: &'a [AgentProfile],
    pub workflows: &'a [WorkflowDef],
    pub log_cb: L,
    pub status_cb: S,
    pub subcall_cb: C,
    pub max_gen_tokens: usize,
    pub model_params: &'a ModelParams,
    pub format_type: &'a str,
    pub cancel_flag: Arc<AtomicBool>,
    pub mcp_servers_dir: &'a Path,
    pub bins_dir: &'a Path,
    /// agents/<папка>/grammars/ — per-agent GBNF для llm_worker узлов
    pub grammars_dir: &'a Path,
    /// Корень `agents/` — от него резолвятся файлы агентов (grammars/, layers/).
    pub agents_dir: &'a Path,
    pub all_sub_calls: &'a mut Vec<SubCall>,
    pub msg_counter: &'a mut u32,
    pub stream_meta: Arc<Mutex<orchestrator::StreamMeta>>,
    /// Именованные пресеты параметров LLM (sampling_presets.json)
    pub sampling_presets: &'a SamplingPresets,
    /// Путь к логу снимков входа модели (prompt-log). None — не писать.
    pub prompt_log: Option<std::path::PathBuf>,
    /// ID сессии чата (для permission-грантов «в этом чате»).
    pub session_id: String,
    /// Корень проекта (запись внутри — авто-разрешена без плашки).
    pub workspace_root: std::path::PathBuf,
    /// Авто-зона записи пайплайна (обычно == workspace_root; для аналитического —
    /// <workspace_root>/.agents_workspace).
    pub write_root: std::path::PathBuf,
    pub request_media: Arc<RequestMedia>,
    pub image_artifacts: Arc<crate::infra::ImageArtifactRegistry>,
    /// Поведение при записи вне write_root (Prompt | Deny).
    pub write_outside: crate::infra::WriteOutside,
}

fn sanitized_llm_message(message: &ChatMessage) -> LlmMessage {
    let mut llm_message = message.to_llm_message();
    llm_message.content = orchestrator::prompt::sanitize_model_visible_text(&message.content);
    llm_message
}

/// Вычисляет результирующие ModelParams по каскаду приоритетов (SSOT):
/// 1. Явный node.llm_params на узле (пресет sampling_presets.json) — строгий приоритет (пинит всё).
/// 2. Иначе: база = config.default_llm_params (пресет) либо user_params (база пользователя).
/// 3. frontmatter агента (agent.temperature) — overlay поверх базы (переопределяет только temperature).
pub fn resolve_params_cascade(
    sampling_presets: &SamplingPresets,
    user_params: &ModelParams,
    node_llm_params: &Option<String>,
    workflow_config: &Option<WorkflowConfig>,
    agent: Option<&AgentProfile>,
) -> ModelParams {
    // 1. Приоритет: явный пресет llm_params на узле
    if let Some(ref preset_name) = node_llm_params {
        if let Some(preset) = sampling_presets.get(preset_name) {
            return preset.clone();
        }
        eprintln!(
            "[workflow] Пресет '{}' не найден в sampling_presets.json, fallback на base params",
            preset_name
        );
    }

    // 2. База: default_llm_params в config workflow, либо параметры пользователя
    let mut params = if let Some(ref config) = workflow_config {
        if let Some(ref default_name) = config.default_llm_params {
            if let Some(preset) = sampling_presets.get(default_name) {
                preset.clone()
            } else {
                eprintln!("[workflow] Дефолтный пресет '{}' не найден в sampling_presets.json, fallback на base params", default_name);
                user_params.clone()
            }
        } else {
            user_params.clone()
        }
    } else {
        user_params.clone()
    };

    // 3. Overlay frontmatter: если у агента задана кастомная temperature
    if let Some(agent) = agent {
        if let Some(temp) = agent.temperature {
            params.temperature = temp;
        }
    }

    params
}

impl<'a, L, S, C> WorkflowRunner<'a, L, S, C>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    /// Резолвит параметры LLM для узла с учётом каскада приоритетов (SSOT).
    pub fn resolve_llm_params(
        &self,
        node_llm_params: &Option<String>,
        workflow_config: &Option<WorkflowConfig>,
        agent: Option<&AgentProfile>,
    ) -> ModelParams {
        resolve_params_cascade(
            self.sampling_presets,
            self.model_params,
            node_llm_params,
            workflow_config,
            agent,
        )
    }

    /// Выполняет .md агента через `run_agent_node()`
    pub fn call_agent(
        &mut self,
        agent: &AgentProfile,
        task: &str,
        messages: &mut Vec<ChatMessage>,
        injected_reports: &str,
        allow_stream: bool,
        resolved_params: &ModelParams,
        out_pending_signal: &mut Option<ChatMessage>,
    ) -> Result<String, String> {
        let mcp_pool: crate::infra::mcp_client::McpPool =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
                String,
                crate::infra::mcp_client::SharedMcpClient,
            >::new()));

        orchestrator::run_agent_node(
            self.log_cb.clone(),
            self.status_cb.clone(),
            self.subcall_cb.clone(),
            self.engine,
            agent,
            self.agents,
            task.to_string(),
            vec![],
            self.request_media.clone(),
            self.image_artifacts.clone(),
            self.max_gen_tokens,
            resolved_params,
            self.format_type,
            self.cancel_flag.clone(),
            1,
            self.all_sub_calls,
            Some("workflow_engine".to_string()),
            self.mcp_servers_dir,
            self.bins_dir,
            self.grammars_dir,
            mcp_pool,
            messages,
            self.msg_counter,
            injected_reports.to_string(),
            self.stream_meta.clone(),
            allow_stream,
            self.prompt_log.clone(),
            self.session_id.clone(),
            self.workspace_root.clone(),
            self.write_root.clone(),
            self.write_outside,
            out_pending_signal,
        )
    }

    /// Зовёт LLM со свободным ответом (без системного промпта) — для llm_freeform
    pub fn call_llm_freeform(
        &self,
        user_text: &str,
        history: &[ChatMessage],
        resolved_params: &ModelParams,
        ctx_label: &str,
    ) -> Result<String, String> {
        // Явная инструкция по языку: у freeform нет системного промпта агента,
        // поэтому вшиваем минимальный system-промпт ТОЛЬКО с правилом языка,
        // чтобы ответ никогда не уходил на другой язык, чем у пользователя.
        let mut lang_messages: Vec<ChatMessage> = history.to_vec();
        lang_messages.push(ChatMessage {
            id: None,
            msg_type: "message".to_string(),
            content: user_text.to_string(),
            sub_calls: None,
            author: Some("user".to_string()),
            model: None,
            time_sec: None,
            attachments: None,
            phase: None,
        });
        let directive = crate::domain::orchestrator::prompt::language_directive(&lang_messages);

        let mut msgs: Vec<LlmMessage> = vec![LlmMessage {
            role: "system".to_string(),
            content: directive,
            ..Default::default()
        }];
        msgs.extend(history.iter().map(sanitized_llm_message));
        msgs.push(LlmMessage {
            role: "user".to_string(),
            content: orchestrator::prompt::sanitize_model_visible_text(user_text),
            ..Default::default()
        });
        let gen = self
            .engine
            .generate_chat(
                &msgs,
                self.max_gen_tokens,
                resolved_params,
                self.format_type,
                false,
                self.cancel_flag.clone(),
                ctx_label,
                None,
                |_, _| {},
                self.log_cb.clone(),
            )
            .map_err(|e| format!("Ошибка LLM в freeform: {}", e))?;
        Ok(clean_thought_tags(&gen.text))
    }

    /// Зовёт LLM напрямую (без .md агента) — для fact-экстрактора.
    /// `grammar` — строгая GBNF по контракту facts.yaml (точные ключи); если не передана —
    /// любой JSON-объект (запасной вариант).
    pub fn call_llm_direct(
        &self,
        system_prompt: &str,
        user_text: &str,
        resolved_params: &ModelParams,
        ctx_label: &str,
        grammar: Option<String>,
        disable_reasoning: bool,
    ) -> Result<(String, String), String> {
        // Fact-экстрактор обязан вернуть строгий JSON-объект по контракту facts.yaml:
        // точные ключи, фиксированный порядок, без опций.
        //
        // Reasoning-модели (Qwen3/DeepSeek при --reasoning-format deepseek) могут
        // спрятать JSON целиком в блоке размышлений (reasoning_content), оставив
        // content пустым. Поэтому возвращаем ОБА канала; узел LlmFactExtractor
        // парсит content, а при неудаче — reasoning_content. Это и есть ответ
        // модели, просто вынесенный в отдельный канал (принцип deepseek-harness:
        // reasoning и content разделены, финальный ответ берётся из нужного канала).
        let msgs = vec![
            LlmMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
                ..Default::default()
            },
            LlmMessage {
                role: "user".to_string(),
                content: orchestrator::prompt::sanitize_model_visible_text(user_text),
                ..Default::default()
            },
        ];
        (self.log_cb)(format!("[direct] LLM вызов (fact_extractor)..."));
        self.engine.set_grammar(Some(GrammarSpec {
            gbnf: Some(grammar.unwrap_or_else(build_json_only_grammar)),
            json_schema: None,
        }));
        let start = std::time::Instant::now();

        let gen = self
            .engine
            .generate_chat(
                &msgs,
                self.max_gen_tokens,
                resolved_params,
                self.format_type,
                disable_reasoning,
                self.cancel_flag.clone(),
                ctx_label,
                None,
                |_, _| {},
                self.log_cb.clone(),
            )
            .map_err(|e| format!("Ошибка LLM: {}", e));
        (self.log_cb)(format!(
            "[llm] LLM ответ за {:.1}с",
            start.elapsed().as_secs_f32()
        ));
        gen.map(|g| (g.text, g.reasoning))
    }
}

/// Запускает workflow на выполнение.
/// llm_worker узлы вызывают `run_agent_node()`, llm_classifier — built-in.
pub fn run_workflow<L, S, C>(
    workflow: &WorkflowDef,
    context: &mut WorkflowContext,
    runner: &mut WorkflowRunner<L, S, C>,
) -> Result<String, (String, Vec<ChatMessage>)>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    let start_time = Instant::now();
    (runner.log_cb)(format!(
        "[workflow] Запуск '{}', узлов: {}, рёбер: {}",
        workflow.name,
        workflow.nodes.len(),
        workflow.edges.len()
    ));

    // Точка входа объявлена в YAML (`entry:`). Позиция в `nodes:` не является
    // объявлением: визуальный редактор переписывает порядок нод при сохранении.
    let entry = resolve_entry(workflow).map_err(|e| (e, context.messages.clone()))?;
    (runner.log_cb)(format!(
        "[workflow] '{}': вход -> '{}'",
        workflow.name, entry.id
    ));
    let mut queue: Vec<String> = vec![entry.id.clone()];
    // visited → visits: узел может выполниться несколько раз (циклы через рёбра),
    // но не больше node.max_visits (default 1 = прежнее поведение visited-логики).
    let max_visits_map: std::collections::HashMap<String, u32> = workflow
        .nodes
        .iter()
        .map(|n| (n.id.clone(), n.max_visits.unwrap_or(1)))
        .collect();
    let mut visits: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let max_steps = workflow
        .config
        .as_ref()
        .and_then(|c| c.max_steps)
        .unwrap_or(200);
    let mut executed_steps: usize = 0;
    let mut last_node_output: Option<serde_json::Value> = None;

    // Режим исполнения определяется ОДИН раз до цикла. Режима `auto` нет:
    // любая эвристика «модель достаточно сильная» была бы порогом без замера.
    let (strategy, strategy_source) = groups::resolve_strategy(workflow, context.strategy_override)
        .map_err(|e| (e, context.messages.clone()))?;
    (runner.log_cb)(format!(
        "[workflow] '{}': режим исполнения {} (источник: {})",
        workflow.name,
        strategy.as_str(),
        strategy_source
    ));
    if strategy == groups::ExecutionStrategy::Fused {
        let fusable = workflow
            .groups
            .iter()
            .filter(|g| {
                g.fused_instruction
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|i| !i.is_empty())
            })
            .count();
        (runner.log_cb)(format!(
            "[workflow] '{}': режим FUSED, схлопываемых групп: {}",
            workflow.name, fusable
        ));
    }

    while let Some(node_id) = {
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    } {
        if runner.cancel_flag.load(Ordering::SeqCst) {
            return Err((
                "Прервано пользователем".to_string(),
                context.messages.clone(),
            ));
        }

        let node = match workflow.nodes.iter().find(|n| n.id == node_id) {
            Some(n) => n,
            None => {
                return Err((
                    format!("Узел '{}' не найден в workflow", node_id),
                    context.messages.clone(),
                ))
            }
        };

        let node_max_visits = max_visits_map.get(&node_id).copied().unwrap_or(1);
        let visit_count = visits.get(&node_id).copied().unwrap_or(0);
        if visit_count >= node_max_visits {
            continue;
        }
        visits.insert(node_id.clone(), visit_count + 1);

        // Глобальный предохранитель от бесконечных циклов: честная ошибка,
        // а не тихий выход (§2.2).
        executed_steps += 1;
        if executed_steps > max_steps {
            return Err((
                format!(
                "[workflow] '{}' превысил лимит шагов (max_steps={}) — вероятно бесконечный цикл",
                workflow.name, max_steps
            ),
                context.messages.clone(),
            ));
        }

        let node_start = Instant::now();
        (runner.log_cb)(format!(
            "[workflow] Узел: {} (тип: {:?})",
            node.id, node.node_type
        ));

        // Отключённая нода пропускается целиком, ВКЛЮЧАЯ фьюжинг её группы:
        // автор сказал «здесь не выполнять», и молча выполнить вместо этого всю
        // группу означало бы проигнорировать его намерение.
        let fused_group = if node.disabled {
            None
        } else {
            groups::fused_group_for(workflow, node, strategy, &context.executed_groups)
        };

        // Выход ТЕКУЩЕГО шага и нода, из которой будет построена следующая
        // очередь. Для макро-ноды это выход группы, для обычной — сама нода.
        let mut step = step_output::StepOutput::empty();
        let mut jump_to_exit: Option<String> = None;
        // Ноды группы, свёрнутой в макро-вызов: после прыжка их не должно
        // остаться в очереди (см. перестроение очереди ниже).
        let mut collapsed_group: Option<std::collections::HashSet<String>> = None;

        let result = if let Some(group) = fused_group {
            // МАКРО-НОДА. Группа выполняется ОДНИМ вызовом, её внутренние ноды не
            // попадают в очередь планировщика вообще, а прогон продолжается с
            // выхода группы (`jump_to_exit` ниже). Это и есть «Fused»: раньше
            // склеенный вызов делался, а внутренние ноды потом всё равно
            // выполнялись по отдельности — группа стоила БОЛЬШЕ подробного режима.
            groups::ensure_macro_possible(workflow, group)
                .map_err(|e| (e, context.messages.clone()))?;
            let (bool_keys, string_keys) =
                groups::output_contract(workflow, group, Some(std::path::Path::new(&workflow.parent_dir)));
            let expected: Vec<String> = bool_keys
                .iter()
                .chain(string_keys.iter())
                .cloned()
                .collect();
            let signals_json = signals_json(context);
            let system_prompt =
                build_fused_system_prompt(workflow, group, &expected, &signals_json, runner.agents)
                    .map_err(|e| (e, context.messages.clone()))?;
            let grammar = groups::build_fused_grammar(
                workflow,
                group,
                Some(std::path::Path::new(&workflow.parent_dir)),
            );
            let llm_nodes = groups::llm_nodes_of_group(workflow, &group.id);
            let exit = groups::exit_node_of_group(workflow, group.id.trim())
                .map_err(|e| (e, context.messages.clone()))?;
            (runner.log_cb)(format!(
                "[workflow] ⚡ группа '{}' как макро-нода: {} LLM-вызов(ов) схлопывается в 1 \
                 (вход: {}, выход: {}, ноды: {})",
                group.id,
                llm_nodes.len(),
                node.id,
                exit,
                llm_nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>().join(", ")
            ));
            match run_fused_group(
                workflow.config.as_ref(),
                group,
                &system_prompt,
                &grammar,
                &expected,
                context,
                runner,
            ) {
                Ok(output) => {
                    context.executed_groups.insert(group.id.clone());
                    // Прыжок: очередь перестроится от ВЫХОДА группы, а не от ноды,
                    // на которой мы остановились. Внутренние ноды не попадут в
                    // неё ни при каких обстоятельствах.
                    jump_to_exit = Some(exit.clone());
                    collapsed_group = Some(
                        groups::nodes_of_group(workflow, group.id.trim())
                            .iter()
                            .map(|n| n.id.clone())
                            .collect(),
                    );
                    step.signals.extend(output.signals);
                    step.message = output.message.clone();
                    nodes::NodeResult {
                        output: serde_json::json!({
                            "fused_group": group.id,
                            "macro": true,
                            "exit_node": exit,
                        }),
                        next_node: None,
                        next_nodes: vec![],
                    }
                }
                Err(e) => return Err((e, context.messages.clone())),
            }
        } else if let Some(inside) = groups::executed_group_of_node(
            workflow,
            node,
            strategy,
            &context.executed_groups,
        ) {
            // Внутренняя нода макро-группы, до которой дошла очередь НЕ по её
            // входу. Так быть не должно (диагностика `GROUP_FUSED_MULTIPLE_ENTRY`
            // ловит это в редакторе) — но если файл правили руками, честная
            // ошибка лучше тихого повторного вызова LLM: иначе Fused снова станет
            // дороже Detailed, и никто об этом не узнает.
            let _ = inside;
            return Err((
                format!(
                    "внутренняя нода группы '{}' попала в исполнение отдельно: группа уже \
                     выполнена макро-вызовом в этом ходу. Проверьте, что в группу ведёт ровно \
                     одно ребро извне (диагностика GROUP_FUSED_MULTIPLE_ENTRY)",
                    node.group.as_deref().unwrap_or("?").trim()
                ),
                context.messages.clone(),
            ));
        } else if node.disabled {
            (runner.log_cb)(format!(
                "[workflow] Узел '{}' отключён (disabled), пропускаем",
                node.id
            ));
            // При пропуске отключённой ноды продолжаем по её линейному
            // продолжению (sequential_to). Иначе пайплайн доходит до
            // тупика: у нод ConditionCheck/Switch/SignalRouter преемник
            // задан в полях самой ноды, а не ребром, и без вызова
            // execute_node next_node остаётся None.
            let fallthrough = node.sequential_to.clone();
            if fallthrough.is_some() {
                (runner.log_cb)(format!(
                    "[workflow]   skip -> продолжение по sequential_to: {}",
                    fallthrough.as_ref().unwrap()
                ));
            }
            nodes::NodeResult {
                output: serde_json::Value::Null,
                next_node: fallthrough,
                next_nodes: vec![],
            }
        } else {
            match nodes::execute_node(node, workflow, context, runner, &mut step) {
                Ok(r) => r,
                Err(e) => return Err((e, context.messages.clone())),
            }
        };

        // ЕДИНСТВЕННАЯ дверь записи в signal bus. Оба пути — одиночная нода и
        // макро-вызов группы — отдают сюда один и тот же `StepOutput`, поэтому
        // «кто записал ключ» больше не зависит от того, какой путь выполнился.
        step_output::handle_step_output(&step, &node.id, context);

        if !node.disabled {
            (runner.log_cb)(format!(
                "[workflow] Узел '{}' выполнен за {:.1}с",
                node.id,
                node_start.elapsed().as_secs_f32()
            ));
            context
                .node_outputs
                .insert(node.id.clone(), result.output.clone());
            last_node_output = Some(result.output.clone());
        }

        // Строим новый порядок очереди: [next_node, ...next_nodes, ...остаток очереди]
        // Узел попадает в очередь, пока не исчерпан его max_visits — это и есть
        // поддержка циклов: ребро-петля перестаёт срабатывать после N выполнений.
        let within_visits = |nid: &str| -> bool {
            let max = max_visits_map.get(nid).copied().unwrap_or(1);
            visits.get(nid).copied().unwrap_or(0) < max
        };
        // МАКРО-ПРЫЖОК: очередь строится от ВЫХОДА группы, а не от ноды, на
        // которой мы остановились. У результата макро-вызова нет `status` /
        // `matched_case`, поэтому маршрут берётся по безусловным рёбрам выхода —
        // условные после группы запрещены диагностикой и `ensure_macro_possible`.
        let next = match &jump_to_exit {
            Some(exit) => {
                let plain = nodes::NodeResult {
                    output: serde_json::Value::Null,
                    next_node: None,
                    next_nodes: vec![],
                };
                find_next_node(exit, &workflow.edges, &plain)
            }
            None => find_next_node(&node_id, &workflow.edges, &result),
        };
        let mut new_queue: Vec<String> = Vec::new();
        if let Some(nid) = next {
            if nid != "__END__" && nid != "END" && within_visits(&nid) {
                new_queue.push(nid);
            }
        }

        for nid in &result.next_nodes {
            if within_visits(nid) {
                new_queue.push(nid.clone());
            }
        }

        // Остаток очереди. После макро-прыжка из неё убираются ноды СВЕРНУТОЙ
        // группы: группа теперь одна нода, и нода внутри неё — не следующий шаг,
        // а внутренность уже выполненной. Это не «пропуск нод», а следствие
        // свёртки: очередь не должна содержать то, чего в графе уже нет.
        for nid in &queue {
            if let Some(collapsed) = &collapsed_group {
                if collapsed.contains(nid.as_str()) {
                    continue;
                }
            }
            if within_visits(nid) {
                new_queue.push(nid.clone());
            }
        }

        queue = new_queue;
    }

    let final_output = last_node_output
        .map(|v| {
            // {"result": "text"} → "text"
            if let Some(result_str) = v.get("result").and_then(|r| r.as_str()) {
                return result_str.to_string();
            }
            // plain string value
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
            serde_json::to_string(&v).unwrap_or_default()
        })
        .unwrap_or_default();

    (runner.log_cb)(format!(
        "[workflow] '{}' завершён за {:.1}с",
        workflow.name,
        start_time.elapsed().as_secs_f32()
    ));

    Ok(final_output)
}

/// Системный промпт макро-ноды: явная инструкция автора ИЛИ авто-склейка ролей.
///
/// Явная `fused_instruction` — ручной оверрайд (короче и под конкретную задачу).
/// Без неё движок берёт роль каждой ноды группы из её агента: тот же
/// `AgentProfile.system_prompt`, из которого оркестратор строит промпт при
/// обычном вызове. Служебные добавки `build_system_prompt` (лимит генерации,
/// правила вызова инструментов, требование emit_signal) НЕ склеиваются: у
/// склеенного вызова нет ни инструментов, ни отдельного вызова на агента, и эти
/// секции противоречили бы его грамматике.
///
/// Берёт каталог агентов, а не весь `WorkflowRunner`: всё, что нужно, — роли.
/// Это же делает функцию проверяемой без живого движка.
fn build_fused_system_prompt(
    workflow: &WorkflowDef,
    group: &GroupDef,
    keys: &[String],
    signals_json: &str,
    agents: &[AgentProfile],
) -> Result<String, String> {
    let explicit = group.fused_instruction.as_deref().unwrap_or("").trim();
    if !explicit.is_empty() {
        return Ok(groups::build_fused_prompt(group, keys, signals_json));
    }

    let ordered = groups::topo_order_of_group(workflow, group.id.trim())
        .map_err(|e| format!("группа '{}': {}", group.id, e))?;
    if ordered.is_empty() {
        return Err(format!(
            "группа '{}' пуста: нечего выполнять макро-вызовом",
            group.id
        ));
    }
    let mut stages: Vec<groups::FusedStage> = Vec::with_capacity(ordered.len());
    for node in ordered {
        if let Some(agent_id) = node.agent.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            let agent = agents.iter().find(|a| a.id == agent_id).ok_or_else(|| {
                format!(
                    "в группе '{}': нода '{}' ссылается на агента '{}', которого нет в каталоге",
                    group.id, node.id, agent_id
                )
            })?;
            if agent.system_prompt.trim().is_empty() {
                return Err(format!(
                    "в группе '{}': у агента '{}` (нода '{}') пустой системный промпт — \
                     склеить из него этап нельзя. Заполните `.md` агента или задайте группе \
                     `fused_instruction`",
                    group.id, agent_id, node.id
                ));
            }
            stages.push(groups::FusedStage {
                node_id: node.id.clone(),
                role: agent.name.clone(),
                prompt: agent.system_prompt.clone(),
            });
        } else if let Some(input) = node.input.as_deref().map(str::trim).filter(|i| !i.is_empty()) {
            stages.push(groups::FusedStage {
                node_id: node.id.clone(),
                role: "свободная роль ноды".to_string(),
                prompt: input.to_string(),
            });
        } else {
            return Err(format!(
                "в группе '{}`: у ноды '{}` нет ни агента, ни `input` — склеить из неё роль \
                 нельзя. Задайте группе `fused_instruction` явно",
                group.id, node.id
            ));
        }
    }
    Ok(groups::build_auto_fused_prompt(group, &stages, keys, signals_json))
}

/// Выполнить одну группу нод одним вызовом LLM (МАКРО-НОДА) и вернуть её выход.
///
/// Возвращает `StepOutput` — тот же контракт, что и одиночная нода; в signal bus
/// его записывает планировщик через `handle_step_output`. Сообщение в чат группа
/// отправляет сама: её авторство (`fused_agent` ?? id группы) не совпадает с
/// авторством обычной ноды, и решать это должен тот, кто знает id группы.
///
/// Форма ответа задана грамматикой `build_fused_grammar`, поэтому «текст вместо
/// JSON» или «JSON без обязательного ключа» — честная ошибка с перечнем ключей,
/// а не пустые all-false сигналы, которые отправили бы граф по ложной ветке.
fn run_fused_group<L, S, C>(
    graph_config: Option<&WorkflowConfig>,
    group: &GroupDef,
    system_prompt: &str,
    grammar: &str,
    expected: &[String],
    context: &mut WorkflowContext,
    runner: &mut WorkflowRunner<L, S, C>,
) -> Result<step_output::StepOutput, String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    groups::ensure_supported(group)?;

    // История переписки нужна так же, как обычной ноде: склеенный вызов отвечает
    // юзеру в том же ходе, что и соседние ноды графа.
    let user_text = context.resolve_template("{{ messages }}");
    // Каскад параметров — тот же SSOT, что у обычных нод (пункт 1 взят из
    // графа-владельца группы): свой пресет у группы не появится, иначе
    // приоритеты пришлось бы разводить в третий раз.
    let resolved = resolve_params_cascade(
        &runner.sampling_presets,
        runner.model_params,
        &None,
        &graph_config.cloned(),
        None,
    );
    let (text, _reasoning) = runner.call_llm_direct(
        system_prompt,
        &user_text,
        &resolved,
        &format!("fused:{}", group.id),
        Some(grammar.to_string()),
        // Строгая грамматика JSON: размышление полезно для качества сигналов,
        // но бюджет n_predict ушёл бы в него (см. fact_extractor::build_facts_grammar).
        true,
    )?;
    let output = groups::parse_fused_output(&text, expected)?;

    if !output.message.is_empty() {
        let msg = ChatMessage {
            id: Some(format!("msg_{}", runner.msg_counter)),
            msg_type: "message".to_string(),
            content: output.message.clone(),
            sub_calls: None,
            author: Some(group.fused_agent.clone().unwrap_or_else(|| group.id.clone())),
            model: None,
            time_sec: None,
            attachments: None,
            phase: None,
        };
        context.messages.push(msg);
        *runner.msg_counter += 1;
        context.output_emitted = true;
    }
    (runner.log_cb)(format!(
        "[workflow] ⚡ группа '{}': склеенный ответ разобран, сигналов: {}, текст: {}",
        group.id,
        output.signals.len(),
        if output.message.is_empty() { "нет" } else { "есть" }
    ));
    // В шину сигналы пишет планировщик — здесь только объявляем результат шага.
    Ok(step_output::StepOutput {
        message: if output.message.is_empty() {
            None
        } else {
            Some(output.message.clone())
        },
        signals: output.signals.into_iter().collect(),
    })
}

/// JSON текущих сигналов хода — для блока `[СИГНАЛЫ И ФАКТЫ ТЕКУЩЕГО ШАГА]`.
///
/// Порядок ключей стабилизирован (BTreeMap), иначе одинаковый ход давал бы
/// разный промпт и сравнивать прогоны было бы невозможно.
fn signals_json(context: &WorkflowContext) -> String {
    let ordered: std::collections::BTreeMap<&String, &serde_json::Value> =
        context.signals.iter().collect();
    serde_json::to_string(&ordered).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_presets() -> SamplingPresets {
        let mut presets = HashMap::new();
        let mut strict = ModelParams::default();
        strict.temperature = 0.0;
        strict.top_p = 1.0;
        strict.repetition_penalty = 1.1;
        presets.insert("strict".to_string(), strict);

        let mut creative = ModelParams::default();
        creative.temperature = 0.8;
        creative.top_p = 0.95;
        presets.insert("creative".to_string(), creative);
        presets
    }

    fn test_agent(id: &str, temp: Option<f32>) -> AgentProfile {
        AgentProfile {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            system_prompt: String::new(),
            mode: "worker".to_string(),
            mcp_servers: Vec::new(),
            subagents: Vec::new(),
            folder: None,
            replace_report: false,
            tools: Vec::new(),
            current_date: false,
            temperature: temp,
            vision: false,
            think_budget_chars: None,
            disable_reasoning: false,
            visible: true,
        }
    }

    #[test]
    fn freeform_history_message_hides_filesystem_paths() {
        let message = ChatMessage {
            id: Some("msg_1".to_string()),
            msg_type: "message".to_string(),
            content: "D:\\private\\image.png /home/user/image.png output/image.png \\\\server\\share\\image.png".to_string(),
            sub_calls: None,
            author: Some("user".to_string()),
            model: None,
            time_sec: None,
            attachments: None,
            phase: None,
        };
        let sanitized = sanitized_llm_message(&message);
        assert!(!sanitized.content.contains("D:\\private"));
        assert!(!sanitized.content.contains("/home/user"));
        assert!(!sanitized.content.contains("output/image.png"));
        assert!(!sanitized.content.contains("\\\\server\\share"));
        assert_eq!(sanitized.content.matches("[filesystem path omitted]").count(), 4);
    }

    // ─── SMOKE: авто-склейка системного промпта макро-ноды ────────────────

    /// Каталог агентов для склейки: роли берутся из их `.md`.
    fn role_agent(id: &str, name: &str, prompt: &str) -> AgentProfile {
        let mut a = test_agent(id, None);
        a.name = name.to_string();
        a.system_prompt = prompt.to_string();
        a
    }

    fn chain_workflow(with_instruction: Option<&str>) -> WorkflowDef {
        let group = GroupDef {
            id: "group_1".to_string(),
            title: "Декомпозиция и синтез".to_string(),
            fused_instruction: with_instruction.map(str::to_string),
            ..Default::default()
        };
        let mut workflow = WorkflowDef {
            name: "W".to_string(),
            description: None,
            visible: true,
            file_stem: String::new(),
            parent_dir: String::new(),
            config: None,
            entry: None,
            strategy: Some("fused".to_string()),
            groups: vec![group],
            nodes: vec![],
            edges: vec![],
        };
        for (id, agent) in [("call_decomposer", "decomposer"), ("call_synthesizer", "synthesizer")] {
            workflow.nodes.push(NodeDef {
                id: id.to_string(),
                node_type: NodeType::LlmWorker,
                group: Some("group_1".to_string()),
                agent: Some(agent.to_string()),
                ..Default::default()
            });
        }
        workflow.edges.push(EdgeDef {
            from: "call_decomposer".to_string(),
            to: "call_synthesizer".to_string(),
            condition: None,
            case: None,
        });
        workflow
    }

    fn smoke_agents() -> Vec<AgentProfile> {
        vec![
            role_agent("decomposer", "Декомпозитор", "Разбей запрос на отдельные мишени."),
            role_agent("synthesizer", "Синтезатор", "Собери из мишеней связный ответ."),
        ]
    }

    /// ДЫМОВОЙ ТЕСТ: автор не написал ни одной инструкции — движок сам собрал
    /// промпт из ролей агентов, в порядке выполнения нод.
    #[test]
    fn smoke_group_without_instruction_stitches_agent_roles() {
        let wf = chain_workflow(None);
        let keys = vec![groups::MESSAGE_KEY.to_string()];
        let prompt = build_fused_system_prompt(
            &wf,
            &wf.groups[0],
            &keys,
            r#"{"has_problem":true}"#,
            &smoke_agents(),
        )
        .expect("склейка");

        assert!(prompt.contains("ЭТАП 1") && prompt.contains("ЭТАП 2"), "этапов нет:\n{}", prompt);
        assert!(prompt.contains("Разбей запрос на отдельные мишени."), "роль 1 не попала");
        assert!(prompt.contains("Собери из мишеней связный ответ."), "роль 2 не попала");
        assert!(
            prompt.find("call_decomposer") < prompt.find("call_synthesizer"),
            "этапы в неверном порядке:\n{}",
            prompt
        );
        assert!(prompt.contains("has_problem"), "сигналы хода не попали");
        assert!(prompt.contains("ТОЛЬКО JSON"), "нет требования формата");
        assert!(prompt.contains("message"), "нет ключа message");
    }

    /// Явная инструкция автора — оверрайд: склейка не используется.
    #[test]
    fn smoke_explicit_instruction_wins_over_stitching() {
        let wf = chain_workflow(Some("Сделай всё сам."));
        let keys = vec![groups::MESSAGE_KEY.to_string()];
        let prompt =
            build_fused_system_prompt(&wf, &wf.groups[0], &keys, "{}", &smoke_agents())
                .expect("инструкция");

        assert!(prompt.contains("Сделай всё сам."));
        assert!(!prompt.contains("ЭТАП 1"), "склейка не должна срабатывать:\n{}", prompt);
    }

    /// Неизвестный агент — ошибка с именем, а не молчаливая потеря этапа.
    #[test]
    fn smoke_unknown_agent_is_an_error_naming_it() {
        let wf = chain_workflow(None);
        let err = build_fused_system_prompt(
            &wf,
            &wf.groups[0],
            &[groups::MESSAGE_KEY.to_string()],
            "{}",
            &[],
        )
        .expect_err("нет агентов");
        assert!(err.contains("decomposer"), "в ошибке нет имени агента: {}", err);
    }

    /// Нода без агента и без `input` — склеить нельзя, и это сказано прямо.
    #[test]
    fn smoke_node_without_role_is_an_error_with_a_way_out() {
        let mut wf = chain_workflow(None);
        wf.nodes[0].agent = None;
        let err = build_fused_system_prompt(
            &wf,
            &wf.groups[0],
            &[groups::MESSAGE_KEY.to_string()],
            "{}",
            &smoke_agents(),
        )
        .expect_err("нечем склеивать");
        assert!(err.contains("call_decomposer"), "{}", err);
        assert!(err.contains("fused_instruction"), "в ошибке нет выхода: {}", err);
    }

    #[test]
    fn node_preset_has_absolute_priority() {
        let presets = test_presets();
        let user = ModelParams { temperature: 0.5, ..Default::default() };
        let node_preset = Some("strict".to_string());
        let wf_config = Some(WorkflowConfig {
            default_llm_params: Some("creative".to_string()),
            ..Default::default()
        });
        let agent = test_agent("primary_coder", Some(0.3));

        let res = resolve_params_cascade(&presets, &user, &node_preset, &wf_config, Some(&agent));
        // Явный пресет узла "strict" пинит всё, включая temperature=0.0
        assert_eq!(res.temperature, 0.0);
        assert_eq!(res.repetition_penalty, 1.1);
    }

    #[test]
    fn agent_frontmatter_temperature_overlays_on_graph_default_preset() {
        let presets = test_presets();
        let user = ModelParams { temperature: 0.5, ..Default::default() };
        let node_preset = None;
        let wf_config = Some(WorkflowConfig {
            default_llm_params: Some("strict".to_string()),
            ..Default::default()
        });
        let agent = test_agent("primary_coder", Some(0.1));

        let res = resolve_params_cascade(&presets, &user, &node_preset, &wf_config, Some(&agent));
        // База strict (rep_pen 1.1), но temperature перетерта на 0.1 из frontmatter
        assert_eq!(res.temperature, 0.1);
        assert_eq!(res.repetition_penalty, 1.1);
    }

    #[test]
    fn agent_without_temperature_keeps_graph_default_preset() {
        let presets = test_presets();
        let user = ModelParams { temperature: 0.5, ..Default::default() };
        let node_preset = None;
        let wf_config = Some(WorkflowConfig {
            default_llm_params: Some("strict".to_string()),
            ..Default::default()
        });
        let agent = test_agent("analyst", None);

        let res = resolve_params_cascade(&presets, &user, &node_preset, &wf_config, Some(&agent));
        assert_eq!(res.temperature, 0.0);
        assert_eq!(res.repetition_penalty, 1.1);
    }

    #[test]
    fn agent_frontmatter_overlays_on_user_base_when_no_presets() {
        let presets = test_presets();
        let user = ModelParams { temperature: 0.5, top_p: 0.9, ..Default::default() };
        let node_preset = None;
        let wf_config = None;
        let agent = test_agent("ux_ui_designer", Some(0.4));

        let res = resolve_params_cascade(&presets, &user, &node_preset, &wf_config, Some(&agent));
        assert_eq!(res.temperature, 0.4);
        assert_eq!(res.top_p, 0.9);
    }
}
