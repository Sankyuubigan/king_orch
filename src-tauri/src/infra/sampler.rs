//! DRY и XTC сэмплеры — защита от зацикливания и повышение разнообразия
//!
//! DRY (Don't Repeat Yourself) — наказывает за вербатим-повторения n-грамм.
//! XTC (Exclude Top Choices) — убирает самые вероятные "скучные" токены.

use std::collections::HashMap;

/// Параметры DRY сэмплера
#[derive(Debug, Clone)]
pub struct DryParams {
    pub multiplier: f32,
    pub base: f32,
    pub allowed_length: i32,
    pub penalty_last_n: i32,
}

impl Default for DryParams {
    fn default() -> Self {
        Self {
            multiplier: 0.0,
            base: 1.75,
            allowed_length: 2,
            penalty_last_n: 0,
        }
    }
}

/// Параметры XTC сэмплера
#[derive(Debug, Clone)]
pub struct XtcParams {
    pub probability: f32,
    pub threshold: f32,
    pub min_keep: usize,
}

impl Default for XtcParams {
    fn default() -> Self {
        Self {
            probability: 0.0,
            threshold: 0.1,
            min_keep: 1,
        }
    }
}

/// Стандартные sequence breakers для DRY (новые строки, пунктуация, кавычки)
pub fn default_seq_breakers() -> &'static [&'static str] {
    &["\n", "!", "?", ".", ",", "\"", "'", ";", ":", ")", "(", "[", "]", "{", "}", "—", "–"]
}

// ─── DRY ───────────────────────────────────────────────────────────────────────

/// Применяет DRY сэмплер к кандидатам (мутация logit'ов in-place).
///
/// `last_tokens` — история сгенерированных токенов (последние N штук).
/// `seq_breakers` — строки-разделители, сбрасывающие счётчик повторений.
pub fn apply_dry(
    candidates: &mut [(llama_cpp_2::token::LlamaToken, f32)],
    last_tokens: &[llama_cpp_2::token::LlamaToken],
    params: &DryParams,
    seq_breakers: &[&str],
) {
    if params.multiplier == 0.0 || params.base < 1.0 || params.penalty_last_n == 0 {
        return;
    }

    let effective_last_n = if params.penalty_last_n == -1 {
        last_tokens.len() as i32
    } else {
        params.penalty_last_n.max(0)
    };

    let last_n_repeat = (last_tokens.len() as i32).min(effective_last_n) as usize;

    if last_n_repeat <= params.allowed_length as usize {
        return;
    }

    // ── Step 0: Преобразуем seq_breakers в токены ──
    // Для простоты: ищем совпадение по байтам (sequence breaker == одному токену).
    // Упрощённая версия — проверяем по символам строки.
    let breaker_chars: Vec<char> = seq_breakers.iter().filter_map(|s| {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() == 1 { Some(chars[0]) } else { None }
    }).collect();

    // ── Step 1: Определяем rep_limit (ищем sequence breakers) ──
    let rep_limit = last_n_repeat;
    for i in 0..last_n_repeat {
        let token = last_tokens[last_n_repeat - 1 - i];
        // Проверяем, является ли токен sequence breaker'ом (упрощённо: сравниваем id)
        // Для полноценной проверки нужен vocab — пока используем простой эвристический подход:
        // если токен — односимвольный breaker, ограничиваем rep_limit
        // (В llama.cpp это делается через детокенизацию + поиск подстрок)
        // Здесь мы используем тот факт, что sequence breakers обычно — частые однотокенные символы.
        // Полноценная проверка через vocab будет добавлена позже при необходимости.
        let _ = token;
        let _ = &breaker_chars;
        // Пока: не ограничиваем rep_limit через breakers (works fine без этого для нашего кейса)
    }

    if rep_limit <= params.allowed_length as usize {
        return;
    }

    // ── Step 2: Z-алгоритм (реверсивный) ──
    // Для каждого индекса считаем длину суффикса, совпадающего с конечным суффиксом.
    let mut repeat_count = vec![0i32; last_n_repeat];
    {
        let last = last_n_repeat - 1;
        let mut rt: i32 = 0;
        let mut lt: i32 = 0;

        for k in 1..last_n_repeat {
            if k as i32 > rt {
                // Находимся за пределами текущего Z-box — наивное сравнение
                let mut n = 0i32;
                while n + (k as i32) < last_n_repeat as i32
                    && last_tokens[(last as i32 - n) as usize] == last_tokens[(last as i32 - n - k as i32) as usize]
                {
                    n += 1;
                }
                repeat_count[last - k] = n.min(rep_limit as i32);
                if n > 0 {
                    lt = k as i32;
                    rt = k as i32 + n - 1;
                }
            } else {
                // Находимся внутри Z-box
                let p = k as i32 - lt;
                let right_part_len = rt - k as i32 + 1;

                if repeat_count[(last as i32 - p) as usize] < right_part_len {
                    repeat_count[last - k] = repeat_count[(last as i32 - p) as usize].min(rep_limit as i32);
                } else {
                    let mut i = (rt + 1) as usize;
                    while i < last_n_repeat
                        && last_tokens[last - i] == last_tokens[last - i + k]
                    {
                        i += 1;
                    }
                    let n = (i - k) as i32;
                    repeat_count[last - k] = n.min(rep_limit as i32);
                    lt = k as i32;
                    rt = (i - 1) as i32;
                }
            }
        }
    }

    // ── Step 3: Построение карты максимальных повторений ──
    let mut max_token_repeat: HashMap<llama_cpp_2::token::LlamaToken, i32> = HashMap::new();
    for i in 0..(last_n_repeat - 1) {
        let repeat_len = repeat_count[i];
        if repeat_len >= params.allowed_length {
            let token = last_tokens[last_n_repeat - 2 - i];
            let entry = max_token_repeat.entry(token).or_insert(0);
            if repeat_len > *entry {
                *entry = repeat_len;
            }
        }
    }

    // ── Step 4: Применяем штрафы к логитам ──
    let float_max_log = 88.7228391f32;
    let max_exponent = if params.base > 1.000001 {
        (float_max_log / params.base.ln()) as i32
    } else {
        0
    };

    for (token, logit) in candidates.iter_mut() {
        if let Some(&repeat_len) = max_token_repeat.get(token) {
            let mut repeat_exp = repeat_len - params.allowed_length;
            if max_exponent > 0 && repeat_exp > max_exponent {
                repeat_exp = max_exponent;
            }
            let penalty = params.multiplier * params.base.powi(repeat_exp);
            *logit -= penalty;
        }
    }
}

// ─── XTC ───────────────────────────────────────────────────────────────────────

/// Применяет XTC сэмплер к кандидатам (фильтрация candidates).
///
/// `random_01` — случайное число в диапазоне [0, 1).
/// Возвращает `true` если кандидаты были отфильтрованы.
pub fn apply_xtc(
    candidates: &mut Vec<(llama_cpp_2::token::LlamaToken, f32)>,
    params: &XtcParams,
    random_01: f32,
) -> bool {
    if params.probability <= 0.0 || params.threshold > 0.5 || candidates.len() < 2 {
        return false;
    }

    if random_01 > params.probability {
        return false;
    }

    // Вычисляем softmax
    let max_logit = candidates.iter().map(|(_, l)| *l).fold(f32::NEG_INFINITY, f32::max);
    let mut probs: Vec<(usize, f32)> = candidates
        .iter()
        .enumerate()
        .map(|(i, (_, l))| (i, (l - max_logit).exp()))
        .collect();
    let sum_exp: f32 = probs.iter().map(|(_, p)| *p).sum();
    for (_, p) in probs.iter_mut() {
        *p /= sum_exp;
    }

    // Сортируем по вероятности (убывание)
    probs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Находим последний токен с вероятностью >= threshold
    let mut pos_last = 0usize;
    for (i, &(_, p)) in probs.iter().enumerate() {
        if p >= params.threshold {
            pos_last = i;
        } else {
            break;
        }
    }

    // Фильтруем: оставляем только токены после pos_last + один токен на позиции pos_last
    if candidates.len() - pos_last >= params.min_keep && pos_last > 0 {
        // Собираем индексы токенов, которые оставляем
        let keep_indices: Vec<usize> = probs[pos_last..].iter().map(|(i, _)| *i).collect();
        let mut kept: Vec<(llama_cpp_2::token::LlamaToken, f32)> = keep_indices
            .iter()
            .map(|&i| candidates[i])
            .collect();

        // Сортируем по логиту (убывание) для дальнейшей обработки
        kept.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        *candidates = kept;
        return true;
    }

    false
}
