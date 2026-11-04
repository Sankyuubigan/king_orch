pub(crate) const AGENT_ERROR_PREFIX: &str = "⚠️ ОШИБКА_АГЕНТА:";
pub(crate) const MAX_CONTINUATIONS: usize = 3; // предел «докачек» после обрыва
pub(crate) const MAX_CONTINUATION_RESTARTS: usize = 3;
/// Кап перегенераций «начисто» при рестарте-классе докачки (модель повторяет начало
/// вместо продолжения): исчерпание → честная ошибка агента, не бесконечный цикл.
pub(crate) const MAX_RESTART_REGENS: usize = 2;
pub(crate) const COMPACT_THRESHOLD_CHARS: usize = 6000; // накопленных размышлений → сжать в тезисы
pub(crate) const COMPACT_MAX_TOKENS: usize = 300;
pub(crate) const THOUGHT_STORE_MAX_CHARS: usize = 2100; // в сессию сохраняем мысль срезом (приоритет ответу)
pub(crate) const MAX_STALLED_CONTINUATIONS: usize = 3; // докачек подряд без прогресса
pub(crate) const MIN_THINKING_GROWTH_CHARS: isize = 128; // порог «размышления растут» за одну докачку
pub(crate) const SPILL_THRESHOLD: usize = 8000;
pub(crate) const NATIVE_FINAL_RETRIES_MAX: usize = 2; // GUARD «финал без тулов»: макс. ретраев native-агента
