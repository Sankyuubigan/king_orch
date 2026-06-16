//! 🚂 Workflow Engine — графовый движок маршрутизации
//!
//! Отвечает за выполнение YAML-графов (workflows).
//! Каждый workflow — это направленный граф узлов (nodes) и рёбер (edges).

pub mod context;
pub mod fact_extractor;
pub mod nodes;
pub mod parser;

pub use context::WorkflowContext;
pub use parser::{find_workflow_by_stem, load_workflows, WorkflowDef};

use crate::domain::agent_manager::AgentProfile;
use crate::domain::orchestrator;
use crate::domain::parsers::clean_thought_tags;
use crate::infra::{ChatMessage, LlamaEngine, ModelParams, SubCall};
use nodes::find_next_node;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    pub all_sub_calls: &'a mut Vec<SubCall>,
    pub msg_counter: &'a mut u32,
}

impl<'a, L, S, C> WorkflowRunner<'a, L, S, C>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    /// Выполняет .md агента через `run_agent_node()`
    pub fn call_agent(
        &mut self,
        agent: &AgentProfile,
        task: &str,
        namespace: &str,
        messages: &mut Vec<ChatMessage>,
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
            namespace,
            self.max_gen_tokens,
            self.model_params,
            self.format_type,
            self.cancel_flag.clone(),
            1,
            self.all_sub_calls,
            Some("workflow_engine".to_string()),
            self.mcp_servers_dir,
            messages,
            self.msg_counter,
        )
    }

    /// Зовёт LLM со свободным ответом (без системного промпта) — для llm_freeform
    pub fn call_llm_freeform(&self, user_text: &str, history: &[ChatMessage]) -> Result<String, String> {
        let mut msgs = history.to_vec();
        msgs.push(ChatMessage {
            id: None,
            msg_type: "message".to_string(),
            content: user_text.to_string(),
            namespace: None,
            sub_calls: None,
            author: Some("user".to_string()),
        });
        let raw = self
            .engine
            .generate_chat(
                &msgs,
                self.max_gen_tokens,
                self.model_params,
                self.format_type,
                self.cancel_flag.clone(),
                |_, _| {},
                self.log_cb.clone(),
            )
            .map_err(|e| format!("Ошибка LLM в freeform: {}", e))?;
        Ok(clean_thought_tags(&raw))
    }

    /// Зовёт LLM напрямую (без .md агента) — для fact-экстрактора
    pub fn call_llm_direct(&self, system_prompt: &str, user_text: &str) -> Result<String, String> {
        let msgs = vec![
            ChatMessage {
                id: None,
                msg_type: "message".to_string(),
                content: system_prompt.to_string(),
                namespace: None,
                sub_calls: None,
                author: Some("system".to_string()),
            },
            ChatMessage {
                id: None,
                msg_type: "message".to_string(),
                content: user_text.to_string(),
                namespace: None,
                sub_calls: None,
                author: Some("user".to_string()),
            },
        ];
        (self.log_cb)("[direct] LLM вызов (fact_extractor)...".to_string());
        let start = std::time::Instant::now();
        let result = self.engine
            .generate_chat(
                &msgs,
                self.max_gen_tokens,
                self.model_params,
                self.format_type,
                self.cancel_flag.clone(),
                |_, _| {},
                self.log_cb.clone(),
            )
            .map_err(|e| format!("Ошибка LLM: {}", e));
        (self.log_cb)(format!("[direct] LLM ответ за {:.1}с", start.elapsed().as_secs_f32()));
        result
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
    let mut visited = std::collections::HashSet::new();
    let mut last_node_output: Option<serde_json::Value> = None;

    while let Some(node_id) = {
        if queue.is_empty() { None } else { Some(queue.remove(0)) }
    } {
        if runner.cancel_flag.load(Ordering::SeqCst) {
            return Err("Прервано пользователем".to_string());
        }

        if visited.contains(&node_id) {
            continue;
        }
        visited.insert(node_id.clone());

        let node = workflow
            .nodes
            .iter()
            .find(|n| n.id == node_id)
            .ok_or_else(|| format!("Узел '{}' не найден в workflow", node_id))?;

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
            nodes::NodeResult {
                output: serde_json::Value::Null,
                next_node: None,
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
        let next = find_next_node(&node_id, &workflow.edges, &result);
        let mut new_queue: Vec<String> = Vec::new();

        if let Some(nid) = next {
            if nid != "__END__" && nid != "END" && !visited.contains(&nid) {
                new_queue.push(nid);
            }
        }

        for nid in &result.next_nodes {
            if !visited.contains(nid) {
                new_queue.push(nid.clone());
            }
        }

        for nid in &queue {
            if !visited.contains(nid) {
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
