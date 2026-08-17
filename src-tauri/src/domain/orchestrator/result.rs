use super::*;
use std::path::Path;
use std::fs;
use std::io::Write;
use serde_json::Value;
use crate::infra::{ChatMessage, LlmMessage, SubCall, ToolCallInfo, ModelParams, ChatAttachment, LlamaEngine, GrammarSpec, extract_model_filename, push_report};
use crate::domain::agent_manager::AgentProfile;

/// Результат чата: текст ответа + собранные sub-calls + обновлённый массив сообщений
/// + диагностика движка (режим GPU/CPU и скорость) для UI-индикатора.
#[derive(Debug, Clone)]
pub struct ChatRunResult {
    pub text: String,
    pub sub_calls: Vec<SubCall>,
    pub messages: Vec<ChatMessage>,
    /// "gpu" / "cpu" — как реально работала модель в этом запросе
    pub engine_mode: String,
    /// Скорость последней генерации (tok/s)
    pub engine_tok_per_sec: f64,
    /// Причина CPU-режима (пусто, если GPU)
    pub engine_mode_detail: String,
}

