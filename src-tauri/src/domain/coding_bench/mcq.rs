// mcq-задачи (Группа A): извлечение ответа модели из текста и сверка с эталоном.
// Форма ответа задаётся промптом задачи (якорь «Answer:» / «#### N»); извлечение —
// двухъярусное, непарсируемый ответ — честный Err (провал задачи), а не угадывание (§00.2).
use regex::Regex;

use super::tasks::CodingTask;

/// Срезает think-блок reasoning-моделей, чтобы буквы/числа из рассуждений не
/// подмешивались к извлечению. Незакрытый think → пустой текст (извлечение
/// ниже честно провалится — «думатель без ответа» не должен выдавать ответ).
fn strip_think(text: &str) -> &str {
    match text.find("<think>") {
        None => text,
        Some(start) => match text[start..].find("</think>") {
            Some(rel) => &text[start + rel + "</think>".len()..],
            None => "",
        },
    }
}

/// Буква в допустимом диапазоне вариантов (A..N)?
fn letter_in_range(c: char, num_choices: usize) -> bool {
    (c as u8) >= b'A' && ((c as u8) - b'A') < num_choices as u8
}

/// Последний матч регэкспа → его последний символ (захват группы в конце шаблона),
/// поднятый до заглавной и отфильтрованный по диапазону вариантов.
fn last_letter_in_range(re: &Regex, visible: &str, num_choices: usize) -> Option<char> {
    re.find_iter(visible)
        .filter_map(|m| m.as_str().chars().last())
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| letter_in_range(*c, num_choices))
        .last()
}

/// Извлекает букву ответа из текста модели.
/// Ярус 1 — явные маркеры («answer is B», «Ответ: B», «option (B)», «**B**» после
/// «answer:») — последнее вхождение; ярус 2 — последняя standalone-заглавная буква
/// текста. Буква вне диапазона A..N или её отсутствие — честный Err.
pub fn extract_letter(text: &str, num_choices: usize) -> Result<char, String> {
    if num_choices < 2 || num_choices > 26 {
        return Err(format!("Недопустимое число вариантов: {}", num_choices));
    }
    let visible = strip_think(text);
    let marker = Regex::new(
        r"(?i)(?:answer|ответ|option|вариант)\s*(?:is|:|=|—|–|-)?\s*\(?\*{0,2}([A-Z])\b",
    )
    .map_err(|e| format!("Ошибка регэкспа маркера: {}", e))?;
    let standalone = Regex::new(r"\b[A-Z]\b").map_err(|e| format!("Ошибка регэкспа: {}", e))?;

    last_letter_in_range(&marker, visible, num_choices)
        .or_else(|| last_letter_in_range(&standalone, visible, num_choices))
        .ok_or_else(|| "Не удалось извлечь букву ответа".to_string())
}

/// Приводит число к каноничному виду: без пробелов/$/%/запятых, без хвостовой
/// точки и ведущего «+» («$1,000.50» → «1000.50», «42.» → «42»).
fn normalize_number(s: &str) -> String {
    let mut t = s.trim().to_string();
    for ch in [' ', ',', '$', '%'] {
        t = t.replace(ch, "");
    }
    if t.ends_with('.') {
        t.pop();
    }
    if let Some(stripped) = t.strip_prefix('+') {
        t = stripped.to_string();
    }
    t
}

/// Числа равны? Сначала строковое сравнение каноничных форм, затем числовое
/// (f64) — «42» ≡ «42.0», «1,234» ≡ «1234».
fn numbers_equal(a: &str, b: &str) -> bool {
    let (na, nb) = (normalize_number(a), normalize_number(b));
    if na == nb {
        return true;
    }
    match (na.parse::<f64>(), nb.parse::<f64>()) {
        (Ok(x), Ok(y)) => (x - y).abs() < 1e-9,
        _ => false,
    }
}

/// Последняя группа регэкспа (с нормализацией числа) — последний матч.
/// Регэксп без capture-групп (фолбэк «последнее число») отдаёт весь матч —
/// иначе `c[1]` паникует «no group at index '1'» (поймано cargo 07.11.26).
fn last_capture(re: &Regex, visible: &str) -> Option<String> {
    re.captures_iter(visible)
        .last()
        .map(|c| {
            normalize_number(
                c.get(1)
                    .or_else(|| c.get(0))
                    .map(|m| m.as_str())
                    .unwrap_or_default(),
            )
        })
}

/// Извлекает числовой ответ из текста модели.
/// Ярус 1 — маркер «#### N» (GSM8K-формат); ярус 2 — «answer is N»; ярус 3 —
/// последнее число в тексте. Чисел нет — честный Err.
pub fn extract_number(text: &str) -> Result<String, String> {
    let visible = strip_think(text);
    let num = r"-?[0-9][0-9,]*(?:\.[0-9]+)?";
    let hash = Regex::new(&format!(r"###+\s*\$?({})", num))
        .map_err(|e| format!("Ошибка регэкспа: {}", e))?;
    let answer_is =
        Regex::new(&format!(r"(?i)(?:answer|ответ)\D{{0,20}}?({})", num))
            .map_err(|e| format!("Ошибка регэкспа: {}", e))?;
    let any_num = Regex::new(num).map_err(|e| format!("Ошибка регэкспа: {}", e))?;

    last_capture(&hash, visible)
        .or_else(|| last_capture(&answer_is, visible))
        .or_else(|| last_capture(&any_num, visible))
        .ok_or_else(|| "Не удалось извлечь числовой ответ".to_string())
}

/// Оценивает mcq-ответ модели по задаче: диспетчер по mcq_mode.
/// Возвращает (прошла, деталь) — деталь попадает в record.stdout отчёта.
/// Битая задача (нет answer/choices, неизвестный mode) — честный Err.
pub fn grade_mcq(task: &CodingTask, completion: &str) -> Result<(bool, String), String> {
    let expected = task
        .answer
        .as_deref()
        .ok_or_else(|| "У mcq-задачи нет поля answer".to_string())?
        .trim()
        .to_string();
    match task.mcq_mode.as_deref() {
        Some("letter") | None => {
            let choices = task
                .choices
                .as_ref()
                .ok_or_else(|| "У mcq-задачи (letter) нет поля choices".to_string())?;
            if choices.len() < 2 {
                return Err(format!("Меньше двух вариантов: {}", choices.len()));
            }
            let expected_letter = expected.to_ascii_uppercase();
            let expected_char = expected_letter
                .chars()
                .next()
                .filter(|c| expected_letter.len() == 1 && letter_in_range(*c, choices.len()))
                .ok_or_else(|| format!("answer '{}' не буква в диапазоне выбора", expected))?;
            let letter = extract_letter(completion, choices.len())?;
            let passed = letter == expected_char;
            Ok((passed, format!("Извлечено: {}; верно: {}", letter, expected_char)))
        }
        Some("number") => {
            let got = extract_number(completion)?;
            let passed = numbers_equal(&got, &expected);
            Ok((passed, format!("Извлечено: {}; верно: {}", got, expected)))
        }
        Some(other) => Err(format!("Неизвестный mcq_mode '{}'", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- extract_letter ----------

    #[test]
    fn letter_standalone() {
        assert_eq!(extract_letter("B", 4).unwrap(), 'B');
        assert_eq!(extract_letter("  C  ", 4).unwrap(), 'C');
    }

    #[test]
    fn letter_marker_answer_is() {
        assert_eq!(extract_letter("The answer is C.", 4).unwrap(), 'C');
        assert_eq!(extract_letter("The correct answer is: C. Because...", 4).unwrap(), 'C');
        assert_eq!(extract_letter("Ответ: A", 4).unwrap(), 'A');
        assert_eq!(extract_letter("option (B) seems right", 4).unwrap(), 'B');
        assert_eq!(extract_letter("answer: **D**", 4).unwrap(), 'D');
    }

    #[test]
    fn letter_last_marker_wins() {
        assert_eq!(
            extract_letter("First I thought A, but the answer is D.", 4).unwrap(),
            'D'
        );
    }

    #[test]
    fn letter_pronoun_i_then_choice() {
        // «I» — местоимение, не вариант ответа (при 4 вариантах I вне диапазона).
        assert_eq!(extract_letter("I think B", 4).unwrap(), 'B');
    }

    #[test]
    fn letter_think_stripped() {
        assert_eq!(extract_letter("<think>maybe A? or C?</think>B", 4).unwrap(), 'B');
        // Незакрытый think — извлечение честно проваливается.
        assert!(extract_letter("<think>maybe A? or C?", 4).is_err());
    }

    #[test]
    fn letter_out_of_range_is_err() {
        // Модель ответила E при 4 вариантах — честный провал, не угадывание.
        assert!(extract_letter("E", 4).is_err());
        // Но при 5 вариантах E валиден.
        assert_eq!(extract_letter("E", 5).unwrap(), 'E');
    }

    #[test]
    fn letter_no_letter_is_err() {
        assert!(extract_letter("The answer is forty-two.", 4).is_err());
        assert!(extract_letter("", 4).is_err());
    }

    #[test]
    fn letter_choices_count_bounds() {
        assert!(extract_letter("B", 1).is_err());
        assert!(extract_letter("B", 27).is_err());
        // 2 варианта: C недопустима.
        assert!(extract_letter("C", 2).is_err());
        assert_eq!(extract_letter("B", 2).unwrap(), 'B');
    }

    #[test]
    fn letter_lowercase_answer_after_marker() {
        // Модель ответила строчной: маркер поднимает до заглавной.
        assert_eq!(extract_letter("answer is b", 4).unwrap(), 'B');
    }

    // ---------- extract_number ----------

    #[test]
    fn number_hash_marker() {
        assert_eq!(extract_number("reasoning 7 x 6\n#### 42").unwrap(), "42");
        assert_eq!(extract_number("#### 1,234").unwrap(), "1234");
    }

    #[test]
    fn number_answer_is_marker() {
        assert_eq!(extract_number("The answer is 1,234.").unwrap(), "1234");
        assert_eq!(extract_number("Ответ: 42").unwrap(), "42");
    }

    #[test]
    fn number_last_number_fallback() {
        assert_eq!(extract_number("We add 2 and 3 to get 5").unwrap(), "5");
        assert_eq!(extract_number("$1,000.50").unwrap(), "1000.50");
    }

    #[test]
    fn number_negative_and_decimal() {
        assert_eq!(extract_number("#### -3").unwrap(), "-3");
        assert_eq!(extract_number("The answer is 0.5").unwrap(), "0.5");
    }

    #[test]
    fn number_no_number_is_err() {
        assert!(extract_number("no digits here").is_err());
        assert!(extract_number("").is_err());
    }

    #[test]
    fn number_think_stripped() {
        assert_eq!(
            extract_number("<think>maybe 7?</think>The answer is 42.").unwrap(),
            "42"
        );
    }

    // ---------- numbers_equal ----------

    #[test]
    fn equal_numeric_forms() {
        assert!(numbers_equal("42", "42.0"));
        assert!(numbers_equal("1234", "1,234"));
        assert!(numbers_equal("0.5", "0.50"));
        assert!(numbers_equal("0.5", "0.75") == false);
    }

    #[test]
    fn not_equal_numbers() {
        assert!(!numbers_equal("100", "10"));
        assert!(!numbers_equal("42", "43"));
    }

    // ---------- grade_mcq ----------

    use crate::domain::coding_bench::tasks::CodingTask;

    fn mcq_task(mode: Option<&str>, answer: &str, choices: Option<Vec<&str>>) -> CodingTask {
        serde_json::from_str(&format!(
            r#"{{"id":"t","suite":"s","language":"en","category":"mcq","run_with":"mcq",
            "model_prompt":"p","solution_name":"answer.txt","answer":"{}","mcq_mode":{},
            "choices":{} }}"#,
            answer,
            match mode {
                Some(m) => format!("\"{}\"", m),
                None => "null".to_string(),
            },
            match choices {
                Some(v) => format!("[{}]", v.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(",")),
                None => "null".to_string(),
            }
        ))
        .unwrap()
    }

    #[test]
    fn grade_letter_correct_and_wrong() {
        let t = mcq_task(Some("letter"), "C", Some(vec!["a", "b", "c", "d"]));
        let (passed, detail) = grade_mcq(&t, "The answer is C.").unwrap();
        assert!(passed);
        assert!(detail.contains('C'));
        let (passed, _) = grade_mcq(&t, "B").unwrap();
        assert!(!passed);
    }

    #[test]
    fn grade_number_numeric_equivalence() {
        let t = mcq_task(Some("number"), "42", None);
        let (passed, _) = grade_mcq(&t, "#### 42.0").unwrap();
        assert!(passed);
        let (passed, _) = grade_mcq(&t, "#### 43").unwrap();
        assert!(!passed);
    }

    #[test]
    fn grade_honest_errors() {
        // Нет answer
        let t: CodingTask = serde_json::from_str(
            r#"{"id":"t","suite":"s","language":"en","category":"mcq","run_with":"mcq",
            "model_prompt":"p","solution_name":"answer.txt"}"#,
        )
        .unwrap();
        assert!(grade_mcq(&t, "B").is_err());
        // letter без choices
        let t2 = mcq_task(Some("letter"), "B", None);
        assert!(grade_mcq(&t2, "B").is_err());
        // Неизвестный mode
        let t3 = mcq_task(Some("boolean"), "true", None);
        assert!(grade_mcq(&t3, "true").is_err());
        // answer вне диапазона
        let t4 = mcq_task(Some("letter"), "Z", Some(vec!["a", "b"]));
        assert!(grade_mcq(&t4, "B").is_err());
    }

    #[test]
    fn grade_mode_defaults_to_letter() {
        // mcq_mode не задан, choices есть → letter.
        let t = mcq_task(None, "A", Some(vec!["x", "y"]));
        let (passed, _) = grade_mcq(&t, "Answer: A").unwrap();
        assert!(passed);
    }
}
