//! Разбор ответа одного вызова LLM и тип отказа — общий для всех стендов
//! замера 9 элементов (`#[cfg(test)]`).
//!
//! # Почему это один модуль, а не по копии на стенд
//!
//! Разбор конверта `emit_signal` — это **ровно тот же код**, что работает в бою:
//! `parsers::parse_tool_call` плюс честная `signals::validate_signal_value`.
//! Свой парсер в стенде означал бы, что стенд принимает формы, которые бой
//! отвергнет, и мы сравниваем не боевую форму ответа, а свою.
//!
//! Копии этого разбора стоили дороже, чем кажется: воронка и плоская форма
//! держали по два почти идентичных тела (`read_layer`/`read_flat` плюс
//! `preview`). Правка одной из них не попадала в другую — и два замера, которые
//! мы считали «одинаковыми правилами чтения», оказывались посчитаны по разным.
//!
//! # Проверяются оба канала ответа
//!
//! У reasoning-моделей конверт может уйти в `reasoning_content`, оставив `content`
//! пустым (наблюдалось на `somatic_throat_fever`: 0 токенов ответа при 2042
//! токенах думателя). Проверка только `content` превращала бы такой случай в
//! ложный «слой не ответил».
//!
//! # Отказ несёт сырые ответы
//!
//! Отдельное поле `steps` у отказа обязательно: симптом «конверт не найден» сам
//! по себе не говорит, дошёл ли ответ до границы грамматики. Прежняя форма
//! отказа выбрасывала сырой ответ, и длина неразобранного ответа оставалась
//! невидимой — из-за чего не замечался ответ длиннее предела (задача
//! `tasks/11.10.26 …md` §10.4).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::domain::signals::SignalContract;

/// Отказ одного вызова вместе с его сырым ответом.
///
/// `steps` — сырые ответы всех выполненных вызовов кейса, а не только
/// провалившегося: без предыдущих шагов отказ неразличим с «модель не ответила
/// сразу».
#[derive(Debug)]
pub struct CallFailure {
    pub message: String,
    pub steps: Vec<Value>,
}

impl CallFailure {
    pub fn new(message: impl Into<String>, steps: Vec<Value>) -> Self {
        Self {
            message: message.into(),
            steps,
        }
    }
}

impl std::fmt::Display for CallFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Конверт `emit_signal` → объект булевых вердиктов.
pub fn read_verdicts(
    contract: &SignalContract,
    text: &str,
    reasoning: &str,
) -> Result<BTreeMap<String, bool>, String> {
    let mut reasons = Vec::new();
    for channel in [("content", text), ("reasoning", reasoning)] {
        if channel.1.trim().is_empty() {
            reasons.push(format!("{}: пусто", channel.0));
            continue;
        }
        match read_verdicts_from(contract, channel.1) {
            Ok(verdicts) => return Ok(verdicts),
            Err(error) => reasons.push(format!("{}: {}", channel.0, error)),
        }
    }
    Err(format!("конверт не найден ({})", reasons.join("; ")))
}

/// Конверт из одного канала ответа.
pub fn read_verdicts_from(
    contract: &SignalContract,
    text: &str,
) -> Result<BTreeMap<String, bool>, String> {
    let (tool, arguments, _thought) = crate::domain::parsers::parse_tool_call(text)
        .ok_or_else(|| format!("в ответе нет конверта инструмента: {}", preview(text)))?;
    if tool != "emit_signal" {
        return Err(format!("вместо emit_signal вызван '{}'", tool));
    }
    let value = arguments
        .get("value")
        .ok_or_else(|| format!("в конверте нет value: {}", preview(text)))?;
    crate::domain::signals::validate_signal_value(contract, value)?;

    let object = value
        .as_object()
        .ok_or_else(|| format!("value — не объект: {}", preview(text)))?;
    let mut verdicts = BTreeMap::new();
    for (element, flag) in object {
        let flag = flag
            .as_bool()
            .ok_or_else(|| format!("поле {} не boolean", element))?;
        verdicts.insert(element.clone(), flag);
    }
    Ok(verdicts)
}

/// Начало ответа с длиной — чтобы отказ читался как диагноз, а не как «ошибка».
pub fn preview(text: &str) -> String {
    let head: String = text.chars().take(160).collect();
    format!("{} симв., начало: {:?}", text.chars().count(), head)
}

/// Записать JSON отчёта, создав папку.
///
/// Общее у всех стендов: тело отчёта у каждого своё (приметы конкретного
/// эксперимента), а запись — одна и та же, и ошибка записи обязана быть
/// паникой с путём: молчаливо потерянный отчёт длинного прогона выглядит как
/// «стенд ничего не намерил».
pub fn write_report(path: &Path, report: &Value) {
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            panic!("не создать {}: {}", parent.display(), error);
        }
    }
    std::fs::write(path, serde_json::to_string_pretty(report).expect("json"))
        .unwrap_or_else(|error| panic!("не записать {}: {}", path.display(), error));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn contract() -> SignalContract {
        SignalContract {
            key: "validator_flat".to_string(),
            value_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {"e1": {"type": "boolean"}, "e2": {"type": "boolean"}},
                "required": ["e1", "e2"]
            }),
        }
    }

    fn envelope(value: serde_json::Value) -> String {
        format!(
            r#"<think>размышление</think>{{"tool":"emit_signal","arguments":{{"key":"validator_flat","value":{}}}}}"#,
            value
        )
    }

    #[test]
    fn envelope_is_read_into_verdicts() {
        let verdicts = read_verdicts(&contract(), &envelope(json!({"e1": true, "e2": false})), "")
            .expect("конверт прочитан");
        assert_eq!(verdicts.get("e1"), Some(&true));
        assert_eq!(verdicts.get("e2"), Some(&false));
    }

    /// Конверт, ушедший в `reasoning_content`, — не отказ. Проверка только
    /// `content` превращала бы такой ответ в ложное «не ответил».
    #[test]
    fn verdicts_are_found_in_the_reasoning_channel() {
        let verdicts = read_verdicts(&contract(), "", &envelope(json!({"e1": true, "e2": true})))
            .expect("конверт в reasoning обязан читаться");
        assert_eq!(verdicts.len(), 2);
    }

    /// Отказ перечисляет ОБА канала: иначе «конверт не найден» без указания,
    /// где искали, — это не диагноз.
    #[test]
    fn failure_names_both_channels() {
        let error = read_verdicts(&contract(), "просто текст", "тоже текст")
            .expect_err("конверта нет");
        assert!(error.contains("content"), "{}", error);
        assert!(error.contains("reasoning"), "{}", error);
    }

    /// Пустой канал должен называться явно: «content: пусто» отличает «модель
    /// молчала» от «модель ответила, но не конвертом». Без такой пометки отказ
    /// выглядит как «конверт не найден», хотя ответа просто не было ни в одном
    /// канале — а это разные причины с разными лекарствами.
    #[test]
    fn empty_channel_is_named_in_the_failure() {
        let error = read_verdicts(&contract(), "   ", "")
            .expect_err("пусто в обоих каналах");
        assert!(error.contains("content: пусто"), "{}", error);
        assert!(error.contains("reasoning: пусто"), "{}", error);

        // Непустой, но бесконвертный канал обязан называться иначе — иначе
        // «молчала» и «ответила не тем» сливаются в одно сообщение.
        let error = read_verdicts(&contract(), "просто текст", "")
            .expect_err("конверта нет");
        assert!(error.contains("content: в ответе нет конверта"), "{}", error);
        assert!(!error.contains("content: пусто"), "{}", error);
    }

    /// Другая форма ответа обязана отвергаться: стенд, принимающий её, мерил бы
    /// не боевую форму.
    #[test]
    fn foreign_tool_is_rejected() {
        let text = r#"{"tool":"send_message","arguments":{"key":"x"}}"#;
        let error = read_verdicts(&contract(), text, "").expect_err("чужой инструмент");
        assert!(error.contains("вместо emit_signal"), "{}", error);
    }
}