use super::*;
use std::sync::{Arc, Mutex};
use crate::infra::{ChatMessage, LlmMessage, SubCall, ToolCallInfo, ModelParams, ChatAttachment, LlamaEngine, GrammarSpec, extract_model_filename, push_report};
use crate::domain::agent_manager::AgentProfile;

/// Метаданные текущего стрима: куда выводить токены.
/// `kind == "message"` → печатать в основной чат юзеру.
/// `kind == "thought"` → печатать в блок «Мысли агентов».
/// Пустой `kind` → не стримить вообще (внутренние вызовы: fact-extractor и т.п.).
#[derive(Clone, Default)]
pub struct StreamMeta {
    pub kind: String,
    pub author: String,
    /// Накопленный сырой текст текущего стрима. Нужен, чтобы фильтровать
    /// служебные теги LLM (`<|channel>...`, `<|turn>`) по ПОЛНОМУ тексту,
    /// а не по отдельному чанку (тег может быть разорван между чанками).
    pub buffer: String,
    /// Флаг: `<think>` уже закрылся. Используется в stream_cb для
    /// динамического переключения kind с "thought" на "message".
    pub thinking_done: bool,
}

/// Восстанавливает предыдущее значение `StreamMeta` при выходе из узла/агента,
/// чтобы вложенные сабагенты не оставляли флаг включённым навсегда.
pub(crate) struct StreamGuard {
    pub(crate) meta: Arc<Mutex<StreamMeta>>,
    pub(crate) prev: StreamMeta,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        if let Ok(mut m) = self.meta.lock() {
            *m = self.prev.clone();
        }
    }
}

