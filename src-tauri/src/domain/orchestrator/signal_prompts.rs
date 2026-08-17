use super::*;
use std::path::Path;
use std::fs;
use std::io::Write;
use serde_json::Value;
use crate::infra::{ChatMessage, LlmMessage, SubCall, ToolCallInfo, ModelParams, ChatAttachment, LlamaEngine, GrammarSpec, extract_model_filename, push_report};
use crate::domain::agent_manager::AgentProfile;

/// Промпт для сигнального LLM-вызова: сохранить результат анализа как сигнал.
pub(crate) fn signal_request_prompt(contract_key: &str) -> String {
    format!(
        "Отлично. Теперь сохрани результат анализа как сигнал: вызови инструмент emit_signal с key=\"{}\" и value по контракту (точно той структуры, как описано в системном промпте). Ответь ТОЛЬКО JSON с вызовом эмиссии — без пояснений.",
        contract_key
    )
}

/// Корректирующий хинт для ретрая сигнального вызова: модель должна вернуть
/// РОВНО один JSON-конверт emit_signal (без markdown и пояснений).
pub(crate) fn signal_retry_hint(contract_key: &str) -> String {
    format!(
        "⚠️ Твой ответ не был распознан как вызов emit_signal. Верни РОВНО ОДИН JSON без пояснений и без markdown: {{\"tool\": \"emit_signal\", \"arguments\": {{\"key\": \"{}\", \"value\": <значение по контракту из системного промпта>}}}}.",
        contract_key
    )
}

