//! Контракт «вердикты по правилам» — общий для System-1 и System-2.
//!
//! # Зачем отдельный модуль
//!
//! Файл критериев (`element_validation_rules_system1.yaml`) — единственный
//! источник правды об элементах (AGENTS.md §2.1). Из него вырастают ДВА разных
//! способа спросить модель об одном и том же:
//!
//! - **System-1 (Лайя):** ONNX-классификатор. Правило превращается в
//!   бинарный вопрос `TypedQuestion` с полюсами
//!   [`System1Rule::to_typed_question`], движок возвращает вероятность положительного.
//! - **System-2 (генеративная модель):** правило превращается в инструкцию
//!   в промпте, а форма выхода задаётся грамматикой с теми же ключами.
//!
//! Общая часть — разбор ответа и подсчёт — живёт здесь, чтобы System-1 и
//! System-2 мерились по **одной** метрике. Копия счёта в стендах означала бы
//! расхождение чисел на единицу и невозможность честного сравнения.
//!
//! Модуль ничего не знает ни про невроз, ни про девять элементов: всё приходит
//! из списка правил. Набор и имена правил — данные графа, а не константа движа.

use std::collections::BTreeMap;

use serde_json::Value;

use super::system1_rules::System1Rule;

/// Промпт System-2: те же поля правила, что видит System-1, но как инструкция.
///
/// Формулировки `question` / `true_criteria` / `false_criteria` переиспользуются
/// дословно. Правка файла критериев меняет System-1 и System-2 одновременно —
/// иначе «одни и те же критерии» перестали бы быть одними и теми же.
pub fn build_verdict_prompt(rules: &[System1Rule]) -> String {
    let mut prompt = String::from(
        "Ты — строгий аналитик. Для каждого элемента определи, назван ли он в тексте \
         сообщения пользователя СОБСТВЕННЫМИ словами пользователя.\n\n",
    );
    for rule in rules {
        prompt.push_str(&format!(
            "- {id} ({name}): {question}\n    подтверждено (true): {true_criteria}\n    \
             не подтверждено (false): {false_criteria}\n",
            id = rule.id,
            name = rule.name,
            question = rule.question,
            true_criteria = rule.true_criteria,
            false_criteria = rule.false_criteria,
        ));
    }
    // Каноничный блок трактовки критериев — тот же, что у боевого экстрактора
    // (`fact_extractor::CRITERIA_INTERPRETATION_BLOCK`), а не его копия.
    //
    // Раньше здесь стояла собственная редакция правил с асимметричным перекосом:
    // «Ставь true, только если в тексте есть прямое подтверждение; иначе false» и
    // «Если сомневаешься — false». В боевом промпте таких директив нет, поэтому
    // стенд мерил не боевую модель, а модель с чужим перекосом в `false`:
    // на `neurosis_base` получилось FN=110 из 234 полей (51.3% точности) при
    // FP=4. Убирание директив — приведение измерителя к боевому контракту
    // (SSOT), а НЕ подбор промпта под цифры.
    prompt.push_str(super::fact_extractor::CRITERIA_INTERPRETATION_BLOCK);
    prompt.push_str("\n\nСообщение пользователя:\n");
    prompt
}

/// Сколько символов размышления разрешается до `</think>`.
///
/// # Почему граница вообще нужна (измерено, `tasks/07.10.26 …md`)
///
/// Гибридная грамматика боевого факт-экстрактора использует
/// `think-block ::= "<think>" [^<]* "</think>" | ""` — think **не ограничен**.
/// На девяти элементах модель выжигала весь `n_predict` в размышлении и до JSON
/// не доходила: 13 из 16 кейсов вернули ответ без `{` вообще. Это механический
/// отказ (бюджет не разделён между фазами), и лечится он зашив бюджет в ФОРМАТ,
/// а не увеличением `n_predict` или ретраем (AGENTS.md §00.2).
///
/// Граница ставится в символах размышления: после неё грамматика принудительно
/// закрывает `</think>`, поэтому JSON-фаза **структурно достижима всегда**.
///
/// Класс символов внутри think — `[^\n<]`: между `<think>` и `</think>` перевод
/// строки невозможен, поэтому грамматика принудительно закрывает think раньше,
/// чем это сделала бы модель. Граница обязана быть **меньше** `n_predict`,
/// иначе JSON-фаза недостижима (см. [`DEFAULT_THINK_BUDGET_CHARS`]).
pub const DEFAULT_THINK_BUDGET_CHARS: usize = 1400;

/// Грамматика выхода: `{"<id>": true|false, …}` с фиксированным набором ключей.
///
/// `think_budget = None` — без размышления (ответ сразу JSON). `Some(n)` —
/// с размышлением, ограниченным `n` символами.
pub fn build_verdict_grammar(rules: &[System1Rule], think_budget: Option<usize>) -> String {
    let bool_keys: Vec<String> = rules.iter().map(|rule| rule.id.clone()).collect();
    let base = crate::infra::build_json_object_grammar_with_keys(&bool_keys, &[]);
    let Some(budget) = think_budget else {
        return base;
    };
    let renamed = base.replacen("root ::=", "json-object ::=", 1);
    format!(
        "root ::= think-block json-object\n\
         think-block ::= \"<think>\" [^\n<]{{0,{budget}}} \"</think>\" | \"\"\n\n{body}",
        body = renamed,
    )
}

/// Что вернул движок по двум каналам — для диагностики провала разбора.
///
/// Без этого «нет символа `{`» — утверждение без доказательства: неизвестно, был
/// ли JSON пустым, ушёл ли в размышление или обрезался по `n_predict`
/// (core rules §1.11).
#[derive(Debug, Default, Clone)]
pub struct RawChannels {
    pub content: String,
    pub reasoning: String,
}

impl RawChannels {
    fn first_object_brace(&self) -> Option<(bool, usize)> {
        [("content", &self.content), ("reasoning", &self.reasoning)]
            .iter()
            .find_map(|(channel, raw)| raw.find('{').map(|index| (*channel == "content", index)))
    }
}

/// Разбор ответа в вердикты.
///
/// **Все ключи правил обязательны.** Ответ без полного набора — ошибка, а не
/// «все элементы false»: молчаливый all-false попал бы в отчёт как честный
/// результат и сделал бы замер бессмысленным (core rules §2.2).
///
/// `content` пустым бывает у reasoning-моделей: JSON уходит в `reasoning_content`.
/// Поэтому проверяются оба канала — как это делает боевая нода экстрактора.
pub fn parse_verdicts(
    channels: &RawChannels,
    rules: &[System1Rule],
) -> Result<BTreeMap<String, bool>, String> {
    let mut last_error = String::new();
    for (channel, raw) in [
        ("content", channels.content.as_str()),
        ("reasoning", channels.reasoning.as_str()),
    ] {
        if raw.trim().is_empty() {
            continue;
        }
        let Some(start) = raw.find('{') else {
            last_error = format!(
                "в канале {} ({} симв.) нет символа '{{'",
                channel,
                raw.chars().count()
            );
            continue;
        };
        let parsed: Value = match serde_json::from_str(&raw[start..]) {
            Ok(value) => value,
            Err(error) => {
                last_error = format!(
                    "JSON из канала {} не разобран: {} (начало: {:?})",
                    channel,
                    error,
                    raw[start..].chars().take(120).collect::<String>()
                );
                continue;
            }
        };
        let Some(object) = parsed.as_object() else {
            last_error = format!("ответ из канала {} — не объект", channel);
            continue;
        };
        let mut verdicts = BTreeMap::new();
        let mut missing = Vec::new();
        for rule in rules {
            match object.get(&rule.id).and_then(Value::as_bool) {
                Some(flag) => {
                    verdicts.insert(rule.id.clone(), flag);
                }
                None => missing.push(rule.id.clone()),
            }
        }
        if !missing.is_empty() {
            last_error = format!("нет boolean-ключей: {}", missing.join(", "));
            continue;
        }
        return Ok(verdicts);
    }
    let where_brace = channels
        .first_object_brace()
        .map(|(in_content, index)| {
            format!(
                " (символ '{{' есть в канале {} на позиции {})",
                if in_content { "content" } else { "reasoning" },
                index
            )
        })
        .unwrap_or_default();
    Err(format!(
        "оба канала без пригодного JSON: content={} симв., reasoning={} симв.{}",
        channels.content.chars().count(),
        channels.reasoning.chars().count(),
        where_brace,
    ))
}

/// Счёт по полям: `(TP + TN) / полей`.
///
/// `None` — у кейса нет разметки элементов (набор метафор): там проверяется сам
/// факт «элементы найдены», а не поля.
///
/// Отсутствующий ключ в вердиктах — **промах**, а не «false»: иначе модель,
/// выдавшая неполный объект, получила бы полный балл за TN. Такой промах
/// засчитывается как FP (модель заявила элемент, которого нет), а ожидаемый
/// `true` без ключа — как FN.
#[allow(clippy::type_complexity)]
pub fn score_fields(
    expected: Option<&BTreeMap<String, bool>>,
    verdicts: &BTreeMap<String, bool>,
) -> Option<(usize, usize, usize, usize, usize, usize)> {
    let expected = expected?;
    let mut correct = 0usize;
    let (mut tp, mut tn, mut fp, mut fn_) = (0usize, 0usize, 0usize, 0usize);
    for (element, want) in expected {
        let got = verdicts.get(element).copied();
        if got == Some(*want) {
            correct += 1;
        }
        match (want, got) {
            (true, Some(true)) => tp += 1,
            (false, Some(false)) => tn += 1,
            (false, Some(true)) => fp += 1,
            (false, None) => fp += 1,
            (true, _) => fn_ += 1,
        }
    }
    Some((correct, expected.len(), tp, tn, fp, fn_))
}

/// Нулевой счёт для кейса, который вообще не разобран.
///
/// **Ошибка разбора обязана попадать в знаменатель.** Раньше проваленный кейс
/// просто не попадал в `fields`, и точность считалась по уцелевшим — провал 41
/// из 63 кейсов при этом давал бы 73.2% вместо честных ~30% (core rules §2.2:
/// молчаливый «успех» вместо правды).
///
/// Пустой `verdicts` даёт `0` верных полей: элемент, который не назван, не
/// может быть назван правильно.
pub fn score_failed_case(
    expected: &BTreeMap<String, bool>,
) -> (usize, usize, usize, usize, usize, usize) {
    let (_, _, _, _, _, fn_) = score_fields(Some(expected), &BTreeMap::new())
        .expect("разметка передана явно");
    (0, expected.len(), 0, 0, 0, fn_)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str) -> System1Rule {
        System1Rule {
            id: id.to_string(),
            name: "n".to_string(),
            question: "q".to_string(),
            true_criteria: "t".to_string(),
            false_criteria: "f".to_string(),
            threshold: None,
        }
    }

    #[test]
    fn verdict_prompt_carries_no_asymmetric_directive() {
        let rules = vec![rule("e1")];
        let prompt = build_verdict_prompt(&rules);

        // Каноничный блок боевого экстрактора — обязателен и берётся из него же.
        assert!(
            prompt.contains(super::super::fact_extractor::CRITERIA_INTERPRETATION_BLOCK),
            "в промпте System-2 нет каноничного блока трактовки критериев"
        );
        // Асимметричного перекоса в `false` быть не должно: он сдвинул замер
        // System-2 на FN=110 из 234 полей, не отражая боевого поведения.
        for forbidden in [
            "сомневаешься",
            "сомневаешься — false",
            "прямое подтверждение",
            "только если в тексте есть",
        ] {
            assert!(
                !prompt.contains(forbidden),
                "в промпте осталась директива перекоса {:?}: {}",
                forbidden,
                prompt
            );
        }
    }

    #[test]
    fn grammar_contains_every_rule_id_and_think_budget() {
        let rules = vec![rule("e1"), rule("e2")];
        let plain = build_verdict_grammar(&rules, None);
        assert!(plain.contains("\\\"e1\\\""), "в грамматике нет ключа e1");
        assert!(!plain.contains("<think>"), "безбюджетная грамматика не должна иметь think");

        let thinking = build_verdict_grammar(&rules, Some(120));
        assert!(thinking.contains("<think>"));
        // Граница размышления обязана быть в грамматике: без неё модель снова
        // выжигает весь n_predict до JSON (см. DEFAULT_THINK_BUDGET_CHARS).
        assert!(
            thinking.contains("{0,120}"),
            "в грамматике нет границы think-блока: {}",
            thinking.lines().nth(1).unwrap_or_default()
        );
    }

    #[test]
    fn parse_verdicts_rejects_incomplete_json() {
        let rules = vec![rule("e1"), rule("e2")];
        let incomplete = RawChannels {
            content: "{\"e1\": true}".to_string(),
            reasoning: String::new(),
        };
        assert!(parse_verdicts(&incomplete, &rules).is_err());

        let both = RawChannels {
            content: "<think>разбор</think> {\"e1\": true, \"e2\": false}".to_string(),
            reasoning: String::new(),
        };
        let ok = parse_verdicts(&both, &rules).expect("полный JSON разобран");
        assert_eq!(ok.get("e1"), Some(&true));
        assert_eq!(ok.get("e2"), Some(&false));

        // JSON в канале размышлений (reasoning_content) — тоже валидный ответ.
        let from_reasoning = RawChannels {
            content: String::new(),
            reasoning: "думаю… {\"e1\": false, \"e2\": true}".to_string(),
        };
        let parsed = parse_verdicts(&from_reasoning, &rules).expect("reasoning");
        assert_eq!(parsed.get("e1"), Some(&false));
    }

    /// Сообщение об ошибке обязано называть РАЗМЕРЫ каналов: иначе «нет `{`» —
    /// утверждение без доказательства (проверялось на реальном провале).
    #[test]
    fn parse_error_names_channel_sizes() {
        let rules = vec![rule("e1")];
        let truncated = RawChannels {
            content: "{".to_string(),
            reasoning: "<think>долгое размышление без конца".to_string(),
        };
        let error = parse_verdicts(&truncated, &rules).expect_err("обрезанный JSON не годен");
        assert!(error.contains("content=1"), "нет размера content: {}", error);
        assert!(
            error.contains(&format!("reasoning={}", truncated.reasoning.chars().count())),
            "нет размера reasoning: {}",
            error
        );
        assert!(
            error.contains("позиции 0"),
            "не сказано, что brace есть в content: {}",
            error
        );
    }

    #[test]
    fn score_counts_missing_key_as_error_not_as_true_negative() {
        let expected: BTreeMap<String, bool> = [("e1".to_string(), false), ("e2".to_string(), false)]
            .into_iter()
            .collect();
        let (correct, fields, _, tn, fp, _) =
            score_fields(Some(&expected), &BTreeMap::new()).expect("разметка есть");
        assert_eq!((correct, fields, tn, fp), (0, 2, 0, 2));
        assert!(score_fields(None, &BTreeMap::new()).is_none(), "нет разметки — не счёт");
    }

    /// Кейс, который не разобран, обязан дать 0 из N и остаться в знаменателе.
    #[test]
    fn failed_case_scores_zero_but_keeps_its_fields() {
        let expected: BTreeMap<String, bool> = [
            ("e1".to_string(), true),
            ("e2".to_string(), false),
            ("e3".to_string(), true),
        ]
        .into_iter()
        .collect();
        let (correct, fields, _, _, _, fn_) = score_failed_case(&expected);
        assert_eq!(correct, 0, "нераспарсенный кейс не может дать верных полей");
        assert_eq!(fields, 3, "поля кейса обязаны попасть в знаменатель");
        assert_eq!(fn_, 2, "два ожидаемых true без ответа — это FN");
    }
}
