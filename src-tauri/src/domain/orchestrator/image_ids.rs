//! Терпимый ресолв id изображений (`hist_*` / `req_*`) из свободного текста модели.
//!
//! Наблюдение из `test/last_logs.txt` (сессия 21.10.2026 00:51–00:57): модель
//! протащила в `source_image_ids` три искажённых варианта корректного id —
//! `hist_26dba55999405f91` (лишняя цифра), `hist_26dba559405f91` (потерянная
//! пара), `hist_482db67_8441a1ec1` (подчёркивание внутри hex + «ошибка хвоста»)
//! — и трижды получила «Неизвестный source_image_id» несмотря на то, что id был
//! в её же контексте.
//!
//! Решается это доменом id: каноничный вид = строчные ASCII-буквы/цифры
//! (нижнее подчёркивание, скобки, кавычки, точки и прочие «рамки» выкидываются
//! целиком, а не частично). Стадии:
//! 1) точное совпадение каноничного вида;
//! 2) каноничный id встречается в ответе как подстрока (модель обрамила его
//!    текстом);
//! 3) расстояние Левенштейна ≤2 — ровно класс наблюдённых опечаток
//!    (1 вставка/удаление, 1 замена, соседняя перестановка).
//! Всё сверх этого — честный отказ со списком допустимых id, а не «возьмём
//! первый попавшийся». Двойной матч — отказ тоже: однозначность важнее
//! угадывания.

/// Максимальное расстояние Левенштейна для стадии 3.
///
/// Обоснование — не «порог качества», а класс опечаток по каноничному виду:
/// 1 замена, 1 вставка/удаление или соседняя перестановка (в чистом
/// Левенштейне = 2). Три правки и больше — это уже неидентифицируемо без
/// подгонки, фиксация отказа честнее.
pub(crate) const FUZZY_MAX_DISTANCE: usize = 2;

pub(crate) enum IdLookup<'a> {
    /// Ровно из каталога — вычесленное каноничное совпадение.
    Exact(&'a str),
    /// Каноничный id встречается внутри ответа модели (обрамлён текстом).
    Embedded(&'a str),
    /// Восстановлено по ≤2 простым правкам вокруг каноничного вида.
    Fixed { id: &'a str, distance: usize },
    /// Сразу несколько кандидатов подошли — безопасный выбор невозможен.
    Ambiguous { matches: Vec<&'a str> },
    /// Никто не подошёл — каноничный вид запроса вернём для диагноза.
    NotFound { canonical: String },
}

/// Каноничный вид: только строчные [a-z0-9] — всё прочее (рамки) выкидывается.
/// Контракт каталога: id = префикс + ASCII-hex (`hist_`, `req_`), поэтому
/// выкидывание «не-алфанумерики» не искажает ни один каноничный id, но
/// мигновенно чинит обрамления («(hist_xxx)», «id="hist_xxx"», «hist x .png»)
/// и лишние подчёркивания внутри hex — класс наблюдённых искажений.
pub(crate) fn canonical(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
        }
    }
    out
}

pub(crate) fn lookup_id<'a>(raw: &str, candidate_ids: &[&'a str]) -> IdLookup<'a> {
    let canonical_raw = canonical(raw);

    if let Some(id) = candidate_ids
        .iter()
        .copied()
        .find(|candidate| canonical(candidate) == canonical_raw)
    {
        return IdLookup::Exact(id);
    }

    let embedded: Vec<&'a str> = candidate_ids
        .iter()
        .copied()
        .filter(|candidate| canonical_raw.contains(&canonical(candidate)))
        .collect();
    if embedded.len() == 1 {
        return IdLookup::Embedded(embedded[0]);
    }
    if embedded.len() > 1 {
        return IdLookup::Ambiguous { matches: embedded };
    }

    // Стадия 3: Левенштейн (ранняя отсечка — по длине и по минимуму строки).
    let mut scored: Vec<(usize, &'a str)> = candidate_ids
        .iter()
        .copied()
        .filter_map(|candidate| {
            levenshtein(
                canonical_raw.as_bytes(),
                canonical(candidate).as_bytes(),
                FUZZY_MAX_DISTANCE,
            )
            .map(|distance| (distance, candidate))
        })
        .collect();
    if let Some((min_distance, id)) = scored
        .iter()
        .copied()
        .min_by_key(|(distance, _)| *distance)
    {
        let matches: Vec<&'a str> = scored
            .iter()
            .copied()
            .filter(|(distance, _)| *distance == min_distance)
            .map(|(_, candidate)| candidate)
            .collect();
        if matches.len() == 1 {
            return IdLookup::Fixed {
                id,
                distance: min_distance,
            };
        }
        return IdLookup::Ambiguous { matches };
    }

    IdLookup::NotFound {
        canonical: canonical_raw,
    }
}

/// Финализация стадий в «id или честный отказ» + диагностический лог
/// (правило AGENTS: причины правок и восстановления растворяются в логе).
/// Сообщения отказов строятся здесь, чтобы `resolve` каталога оставался
/// коротким, а текст ошибок жил рядом с их источником.
pub(crate) fn finalize_lookup<'a>(
    lookup: IdLookup<'a>,
    raw_request: &str,
    valid_ids: &str,
) -> Result<&'a str, String> {
    match lookup {
        IdLookup::Exact(id) => Ok(id),
        IdLookup::Embedded(id) => {
            log::warn!(
                "[media] source_image_id '{}' найден как вложение каноничного '{}' в тексте ответа",
                raw_request,
                id
            );
            Ok(id)
        }
        IdLookup::Fixed { id, distance } => {
            log::warn!(
                "[media] source_image_id '{}' восстановлен в '{}' (правок: {})",
                raw_request,
                id,
                distance
            );
            Ok(id)
        }
        IdLookup::Ambiguous { matches } => Err(format!(
            "source_image_id '{}' неоднозначен: подходят сразу несколько ID ({}). Уточни один из: {}",
            raw_request,
            matches.join(", "),
            valid_ids
        )),
        IdLookup::NotFound { canonical } => Err(format!(
            "Неизвестный source_image_id '{}' (канонично: '{}'). Допустимые ID: {}",
            raw_request, canonical, valid_ids
        )),
    }
}

/// Классический DP-Левенштейн (замена/вставка/удаление = 1), ранняя отсечка,
/// если минимум ДП-строки превысил `max` (id короткие, вызовов немного —
/// производительность некритична, важна простота и честность).
fn levenshtein(a: &[u8], b: &[u8], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current: Vec<usize> = vec![0; b.len() + 1];
    for (i, &a_char) in a.iter().enumerate() {
        current[0] = i + 1;
        let mut row_min = current[0];
        for (j, &b_char) in b.iter().enumerate() {
            let substitution_cost = usize::from(a_char != b_char);
            current[j + 1] = (previous[j] + substitution_cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
            row_min = row_min.min(current[j + 1]);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[b.len()];
    if distance <= max {
        Some(distance)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: [&str; 3] = [
        "hist_26dba55994405f91",
        "hist_8c617df7bb7b0f3f",
        "hist_482db678441a1ec6",
    ];

    fn found_id(raw: &str) -> Option<String> {
        match lookup_id(raw, &CATALOG) {
            IdLookup::Exact(id)
            | IdLookup::Embedded(id)
            | IdLookup::Fixed { id, .. } => Some(id.to_string()),
            _ => None,
        }
    }

    #[test]
    fn canonical_strips_frames_and_case() {
        assert_eq!(canonical("hist_482db678441a1ec6"), "hist482db678441a1ec6");
        assert_eq!(canonical(" id=\"hist_482DB678441A1EC6\" "), "idhist482db678441a1ec6");
        assert_eq!(canonical("hist_482db678(441a1ec6)"), "hist482db678441a1ec6");
        assert_eq!(canonical(""), "");
    }

    #[test]
    fn exact_id_from_catalog() {
        assert_eq!(
            found_id("hist_26dba55994405f91"),
            Some("hist_26dba55994405f91".to_string())
        );
    }

    /// Реальный кейс лога 00:51:37 — лишняя «9» (вставка, дистанция 1).
    #[test]
    fn recovers_digit_insertion_from_log() {
        match lookup_id("hist_26dba55999405f91", &CATALOG) {
            IdLookup::Fixed { id, distance } => {
                assert_eq!(id, "hist_26dba55994405f91");
                assert_eq!(distance, 1);
            }
            other => panic!("ожидался Fixed, получено: {}", describe(&other)),
        }
    }

    /// Реальный кейс лога 00:52:06 — потерянная пара «94» (удаления, дистанция 2).
    #[test]
    fn recovers_digit_deletions_from_log() {
        match lookup_id("hist_26dba559405f91", &CATALOG) {
            IdLookup::Fixed { id, distance } => {
                assert_eq!(id, "hist_26dba55994405f91");
                assert_eq!(distance, 2);
            }
            other => panic!("ожидался Fixed, получено: {}", describe(&other)),
        }
    }

    /// Реальный кейс лога 00:57:52 — подчёркивание внутри hex + смена хвоста.
    #[test]
    fn recovers_underscore_and_tail_substitution_from_log() {
        match lookup_id("hist_482db67_8441a1ec1", &CATALOG) {
            IdLookup::Fixed { id, distance } => {
                assert_eq!(id, "hist_482db678441a1ec6");
                assert_eq!(distance, 1);
            }
            other => panic!("ожидался Fixed, получено: {}", describe(&other)),
        }
    }

    /// Реальный кейс лога 00:57:03 — потерянный «4» (вставка, дистанция 2).
    #[test]
    fn recovers_lost_head_digit_from_log() {
        match lookup_id("hist_482db67891a1ec6", &CATALOG) {
            IdLookup::Fixed { id, distance } => {
                assert_eq!(id, "hist_482db678441a1ec6");
                assert_eq!(distance, 2);
            }
            other => panic!("ожидался Fixed, получено: {}", describe(&other)),
        }
    }

    /// Модель обрамила каноничный id текстом — достаёт стадия 2.
    #[test]
    fn embedded_id_inside_prose() {
        assert_eq!(
            found_id("последнее изображение hist_482db678441a1ec6.jpg"),
            Some("hist_482db678441a1ec6".to_string())
        );
    }

    /// 3+ правки — мы НЕ угадываем: честный отказ.
    #[test]
    fn far_mangle_is_not_found() {
        assert!(matches!(
            lookup_id("hist_111db345601a2e33", &CATALOG),
            IdLookup::NotFound { .. }
        ));
    }

    /// Двойной матч (в одном запросе два id) — отказ со списком,
    /// а не молчаливое «первый попавшийся».
    #[test]
    fn ambiguous_two_matches_rejected() {
        match lookup_id(
            "hist_26dba55994405f91, а также hist_482db678441a1ec6",
            &CATALOG,
        ) {
            IdLookup::Ambiguous { matches } => assert_eq!(matches.len(), 2),
            other => panic!("ожидался Ambiguous, получено: {}", describe(&other)),
        }
    }

    /// Отрицательный тест levenshtein: расстояние > max — None.
    #[test]
    fn levenshtein_respects_max_cutoff() {
        assert_eq!(levenshtein(b"ab", b"ab", 2), Some(0));
        assert_eq!(levenshtein(b"abc", b"abd", 2), Some(1));
        // Разница длин = 2 укладывается в порог и даёт дистанцию 2.
        assert_eq!(levenshtein(b"abcdef", b"abcdefgh", 2), Some(2));
        // Разница длин = 3 — ранняя отсечка по длине, без полного ДП.
        assert_eq!(levenshtein(b"abcdef", b"abcdefghi", 2), None);
    }

    fn describe(other: &IdLookup<'_>) -> String {
        match other {
            IdLookup::Exact(id) => format!("Exact({id})"),
            IdLookup::Embedded(id) => format!("Embedded({id})"),
            IdLookup::Fixed { id, distance } => format!("Fixed({id}, {distance})"),
            IdLookup::Ambiguous { matches } => format!("Ambiguous({matches:?})"),
            IdLookup::NotFound { canonical } => format!("NotFound({canonical})"),
        }
    }
}
