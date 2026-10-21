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
//! Слоты доставки (`system_prompt_delivery`):
//! - `system` — отдельное system-сообщение (дефолт);
//! - `user_prefix` — user-сообщение на той же позиции (доказанная на
//!   gemini-3.8-flash работающая форма).
//!
//! Источники фрейма (SSOT merge — `effective_frame`):
//! - граф-уровень: `config.default_system_prompt_file` +
//!   `config.default_system_prompt_delivery` — объявляются ОДИН раз на граф,
//!   применяются ко всем нодам `llm_worker`;
//! - нода: `system_prompt_file` + `system_prompt_delivery` — переопределяет
//!   дефолт ПО ПОЛЮ (нода может сменить только слот доставки у дефолтного файла).
//!
//! Резолв пути повторяет прецедент `config.facts_file`
//! (`fact_extractor::load_facts_file`): путь относительно папки YAML-графа.
//! Отсутствующий или пустой файл — ЯВНАЯ ошибка (как сломанный facts.yaml
//! сегодня): бизнес-логика не должна тихо исчезать, молчаливый пропуск
//! выглядел бы как «промпта нет и не нужен».

use std::fs;
use std::path::Path;

use super::parser::{NodeDef, WorkflowConfig};
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

/// Эффективная пара «файл + сырой слот доставки» ноды с учётом дефолта графа.
///
/// Объединение ПО ПОЛЮ: нодовые поля переопределяют дефолт независимо
/// (нода вправе сменить только слот доставки у дефолтного файла). Пустая
/// строка считается «не задано» — та же семантика, что и у старого
/// `resolve_node_frame` (trim + filter: пустое поле = поля нет).
///
/// Функция ЧИСТАЯ: файл не читает, значения слотов не валидирует. Валидация
/// слота — в единственном месте `FrameDelivery::parse`, куда сходят и рантайм
/// (`resolve_node_frame`), и редакторская диагностика (`editor_fidelity`).
/// Дефолт применяется ТОЛЬКО к нодам `llm_worker` — контракт вызывает эту
/// функцию только в ветке `LlmWorker` (`nodes.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveFrameConfig {
    pub file: Option<String>,
    pub delivery_raw: Option<String>,
}

fn trim_non_empty(raw: Option<&String>) -> Option<String> {
    raw.as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

pub fn effective_frame(
    node: &NodeDef,
    config: Option<&WorkflowConfig>,
) -> EffectiveFrameConfig {
    let (default_file, default_delivery) = match config {
        Some(c) => (
            trim_non_empty(c.default_system_prompt_file.as_ref()),
            trim_non_empty(c.default_system_prompt_delivery.as_ref()),
        ),
        None => (None, None),
    };
    EffectiveFrameConfig {
        file: trim_non_empty(node.system_prompt_file.as_ref()).or(default_file),
        delivery_raw: trim_non_empty(node.system_prompt_delivery.as_ref()).or(default_delivery),
    }
}

/// Прочитать фрейм ноды из файла (`NodeDef.system_prompt_file`; при отсутствии
/// нодового поля — графовый дефолт `config.default_system_prompt_file`).
///
/// `Ok(None)` — фрейма нет ни на ноде, ни в дефолте графа (нода без фрейма,
/// поведение идентично прошлому). Отсутствующий или пустой файл — Err (плюс
/// `log::error!` с полным путём — паттерн `load_facts_file`): дефолт,
/// указывающий на сломанный файл, — та же честная ошибка, что и нодовый
/// (никаких тихих «фрейма нет»).
pub fn resolve_node_frame(
    node: &NodeDef,
    workflow_dir: Option<&Path>,
    config: Option<&WorkflowConfig>,
) -> Result<Option<AgentFrame>, String> {
    let effective = effective_frame(node, config);
    let Some(raw_path) = effective.file else {
        return Ok(None);
    };
    let delivery = FrameDelivery::parse(effective.delivery_raw.as_deref())?;
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

    fn config_with(file: Option<&str>, delivery: Option<&str>) -> WorkflowConfig {
        WorkflowConfig {
            default_system_prompt_file: file.map(|s| s.to_string()),
            default_system_prompt_delivery: delivery.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    /// Каталог для фикстур — УНИКАЛЬНЫЙ на вызов.
    ///
    /// Раньше здесь был каталог, общий для всех тестов модуля, и каждый вызов
    /// его удалял: при параллельном выполнении тесты затирали каталог друг
    /// друга, и прогоны падали с «Отказано в доступе» / «файла нет» — то есть
    /// гонка выглядела как дефект логики. Имя с наносекундной меткой делает
    /// каталоги независимыми.
    fn temp_dir() -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "agent_frame_tests_{}_{nanos}",
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
        assert!(resolve_node_frame(&node, Some(Path::new(".")), None)
            .expect("нет файла — нет фрейма")
            .is_none());
    }

    #[test]
    fn missing_file_is_explicit_error() {
        let dir = temp_dir();
        let node = node_with(Some("missing.md"), None);
        let err = resolve_node_frame(&node, Some(&dir), None).expect_err("файла нет");
        assert!(err.contains("не удалось прочитать"), "got: {}", err);
    }

    #[test]
    fn empty_file_is_explicit_error() {
        let dir = temp_dir();
        fs::write(dir.join("empty.md"), "   \n  ").expect("записать пустой файл");
        let node = node_with(Some("empty.md"), None);
        let err = resolve_node_frame(&node, Some(&dir), None).expect_err("файл пуст");
        assert!(err.contains("файл пуст"), "got: {}", err);
    }

    #[test]
    fn resolves_relative_path_with_trimmed_content() {
        let dir = temp_dir();
        fs::write(dir.join("frame.md"), "\nТЕКСТ РАМКИ\n").expect("записать файл");
        let node = node_with(Some("frame.md"), Some("user_prefix"));
        let frame = resolve_node_frame(&node, Some(&dir), None)
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
        let frame = resolve_node_frame(&node, Some(&dir), None)
            .expect("ок")
            .expect("фрейм есть");
        assert_eq!(frame.delivery, FrameDelivery::System);
    }

    #[test]
    fn unknown_delivery_blocks_resolution() {
        let dir = temp_dir();
        fs::write(dir.join("frame.md"), "X").expect("записать файл");
        let node = node_with(Some("frame.md"), Some("в воздухе"));
        let err = resolve_node_frame(&node, Some(&dir), None).expect_err("неизвестный слот");
        assert!(err.contains("неизвестное значение"), "got: {}", err);
    }

    // --- Дефолт графа (`config.default_system_prompt_*`) ---

    #[test]
    fn workflow_default_frame_resolves_when_node_empty() {
        let dir = temp_dir();
        fs::write(dir.join("frame.md"), "ДЕФОЛТ").expect("записать файл");
        let node = node_with(None, None);
        let config = config_with(Some("frame.md"), Some("user_prefix"));
        let frame = resolve_node_frame(&node, Some(&dir), Some(&config))
            .expect("ок")
            .expect("фрейм из дефолта");
        assert_eq!(frame.content, "ДЕФОЛТ");
        assert_eq!(frame.delivery, FrameDelivery::UserPrefix);
    }

    #[test]
    fn node_file_overrides_workflow_default() {
        let dir = temp_dir();
        fs::write(dir.join("own.md"), "СВОЯ").expect("записать файл");
        let node = node_with(Some("own.md"), None);
        let config = config_with(Some("missing.md"), None);
        let frame = resolve_node_frame(&node, Some(&dir), Some(&config))
            .expect("нода переопределяет дефолт")
            .expect("фрейм есть");
        assert_eq!(frame.content, "СВОЯ");
    }

    #[test]
    fn node_delivery_overrides_workflow_default_delivery() {
        let dir = temp_dir();
        fs::write(dir.join("frame.md"), "X").expect("записать файл");
        // Нода меняет ТОЛЬКО слот: file наследуется из дефолта графа (merge по полю).
        let node = node_with(None, Some("system"));
        let config = config_with(Some("frame.md"), Some("user_prefix"));
        let frame = resolve_node_frame(&node, Some(&dir), Some(&config))
            .expect("ок")
            .expect("фрейм есть");
        assert_eq!(frame.content, "X");
        assert_eq!(frame.delivery, FrameDelivery::System);
    }

    #[test]
    fn workflow_default_missing_file_is_explicit_error() {
        let dir = temp_dir();
        let node = node_with(None, None);
        let config = config_with(Some("missing.md"), None);
        let err = resolve_node_frame(&node, Some(&dir), Some(&config)).expect_err("сломанный дефолт");
        assert!(err.contains("не удалось прочитать"), "got: {}", err);
        assert!(err.contains("missing.md"), "got: {}", err);
    }

    #[test]
    fn empty_default_file_falls_back_to_no_frame() {
        // Пустая строка в дефолте = «не задано» (та же семантика, что у ноды):
        // не ошибка, а отсутствие фрейма.
        let node = node_with(None, None);
        let config = config_with(Some("   "), None);
        assert!(resolve_node_frame(&node, Some(Path::new(".")), Some(&config))
            .expect("пустой дефолт = не задано")
            .is_none());
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
