//! Общая сводка и чтение отчётов для стендов замеров (`#[cfg(test)]`).
//!
//! # Почему одна реализация на все стенды
//!
//! Стендов замеров 9 элементов было три (плоская генеративная модель, Лайя,
//! иерархическая воронка), и у каждого была своя копия подсчёта. Копии счёта
//! дают расхождение на единицу и делают сравнение систем бессмысленным —
//! сравнивать надо ОДНОЙ мерой (см. `verdict_contract`: счёт общий для System-1
//! и System-2). Здесь собрано всё, что не зависит от того, что именно меряется:
//! чтение строк отчёта, upsert кейса и сводка по наборам с раздельными
//! recall/specificity.
//!
//! Форма отчёта (`write_report`) у стендов своя: в нём живут гейты и приметы
//! конкретного эксперимента.

use serde_json::{json, Map, Value};

use super::probe_cases;

/// Кейс с тем же `(dataset, id)` перезаписывается, новый — добавляется. Иначе
/// повторный прогон одного набора удваивал бы строки и портил сводку.
pub fn upsert_row(rows: &mut Vec<Value>, row: Value) {
    let key = (
        row.get("dataset").and_then(Value::as_str).unwrap_or_default(),
        row.get("id").and_then(Value::as_str).unwrap_or_default(),
    );
    rows.retain(|existing| {
        (
            existing.get("dataset").and_then(Value::as_str).unwrap_or_default(),
            existing.get("id").and_then(Value::as_str).unwrap_or_default(),
        ) != key
    });
    rows.push(row);
}

/// Сводка по наборам: поля, TP/TN/FP/FN, accuracy, recall и specificity
/// раздельно, плюс провалы и потерянные метафоры.
///
/// Поля кейса считаются **всегда**, в том числе у неразобранного: иначе провал
/// выпадает из знаменателя и завышает точность (на 26 кейсах это 41 ошибка из
/// 63, которые дали бы 73.2% вместо честных ~30%).
pub fn summarize(rows: &[Value]) -> Value {
    let mut out = Map::new();
    let (mut total_fields, mut total_correct) = (0usize, 0usize);
    let mut unparsed_total = 0usize;
    let mut metaphors_lost = 0usize;
    for dataset in probe_cases::REPORT_ORDER {
        let subset: Vec<&Value> = rows
            .iter()
            .filter(|row| row.get("dataset").and_then(Value::as_str) == Some(*dataset))
            .collect();
        if subset.is_empty() {
            continue;
        }
        let mut fields = 0usize;
        let mut correct = 0usize;
        let (mut tp, mut tn, mut fp, mut fn_) = (0usize, 0usize, 0usize, 0usize);
        let mut errors: Vec<Value> = Vec::new();
        let mut lost: Vec<Value> = Vec::new();
        for row in &subset {
            let id = row.get("id").cloned().unwrap_or(Value::Null);
            if let Some(error) = row.get("error").and_then(Value::as_str) {
                errors.push(Value::Object(vec![
                    ("id".to_string(), id.clone()),
                    ("error".to_string(), Value::String(error.to_string())),
                ].into_iter().collect()));
            }
            match row.get("fields").and_then(Value::as_u64) {
                Some(row_fields) => {
                    fields += row_fields as usize;
                    correct += row.get("correct").and_then(Value::as_u64).unwrap_or(0) as usize;
                    tp += row.get("tp").and_then(Value::as_u64).unwrap_or(0) as usize;
                    tn += row.get("tn").and_then(Value::as_u64).unwrap_or(0) as usize;
                    fp += row.get("fp").and_then(Value::as_u64).unwrap_or(0) as usize;
                    fn_ += row.get("fn").and_then(Value::as_u64).unwrap_or(0) as usize;
                }
                None => {
                    let any_true = row
                        .get("verdicts")
                        .and_then(Value::as_object)
                        .map(|object| object.values().any(|value| value == &Value::Bool(true)))
                        .unwrap_or(false);
                    if !any_true {
                        lost.push(id);
                    }
                }
            }
        }
        unparsed_total += errors.len();
        if *dataset == "body_metaphor" {
            metaphors_lost = lost.len();
        } else {
            total_fields += fields;
            total_correct += correct;
        }
        let mut entry = Map::new();
        entry.insert("cases".to_string(), Value::from(subset.len()));
        entry.insert("fields".to_string(), Value::from(fields));
        entry.insert("correct".to_string(), Value::from(correct));
        entry.insert("tp".to_string(), Value::from(tp));
        entry.insert("tn".to_string(), Value::from(tn));
        entry.insert("fp".to_string(), Value::from(fp));
        entry.insert("fn".to_string(), Value::from(fn_));
        // Recall и Specificity по классам РАЗДЕЛЬНО: сводная (TP+TN)/полей на
        // несбалансированном золоте скрывает направление сдвига модели. Золото
        // `mixed_body_mental` — 24 true из 90 полей, `neurosis_base` — 170 из 234,
        // поэтому одна и та же настройка априора на `true` поднимает одну точность
        // и роняет другую (см. tasks/08.10.26 …md §2).
        entry.insert("gold_true".to_string(), Value::from(tp + fn_));
        entry.insert("gold_false".to_string(), Value::from(tn + fp));
        entry.insert(
            "recall_true".to_string(),
            ratio(tp, tp + fn_),
        );
        entry.insert(
            "specificity_false".to_string(),
            ratio(tn, tn + fp),
        );
        entry.insert("errors".to_string(), Value::Array(errors));
        if fields > 0 {
            entry.insert("accuracy".to_string(), ratio(correct, fields));
        } else {
            entry.insert("accuracy".to_string(), Value::Null);
            entry.insert("lost_all_elements".to_string(), Value::Array(lost));
        }
        out.insert((*dataset).to_string(), Value::Object(entry));
    }
    let mut overall = Map::new();
    overall.insert("fields".to_string(), Value::from(total_fields));
    overall.insert("correct".to_string(), Value::from(total_correct));
    overall.insert("accuracy".to_string(), ratio(total_correct, total_fields));
    overall.insert("unparsed_cases".to_string(), Value::from(unparsed_total));
    overall.insert(
        "body_metaphor_lost".to_string(),
        Value::from(metaphors_lost),
    );
    out.insert("overall".to_string(), Value::Object(overall));
    Value::Object(out)
}

fn ratio(numerator: usize, denominator: usize) -> Value {
    if denominator == 0 {
        Value::Null
    } else {
        Value::from(numerator as f64 / denominator as f64)
    }
}

/// Значение метрики набора из сводки: `Accuracy` / `Recall` / `Specificity`.
pub fn metric_of(summary: &Value, dataset: &str, metric: &str) -> Option<f64> {
    summary
        .get(dataset)
        .and_then(|entry| entry.get(metric))
        .and_then(Value::as_f64)
}

/// Сколько кейсов пошло мимо валидатора по решению боевого гейта.
///
/// `called_label` — как стенд называет сработавший маршрут: у воронки это
/// `funnel`, у плоской формы `flat`. Подпись приходит из стенда, потому что
/// различие не в механике, а в том, что именно было вызвано.
///
/// Блок публикуется намеренно: без него точность на соматике неотличима от
/// точности гейта. Тот же дефект находили в базе Лайи (`tasks/10.10.26 …md` §6:
/// «96.3% — это не работа Лайи, а поведение гейта»).
pub fn routing_stats(rows: &[Value], called_label: &str) -> Value {
    let mut by_dataset = Map::new();
    let mut short_circuited = 0usize;
    for dataset in probe_cases::REPORT_ORDER {
        let subset: Vec<&Value> = rows
            .iter()
            .filter(|row| row.get("dataset").and_then(Value::as_str) == Some(*dataset))
            .collect();
        if subset.is_empty() {
            continue;
        }
        let skipped = subset
            .iter()
            .filter(|row| row.get("route").and_then(Value::as_str) == Some("short_circuit"))
            .count();
        short_circuited += skipped;
        by_dataset.insert(
            (*dataset).to_string(),
            json!({
                called_label: subset.len() - skipped,
                "short_circuit": skipped,
            }),
        );
    }
    json!({
        "note": "short_circuit — боевой граф не вызывает валидатор при mental_presence == none. \
                 Поля таких кейсов остаются в знаменателе, но вердикты поставил гейт, а не валидатор.",
        "short_circuited_cases": short_circuited,
        "by_dataset": by_dataset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Кейс, который не разобран, обязан остаться в знаменателе полей.
    #[test]
    fn failed_case_stays_in_denominator() {
        let rows = vec![
            json!({
                "dataset": "neurosis_base", "id": "ok_case",
                "correct": 9, "fields": 9, "tp": 4, "tn": 5, "fp": 0, "fn": 0,
                "error": Value::Null,
            }),
            json!({
                "dataset": "neurosis_base", "id": "broken_case",
                "correct": 0, "fields": 9, "tp": 0, "tn": 0, "fp": 0, "fn": 4,
                "error": "оба канала без пригодного JSON",
            }),
        ];
        let summary = summarize(&rows);
        let entry = summary.get("neurosis_base").expect("набор в сводке");
        assert_eq!(entry.get("fields").and_then(Value::as_u64), Some(18));
        assert_eq!(entry.get("correct").and_then(Value::as_u64), Some(9));
        let accuracy = entry.get("accuracy").and_then(Value::as_f64).expect("точность");
        assert!(
            (accuracy - 0.5).abs() < 1e-9,
            "провал обязан резать точность вдвое, получено {}",
            accuracy
        );
        assert_eq!(summary["overall"]["unparsed_cases"].as_u64(), Some(1));

        // Набор без разметки: проваленный кейс считается потерянной метафорой.
        let mut with_metaphor = rows;
        with_metaphor.push(json!({"dataset": "body_metaphor", "id": "m1", "error": "нет JSON"}));
        let summary = summarize(&with_metaphor);
        assert_eq!(
            summary["body_metaphor"]["lost_all_elements"].as_array().map(Vec::len),
            Some(1)
        );
    }

    /// Сводная (TP+TN)/полей не показывает, КУДА сдвинулся баланс модели, а на
    /// золоте с противоположным балансом классов это ровно то, что настраивают.
    #[test]
    fn summary_separates_recall_and_specificity() {
        let rows = vec![json!({
            "dataset": "neurosis_base", "id": "case",
            // 170 gold true, 64 gold false: recall 60/170, specificity 60/64.
            "correct": 120, "fields": 234, "tp": 60, "tn": 60, "fp": 4, "fn": 110,
            "error": Value::Null,
        })];
        let summary = summarize(&rows);
        let entry = summary.get("neurosis_base").expect("набор в сводке");
        assert_eq!(entry.get("gold_true").and_then(Value::as_u64), Some(170));
        assert_eq!(entry.get("gold_false").and_then(Value::as_u64), Some(64));
        let recall = entry.get("recall_true").and_then(Value::as_f64).expect("recall");
        assert!((recall - 60.0 / 170.0).abs() < 1e-9, "recall = {}", recall);
        let specificity = entry
            .get("specificity_false")
            .and_then(Value::as_f64)
            .expect("specificity");
        assert!(
            (specificity - 60.0 / 64.0).abs() < 1e-9,
            "specificity = {}",
            specificity
        );
        assert_eq!(
            metric_of(&summary, "neurosis_base", "recall_true"),
            Some(60.0 / 170.0)
        );
        assert_eq!(
            metric_of(&summary, "neurosis_base", "accuracy"),
            Some(120.0 / 234.0)
        );
    }

    /// Повторный прогон одного кейса не удваивает строку отчёта.
    #[test]
    fn upsert_row_replaces_same_case() {
        let mut rows = Vec::new();
        upsert_row(&mut rows, json!({"dataset": "a", "id": "x", "correct": 1}));
        upsert_row(&mut rows, json!({"dataset": "a", "id": "x", "correct": 9}));
        upsert_row(&mut rows, json!({"dataset": "a", "id": "y", "correct": 2}));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("correct").and_then(Value::as_u64), Some(9));
    }

    /// Маршрут кейса обязан быть опубликован: без него точность на соматике
    /// неотличима от точности боевого гейта.
    #[test]
    fn routing_stats_separates_short_circuit_from_called() {
        let rows = vec![
            json!({"dataset": "somatic_only", "id": "s1", "route": "short_circuit"}),
            json!({"dataset": "somatic_only", "id": "s2", "route": "funnel"}),
            json!({"dataset": "neurosis_base", "id": "t1", "route": "funnel"}),
        ];
        let routing = routing_stats(&rows, "funnel");
        assert_eq!(routing["short_circuited_cases"], json!(1));
        assert_eq!(routing["by_dataset"]["somatic_only"]["funnel"], json!(1));
        assert_eq!(routing["by_dataset"]["somatic_only"]["short_circuit"], json!(1));
        assert_eq!(routing["by_dataset"]["neurosis_base"]["short_circuit"], json!(0));

        // Подпись приходит из стенда: механика та же, названо по-своему.
        let flat = routing_stats(&rows, "flat");
        assert_eq!(flat["by_dataset"]["somatic_only"]["flat"], json!(1));
        assert_eq!(flat["short_circuited_cases"], json!(1));
    }

    /// Кейс без поля `route` считается сработавшим, а не шорт-циркутом:
    /// отсутствие отметки не должно молча зачислять кейс в пройденные мимо.
    #[test]
    fn routing_stats_does_not_invent_short_circuit() {
        let rows = vec![json!({"dataset": "neurosis_base", "id": "t1"})];
        let routing = routing_stats(&rows, "funnel");
        assert_eq!(routing["short_circuited_cases"], json!(0));
        assert_eq!(routing["by_dataset"]["neurosis_base"]["funnel"], json!(1));
    }
}