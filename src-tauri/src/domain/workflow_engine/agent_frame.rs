//! Фрейм агента: опциональный ДОПОЛНИТЕЛЬНЫЙ системный промпт ноды,
//! заданный markdown-файлом бизнес-логики рядом с агентом
//! (`NodeDef.system_prompt_file`).
//!
//! Это GENERIC-механика движка: содержимое файла не знает ничего о целях
//! приложения и не хардкодится в Rust. Место вставки сообщения — сразу после
//! системного сообщения агента и ПЕРЕД всей инжектируемой историей переписки.
//! Позиция «до контента» — часть контракта: измерено на gemini-3.8-flash
//! (20.10.26, `tasks/20.10.26 Пустой успех облака - системный промпт агента.md`
//! §1.2/§4) — тот же текст, добавленный внутрь system-текста или ПОСЛЕ
//! контента, отказ классификации не снимает; отдельное сообщение в начале
//! запроса — снимает.
//!
//! Слоты доставки (`NodeDef.system_prompt_delivery`):
//! - `system` — отдельное system-сообщение (дефолт);
//! - `user_prefix` — user-сообщение на той же позиции (доказанная на
//!   gemini-3.8-flash работающая форма).
//!
//! Резолв пути повторяет прецедент `config.facts_file`
//! (`fact_extractor::load_facts_file`): путь относительно папки YAML-графа.
//! Отсутствующий или пустой файл — ЯВНАЯ ошибка (как сломанный facts.yaml
//! сегодня): бизнес-логика не должна тихо исчезать, молчаливый пропуск
//! выглядел бы как «промпта нет и не нужен».

use std::fs;
use std::path::Path;

use super::parser::NodeDef;
use crate::infra::LlmMessage;

/// Слот доставки фрейма в список сообщений LLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDelivery {
    /// Отдельное system-сообщение после главного системного промпта агента.
    System,
    /// user-сообщение в той же позиции (zip: до всей истории переписки).
    UserPrefix,
}

impl FrameDelivery {
    /// Разбор значения `system_prompt_delivery`. `Ok(None)` — поле не задано
    /// (дефолт контракта применяется вызывающей стороной).
    ///
    /// SSOT значений: и редакторская диагностика (`editor_fidelity`), и
    /// рантайм валидируют через этот же parse.
    pub fn parse(raw: Option<&str>) -> Result<Option<FrameDelivery>, String> {
        let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        match raw {
            "system" => Ok(Some(FrameDelivery::System)),
            "user_prefix" => Ok(Some(FrameDelivery::UserPrefix)),
            other => Err(format!(
                "system_prompt_delivery: неизвестное значение '{}' (допустимо: system | user_prefix)",
                other
            )),
        }
    }

    /// Роль сообщения LLM для вставки.
    pub fn role(&self) -> &'static str {
        match self {
            FrameDelivery::System => "system",
            FrameDelivery::UserPrefix => "user",
        }
    }

    /// Строковое значение слота (для логов).
    pub fn as_str(&self) -> &'static str {
        match self {
            FrameDelivery::System => "system",
            FrameDelivery::UserPrefix => "user_prefix",
        }
    }
}

/// Разрешённый фрейм ноды: готовый текст + слот доставки.
#[derive(Debug, Clone)]
pub struct AgentFrame {
    pub content: String,
    pub delivery: FrameDelivery,
}

/// Прочитать фрейм ноды из файла (`NodeDef.system_prompt_file`).
///
/// `Ok(None)` — поле не задано (нода без фрейма, поведение идентично прошлому).
/// Отсутствующий или пустой файл — Err (плюс `log::error!` с полным путём —
/// паттерн `load_facts_file`).
pub fn resolve_node_frame(
    node: &NodeDef,
    workflow_dir: Option<&Path>,
) -> Result<Option<AgentFrame>, String> {
    let Some(raw_path) = node
        .system_prompt_file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    let delivery = FrameDelivery::parse(node.system_prompt_delivery.as_deref())?;
    let full_path = match workflow_dir {
        Some(dir) => dir.join(raw_path),
        None => {
            return Err(format!(
                "system_prompt_file: '{}' задан, но папка графа неизвестна",
                raw_path
            ))
        }
    };
    let content = fs::read_to_string(&full_path).map_err(|e| {
        log::error!(
            "[agent_frame] Не удалось прочитать системный промпт ноды {}: {}",
            full_path.display(),
            e
        );
        format!(
            "system_prompt_file: не удалось прочитать {}: {}",
            full_path.display(),
            e
        )
    })?;
    let content = content.trim();
    if content.is_empty() {
        log::error!(
            "[agent_frame] Файл системного промпта пуст: {}",
            full_path.display()
        );
        return Err(format!("system_prompt_file: файл пуст — {}", full_path.display()));
    }
    Ok(Some(AgentFrame {
        content: content.to_string(),
        delivery: delivery.unwrap_or(FrameDelivery::System),
    }))
}

/// Вставить фрейм в список сообщений LLM НА ПОЗИЦИЮ 1: сразу после
/// системного сообщения агента, ПЕРЕД всей историей переписки.
///
/// Позиция — часть контракта (см. док-строку модуля): рамка ПОСЛЕ контента
/// отказ классификации не снимает. `None` — нода без фрейма: список
/// сообщений остаётся ИДЕНТИЧНЫМ прошлому поведению (регресс исключён).
pub fn apply_frame_to_llm_messages<F>(llm_messages: &mut Vec<LlmMessage>, frame: Option<&AgentFrame>, log_cb: F)
where
    F: Fn(String),
{
    let Some(frame) = frame else {
        return;
    };
    llm_messages.insert(
        1,
        LlmMessage {
            role: frame.delivery.role().to_string(),
            content: frame.content.clone(),
            ..Default::default()
        },
    );
    log_cb(format!(
        "[frame] Добавлен промпт {} символов (role={}, файл-рамка, delivery={})",
        frame.content.chars().count(),
        frame.delivery.role(),
        frame.delivery.as_str()
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_with(file: Option<&str>, delivery: Option<&str>) -> NodeDef {
        NodeDef {
            system_prompt_file: file.map(|s| s.to_string()),
            system_prompt_delivery: delivery.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agent_frame_tests_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("создать tmp-каталог");
        dir
    }

    #[test]
    fn parses_known_delivery_values() {
        assert_eq!(FrameDelivery::parse(None), Ok(None));
        assert_eq!(FrameDelivery::parse(Some("")), Ok(None));
        assert_eq!(FrameDelivery::parse(Some("  ")), Ok(None));
        assert_eq!(
            FrameDelivery::parse(Some("system")),
            Ok(Some(FrameDelivery::System))
        );
        assert_eq!(
            FrameDelivery::parse(Some("user_prefix")),
            Ok(Some(FrameDelivery::UserPrefix))
        );
        assert!(FrameDelivery::parse(Some("чат")).is_err());
    }

    #[test]
    fn no_field_means_no_frame() {
        let node = node_with(None, None);
        assert!(resolve_node_frame(&node, Some(Path::new(".")))
            .expect("нет файла — нет фрейма")
            .is_none());
    }

    #[test]
    fn missing_file_is_explicit_error() {
        let dir = temp_dir();
        let node = node_with(Some("missing.md"), None);
        let err = resolve_node_frame(&node, Some(&dir)).expect_err("файла нет");
        assert!(err.contains("не удалось прочитать"), "got: {}", err);
    }

    #[test]
    fn empty_file_is_explicit_error() {
        let dir = temp_dir();
        fs::write(dir.join("empty.md"), "   \n  ").expect("записать пустой файл");
        let node = node_with(Some("empty.md"), None);
        let err = resolve_node_frame(&node, Some(&dir)).expect_err("файл пуст");
        assert!(err.contains("файл пуст"), "got: {}", err);
    }

    #[test]
    fn resolves_relative_path_with_trimmed_content() {
        let dir = temp_dir();
        fs::write(dir.join("frame.md"), "\nТЕКСТ РАМКИ\n").expect("записать файл");
        let node = node_with(Some("frame.md"), Some("user_prefix"));
        let frame = resolve_node_frame(&node, Some(&dir))
            .expect("файл существует")
            .expect("фрейм есть");
        assert_eq!(frame.content, "ТЕКСТ РАМКИ");
        assert_eq!(frame.delivery, FrameDelivery::UserPrefix);
    }

    #[test]
    fn default_delivery_is_system() {
        let dir = temp_dir();
        fs::write(dir.join("frame.md"), "X").expect("записать файл");
        let node = node_with(Some("frame.md"), None);
        let frame = resolve_node_frame(&node, Some(&dir))
            .expect("ок")
            .expect("фрейм есть");
        assert_eq!(frame.delivery, FrameDelivery::System);
    }

    #[test]
    fn unknown_delivery_blocks_resolution() {
        let dir = temp_dir();
        fs::write(dir.join("frame.md"), "X").expect("записать файл");
        let node = node_with(Some("frame.md"), Some("в воздухе"));
        let err = resolve_node_frame(&node, Some(&dir)).expect_err("неизвестный слот");
        assert!(err.contains("неизвестное значение"), "got: {}", err);
    }

    fn msgs() -> Vec<LlmMessage> {
        vec![
            LlmMessage {
                role: "system".to_string(),
                content: "агент".to_string(),
                ..Default::default()
            },
            LlmMessage {
                role: "user".to_string(),
                content: "история 1".to_string(),
                ..Default::default()
            },
        ]
    }

    fn roles(messages: &[LlmMessage]) -> Vec<&str> {
        messages.iter().map(|m| m.role.as_str()).collect()
    }

    #[test]
    fn frame_without_frame_is_noop() {
        let mut messages = msgs();
        apply_frame_to_llm_messages(&mut messages, None, |_| {});
        assert_eq!(roles(&messages), vec!["system", "user"]);
    }

    #[test]
    fn system_frame_stands_after_system_before_history() {
        let mut messages = msgs();
        let frame = AgentFrame {
            content: "рамка".to_string(),
            delivery: FrameDelivery::System,
        };
        apply_frame_to_llm_messages(&mut messages, Some(&frame), |_| {});
        assert_eq!(roles(&messages), vec!["system", "system", "user"]);
        assert_eq!(messages[1].content, "рамка");
    }

    #[test]
    fn user_prefix_frame_stands_after_system_before_history() {
        let mut messages = msgs();
        let frame = AgentFrame {
            content: "рамка".to_string(),
            delivery: FrameDelivery::UserPrefix,
        };
        apply_frame_to_llm_messages(&mut messages, Some(&frame), |_| {});
        assert_eq!(roles(&messages), vec!["system", "user", "user"]);
        assert_eq!(messages[1].content, "рамка");
    }
}
