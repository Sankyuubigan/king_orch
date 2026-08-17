//! Мультимодальная генерация (изображения через mmproj llama-server).
//!
//! Движок запускается с `--mmproj` (см. LlamaEngine::new_with_mmproj), а
//! изображения передаются в `/completion` как base64-массив `multimodal_data`
//! вместе с маркерами `<__media__>` в тексте промпта (тот же механизм, что и
//! `mtmd_default_marker()` в llama.cpp).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::infra::config::ModelParams;
use super::llm::LlamaEngine;
use super::llm_types::{ChatAttachment, LlmMessage, GenerationResult, GrammarSpec, build_base_grammar};

/// Маркер вставки медиа в промпт. Сервер llama.cpp с версии b10456 использует
/// СЛУЧАЙНЫЙ маркер при каждом старте (см. get_media_marker() в llama.cpp);
/// фактическое значение движок читает из GET /props (см. LlamaEngine::media_marker).
/// Эта константа — фолбэк по умолчанию для старых версий сервера.
pub(crate) const MTMD_MEDIA_MARKER: &str = "<__media__>";

impl LlamaEngine {
    pub fn generate_chat_multimodal<F, L>(
        &self,
        messages: &[LlmMessage],
        attachments: &[ChatAttachment],
        max_tokens: usize,
        model_params: &ModelParams,
        format_type: &str,
        cancel_flag: Arc<AtomicBool>,
        ctx_label: &str,
        progress_cb: F,
        log_cb: L,
    ) -> Result<GenerationResult, String>
    where F: FnMut(f32, &str), L: Fn(String) {
        // Маркер(ы) медиа вставляем ВНУТРЬ последнего user-сообщения (перед текстом),
        // а не в конец промпта: изображение должно оказаться в user-повороте
        // (`<|turn>user\n...<turn|>`), а не после `<|turn>model\n` — иначе Gemma-4
        // получает картинку в генерационной позиции и не может её обработать.
        let mut msgs: Vec<LlmMessage> = messages.to_vec();
        let media_marker = self.media_marker();
        if !attachments.is_empty() {
            let markers = media_marker.repeat(attachments.len());
            match msgs.iter_mut().rev().find(|m| m.role == "user") {
                Some(last_user) => last_user.content = format!("{}\n{}", markers, last_user.content),
                None => msgs.push(LlmMessage { role: "user".to_string(), content: markers }),
            }
        }

        let (full_prompt, actual_format) = self.build_prompt(&msgs, format_type, &log_cb);
        log_cb(format!("🔤 Определен формат промпта (мультимодальный): {:?}", actual_format));

        log_cb(format!("📐 Мультимодальный промпт: {} символов, {} вложений (маркер в user-повороте)", full_prompt.len(), attachments.len()));

        let multimodal_data: Vec<String> = attachments.iter().map(|a| a.data_base64.clone()).collect();
        let words = actual_format.get_stop_words();
        let stop_words = super::llm::merged_stop_words(&words);

        let pending = self.take_pending_grammar();
        let grammar = pending.or_else(|| build_base_grammar(&actual_format).map(|gbnf| GrammarSpec { gbnf: Some(gbnf), json_schema: None }));

        self.run_completion(
            &full_prompt,
            Some(multimodal_data),
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
