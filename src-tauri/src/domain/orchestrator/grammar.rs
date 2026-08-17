use super::*;
use std::path::Path;
use std::fs;
use std::io::Write;
use serde_json::Value;
use crate::infra::{ChatMessage, LlmMessage, SubCall, ToolCallInfo, ModelParams, ChatAttachment, LlamaEngine, GrammarSpec, extract_model_filename, push_report};
use crate::domain::agent_manager::AgentProfile;

/// Загружает per-agent GBNF-грамматику из `grammars_dir/<agent_id>.gbnf`.
/// Если файла нет — агент работает без per-agent грамматики (только база движка).
pub(crate) fn load_agent_grammar(grammars_dir: &Path, agent_id: &str) -> Option<String> {
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

