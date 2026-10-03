//! Адаптер System-1: единственное место, где хост трогает плагин.
//!
//! # Зачем он в `infra/`
//!
//! `tauri-plugin-system1` держит загруженную ONNX-сессию в своём состоянии —
//! это инфраструктура (диск, процесс, нативная DLL), а не домен.
//! `domain/system1.rs` до этого тоже дёргал `state::shared()` напрямую, и это
//! ломало объявленную в `domain/mod.rs` границу «домен не зависит от Tauri».
//!
//! Контракт плагина доменному не протекает: наружу отдаются только
//! вероятности, а правило решения `p_true >= threshold` остаётся в хосте.

use std::collections::BTreeMap;

use tauri_plugin_system1::contract::{DecisionRequest, PermutationPolicy, TypedQuestion};

/// Результат инференса с разделением времени загрузки и времени счёта.
///
/// `load_ms` отделён от `elapsed_ms` намеренно: раньше модель перечитывалась на
/// каждом прогоне графа, и это не было видно — таймер инференса стартовал уже
/// после загрузки. Разделив их, первый вызов перестал выглядеть как «медленный
/// инференс».
#[derive(Debug, Clone)]
pub struct System1Outcome {
    /// Вероятность истинного варианта по каждому идентификатору вопроса.
    pub probabilities: BTreeMap<String, f32>,
    /// Время инференса, мс.
    pub elapsed_ms: u64,
    /// Время ленивой загрузки модели в первый вызов, мс.
    pub load_ms: u64,
    /// Устройство исполнения (`cpu`).
    pub device: String,
}

/// Идентификатор модели System-1 по умолчанию — SSOT в плагине.
pub use tauri_plugin_system1::catalog::default_model_id as model_id;

/// Выполнить типизированные вопросы через плагин.
///
/// Один вызов на все вопросы: энкодер проходит текст пациента один раз, а не
/// по разу на правило.
pub fn decide(state_text: &str, questions: &[TypedQuestion]) -> Result<System1Outcome, String> {
    let started = std::time::Instant::now();
    let slot = tauri_plugin_system1::state::shared();
    let model = slot.get_or_load(&model_id())?;
    let load_ms = started.elapsed().as_millis() as u64;

    let result = model
        .decide(&DecisionRequest {
            state: state_text.to_string(),
            questions: questions.to_vec(),
            // Две буквенные перестановки — единственный измеренный режим
            // (`docs/LAYA_MODEL.md` плагина, §20.3: усреднение по позиции
            // отклонено, 170/234 против 189/234).
            permutations: PermutationPolicy::Letters,
        })
        .map_err(|error| format!("инференс System-1 не удался: {}", error))?;

    Ok(System1Outcome {
        probabilities: result.probabilities(),
        elapsed_ms: result.elapsed_ms,
        load_ms,
        device: result.device.as_str().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Адаптер не должен требовать девять правил: количество — данные графа.
    /// Здесь проверяется, что типы совместимы с произвольным числом вопросов.
    #[test]
    fn outcome_carries_arbitrary_number_of_answers() {
        let mut probabilities = BTreeMap::new();
        probabilities.insert("e1".to_string(), 0.5);
        probabilities.insert("somatika".to_string(), 0.9);
        let outcome = System1Outcome {
            probabilities,
            elapsed_ms: 10,
            load_ms: 0,
            device: "cpu".to_string(),
        };
        assert_eq!(outcome.probabilities.len(), 2);
        assert_eq!(outcome.device, "cpu");
    }
}