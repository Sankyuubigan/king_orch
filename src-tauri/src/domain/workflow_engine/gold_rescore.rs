//! Пересчёт замеров по текущему золоту — без единого вызова модели.
//!
//! # Зачем это отдельная вещь, а не «просто посчитать в питоне»
//!
//! Метрика обязана считаться той же функцией, что и в прогоне
//! ([`verdict_contract::score_fields`] и [`probe_report::summarize`]).
//! Пересчёт на другой реализации даёт расхождение на единицу и делает
//! несопоставимыми числа до и после правки разметки.
//!
//! # Что пересчитывается
//!
//! Отчёты хранят `verdicts` (что сказала модель) и `expected` (что было
//! разметкой НА МОМЕНТ ПРОГОНА). После правки золота `expected` в отчёте
//! устарел, а вердикты модели остались верными. Поэтому заново берутся кейсы
//! из текущих файлов разметки, и по ним пересчитывается счёт.
//!
//! Оба артефакта пересчитываются одним и тем же кодом:
//!   - `layered_validator_<model>.json`      — трёхслойная воронка
//!   - `gate_end_to_end_ornith.json`         — плоский System-1 (база сравнения)
//!
//! Запуск:
//!   test.bat gold_rescore -- --ignored --nocapture

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};

use super::probe_cases;
use super::probe_report::summarize;
use super::verdict_contract::score_fields;

/// Имена наборов в старых артефактах. Нынешние стенды пишут `somatic_only`,
/// `neurosis_base`, …; артефакт базовой системы сравнения писался раньше под
/// другими именами. Сопоставление идёт по ИМЕНИ КЕЙСА, поэтому здесь только
/// перевод названия набора — содержимое кейсов не трогаем.
const LEGACY_DATASETS: &[(&str, &str)] = &[
    ("D1_somatic", "somatic_only"),
    ("D2_base", "neurosis_base"),
    ("D3_mixed", "mixed_body_mental"),
    ("D4_metaphor", "body_metaphor"),
];

fn dataset_name(raw: &str) -> &str {
    LEGACY_DATASETS
        .iter()
        .find(|(legacy, _)| *legacy == raw)
        .map(|(_, current)| *current)
        .unwrap_or(raw)
}

/// Одна строка артефакта, приведённая к общему виду.
struct StoredRow {
    dataset: String,
    id: String,
    /// `None` — слой не ответил. Такой кейс обязан остаться в знаменателе.
    verdicts: Option<BTreeMap<String, bool>>,
}

fn load_rows(path: &Path) -> Result<Vec<StoredRow>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("не прочитать {}: {}", path.display(), error))?;
    let report: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} не разобран: {}", path.display(), error))?;
    let rows = report
        .get("rows")
        .or_else(|| report.get("cases"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{}: нет ни rows, ни cases", path.display()))?;

    let mut out = Vec::new();
    for row in rows {
        let (Some(dataset), Some(id)) = (
            row.get("dataset").and_then(Value::as_str),
            row.get("id").and_then(Value::as_str),
        ) else {
            continue;
        };
        // Отсутствие verdicts — не повод выбрасывать кейс: он считается как
        // проваленный, теряет все поля, но остаётся в знаменателе.
        let verdicts = match row.get("verdicts").and_then(Value::as_object) {
            None => None,
            Some(object) => {
                let mut parsed = BTreeMap::new();
                for (element, value) in object {
                    let flag = value.as_bool().ok_or_else(|| {
                        format!(
                            "{}: {}/{} поле {} не boolean",
                            path.display(),
                            dataset,
                            id,
                            element
                        )
                    })?;
                    parsed.insert(element.clone(), flag);
                }
                Some(parsed)
            }
        };
        out.push(StoredRow {
            dataset: dataset_name(dataset).to_string(),
            id: id.to_string(),
            verdicts,
        });
    }
    Ok(out)
}

/// Пересчитывает один артефакт по текущей разметке.
fn rescore(path: &Path) -> Result<Value, String> {
    let cases = probe_cases::load_all_cases()?;
    let rows = load_rows(path)?;

    let mut out = Vec::with_capacity(rows.len());
    let mut missing_gold = 0usize;
    for stored in &rows {
        let Some(case) = cases
            .iter()
            .find(|case| case.dataset == stored.dataset && case.id == stored.id)
        else {
            // Кейса нет в разметке — его нельзя счёт, но молчать об этом нельзя.
            missing_gold += 1;
            continue;
        };
        let expected = case.expected.clone().unwrap_or_default();
        // Проваленный слой не выбрасывает кейс: поля остаются в знаменателе.
        let (correct, fields, tp, tn, fp, fn_) = match &stored.verdicts {
            Some(verdicts) => score_fields(Some(&expected), verdicts).ok_or_else(|| {
                format!(
                    "{}: {} — вердикты без разметки",
                    path.display(),
                    stored.id
                )
            })?,
            None => super::verdict_contract::score_failed_case(&expected),
        };
        out.push(json!({
            "dataset": stored.dataset,
            "id": stored.id,
            "expected": expected,
            "verdicts": json!(stored.verdicts.clone().unwrap_or_default()),
            "failed": stored.verdicts.is_none(),
            "correct": correct,
            "fields": fields,
            "tp": tp,
            "tn": tn,
            "fp": fp,
            "fn": fn_,
        }));
    }

    let mut result = summarize(&out);
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "rescored_from".to_string(),
            Value::String(path.display().to_string()),
        );
        object.insert("rows_scored".to_string(), Value::from(out.len()));
        object.insert("cases_without_gold".to_string(), Value::from(missing_gold));
    }
    Ok(result)
}

/// Артефакт, который пересчитывается: подпись + имя файла.
///
/// Список закрытый и явный: «отберём всё, что похоже на отчёт» означало бы
/// через месяц сравнивать уже не тот замер. Каждая строка — конкретная система,
/// которую мы хотим видеть на одном золоте.
const SYSTEMS: &[(&str, &str)] = &[
    ("Система-1 плоская (Лайя) — база сравнения", "gate_end_to_end_ornith.json"),
    ("Лайя, 3 слоя (отклонена 10.10.26)", "laya_layered_validator.json"),
    ("Орнит, 3 слоя, 1 черновик на слой (11.10.26)", "layered_validator_ONE_DRAFT_11.10_oldgold.json"),
    ("Лайя-консолидатор", "laya_consolidator_Ornith-1.5-9B-uncensored.i1-IQ4_NL.json"),
];

#[test]
#[ignore]
fn gold_rescore() {
    let dir = probe_cases::results_dir();

    let mut sections: Vec<(String, Value)> = Vec::new();
    for (name, filename) in SYSTEMS {
        let path = dir.join(filename);
        if !path.exists() {
            eprintln!("[rescore] {}: нет артефакта {}", name, path.display());
            continue;
        }
        match rescore(&path) {
            Ok(summary) => sections.push((name.to_string(), summary)),
            Err(error) => eprintln!("[rescore] {}: {}", name, error),
        }
    }

    for (name, summary) in sections {
        println!("\n===== {} =====", name);
        println!("{}", serde_json::to_string_pretty(&summary).expect("json"));
    }
}