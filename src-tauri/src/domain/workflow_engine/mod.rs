//! 🚂 Workflow Engine — графовый движок маршрутизации
//!
//! Отвечает за выполнение YAML-графов (workflows).
//! Каждый workflow — это направленный граф узлов (nodes) и рёбер (edges).

pub mod context;
pub mod fact_extractor;
pub mod nodes;
pub mod parser;

pub use context::WorkflowContext;
pub use parser::{find_workflow_by_stem, load_workflows, NodeType, WorkflowDef};
pub use parser::WorkflowConfig;

use crate::domain::agent_manager::AgentProfile;
use crate::domain::orchestrator;
use crate::domain::parsers::clean_thought_tags;
use crate::infra::{ChatMessage, LlamaEngine, ModelParams, SamplingPresets, SubCall, LlmMessage, GrammarSpec, build_json_only_grammar};
use nodes::find_next_node;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Общие ресурсы для выполнения workflow — пробрасываются во все узлы.
pub struct WorkflowRunner<'a, L, S, C> {
    pub engine: &'a LlamaEngine,
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
}

impl<'a, L, S, C> WorkflowRunner<'a, L, S, C>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    /// Резолвит параметры LLM для узла: node.llm_params → config.default_llm_params → base params.
    pub fn resolve_llm_params(
        &self,
        node_llm_params: &Option<String>,
        workflow_config: &Option<WorkflowConfig>,
    ) -> ModelParams {
        // 1. Приоритет: llm_params на узле
        if let Some(ref preset_name) = node_llm_params {
            if let Some(preset) = self.sampling_presets.get(preset_name) {
                return preset.clone();
            }
            eprintln!("[workflow] Пресет '{}' не найден в sampling_presets.json, fallback на base params", preset_name);
        }
        // 2. default_llm_params в config workflow
        if let Some(ref config) = workflow_config {
            if let Some(ref default_name) = config.default_llm_params {
                if let Some(preset) = self.sampling_presets.get(default_name) {
                    return preset.clone();
                }
                eprintln!("[workflow] Дефолтный пресет '{}' не найден в sampling_presets.json, fallback на base params", default_name);
            }
        }
        // 3. Базовые параметры пользователя
        self.model_params.clone()
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
    ) -> Result<String, String> {
        orchestrator::run_agent_node(
            self.log_cb.clone(),
            self.status_cb.clone(),
            self.subcall_cb.clone(),
            self.engine,
            agent,
            self.agents,
            task.to_string(),
            vec![],
            &[],
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
            messages,
            self.msg_counter,
            injected_reports.to_string(),
            self.stream_meta.clone(),
            allow_stream,
        )
    }

    /// Зовёт LLM со свободным ответом (без системного промпта) — для llm_freeform
    pub fn call_llm_freeform(&self, user_text: &str, history: &[ChatMessage], ctx_label: &str) -> Result<String, String> {
        let mut msgs: Vec<LlmMessage> = history.iter().map(|m| m.to_llm_message()).collect();
        msgs.push(LlmMessage {
            role: "user".to_string(),
            content: user_text.to_string(),
        });
        let gen = self
            .engine
            .generate_chat(
                &msgs,
                self.max_gen_tokens,
                self.model_params,
                self.format_type,
                self.cancel_flag.clone(),
                ctx_label,
                |_, _| {},
                self.log_cb.clone(),
            )
            .map_err(|e| format!("Ошибка LLM в freeform: {}", e))?;
        Ok(clean_thought_tags(&gen.text))
    }

    /// Зовёт LLM напрямую (без .md агента) — для fact-экстрактора
    pub fn call_llm_direct(&self, system_prompt: &str, user_text: &str, resolved_params: &ModelParams, ctx_label: &str) -> Result<String, String> {
        let msgs = vec![
            LlmMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            },
            LlmMessage {
                role: "user".to_string(),
                content: user_text.to_string(),
            },
        ];
        (self.log_cb)("[direct] LLM вызов (fact_extractor)...".to_string());
        // fact-экстрактор обязан вернуть строгий JSON-объект — жёсткая грамматика
        self.engine.set_grammar(Some(GrammarSpec {
            gbnf: Some(build_json_only_grammar()),
            json_schema: None,
        }));
        let start = std::time::Instant::now();

        let gen = self.engine
            .generate_chat(
                &msgs,
                self.max_gen_tokens,
                resolved_params,
                self.format_type,
                self.cancel_flag.clone(),
                ctx_label,
                |_, _| {},
                self.log_cb.clone(),
            )
            .map_err(|e| format!("Ошибка LLM: {}", e));
        (self.log_cb)(format!("[llm] LLM ответ за {:.1}с", start.elapsed().as_secs_f32()));
        gen.map(|g| g.text)
    }
}

/// Запускает workflow на выполнение.
/// llm_worker узлы вызывают `run_agent_node()`, llm_classifier — built-in.
pub fn run_workflow<L, S, C>(
    workflow: &WorkflowDef,
    context: &mut WorkflowContext,
    runner: &mut WorkflowRunner<L, S, C>,
) -> Result<String, String>
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

    let mut queue: Vec<String> = workflow.nodes.first().map(|n| n.id.clone()).into_iter().collect();
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
        if queue.is_empty() { None } else { Some(queue.remove(0)) }
    } {
        if runner.cancel_flag.load(Ordering::SeqCst) {
            return Err("Прервано пользователем".to_string());
        }

        let node = workflow
            .nodes
            .iter()
            .find(|n| n.id == node_id)
            .ok_or_else(|| format!("Узел '{}' не найден в workflow", node_id))?;

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
            return Err(format!(
                "[workflow] '{}' превысил лимит шагов (max_steps={}) — вероятно бесконечный цикл",
                workflow.name, max_steps
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
            nodes::execute_node(node, workflow, context, runner)?
        };

        if !node.disabled {
            (runner.log_cb)(format!("[workflow] Узел '{}' выполнен за {:.1}с", node.id, node_start.elapsed().as_secs_f32()));
            context.node_outputs.insert(node.id.clone(), result.output.clone());
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