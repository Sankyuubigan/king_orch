use super::*;
use std::path::Path;
use std::fs;
use std::io::Write;
use serde_json::Value;
use crate::infra::{ChatMessage, LlmMessage, SubCall, ToolCallInfo, ModelParams, ChatAttachment, LlamaEngine, GrammarSpec, extract_model_filename, push_report};
use crate::domain::agent_manager::AgentProfile;
use crate::domain::parsers::{clean_thought_tags, split_thinking_and_answer};

/// Хвост строки длиной ≤ `n` символов (без разрыва UTF-8).
pub(crate) fn tail_chars(s: &str, n: usize) -> String {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    if chars.len() <= n {
        return s.to_string();
    }
    let idx = chars[chars.len() - n].0;
    s[idx..].to_string()
}

/// Извлекает финальный ответ из накопленного текста (размышления вырезаны).
pub(crate) fn extract_answer_from_combined(combined: &str, fallback: &str) -> String {
    let (_, answer) = split_thinking_and_answer(combined);
    let cleaned = clean_thought_tags(&answer);
    if !cleaned.trim().is_empty() {
        return cleaned;
    }
    // Нет распознанных маркеров размышлений → обычная чистка всего текста
    let full = clean_thought_tags(combined);
    if !full.trim().is_empty() { full } else { fallback.to_string() }
}

/// Артефакт докачки: финальный ответ начинается с многоточия — модель
/// продолжила оборванный думатель вместо того, чтобы начать ответ с начала.
pub(crate) fn starts_with_ellipsis(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with("...") || t.starts_with('…')
}

pub(crate) fn safe_truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len { return s.to_string(); }
    let end = s.char_indices()
        .take_while(|(i, _)| *i < max_len)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(max_len.min(s.len()));
    format!("{}...", &s[..end])
}

pub(crate) fn truncate_result(text: &str, max_len: usize) -> String {
    if text.len() <= max_len { text.to_string() }
    else {
        let cut = text.char_indices().take_while(|(i, _)| *i < max_len).last()
            .map(|(i, c)| i + c.len_utf8()).unwrap_or(max_len.min(text.len()));
        format!("{}...\n(обрезано)", &text[..cut])
    }
}

pub(crate) fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

