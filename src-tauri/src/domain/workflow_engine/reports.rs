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

use std::collections::HashSet;

use crate::domain::workflow_engine::parser::NodeDef;
use crate::infra::{message_phase, ChatMessage};

/// Собирает блок инжекта для узла: ответы (и сигналы) перечисленных агентов.
///
/// `all_nodes` — ноды графа: по ним определяется тип вывода (`output_type`)
/// перечисленных агентов для гварда дублей. Стенды замеров без графа передают
/// `&[]`.
///
/// Пустая строка — если узел ничего не инжектит; вызывающий код тогда не
/// добавляет раздел в системный промпт.
///
/// # Гвард дублей
///
/// `inject_response` — канал бэкенд-отчётов (`thought`): того, чего нет в
/// истории чата. Ответы фронтенд-агентов (`output_type: frontend`, legacy
/// `message`) и так попадают в историю сессии, которая инжектится каждому
/// `llm_worker` автоматически (оркестратор), — подставлять их в блок
/// «ОТВЕТЫ КОЛЛЕГ» значит дублировать. Поэтому фронтенд-агент пропускается
/// с `log::warn!` (автор графа видит пропуск в «Логах»). Неразрешённые имена
/// (агент не объявлен ни одной нодой графа — стенды, внешние авторы) не
/// режутся.
pub fn build_injected_reports(
    node: &NodeDef,
    messages: &[ChatMessage],
    all_nodes: &[NodeDef],
) -> String {
    let Some(aids) = node.inject_response.as_ref() else {
        return String::new();
    };

    let frontend_agents: HashSet<&str> = all_nodes
        .iter()
        .filter(|n| n.publishes_to_chat())
        .filter_map(|n| n.agent.as_deref().map(str::trim))
        .filter(|a| !a.is_empty())
        .collect();

    let mut reports = String::new();
    for agent_id in aids {
        if frontend_agents.contains(agent_id.trim()) {
            log::warn!(
                "[inject_response] '{}' — фронтенд-агент (output_type: frontend): его ответы и \
                 так в истории чата, инжект пропущен (узел '{}')",
                agent_id.trim(),
                node.id
            );
            continue;
        }
        if reports.is_empty() {
            reports.push_str("### [ОТВЕТЫ КОЛЛЕГ]\n");
        }
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
            author_name: None,
        }
    }

    fn worker(inject_response: Option<Vec<String>>) -> NodeDef {
        NodeDef {
            inject_response,
            ..NodeDef::default()
        }
    }

    /// Нода графа для гварда: id, агент и тип вывода.
    fn graph_node(id: &str, agent: &str, output_type: Option<&str>) -> NodeDef {
        NodeDef {
            id: id.to_string(),
            agent: Some(agent.to_string()),
            output_type: output_type.map(ToOwned::to_owned),
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
        let reports = build_injected_reports(&node, &messages, &[]);
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
        let reports = build_injected_reports(&node, &[], &[]);
        assert!(
            reports.contains("[Ответ не найден]"),
            "пропуск слоя должен быть виден модели, а не молчать: {}",
            reports
        );
    }

    #[test]
    fn no_injection_yields_empty_block() {
        let node = worker(None);
        assert!(build_injected_reports(&node, &[], &[]).is_empty());
    }

    /// Гвард дублей: фронтенд-агент (output_type: frontend) пропускается — его
    /// ответ и так в истории чата; бэкенд-отчёт соседа инжектится как раньше.
    #[test]
    fn frontend_agent_report_is_skipped_as_history_duplicate() {
        let node = worker(Some(vec!["synth".to_string(), "validator".to_string()]));
        let graph = vec![
            graph_node("call_synth", "synth", Some("frontend")),
            graph_node("call_validator", "validator", Some("backend")),
        ];
        let messages = vec![
            message("synth", "message", 2, "ответ синтезатора в чат"),
            message("validator", "thought", 2, "вердикт валидатора"),
        ];
        let reports = build_injected_reports(&node, &messages, &graph);
        assert!(reports.contains("### [ОТВЕТЫ КОЛЛЕГ]"), "{}", reports);
        assert!(
            reports.contains("вердикт валидатора"),
            "бэкенд-отчёт обязан инжектиться: {}",
            reports
        );
        assert!(
            !reports.contains("ответ синтезатора"),
            "фронтенд-ответ продублировал историю: {}",
            reports
        );
        assert!(
            !reports.contains("[Ответ не найден]"),
            "пропуск фронтенда не должен выглядеть как потеря: {}",
            reports
        );
    }

    /// Список из одних фронтенд-агентов — блок целиком пуст: раздел
    /// «ОТВЕТЫ КОЛЛЕГ» без единого ответа модели не подставляется.
    #[test]
    fn all_frontend_inject_list_yields_empty_block() {
        let node = worker(Some(vec!["synth".to_string()]));
        let graph = vec![graph_node("call_synth", "synth", Some("frontend"))];
        let messages = vec![message("synth", "message", 2, "ответ")];
        assert!(
            build_injected_reports(&node, &messages, &graph).is_empty(),
            "фронтенд-инжект целиком — пустой блок"
        );
    }

    /// Неразрешённое имя (нет ни одной ноды с таким агентом — стенды замеров,
    /// внешние авторы) гвард не режет: инжект работает как раньше.
    #[test]
    fn unresolved_agent_is_not_cut() {
        let node = worker(Some(vec!["external_author".to_string()]));
        let messages = vec![message("external_author", "message", 2, "внешний отчёт")];
        let reports = build_injected_reports(&node, &messages, &[]);
        assert!(
            reports.contains("внешний отчёт"),
            "неразрешённый агент не обязан резаться: {}",
            reports
        );
    }
}