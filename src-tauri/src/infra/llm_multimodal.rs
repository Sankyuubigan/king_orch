//! Мультимодальная генерация (изображения через mmproj llama-server).
//!
//! Движок запускается с `--mmproj` (см. LlamaEngine::new_with_mmproj), а
//! изображения передаются в `/v1/chat/completions` как image-части последнего
//! user-сообщения (OAI-совместимый формат content parts, см.
//! LlamaEngine::run_chat_completions). Промпт рендерит сам движок по GGUF
//! chat_template — маркеры вставки медиа в текст промпта не нужны.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::infra::config::ModelParams;
use super::llm::LlamaEngine;
use super::llm_types::{ChatAttachment, LlmMessage, GenerationResult, GrammarSpec, PromptFormat, build_base_grammar};

impl LlamaEngine {
    pub fn generate_chat_multimodal<F, L>(
        &self,
        messages: &[LlmMessage],
        attachments: &[ChatAttachment],
        max_tokens: usize,
        model_params: &ModelParams,
        _format_type: &str,
        cancel_flag: Arc<AtomicBool>,
        ctx_label: &str,
        progress_cb: F,
        log_cb: L,
    ) -> Result<GenerationResult, String>
    where F: FnMut(f32, &str), L: Fn(String) {
        let actual_format = PromptFormat::detect_from_gguf(&self.model_path);
        log_cb(format!(
            "🎯 Рендер промпта (мультимодальный): chat template из GGUF (движок llama.cpp, jinja), формат {:?}",
            actual_format
        ));
        log_cb(format!(
            "📐 Мультимодальный запрос: {} сообщений, {} вложений (image-части в user-сообщении)",
            messages.len(),
            attachments.len()
        ));

        let words = actual_format.get_stop_words();
        let stop_words = super::llm::merged_stop_words(&words);

        let pending = self.take_pending_grammar();
        let grammar = pending.or_else(|| build_base_grammar(&actual_format).map(|gbnf| GrammarSpec { gbnf: Some(gbnf), json_schema: None }));

        self.run_chat_completions(
            messages,
            Some(attachments),
            max_tokens,
            model_params,
            &stop_words,
            grammar,
            cancel_flag,
            ctx_label,
            progress_cb,
            log_cb,
        )
    }
}