//! Приёмка стендов валидатора: гейты читаются из данных, а не из констант кода.
//!
//! # Почему не константами в Rust
//!
//! Гейт — это утверждение о приёмке: «точность на соматике не ниже 0.963».
//! Такое утверждение обязано быть **проверяемым другим** (core rules §1.11) и
//! **объявленным до прогона** (AGENTS.md §00.3.1). Константа в `.rs` хуже по трём
//! причинам:
//!
//! 1. её нельзя прочитать и проверить, не читая код стенда;
//! 2. она молча разъезжается с заданием, где гейт объявлен;
//! 3. она компилируется в релизный бинарь, хотя ничего не измеряет в бою.
//!
//! Поэтому единственный источник — `test_cases/new_tests_for_validator/acceptance_gates.json`.
//!
//! # Гейт, проходящий без измерения, — ложь
//!
//! `body_metaphor_lost` у набора метафор равен 0 и когда «потеряно 0», и когда
//! набор вообще не мерился (`probe_report::summarize` кладёт 0 при отсутствии
//! кейсов). Гейт с обязательным полем `measured` обязан оставаться красным при
//! `actual == null` — иначе «метафоры не мерили» выглядит как «метафоры не потеряны».
//!
//! # Граница шума
//!
//! `noise_floor_fields` — минимальная разница в полях, за которой сравнение двух
//! систем вообще что-то утверждает. На 432 полях ~15 полей (3.5 п.п.) — шум
//! (`docs/LAYA_MODEL.md` §15.4). Пока разница меньше, слово «лучше» не выносится.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Map, Value};

/// Файл приёмки. Путь — от `agents/` рядом, как и остальные файлы фикстур.
const GATES_FILE: &str = "test_cases/new_tests_for_validator/acceptance_gates.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    AtLeast,
    AtMost,
}

impl Bound {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "at_least" => Ok(Bound::AtLeast),
            "at_most" => Ok(Bound::AtMost),
            other => Err(format!("неизвестное направление гейта '{}'", other)),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Bound::AtLeast => "at_least",
            Bound::AtMost => "at_most",
        }
    }

    pub fn holds(self, value: f64, threshold: f64) -> bool {
        match self {
            Bound::AtLeast => value >= threshold,
            Bound::AtMost => value <= threshold,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Gate {
    pub id: String,
    pub dataset: String,
    pub metric: String,
    pub bound: Bound,
    pub threshold: f64,
    /// Набор, факт наличия кейсов в котором обязателен для прохождения. `None` —
    /// гейт считается измеренным, если сама метрика не `null`.
    pub measured_on: Option<String>,
}

/// Ссылка на другую измеренную систему: с чем сравниваем.
#[derive(Debug, Clone)]
pub struct Reference {
    pub label: String,
    pub correct_fields: i64,
    pub artifact: String,
}

#[derive(Debug, Clone)]
pub struct GainGate {
    pub reference: String,
    pub min_fields: i64,
}

#[derive(Debug, Clone)]
pub struct Acceptance {
    pub label: String,
    pub max_unparsed_calls: usize,
    pub gates: Vec<Gate>,
    pub gain: Option<GainGate>,
    /// Кейс для ДИАГНОСТИКИ, не для приёмки: его вердикты попадают в отчёт
    /// рядом с гейтами, но гейтом он не является и на вердикт не влияет.
    /// Объявлен данными, а не константой стенда: конкретный кейс в коде —
    /// это бизнес-знание, которого в движке быть не должно.
    pub diagnostic_case: Option<String>,
}

/// Приёмка одной системы плюс ссылки, с которыми она сравнивается.
#[derive(Debug, Clone)]
pub struct AcceptanceBook {
    pub noise_floor_fields: i64,
    pub references: BTreeMap<String, Reference>,
    systems: BTreeMap<String, Acceptance>,
}

impl AcceptanceBook {
    /// Читает книгу приёмки целиком: система одна, но ссылок на базы может быть
    /// несколько, и их тоже обязан видеть читатель гейтов.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join(GATES_FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("не прочитать {}: {}", path.display(), error))?;
        let report: Value = serde_json::from_str(&text)
            .map_err(|error| format!("{} не разобран: {}", path.display(), error))?;
        Self::from_report(&report, &path)
    }

    pub fn from_report(report: &Value, source: &Path) -> Result<Self, String> {
        let where_ = |what: &str| format!("{}: {}", source.display(), what);

        let noise_floor_fields = report
            .pointer("/noise_floor_fields/value")
            .and_then(Value::as_i64)
            .ok_or_else(|| where_("не задан noise_floor_fields.value — гейт на шум посчитать нечем"))?;

        let mut references = BTreeMap::new();
        let refs = report
            .get("references")
            .and_then(Value::as_object)
            .ok_or_else(|| where_("нет блока references"))?;
        for (id, entry) in refs {
            references.insert(
                id.clone(),
                Reference {
                    label: entry
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or(id)
                        .to_string(),
                    correct_fields: entry
                        .get("correct_fields")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| where_(&format!("в references.{} нет correct_fields", id)))?,
                    artifact: entry
                        .get("artifact")
                        .and_then(Value::as_str)
                        .ok_or_else(|| where_(&format!("в references.{} нет artifact", id)))?
                        .to_string(),
                },
            );
        }

        let mut systems = BTreeMap::new();
        let systems_node = report
            .get("systems")
            .and_then(Value::as_object)
            .ok_or_else(|| where_("нет блока systems"))?;
        for (id, entry) in systems_node {
            let gates = entry
                .get("gates")
                .and_then(Value::as_array)
                .ok_or_else(|| where_(&format!("в systems.{} нет gates", id)))?;
            if gates.is_empty() {
                return Err(where_(&format!(
                    "у системы '{} нет гейтов — приёмка без критериев это не приёмка",
                    id
                )));
            }
            let mut parsed = Vec::with_capacity(gates.len());
            for gate in gates {
                let field = |name: &str| -> Result<String, String> {
                    gate.get(name)
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .ok_or_else(|| where_(&format!("гейт без поля {}", name)))
                };
                parsed.push(Gate {
                    id: field("id")?,
                    dataset: field("dataset")?,
                    metric: field("metric")?,
                    bound: Bound::parse(&field("bound")?)?,
                    threshold: gate
                        .get("threshold")
                        .and_then(Value::as_f64)
                        .ok_or_else(|| where_("гейт без числового threshold"))?,
                    measured_on: gate
                        .get("measured_on")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                });
            }
            let gain = match entry.get("max_gain_against") {
                Some(node) => Some(GainGate {
                    reference: node
                        .get("reference")
                        .and_then(Value::as_str)
                        .ok_or_else(|| where_("max_gain_against без reference"))?
                        .to_string(),
                    min_fields: node
                        .get("min_fields")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| where_("max_gain_against без min_fields"))?,
                }),
                None => None,
            };
            systems.insert(
                id.clone(),
                Acceptance {
                    label: entry
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or(id)
                        .to_string(),
                    max_unparsed_calls: entry
                        .get("max_unparsed_calls")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as usize,
                    gates: parsed,
                    gain,
                    diagnostic_case: entry
                        .pointer("/diagnostics/target_case")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                },
            );
        }
        if systems.is_empty() {
            return Err(where_("ни одной системы — книга приёмки пуста"));
        }

        Ok(Self {
            noise_floor_fields,
            references,
            systems,
        })
    }

    pub fn system(&self, id: &str) -> Result<&Acceptance, String> {
        self.systems
            .get(id)
            .ok_or_else(|| format!("в книге приёмки нет системы '{}'", id))
    }

    pub fn reference(&self, id: &str) -> Result<&Reference, String> {
        self.references
            .get(id)
            .ok_or_else(|| format!("в книге приёмки нет базы для сравнения '{}'", id))
    }

    /// Считает гейты по сводке замера.
    ///
    /// `correct_fields` — сумма полей, набранных системой; нужна гейту «выиграла ли
    /// система у базы», который измеряется в полях, а не в дробях.
    pub fn evaluate(
        &self,
        system_id: &str,
        summary: &Value,
        correct_fields: i64,
        unparsed: usize,
    ) -> Result<Value, String> {
        let acceptance = self.system(system_id)?;
        let mut out = Map::new();

        out.insert(
            "noise_floor_fields".to_string(),
            json!({
                "value": self.noise_floor_fields,
                "note": "разница меньше этого числа полей неотличима от шума на 432 полях \
                         (docs/LAYA_MODEL.md §15.4)",
            }),
        );
        out.insert(
            "G0_unparsed".to_string(),
            json!({"required_max": acceptance.max_unparsed_calls,
                   "actual": unparsed,
                   "passed": unparsed <= acceptance.max_unparsed_calls}),
        );

        for gate in &acceptance.gates {
            let actual = super::probe_report::metric_of(summary, &gate.dataset, &gate.metric);
            // Гейт без измерения обязан остаться красным. Иначе «потеряно 0
            // метафор» и «метафоры не мерили» дают одно и то же число.
            let measured = match &gate.measured_on {
                Some(source) => super::probe_report::metric_of(summary, source, "cases")
                    .map(|cases| cases >= 1.0)
                    .unwrap_or(false),
                None => actual.is_some(),
            };
            out.insert(
                gate.id.clone(),
                json!({
                    "dataset": gate.dataset,
                    "metric": gate.metric,
                    "bound": gate.bound.as_str(),
                    "required": gate.threshold,
                    "actual": actual,
                    "measured": measured,
                    "passed": measured
                        && matches!(actual, Some(value) if gate.bound.holds(value, gate.threshold)),
                }),
            );
        }

        if let Some(gain) = &acceptance.gain {
            let reference = self.reference(&gain.reference)?;
            let delta = correct_fields - reference.correct_fields;
            out.insert(
                "GAIN_vs_baseline".to_string(),
                json!({
                    "baseline": reference.label,
                    "baseline_artifact": reference.artifact,
                    "baseline_correct_fields": reference.correct_fields,
                    "own_correct_fields": correct_fields,
                    "delta_fields": delta,
                    "required_min_delta": gain.min_fields,
                    "note": "вердикт «лучше» выносится только если разница вышла за шум",
                    "passed": delta >= gain.min_fields,
                }),
            );
        }

        Ok(Value::Object(out))
    }

    pub fn gate_ids(&self, system_id: &str) -> Result<Vec<String>, String> {
        Ok(self
            .system(system_id)?
            .gates
            .iter()
            .map(|gate| gate.id.clone())
            .collect())
    }

    /// Кейс для диагностики, объявленный данными. `None` — показывать нечего.
    pub fn diagnostic_case(&self, system_id: &str) -> Option<&str> {
        self.system(system_id)
            .ok()
            .and_then(|acceptance| acceptance.diagnostic_case.as_deref())
    }

    /// Подпись системы для отчёта: артефакт обязан сам называть, кто он, иначе
    /// через месяц файл неотличим от соседних замеров той же задачи.
    pub fn system_label(&self, system_id: &str) -> Result<&str, String> {
        Ok(&self.system(system_id)?.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> AcceptanceBook {
        let report = json!({
            "noise_floor_fields": {"value": 15},
            "references": {
                "funnel_3layer": {
                    "label": "воронка",
                    "correct_fields": 346,
                    "artifact": "funnel.json",
                }
            },
            "systems": {
                "flat_validator": {
                    "label": "плоская",
                    "max_unparsed_calls": 0,
                    "max_gain_against": {"reference": "funnel_3layer", "min_fields": 15},
                    "gates": [
                        {"id": "G1", "dataset": "overall", "metric": "accuracy",
                         "bound": "at_least", "threshold": 0.8},
                        {"id": "G5", "dataset": "overall", "metric": "body_metaphor_lost",
                         "bound": "at_most", "threshold": 0, "measured_on": "body_metaphor"}
                    ]
                }
            }
        });
        AcceptanceBook::from_report(&report, Path::new("acceptance.json")).expect("книга разобрана")
    }

    #[test]
    fn gate_is_red_when_metric_is_missing() {
        let summary = json!({"overall": {"accuracy": null}});
        let gates = book()
            .evaluate("flat_validator", &summary, 100, 0)
            .expect("гейты посчитаны");
        assert_eq!(
            gates.get("G1").and_then(|g| g.get("passed")),
            Some(&json!(false)),
            "accuracy == null обязан давать красный гейт, а не зелёный"
        );
    }

    /// Главный защитный тест: «потеряно 0 метафор» и «метафоры не мерили» —
    /// одно и то же число при разном смысле. Без `measured` гейт был бы зелёным
    /// на пустом наборе.
    #[test]
    fn metaphor_gate_is_red_when_nothing_was_measured() {
        let summary = json!({"overall": {"body_metaphor_lost": 0}});
        let gates = book()
            .evaluate("flat_validator", &summary, 100, 0)
            .expect("гейты посчитаны");
        assert_eq!(
            gates.get("G5").and_then(|g| g.get("passed")),
            Some(&json!(false)),
            "гейт прошёл без единого измеренного кейса метафор"
        );
        assert_eq!(gates.get("G5").and_then(|g| g.get("measured")), Some(&json!(false)));
    }

    #[test]
    fn metaphor_gate_turns_green_once_metaphors_were_measured() {
        let summary = json!({
            "overall": {"body_metaphor_lost": 0},
            "body_metaphor": {"cases": 15},
        });
        let gates = book()
            .evaluate("flat_validator", &summary, 100, 0)
            .expect("гейты посчитаны");
        assert_eq!(gates.get("G5").and_then(|g| g.get("passed")), Some(&json!(true)));
    }

    #[test]
    fn gain_gate_respects_the_noise_floor() {
        // +14 полей — внутри шума: вердикт «лучше» не выносится.
        let summary = json!({"overall": {"accuracy": 0.9}});
        let gates = book()
            .evaluate("flat_validator", &summary, 360, 0)
            .expect("гейты посчитаны");
        assert_eq!(
            gates.get("GAIN_vs_baseline").and_then(|g| g.get("passed")),
            Some(&json!(false))
        );

        // +15 — ровно за шумом.
        let gates = book()
            .evaluate("flat_validator", &summary, 361, 0)
            .expect("гейты посчитаны");
        assert_eq!(
            gates.get("GAIN_vs_baseline").and_then(|g| g.get("passed")),
            Some(&json!(true))
        );
    }

    #[test]
    fn unparsed_gate_fails_on_a_single_lost_call() {
        let summary = json!({"overall": {"accuracy": 1.0}});
        let gates = book()
            .evaluate("flat_validator", &summary, 432, 1)
            .expect("гейты посчитаны");
        assert_eq!(
            gates.get("G0_unparsed").and_then(|g| g.get("passed")),
            Some(&json!(false))
        );
    }

    #[test]
    fn broken_book_is_an_error_not_an_empty_one() {
        // Порядок проверок в `from_report` фиксирован: сначала граница шума (без неё
        // гейт выигрыша нечем считать), затем базы, затем системы.
        for (report, needle) in [
            (json!({"systems": {}}), "не задан noise_floor_fields.value"),
            (json!({"noise_floor_fields": {"value": 15}}), "нет блока references"),
            (
                json!({"noise_floor_fields": {"value": 15}, "references": {}}),
                "нет блока systems",
            ),
            (
                json!({"noise_floor_fields": {"value": 15}, "references": {"a": {}},
                       "systems": {"s": {"gates": []}}}),
                "нет correct_fields",
            ),
            (
                json!({"noise_floor_fields": {"value": 15},
                       "references": {"a": {"correct_fields": 1, "artifact": "x.json"}},
                       "systems": {}}),
                "ни одной системы",
            ),
        ] {
            let error = AcceptanceBook::from_report(&report, Path::new("acceptance.json"))
                .expect_err("битая книга обязана падать");
            assert!(error.contains(needle), "ожидалось {:?}, получено {}", needle, error);
        }
    }

    /// Пустая книга (системы есть, гейтов нет) обязана падать, а не читаться как
    /// «приёмки нет, значит всё прошло».
    #[test]
    fn system_without_gates_is_an_error() {
        let report = json!({
            "noise_floor_fields": {"value": 15},
            "references": {"a": {"correct_fields": 1, "artifact": "x.json"}},
            "systems": {"s": {"gates": []}}
        });
        let error = AcceptanceBook::from_report(&report, Path::new("acceptance.json"))
            .expect_err("система без гейтов обязана падать");
        assert!(error.contains("нет гейтов"), "{}", error);
    }

    #[test]
    fn unknown_direction_is_an_error() {
        let report = json!({
            "noise_floor_fields": {"value": 15},
            "references": {"a": {"correct_fields": 1, "artifact": "x.json"}},
            "systems": {"s": {"gates": [
                {"id": "g", "dataset": "overall", "metric": "accuracy",
                 "bound": "примерно", "threshold": 0.5}
            ]}}
        });
        let error = AcceptanceBook::from_report(&report, Path::new("acceptance.json"))
            .expect_err("неизвестное направление обязано падать");
        assert!(error.contains("неизвестное направление"), "{}", error);
    }

    /// Имя диагностического кейса приходит из данных. Регресс: константа
    /// `TARGET_CASE = "mixed_…"` в коде стенда — это знание о конкретном
    /// кейсе внутри движка, а движку о кейсах знать нечего.
    #[test]
    fn diagnostic_case_comes_from_data_not_from_code() {
        let report = json!({
            "noise_floor_fields": {"value": 15},
            "references": {"a": {"correct_fields": 1, "artifact": "x.json"}},
            "systems": {"s": {
                "diagnostics": {"target_case": "mixed_4_heart_perfectionism"},
                "gates": [{"id": "g", "dataset": "overall", "metric": "accuracy",
                           "bound": "at_least", "threshold": 0.5}]
            }}
        });
        let book = AcceptanceBook::from_report(&report, Path::new("acceptance.json"))
            .expect("книга разобрана");
        assert_eq!(book.diagnostic_case("s"), Some("mixed_4_heart_perfectionism"));
        // Система без диагностики — не ошибка: показывать просто нечего.
        assert_eq!(book.diagnostic_case("нет-такой"), None);
    }
}