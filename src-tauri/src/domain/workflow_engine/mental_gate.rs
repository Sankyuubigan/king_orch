//! Решение боевого гейта `mental_presence` из ИЗМЕРЕННОГО артефакта.
//!
//! # Зачем стенду этот модуль
//!
//! Граф `agents/psychotherapist/experiments/psychotherapist.yaml` не вызывает
//! валидатор, когда факт-экстрактор вернул `mental_presence == none`:
//! `check_mental_presence` (`psychotherapist.yaml:319-322`) уводит поток в
//! `check_critical_elements`, минуя `call_fact_consolidator → validate_system1`.
//! Стенд без этого правила мерил бы систему, которой в бою нет: он звал бы слои
//! даже там, где бой их не зовёт.
//!
//! # Почему решение берётся из артефакта, а не считается заново
//!
//! Факт-экстрактор — отдельный LLM-вызов на каждый кейс. Повторять его внутри
//! воронки значит удвоить стоимость прогона ради числа, которое уже измерено тем
//! же боевым кодом на тех же кейсах (стенд гейта вынесен в
//! `test/laya_probe/experiments/fact_extractor_probe.rs`). Поэтому стенд
//! читает результат `test/laya_probe/results/fact_extractor_mental_<model>.json`.
//!
//! # Почему не золото
//!
//! Эталон 9 элементов (`ProbeCase::expected`) для этого использовать нельзя: это
//! ответчик, и маршрутизация по нему была бы подгонкой результата под себя
//! (AGENTS.md §00.3.1). Здесь допустим только выход боевого узла.
//!
//! # Честность отчёта
//!
//! У кейса, прошедшего шорт-циркут, элементы не «проверены и не найдены» —
//! валидатор не вызывался вовсе. Поэтому число таких кейсов публикуется в отчёте
//! отдельной строкой: иначе точность на соматике выглядела бы заслугой воронки
//! (ровно тот дефект, что найден в `tasks/10.10.26 …md` §6: «96.3% — это не
//! работа Лайи, а поведение гейта»).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::probe_cases::{self, ProbeCase};

/// Значение факта, при котором боевой граф НЕ вызывает валидатор.
pub const MENTAL_NONE: &str = "none";

/// Решения гейта по всем кейсам, снятые боевым факт-экстрактором.
#[derive(Debug)]
pub struct MentalGate {
    decisions: BTreeMap<(String, String), String>,
    source: PathBuf,
}

impl MentalGate {
    /// Читает артефакт замера гейта для указанной модели.
    pub fn load(model_label: &str) -> Result<Self, String> {
        let path = probe_cases::results_dir().join(format!(
            "fact_extractor_mental_{}.json",
            model_label
        ));
        let text = std::fs::read_to_string(&path).map_err(|error| {
            format!(
                "не прочитать замер боевого гейта {}: {}. \
                 Прогон стенда гейта обязан опережать воронку: \
                 без его факта маршрут не воспроизводится \
                 (стенд: test/laya_probe/experiments/fact_extractor_probe.rs)",
                path.display(),
                error
            )
        })?;
        let report: Value = serde_json::from_str(&text)
            .map_err(|error| format!("{} не разобран: {}", path.display(), error))?;
        Self::from_report(&report, &path)
    }

    pub fn from_report(report: &Value, source: &Path) -> Result<Self, String> {
        let rows = report
            .get("rows")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{}: нет списка rows", source.display()))?;
        if rows.is_empty() {
            return Err(format!("{}: ни одного кейса в замере гейта", source.display()));
        }
        let mut decisions = BTreeMap::new();
        for row in rows {
            let dataset = row
                .get("dataset")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{}: строка без dataset", source.display()))?;
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{}: строка без id", source.display()))?;
            // `null` означает, что гейт не ответил. Трактувать это как `none`
            // значило бы молча объявить соматикой кейс, который не измерен.
            let presence = row
                .get("mental_presence")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "{}: у кейса {}/{} гейт не ответил (mental_presence = null или не строка). \
                         Шорт-циркут по неразобранному факту был бы выдумкой",
                        source.display(),
                        dataset,
                        id
                    )
                })?;
            decisions.insert(
                (dataset.to_string(), id.to_string()),
                presence.to_string(),
            );
        }
        Ok(Self {
            decisions,
            source: source.to_path_buf(),
        })
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    /// Значение факта `mental_presence` для кейса. Отсутствие кейса в артефакте —
    /// ошибка: маршрут тогда неизвестен, и молча пропустить слой нельзя (§2.2).
    pub fn decision(&self, case: &ProbeCase) -> Result<&str, String> {
        self.decisions
            .get(&(case.dataset.to_string(), case.id.clone()))
            .map(String::as_str)
            .ok_or_else(|| {
                format!(
                    "{}: кейса {}/{} нет в замере гейта — маршрут неизвестен",
                    self.source.display(),
                    case.dataset,
                    case.id
                )
            })
    }

    /// Вызывает ли боевой граф валидатор для этого кейса.
    pub fn validator_is_called(&self, case: &ProbeCase) -> Result<bool, String> {
        Ok(self.decision(case)? != MENTAL_NONE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(rows: Value) -> Value {
        serde_json::json!({ "rows": rows })
    }

    fn case(dataset: &'static str, id: &str) -> ProbeCase {
        ProbeCase {
            dataset,
            id: id.to_string(),
            text: "жалоба".to_string(),
            expected: None,
        }
    }

    fn gate(rows: Value) -> MentalGate {
        MentalGate::from_report(&report(rows), Path::new("test-gate.json")).expect("гейт прочитан")
    }

    #[test]
    fn none_disables_the_validator_and_anything_else_keeps_it() {
        let gate = gate(serde_json::json!([
            {"dataset": "somatic_only", "id": "s1", "mental_presence": "none"},
            {"dataset": "neurosis_base", "id": "task1", "mental_presence": "new"},
            {"dataset": "neurosis_base", "id": "task2", "mental_presence": "history"},
        ]));
        assert!(!gate.validator_is_called(&case("somatic_only", "s1")).expect("решение"));
        assert!(gate.validator_is_called(&case("neurosis_base", "task1")).expect("решение"));
        // `history` — тоже ментальное присутствие: значение, отличное от `none`.
        assert!(gate.validator_is_called(&case("neurosis_base", "task2")).expect("решение"));
    }

    /// Отсутствие кейса в артефакте — ошибка, а не «маршрут по умолчанию».
    #[test]
    fn unknown_case_is_an_error_not_a_default_route() {
        let gate = gate(serde_json::json!([
            {"dataset": "somatic_only", "id": "s1", "mental_presence": "none"},
        ]));
        let error = gate
            .validator_is_called(&case("somatic_only", "s2"))
            .expect_err("нет кейса — ошибка");
        assert!(error.contains("маршрут неизвестен"), "{}", error);
    }

    /// `null` от гейта — это «не измерено», а не «соматика».
    #[test]
    fn null_fact_is_not_treated_as_none() {
        let error = MentalGate::from_report(
            &report(serde_json::json!([
                {"dataset": "somatic_only", "id": "s1", "mental_presence": null},
            ])),
            Path::new("test-gate.json"),
        )
        .expect_err("null — ошибка");
        assert!(error.contains("гейт не ответил"), "{}", error);
    }

    #[test]
    fn empty_or_broken_report_is_an_error() {
        let error = MentalGate::from_report(&report(serde_json::json!([])), Path::new("g.json"))
            .expect_err("пустой замер");
        assert!(error.contains("ни одного кейса"), "{}", error);

        let error = MentalGate::from_report(&serde_json::json!({"нет": "rows"}), Path::new("g.json"))
            .expect_err("нет rows");
        assert!(error.contains("нет списка rows"), "{}", error);
    }

    /// Отсутствующий артефакт обязан называть, что делать, а не «маршрут неизвестен».
    ///
    /// Проверяется именно путь к стенду: после переноса в
    /// `test/laya_probe/experiments/` старое имя теста перестало существовать, и
    /// ошибка, ведущая в пустоту, хуже, чем её отсутствие.
    #[test]
    fn missing_file_explains_how_to_produce_it() {
        let error = MentalGate::load("модели-которой-нет").expect_err("файла нет");
        assert!(
            error.contains("test/laya_probe/experiments/fact_extractor_probe.rs"),
            "{}",
            error
        );
    }
}