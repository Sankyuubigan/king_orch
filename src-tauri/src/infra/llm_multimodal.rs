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
use super::llm_types::{ChatAttachment, LlmMessage, GenerationResult};

/// Маркер вставки медиа в промпт (эквивалент mtmd_default_marker() из llama.cpp)
const MTMD_MEDIA_MARKER: &str = "<__media__>";

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
        let (mut full_prompt, actual_format) = self.build_prompt(messages, format_type, &log_cb);
        log_cb(format!("🔤 Определен формат промпта (мультимодальный): {:?}", actual_format));

        for _ in attachments.iter() {
            full_prompt.push_str(MTMD_MEDIA_MARKER);
        }

        log_cb(format!("📐 Мультимодальный промпт: {} символов, {} вложений", full_prompt.len(), attachments.len()));

        let multimodal_data: Vec<String> = attachments.iter().map(|a| a.data_base64.clone()).collect();
        let words = actual_format.get_stop_words();
        let stop_words = super::llm::merged_stop_words(&words);

        self.run_completion(
            &full_prompt,
            Some(multimodal_data),
            max_tokens,
            model_params,
            &stop_words,
            cancel_flag,
            ctx_label,
            progress_cb,
            log_cb,
        )
    }
}
