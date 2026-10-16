//! Единый контракт выхода шага графа (`StepOutput`).
//!
//! Рантайму не должно быть дела, кто отработал: одиночная нода в Detailed или
//! склеенная макро-нода группы в Fused. До этого шага запись в signal bus шла из
//! ЧЕТЫРЁХ мест (`nodes.rs` — факт-экстрактор, `system1_validator`, сигнал
//! агента; `mod.rs` — склеенный вызов), и «кто последний записал ключ» зависел
//! от того, какой путь выполнился. Один и тот же факт мог быть записан дважды
//! разными исполнителями, и расхождение выглядело как «модель ошиблась».
//!
//! Теперь исполнитель шага **объявляет** свой выход в `StepOutput`, а единственное
//! место, которое пишет в `context.signals`, — [`handle_step_output`].
//!
//! Почему выход шага и `NodeResult` — разные типы, а не один:
//! `NodeResult` отвечает на вопрос ПЛАНИРОВЩИКА («куда идти дальше» и «что лежит
//! в `node_outputs` для шаблонов»), `StepOutput` — на вопрос СИГНАЛЬНОЙ ШИНЫ
//! («что шаг произвёл»). Смешали бы их — и правку маршрутизации пришлось бы
//! трогать каждый раз, когда меняется состав сигналов.

use std::collections::HashMap;

use super::context::WorkflowContext;

/// Что породил один шаг графа (одна нода ИЛИ одна склеенная группа нод).
///
/// Форма одинакова для обоих режимов — это и есть смысл типа: и Detailed-, и
/// Fused-путь отдают в шину один и тот же контракт, и применяет его одна и та же
/// функция.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepOutput {
    /// Текст, который шаг отдал пользователю.
    ///
    /// УЖЕ отправлен в чат тем исполнителем, который его произвёл:
    /// `run_agent_node` (автор — id агента) или `run_fused_group` (автор —
    /// `fused_agent` ?? id группы). Повторная отправка из `handle_step_output`
    /// продублировала бы каждое сообщение агента, поэтому поле хранит факт
    /// («шаг что-то сказал»), а не команду «скажи это ещё раз».
    pub message: Option<String>,
    /// Сигналы, которые шаг произвёл. Единственный источник пополнения шины.
    pub signals: HashMap<String, serde_json::Value>,
}

impl StepOutput {
    /// Выход пуст: шаг не произвёл ни сигналов, ни текста.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Сигналы из JSON-объекта ответа модели; `thought_process` отбрасывается —
    /// это размышление, а не значение для роутеров.
    pub fn from_model_object(obj: &serde_json::Value) -> Self {
        let mut signals = HashMap::new();
        if let Some(map) = obj.as_object() {
            for (key, value) in map {
                if key == "thought_process" {
                    continue;
                }
                signals.insert(key.clone(), value.clone());
            }
        }
        Self {
            message: None,
            signals,
        }
    }

    /// Есть ли что публиковать в шину.
    pub fn has_signals(&self) -> bool {
        !self.signals.is_empty()
    }
}

/// Единственная дверь записи в signal bus хода.
///
/// Вызывается планировщиком ОДИН раз на шаг — и для одиночной ноды, и для
/// склеенной макро-ноды группы. Возвращает ключи, значение которых изменилось
/// относительно уже записанного: молча перетирать чужой сигнал нельзя (§2.2),
/// поэтому расхождение уходит в лог (вкладка «Логи»), а не теряется.
pub fn handle_step_output(
    step: &StepOutput,
    source: &str,
    context: &mut WorkflowContext,
) -> Vec<String> {
    let mut conflicts = Vec::new();
    for (key, value) in &step.signals {
        if let Some(previous) = context.signals.get(key) {
            if previous != value {
                conflicts.push(format!("{}: {} → {}", key, previous, value));
            }
        }
        context.signals.insert(key.clone(), value.clone());
    }
    if !conflicts.is_empty() {
        log::warn!(
            "[workflow] шаг '{}' переопределил сигналы, уже записанные в этом ходу: {}",
            source,
            conflicts.join("; ")
        );
    }
    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> WorkflowContext {
        WorkflowContext::new("вопрос".to_string(), vec![], vec![])
    }

    #[test]
    fn step_output_writes_into_the_bus_and_keeps_the_bus_the_only_owner() {
        let mut c = ctx();
        let mut step = StepOutput::empty();
        step.signals.insert("has_problem".to_string(), json!(true));
        step.message = Some("привет".to_string());

        assert!(handle_step_output(&step, "n1", &mut c).is_empty());
        assert_eq!(c.signals.get("has_problem"), Some(&json!(true)));
        assert!(
            !c.output_emitted,
            "сам факт текста не эмитит сообщение: это делает исполнитель шага"
        );
    }

    #[test]
    fn rewrite_of_an_existing_signal_is_reported_not_silent() {
        let mut c = ctx();
        c.signals.insert("triage_complexity".to_string(), json!("cluster"));

        let mut step = StepOutput::empty();
        step.signals.insert("triage_complexity".to_string(), json!("specific"));
        let conflicts = handle_step_output(&step, "n2", &mut c);

        assert_eq!(conflicts.len(), 1, "конфликт обязан быть виден: {:?}", conflicts);
        assert!(conflicts[0].contains("triage_complexity"));
        assert_eq!(c.signals.get("triage_complexity"), Some(&json!("specific")));
    }

    #[test]
    fn same_value_is_not_a_conflict() {
        let mut c = ctx();
        let mut first = StepOutput::empty();
        first.signals.insert("k".to_string(), json!(1));
        handle_step_output(&first, "n1", &mut c);

        let mut second = StepOutput::empty();
        second.signals.insert("k".to_string(), json!(1));
        assert!(handle_step_output(&second, "n2", &mut c).is_empty());
    }

    #[test]
    fn thought_process_is_not_a_signal() {
        let step = StepOutput::from_model_object(&json!({
            "thought_process": "размышление",
            "has_problem": true,
        }));
        assert_eq!(step.signals.len(), 1);
        assert!(step.signals.contains_key("has_problem"));
    }

    #[test]
    fn empty_step_leaves_the_bus_untouched() {
        let mut c = ctx();
        assert!(handle_step_output(&StepOutput::empty(), "n", &mut c).is_empty());
        assert!(c.signals.is_empty());
    }
}
