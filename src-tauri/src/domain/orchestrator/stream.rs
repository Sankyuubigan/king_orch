use super::*;
use crate::domain::agent_manager::AgentProfile;
use crate::infra::{
    extract_model_filename, push_report, ChatAttachment, ChatMessage, GrammarSpec, LlamaEngine,
    LlmMessage, ModelParams, SubCall, ToolCallInfo,
};
use std::sync::{Arc, Mutex};

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
    /// Шаг 7 (план 01.11.26): агент МОЕТ emit'ить тул-конверт (`has_tools_for_prompt`).
    /// Фронт по этому флагу режет видимый текст на первой ```json-оградке в ЛЮБОМ
    /// месте буфера (не только в начале). НЕ путать с `require_tool_call` (Шаг 1):
    /// тот строже и про guard недопустимого финала, этот — только про отображение.
    pub expect_tool_call: bool,
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
