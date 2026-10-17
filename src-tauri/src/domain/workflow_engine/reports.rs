//! Инжект отчётов коллег в системный промпт узла: `inject_response`.
//!
//! # Почему отдельный модуль
//!
//! Блок «ОТВЕТЫ КОЛЛЕГ» — это ровно тот канал, через который слой иерархической
//! проверки получает результат предыдущего слоя. Стенд замеров обязан подавать
//! модели **тот же** текст, что подаёт боевая нода, иначе измеряется не боевой
//! промпт (в этом проекте такое уже стоило замера: стенд правил критерии своей
//! редакцией — см. `verdict_contract::build_verdict_prompt`).
//!
//! Поэтому сборка живёт здесь одна, а её вызывают и `execute_node`, и стенды.
//!
//! Формат блока не менялся: заголовки разделов, подпись автора и правило выбора
//! сообщения (`signal` важнее текста) — поведение, зависящее от `messages[]`,
//! поэтому подменять его в стенде нельзя.

use crate::domain::workflow_engine::parser::NodeDef;
use crate::infra::{message_phase, ChatMessage};

/// Собирает блок инжекта для узла: ответы (и сигналы) перечисленных агентов.
///
/// Пустая строка — если узел ничего не инжектит; вызывающий код тогда не
/// добавляет раздел в системный промпт.
pub fn build_injected_reports(node: &NodeDef, messages: &[ChatMessage]) -> String {
    let mut reports = String::new();

    if let Some(aids) = node.inject_response.as_ref() {
        if !aids.is_empty() {
            reports.push_str("### [ОТВЕТЫ КОЛЛЕГ]\n");
            for agent_id in aids {
                let report = messages
                    .iter()
                    .rev()
                    .find(|m| m.author.as_deref() == Some(agent_id.as_str()) && m.msg_type == "signal")
                    .map(|m| m.content.clone())
                    .or_else(|| {
                        messages
                            .iter()
                            .rev()
                            .find(|m| {
                                m.author.as_deref() == Some(agent_id.as_str())
                                    && message_phase(m) != Some(1)
                                    && (m.msg_type == "message" || m.msg_type == "thought")
                            })
                            .map(|m| m.content.clone())
                    })
                    .unwrap_or_else(|| "[Ответ не найден]".to_string());
                reports.push_str(&format!("--- Ответ от {} ---\n{}\n\n", agent_id, report));
            }
        }
    }

    reports
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow_engine::parser::NodeDef;

    fn message(author: &str, msg_type: &str, phase: u8, content: &str) -> ChatMessage {
        ChatMessage {
            id: Some(format!("msg_{}", msg_type)),
            msg_type: msg_type.to_string(),
            content: content.to_string(),
            sub_calls: None,
            author: Some(author.to_string()),
            model: None,
            time_sec: None,
            attachments: None,
            phase: Some(phase),
        }
    }

    fn worker(inject_response: Option<Vec<String>>) -> NodeDef {
        NodeDef {
            inject_response,
            ..NodeDef::default()
        }
    }

    #[test]
    fn response_injection_prefers_signal_over_text() {
        let node = worker(Some(vec!["validator_surface".to_string()]));
        // Порядок сообщений: signal ПОСЛЕ thought — signal должен выиграть.
        let messages = vec![
            message("validator_surface", "thought", 2, "текстовый разбор элемента"),
            message(
                "validator_surface",
                "signal",
                2,
                r#"{"validator_surface": {"e1": true}}"#,
            ),
        ];
        let reports = build_injected_reports(&node, &messages);
        assert!(reports.contains("### [ОТВЕТЫ КОЛЛЕГ]"), "{}", reports);
        assert!(reports.contains("validator_surface"), "{}", reports);
        assert!(reports.contains("e1"), "в инжект попал не сигнал: {}", reports);
        assert!(
            !reports.contains("текстовый разбор"),
            "текстовый ответ предпочтён сигналу: {}",
            reports
        );
    }

    #[test]
    fn missing_report_is_declared_not_skipped() {
        let node = worker(Some(vec!["validator_facade".to_string()]));
        let reports = build_injected_reports(&node, &[]);
        assert!(
            reports.contains("[Ответ не найден]"),
            "пропуск слоя должен быть виден модели, а не молчать: {}",
            reports
        );
    }

    #[test]
    fn no_injection_yields_empty_block() {
        let node = worker(None);
        assert!(build_injected_reports(&node, &[]).is_empty());
    }
}