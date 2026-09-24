//! 🚂 Workflow Engine — графовый движок маршрутизации
//!
//! Отвечает за выполнение YAML-графов (workflows).
//! Каждый workflow — это направленный граф узлов (nodes) и рёбер (edges).

pub mod context;
pub mod fact_extractor;
pub mod nodes;
pub mod parser;

pub use context::WorkflowContext;
pub use parser::WorkflowConfig;
pub use parser::{find_workflow_by_stem, load_workflows, NodeType, WorkflowDef};

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
        two_phase_thinking: bool,
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
            two_phase_thinking,
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

    let mut queue: Vec<String> = workflow
        .nodes
        .first()
        .map(|n| n.id.clone())
        .into_iter()
        .collect();
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

        let result = if node.disabled {
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
            match nodes::execute_node(node, workflow, context, runner) {
                Ok(r) => r,
                Err(e) => return Err((e, context.messages.clone())),
            }
        };

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
        let next = find_next_node(&node_id, &workflow.edges, &result);
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

        for nid in &queue {
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
            is_hidden: false,
            mode: "worker".to_string(),
            mcp_servers: Vec::new(),
            subagents: Vec::new(),
            folder: None,
            replace_report: false,
            tools: Vec::new(),
            current_date: false,
            temperature: temp,
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
