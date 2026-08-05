/// Безопасное вычисление diff для стриминговой детокенизации.
///
/// `from_utf8_lossy` пересчитывает всю строку с нуля на каждом токене.
/// Когда неполная UTF-8 последовательность (замещена на U+FFFD, 3 байта)
/// разрешается в реальный символ (< 3 байт), итоговая строка может стать
/// КОРОЧЕ предыдущей. Прямой slicing `current[prev.len()..]` в таком
/// случае паникует с "byte index out of bounds".
///
/// Эта функция находит точку расхождения через символьное сравнение
/// и безопасно возвращает остаток строки.
pub fn compute_stream_diff<'a>(current_text: &'a str, prev_text: &str) -> &'a str {
    // Быстрый путь (99% токенов): строка выросла и начала совпадает
    if prev_text.len() <= current_text.len()
        && current_text.starts_with(prev_text)
    {
        return &current_text[prev_text.len()..];
    }

    // Медленный путь: from_utf8_lossy укоротил строку (U+FFFD → реальный символ)
    // или текст полностью изменился. Находим точку расхождения по символам.
    let diverge_chars = current_text
        .chars()
        .zip(prev_text.chars())
        .take_while(|(a, b)| a == b)
        .count();

    let byte_pos = current_text
        .char_indices()
        .nth(diverge_chars)
        .map(|(i, _)| i)
        .unwrap_or(current_text.len());

    &current_text[byte_pos..]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Нормальные случаи ──

    #[test]
    fn normal_grow() {
        assert_eq!(compute_stream_diff("Hello, world!", "Hello"), ", world!");
    }

    #[test]
    fn identical_empty_diff() {
        assert_eq!(compute_stream_diff("abc", "abc"), "");
    }

    #[test]
    fn prev_empty() {
        assert_eq!(compute_stream_diff("abc", ""), "abc");
    }

    #[test]
    fn both_empty() {
        assert_eq!(compute_stream_diff("", ""), "");
    }

    #[test]
    fn single_char_append() {
        assert_eq!(compute_stream_diff("ab", "a"), "b");
    }

    // ── Edge case: from_utf8_lossy укорачивает строку ──

    #[test]
    fn utf8_replacement_resolved_single() {
        // U+FFFD = 3 байта (EF BF BD), "С" = 2 байта (D0 A1)
        let prev = format!("Hello {}", '\u{FFFD}'); // "Hello �" = 9 байт
        let curr = "Hello С";                       // "Hello С" = 8 байт
        // prev.len() (9) > curr.len() (8) → fallback
        assert_eq!(compute_stream_diff(&curr, &prev), "С");
    }

    #[test]
    fn utf8_replacement_resolved_multiple() {
        let prev = format!("{}{}", '\u{FFFD}', '\u{FFFD}'); // 6 байт
        let curr = "Си";                                     // 4 байта
        assert_eq!(compute_stream_diff(&curr, &prev), "Си");
    }

    #[test]
    fn utf8_replacement_in_middle() {
        let prev = format!("A{}B", '\u{FFFD}'); // "A�B" = 5 байт
        let curr = "АB";                         // "АB" = 3 байта (А = 2 байта)
        // Расхождение на позиции 0: 'A' != 'А'
        assert_eq!(compute_stream_diff(&curr, &prev), "АB");
    }

    #[test]
    fn utf8_replacement_after_valid_text() {
        let prev = format!("Привет {}", '\u{FFFD}'); // "Привет �" = 13 байт
        let curr = "Привет С";                       // "Привет С" = 12 байт
        assert_eq!(compute_stream_diff(&curr, &prev), "С");
    }

    // ── Полностью разный текст ──

    #[test]
    fn completely_different() {
        assert_eq!(compute_stream_diff("xyz", "abc"), "xyz");
    }

    // ── Кириллица (основной кейс бага) ──

    #[test]
    fn cyrillic_token_splitting() {
        // Модель генерирует "SSD" посимвольно, каждый символ — 2 байта
        let mut generated_bytes: Vec<u8> = Vec::new();
        let mut result_text = String::new();

        // Токен 1: первый байт "С" (0xD0) — неполный UTF-8
        generated_bytes.extend_from_slice(&[0xD0]);
        let current_text = String::from_utf8_lossy(&generated_bytes).into_owned();
        let diff = compute_stream_diff(&current_text, &result_text).to_string();
        assert_eq!(diff, "\u{FFFD}"); // замещающий символ
        result_text = current_text;

        // Токен 2: второй байт "С" (0xA1) — теперь полный символ
        generated_bytes.extend_from_slice(&[0xA1]);
        let current_text = String::from_utf8_lossy(&generated_bytes).into_owned();
        // current_text = "С" (2 байта), result_text = "�" (3 байта)
        // Старый код: current_text[3..] → PANIC!
        // Новый код: compute_stream_diff находит расхождение
        let diff = compute_stream_diff(&current_text, &result_text).to_string();
        assert_eq!(diff, "С");
        result_text = current_text;

        // Токен 3: " " (0x20)
        generated_bytes.extend_from_slice(&[0x20]);
        let current_text = String::from_utf8_lossy(&generated_bytes).into_owned();
        let diff = compute_stream_diff(&current_text, &result_text).to_string();
        assert_eq!(diff, " ");
        result_text = current_text;

        // Токены 4-5: "Н" (0xD0, 0x9D)
        generated_bytes.extend_from_slice(&[0xD0]);
        let current_text = String::from_utf8_lossy(&generated_bytes).into_owned();
        let diff = compute_stream_diff(&current_text, &result_text).to_string();
        assert_eq!(diff, "\u{FFFD}");
        result_text = current_text;

        generated_bytes.extend_from_slice(&[0x9D]);
        let current_text = String::from_utf8_lossy(&generated_bytes).into_owned();
        let diff = compute_stream_diff(&current_text, &result_text).to_string();
        assert_eq!(diff, "Н");
        result_text = current_text;

        assert_eq!(result_text, "С Н");
    }
}
