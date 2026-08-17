use super::*;
use std::path::Path;
use std::fs;
use std::io::Write;
use serde_json::Value;
use crate::infra::{ChatMessage, LlmMessage, SubCall, ToolCallInfo, ModelParams, ChatAttachment, LlamaEngine, GrammarSpec, extract_model_filename, push_report};
use crate::domain::agent_manager::AgentProfile;

/// Единый pipeline компакции контекста перед генерацией. Работает ТОЛЬКО с
/// не-system сообщениями (system-промпт сохраняется целиком). Стратегии по
/// возрастанию агрессивности:
///   1) сворачиваем крупные (ещё не spilled) результаты инструментов в указатель;
///   2) сжимаем самые старые сообщения в одну выжимку (head-эксцерпты);
///   3) жёстко удаляем самые старые, пока не влезем в бюджет (fallback).
/// Бюджет в символах считается вызывающим (global_ctx_limit − max_gen_tokens)·2
/// (консервативно: для кириллицы 2 символа/токен — компактим раньше, чем нужно).
/// Отчёт о проделанной компакции — чтобы вызывающий ОБЯЗАТЕЛЬНО записал в лог,
/// что из контекста было удалено/свёрнуто (правило: тихих операций с данными нет).
pub(crate) struct CompactionReport {
    pub tool_results_pruned: usize,
    pub history_compressed: bool,
    pub old_messages_dropped: usize,
}

pub(crate) fn compact_llm_messages(messages: &mut Vec<LlmMessage>, budget_chars: usize) -> CompactionReport {
    let mut report = CompactionReport {
        tool_results_pruned: 0,
        history_compressed: false,
        old_messages_dropped: 0,
    };
    if messages.len() <= 1 {
        return report;
    }
    let total_chars = |msgs: &[LlmMessage]| -> usize {
        msgs[1..].iter().map(|m| m.content.chars().count()).sum()
    };

    if total_chars(messages) <= budget_chars {
        return report;
    }

    // Стратегия 1: сворачиваем крупные результаты инструментов (кроме уже
    // spilled — у них важен локатор пути для read_spill).
    for m in messages.iter_mut().skip(1) {
        if m.content.contains("[РЕЗУЛЬТАТ ИНСТРУМЕНТА")
            && !m.content.contains("сохранён в файл spills")
            && m.content.chars().count() > 1500
        {
            let tool = m
                .content
                .lines()
                .next()
                .map(|l| {
                    l.trim_start_matches("[РЕЗУЛЬТАТ ИНСТРУМЕНТА ")
                        .trim_end_matches("]:")
                        .trim()
                        .to_string()
                })
                .unwrap_or_else(|| "инструмент".to_string());
            m.content = format!(
                "[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}] — крупный результат свёрнут для экономии контекста (полный текст в истории сессии).]",
                tool
            );
            report.tool_results_pruned += 1;
        }
    }

    if total_chars(messages) <= budget_chars {
        return report;
    }

    // Стратегия 2: сжимаем самые старые сообщения в одну выжимку, сохраняя
    // system-промпт и `keep_recent` последних сообщений.
    let keep_recent = 4usize;
    let n = messages.len();
    if n > keep_recent + 2 {
        let compress_end = n - keep_recent;
        let mut summary = String::from("[СЖАТАЯ ИСТОРИЯ]\n");
        for m in messages.iter().take(compress_end).skip(1) {
            let excerpt: String = m.content.chars().take(200).collect();
            summary.push_str(&format!("({}) {}…\n", m.role, excerpt));
        }
        let dropped = compress_end - 1; // сколько старых сообщений ушло в выжимку
        messages.drain(1..compress_end);
        messages.insert(
            1,
            LlmMessage { role: "system".to_string(), content: summary },
        );
        report.old_messages_dropped += dropped;
        report.history_compressed = true;
    }

    // Стратегия 3: жёстко удаляем самые старые, пока не влезем.
    while messages.len() > 2 {
        if total_chars(messages) <= budget_chars {
            break;
        }
        messages.remove(1);
        report.old_messages_dropped += 1;
    }

    report
}

