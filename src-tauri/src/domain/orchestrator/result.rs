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
    /// Текст ошибки, если прогон завершился сбоем, но был возвращён Ok
    /// с системным сообщением (см. catch в run_chat). None — успешный прогон.
    pub has_error: Option<String>,
}
