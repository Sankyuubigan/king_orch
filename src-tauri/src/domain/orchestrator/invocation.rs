use super::*;
use std::path::Path;
use std::fs;
use std::io::Write;
use serde_json::Value;
use crate::infra::{ChatMessage, LlmMessage, SubCall, ToolCallInfo, ModelParams, ChatAttachment, LlamaEngine, GrammarSpec, extract_model_filename, push_report};
use crate::domain::agent_manager::AgentProfile;

/// Компактное описание вызова агента для subcall (вместо полной копии системного
/// промпта): только task и injected_reports — то, что реально различается между
/// вызовами. Источник промпта — сам .md файл агента (SSOT).
pub(crate) fn build_invocation_dump(task: &str, injected_reports: &str) -> String {
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

pub(crate) fn log_agent_thought(log_cb: &dyn Fn(String), agent: &AgentProfile, action_type: &str, target: &str, thought: &str, thinking_sec: f32, depth: usize) {
    if thought.is_empty() { return; }
    if thinking_sec > 0.0 {
        log_cb(format!("💭 Мысль {} [d={}] ({} {}) [⏱{:.1}с]: {}", agent.name, depth, action_type, target, thinking_sec, thought));
    } else {
        log_cb(format!("💭 Мысль {} [d={}] ({} {}): {}", agent.name, depth, action_type, target, thought));
    }
}

pub(crate) fn valid_agent_ids(agents: &[AgentProfile], exclude_id: &str, exclude_mode: &str) -> Vec<String> {
    agents.iter()
        .filter(|a| a.id != exclude_id && a.mode != exclude_mode)
        .map(|a| a.id.clone())
        .collect()
}

