//! Quote-first механический гейт заземления true-вердиктов слотов.
//!
//! Доказанный класс отказа (батчи 01.11.26, `custom_report_2026-11-01_20-49-35`
//! и `_20-53-30`): деградировавшее тело слота не содержит ни одного дословного
//! фрагмента жалобы, а булев токен выбирается по приоре модели — «весь-true»
//! на пустом месте (LFM2.5: 5–8 true-слотов без единой цитаты). Замер по этим
//! же батчам: заземлённые слоты Орнита содержат 6–14 подряд слов кейса,
//! конфабуляции — 0.
//!
//! Механика (§00.2 — конструктивная проверка, а не порог наLuck): слот с
//! вердиктом `true` обязан содержать непрерывный дословный фрагмент текста
//! кейса длиной >= [`QUOTE_MIN_WORDS`] слов. Нет цитаты — true-вердикт
//! структурно невалиден, гейт считает его провалом (гейт `Q1_ungrounded_true_slots`
//! в книге приёмки, `at_most 0`, объявлен ДО решающего прогона).
//!
//! Границы честно (замер 01.11.26): гейт НЕ ловит класс «дословная цитата есть,
//! классификация неверна» (Qwen3.8 — его цитаты вербатимны) и класс «весь-false»
//! (нулевой приор NeoHorse на либидо: true-слотов нет — проверять нечего).
//! Эти классы держат recall-гейты (O3/O4/G4) и few-shot (отдельный шаг плана).

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::domain::parsers::extract_think_content;

/// Минимальная длина дословного фрагмента жалобы (в словах), которой достаточно,
/// чтобы считать true-вердикт заземлённым. Структурная константа, зафиксирована
/// ДО решающего прогона (§00.3.1, `tasks/01.11.26 Quote-first гейт заземления
/// true-слотов.md`): 4 подряд слова из текста кейса — цитата; 3 и меньше
/// совпадают случайно («у меня есть»). Запас до измеренных значений широкий:
/// заземлённые тела дают 6–14 слов, конфабуляции — 0.
pub const QUOTE_MIN_WORDS: usize = 4;

/// Провал гейта на одном слоте: вердикт `true` без дословного фрагмента жалобы.
#[derive(Debug, Clone, PartialEq)]
pub struct QuoteFailure {
    /// Элемент вердикта (`e6`).
    pub element: String,
    /// Длина самой длинной дословной последовательности слов кейса в теле.
    pub span_words: usize,
    /// Тело слота отсутствует или пусто (деградация сильнее, чем обрез).
    pub body_missing: bool,
}

impl QuoteFailure {
    pub fn to_json(&self, layer: &str) -> Value {
        json!({
            "layer": layer,
            "element": self.element,
            "span_words": self.span_words,
            "body_missing": self.body_missing,
        })
    }
}

/// Слова в нормализованной форме: нижний регистр, ё→е, только буквы и цифры.
fn normalized_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            let lower = ch.to_lowercase().next().unwrap_or(ch);
            current.push(if lower == 'ё' { 'е' } else { lower });
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Длина самой длинной непрерывной последовательности слов кейса, встречающейся
/// в теле дословно (ДП наибольшей общей подстроки двух последовательностей слов).
pub fn longest_verbatim_span(body: &str, case_text: &str) -> usize {
    let body = normalized_words(body);
    let case = normalized_words(case_text);
    if body.is_empty() || case.is_empty() {
        return 0;
    }
    // prev[j] — длина общего отрезка, оканчивающегося на (i-1, j-1).
    let mut prev = vec![0usize; case.len() + 1];
    let mut best = 0usize;
    for body_word in &body {
        let mut curr = vec![0usize; case.len() + 1];
        for (j, case_word) in case.iter().enumerate() {
            if body_word == case_word {
                let run = prev[j] + 1;
                curr[j + 1] = run;
                best = best.max(run);
            }
        }
        prev = curr;
    }
    best
}

/// Тела слотов из сырого ответа, ключ — номер элемента (`e6` → 6).
///
/// Разрез по грамматическим якорям слотов (`Element N: ` — GBNF-литерал из
/// `slot_anchor_labels`, «Грамматика ведёт тебя по слоту») и по русским
/// заголовкам боевого формата («Элемент N (…):»). Якорь — защищённая структура
/// грамматики, поэтому модель не может его пропустить; тело слота — think-блок
/// внутри сегмента (при `disable_reasoning=true` теги литеральные).
pub fn slot_bodies(text: &str, reasoning: &str) -> BTreeMap<u32, String> {
    let mut map = slot_bodies_from(text);
    if map.is_empty() {
        map = slot_bodies_from(reasoning);
    }
    map
}

fn slot_bodies_from(source: &str) -> BTreeMap<u32, String> {
    let mut map = BTreeMap::new();
    for (number, segment) in slot_segments(source) {
        let body = slot_body(&segment);
        map.entry(number).or_insert_with(|| body);
    }
    map
}

/// Тело слота: содержимое закрытого think-блока.
///
/// Пустой закрытый блок (`<think></think>`) — пустое тело: `extract_think_content`
/// пропускает пустые блоки и её фолбэк «незакрытый think» снял бы хвост сегмента
/// вместе с `</think>` (ловилось тестом `slot_bodies_split_by_grammar_anchors`).
/// Сам экстрактор не трогается — им живёт боевой путь.
fn slot_body(segment: &str) -> String {
    if let Ok(re) = regex::Regex::new(r"(?s)<think[^>]*>\s*</think\s*>") {
        if re.is_match(segment) {
            return String::new();
        }
    }
    extract_think_content(segment).join("\n\n").trim().to_string()
}

/// Разрез источника на (номер слота, сегмент от маркера до следующего).
fn slot_segments(source: &str) -> Vec<(u32, String)> {
    let chars: Vec<char> = source.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let needles: [&[char]; 2] = [&['e', 'l', 'e', 'm', 'e', 'n', 't'], &['э', 'л', 'е', 'м', 'е', 'н', 'т']];

    let mut marks: Vec<(usize, u32)> = Vec::new();
    for (start, number) in slot_marks(&lower, &needles) {
        marks.push((start, number));
    }
    marks.sort_by_key(|(pos, _)| *pos);
    marks.dedup_by(|a, b| a.0 == b.0);

    let mut out = Vec::new();
    for (i, (pos, number)) in marks.iter().enumerate() {
        let end = marks.get(i + 1).map(|(next, _)| *next).unwrap_or(chars.len());
        let segment: String = chars[*pos..end].iter().collect();
        out.push((*number, segment));
    }
    out
}

/// Позиции маркеров слотов: `Element N:` (строгий якорь грамматики — с двоеточием)
/// и `Элемент N` (боевой формат, двоеточие может идти после пояснения в скобках).
/// После ключевого слова обязана быть граница слова («элементе 3» — не маркер).
fn slot_marks(lower: &[char], needles: &[&[char]]) -> Vec<(usize, u32)> {
    let mut marks = Vec::new();
    for (start, _) in lower.iter().enumerate() {
        for needle in needles {
            if lower.len() < start + needle.len() || &lower[start..start + needle.len()] != *needle {
                continue;
            }
            let after_word = lower.get(start + needle.len()).copied();
            if after_word.map(|c| c.is_alphabetic()).unwrap_or(false) {
                continue; // «элементе», «elements» — прозa, не маркер
            }
            let mut idx = start + needle.len();
            while idx < lower.len() && lower[idx].is_whitespace() {
                idx += 1;
            }
            let digits_start = idx;
            let mut number: u32 = 0;
            while idx < lower.len() && lower[idx].is_ascii_digit() {
                number = number.saturating_mul(10).saturating_add(lower[idx] as u32 - '0' as u32);
                idx += 1;
            }
            if idx == digits_start || number == 0 || number > 9 {
                continue;
            }
            let strict_anchor = *needle == needles[0]; // английский якорь грамматики
            if strict_anchor {
                while idx < lower.len() && lower[idx].is_whitespace() {
                    idx += 1;
                }
                if lower.get(idx) != Some(&':') {
                    continue; // без двоеточия это не якорь слота, а проза
                }
            }
            marks.push((start, number));
            break;
        }
    }
    marks
}

/// Проверка заземления: каждый true-вердикт обязан опираться на тело слота,
/// содержащее дословный фрагмент жалобы. False-вердикт цитаты не требует
/// (элемент не найден — отсутствие цитаты корректно).
pub fn check_grounding(
    verdicts: &BTreeMap<String, bool>,
    bodies: &BTreeMap<u32, String>,
    case_text: &str,
) -> Vec<QuoteFailure> {
    let mut fails = Vec::new();
    for (element, flag) in verdicts {
        let number = element
            .strip_prefix('e')
            .and_then(|digits| digits.parse::<u32>().ok());
        let body = number
            .and_then(|n| bodies.get(&n))
            .map(String::as_str)
            .unwrap_or("");
        if body.trim().is_empty() {
            // Доказательство-мысль обязательно для КАЖДОГО вердикта: пустое тело
            // означает, что булев токен выбран по приоре, без разбора слота.
            fails.push(QuoteFailure {
                element: element.clone(),
                span_words: 0,
                body_missing: true,
            });
            continue;
        }
        // Дословная цитата — дополнительное требование только для true: для false
        // отсутствие цитаты — корректный ответ (элемент не найден в жалобе).
        if !*flag {
            continue;
        }
        let span = longest_verbatim_span(body, case_text);
        if span < QUOTE_MIN_WORDS {
            fails.push(QuoteFailure {
                element: element.clone(),
                span_words: span,
                body_missing: false,
            });
        }
    }
    fails.sort_by(|a, b| a.element.cmp(&b.element));
    fails
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASE_TEXT: &str = "Я мужчина. У меня есть какой-то психологический блок связанный с либидо \
        влечением к девушкам. Также у меня болят ноги от спазмов. У меня постоянно сонливость, \
        у меня бессонница.";

    fn verdicts(pairs: &[(&str, bool)]) -> BTreeMap<String, bool> {
        pairs
            .iter()
            .map(|(element, flag)| (element.to_string(), *flag))
            .collect()
    }

    #[test]
    fn grounded_true_verdict_passes() {
        let body = "Цитата пациента: «болят ноги от спазмов». evidence_test пройден: \
                    телесный симптом назван прямо. Вердикт true.";
        let span = longest_verbatim_span(body, CASE_TEXT);
        assert!(span >= QUOTE_MIN_WORDS, "заземлённое тело дало span {span}");
        let fails = check_grounding(&verdicts(&[("e1", true)]), &[(1u32, body.to_string())].into_iter().collect(), CASE_TEXT);
        assert!(fails.is_empty(), "заземлённый true не должен провалиться: {fails:?}");
    }

    #[test]
    fn confabulated_english_body_fails() {
        // Класс LFM2.5: англоязычная сводка без единого дословного фрагмента жалобы.
        let body = "The user describes a psychological block around attraction and reports \
                    psychosomatic metaphors. Thus all elements are confirmed true based on \
                    the user's description.";
        let fails = check_grounding(&verdicts(&[("e1", true)]), &[(1u32, body.to_string())].into_iter().collect(), CASE_TEXT);
        assert_eq!(fails.len(), 1, "конфабуляция обязана провалиться");
        assert_eq!(fails[0].element, "e1");
        assert_eq!(fails[0].span_words, 0);
        assert!(!fails[0].body_missing);
    }

    #[test]
    fn missing_body_fails() {
        let fails = check_grounding(&verdicts(&[("e6", true)]), &BTreeMap::new(), CASE_TEXT);
        assert_eq!(fails.len(), 1);
        assert!(fails[0].body_missing, "пустое тело — отдельная пометка");
    }

    #[test]
    fn false_verdict_needs_body_but_no_quote() {
        // false-слот обязан иметь разбор (тело), но цитата не требуется:
        // «элемент не найден в жалобе» — корректный ответ без дословного фрагмента.
        let body = "Разбор слота: пациент не называет нарушений режима сна и бодрствования, \
                    прямых указаний на этот элемент нет. Вердикт false.";
        let fails = check_grounding(
            &verdicts(&[("e1", false)]),
            &[(1u32, body.to_string())].into_iter().collect(),
            CASE_TEXT,
        );
        assert!(fails.is_empty(), "false-вердикт с разбором без цитаты — ок: {fails:?}");
    }

    #[test]
    fn false_verdict_with_empty_body_fails() {
        // Каждый вердикт обязан иметь доказательство-мысль: пустой false-слот —
        // булев токен по приоре, тот же дефект, что и пустой true-слот.
        let fails = check_grounding(&verdicts(&[("e6", false)]), &BTreeMap::new(), CASE_TEXT);
        assert_eq!(fails.len(), 1, "пустое тело false-слота обязано провалиться");
        assert_eq!(fails[0].element, "e6");
        assert!(fails[0].body_missing, "пустое тело — отдельная пометка");
        assert_eq!(fails[0].span_words, 0);
    }

    #[test]
    fn mixed_verdicts_report_only_broken_slots() {
        // Заземлённый true + разобранный false без цитаты + пустой false:
        // провалится ровно пустой слот.
        let grounded = "Цитата пациента: «болят ноги от спазмов». Телесный симптом назван прямо.";
        let reasoned_false = "Разбор: прямых указаний на нарушения режима нет, вердикт false.";
        let fails = check_grounding(
            &verdicts(&[("e1", true), ("e6", false), ("e9", false)]),
            &[
                (1u32, grounded.to_string()),
                (6u32, reasoned_false.to_string()),
                (9u32, String::new()),
            ]
            .into_iter()
            .collect(),
            CASE_TEXT,
        );
        assert_eq!(fails.len(), 1, "ровно пустой слот проваливается: {fails:?}");
        assert_eq!(fails[0].element, "e9");
        assert!(fails[0].body_missing);
    }

    #[test]
    fn short_function_word_overlap_does_not_ground() {
        // «у меня есть» — 3 случайных слова службы, цитатой не считается (< 4).
        let body = "у меня есть ощущение, что тут что-то ещё.";
        let fails = check_grounding(&verdicts(&[("e2", true)]), &[(2u32, body.to_string())].into_iter().collect(), CASE_TEXT);
        assert_eq!(fails.len(), 1);
        assert!(fails[0].span_words < QUOTE_MIN_WORDS);
    }

    #[test]
    fn normalization_ignores_case_yo_and_punctuation() {
        let body = "Пациент: «БОЛЯТ НОГИ ОТ СПАЗМОВ», далее разбор.";
        assert!(longest_verbatim_span(body, CASE_TEXT) >= QUOTE_MIN_WORDS);
        let body_yo = "пишет: «болят ноги от спазмоЁ» — оговорка в теле.";
        assert!(longest_verbatim_span(body_yo, CASE_TEXT) < QUOTE_MIN_WORDS);
    }

    #[test]
    fn slot_bodies_split_by_grammar_anchors() {
        // Якорь `Element N: ` — GBNF-литерал слотовой грамматики; think-теги
        // литеральные (disable_reasoning=true).
        let text = "Element 1: <think>цитата: болят ноги от спазмов</think> \"e1\": true \
                    Element 6: <think></think> \"e6\": false";
        let bodies = slot_bodies(text, "");
        assert_eq!(bodies.get(&1).map(String::as_str), Some("цитата: болят ноги от спазмов"));
        assert_eq!(bodies.get(&6).map(String::as_str), Some(""));
    }

    #[test]
    fn slot_bodies_fall_back_to_reasoning_channel() {
        let reasoning = "Element 2: <think>пациент называет «психологический блок» прямо</think> \"e2\": true";
        let bodies = slot_bodies("текст без якорей", reasoning);
        assert!(bodies.contains_key(&2));
    }

    #[test]
    fn slot_bodies_read_russian_battle_headers() {
        // Боевой формат: «Элемент 1 (Сопротивление): …» — двоеточие после скобки.
        let text = "Элемент 1 (Сопротивление): <think>цитата: психологический блок связанный с либидо</think>";
        let bodies = slot_bodies(text, "");
        assert!(bodies.contains_key(&1));
        assert!(bodies[&1].contains("психологический блок"));
    }

    #[test]
    fn english_word_without_colon_is_not_an_anchor() {
        // Проза «Element 3 устроен иначе» (как в validator_core.md) не должна
        // разрезать тело слота: строгий якорь требует двоеточия.
        let text = "Element 1: <think>Element 3 устроен иначе, цитата: болят ноги от спазмов</think> \"e1\": true";
        let bodies = slot_bodies(text, "");
        assert_eq!(bodies.len(), 1, "проза не разрезает слот");
        assert!(bodies[&1].contains("болят ноги от спазмов"));
    }

    #[test]
    fn russian_prose_with_case_form_is_not_an_anchor() {
        let text = "Элемент 1: <think>сверился с элементом 3, цитата: болят ноги от спазмов</think>";
        let bodies = slot_bodies(text, "");
        assert_eq!(bodies.len(), 1, "«элементе» — не маркер (граница слова)");
    }
}
