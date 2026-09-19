use super::*;
use crate::domain::agent_manager::AgentProfile;
use crate::infra::{
    extract_model_filename, push_report, ChatAttachment, ChatMessage, GrammarSpec, LlamaEngine,
    LlmMessage, ModelParams, SubCall, ToolCallInfo,
};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;

#[allow(clippy::too_many_arguments)]
#[allow(unused_assignments)]
/// Снимок ТОЧНОГО входа модели (`llm_messages`) перед каждым вызовом LLM.
///
/// Реализует правило «модель видит только то, что записано»: сам факт записи
/// всего отправленного делает вход воспроизводимым (база replay-тестов без модели)
/// и устраняет риск «тихо показать модели то, чего нет в логе».
/// Запись best-effort: ошибки НЕ фатальны (правило 2.2 — логируем, не падаем).
pub(crate) fn write_prompt_log(
    path: &Path,
    agent: &str,
    call: usize,
    tokens: usize,
    messages: &[LlmMessage],
) {
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
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(mut f) => {
            let _ = writeln!(f, "{}", line);
        }
        Err(e) => {
            eprintln!("[prompt_log] не удалось записать {}: {}", path.display(), e);
        }
    }
}
