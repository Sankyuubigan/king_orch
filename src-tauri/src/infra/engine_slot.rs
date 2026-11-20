//! Выбор слота движка llama-server (PRP 26.11.20, Вариант B — тир-слот).
//!
//! Слот = ТИР от потребности (`next_tier` плагина: удвоение от 16384,
//! clamp к окну GGUF). Движок спавнится на каждый ход и умирает после него
//! (кэш между ходами C4 откачен 20.11.26 по решению владельца — плата в
//! 5 минут занятой VRAM после хода не оправдала экономию 3-4.6с/ход,
//! см. `tasks/261120_0024-baad336f/201120_2030-Откат C4.md`).
//!
//! Слот ниже окна безопасен: бюджет компакции считается от ЖЕЛЕЗНОГО
//! потолка (`hardware_ctx_limit`), а переполнение слота ловит step-up
//! движка (C6) — усечение истории включается только когда данные не
//! влезают в железо в принципе (астро-баг 3dea2ed «данных нет» не
//! воскрешается).

use plugin_llama_engine::engine::vram_estimate::next_tier;

/// Слот для свежего спавна: тир от потребности, clamp к окну (SSOT —
/// `next_tier` плагина: удвоение от 16384, окно GGUF — потолок, окно 0 → 0).
pub fn choose_slot(need: u32, window: u32) -> u32 {
    next_tier(need, window)
}

/// Потребность в контексте (токены): SSOT-эвристика 2 симв/ток (кириллица,
/// `estimate_chars_per_token`) + запас на генерацию ×2 (второй `max_gen` —
/// margin на материализацию тулов до следующего хода). Точный подсчёт
/// `get_tokens_count` остаётся в компакции: llm_messages собираются ПОСЛЕ
/// выбора слота, а грубые тиры (×2) прощают шум оценки.
pub fn need_ctx(history_chars: usize, user_text_chars: usize, max_gen_tokens: u32) -> u32 {
    ((history_chars.saturating_add(user_text_chars) / 2).min(u32::MAX as usize) as u32)
        .saturating_add(max_gen_tokens.saturating_mul(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn need_ctx_is_half_chars_plus_double_generation() {
        // 1000 символов истории → ~500 токенов; max_gen 2048 → margin 4096.
        assert_eq!(need_ctx(1000, 0, 2048), 4596);
        assert_eq!(need_ctx(0, 500, 2048), 4346);
        // Ноль всего → только margin от генерации.
        assert_eq!(need_ctx(0, 0, 2048), 4096);
        // Кириллица: 2 символа на токен — ровно половина.
        assert_eq!(need_ctx(20000, 0, 0), 10000);
    }

    #[test]
    fn choose_slot_delegates_to_plugin_tiers() {
        // Малый чат — нижний тир 16384, НЕ окно модели.
        assert_eq!(choose_slot(100, 262144), 16384);
        // Потребность между тирами → следующий тир.
        assert_eq!(choose_slot(50000, 262144), 65536);
        // Потребность выше окна → потолок = окно GGUF.
        assert_eq!(choose_slot(1_000_000, 262144), 262144);
        // Узкое окно ниже нижнего тира → окно.
        assert_eq!(choose_slot(5, 8192), 8192);
    }
}
