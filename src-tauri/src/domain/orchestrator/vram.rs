//! Очередь тяжёлых вычислений за VRAM: LLM и image-движок живут по очереди.
//!
//! Правило `desktop_rust_tauri/rules.md §6.5`: тяжёлые модели грузятся
//! поочередно, после отработки объект уничтожается (`Drop`) ради VRAM
//! следующего шага. Здесь — та же идея для двух процессов: перед
//! `generate_image`/`edit_image` llama-server приостанавливается
//! (`suspend_for_external_compute`, владение через scope, не taskkill),
//! после — перезапускается (`resume_after_external_compute`, KV-кэш теряется).
//!
//! Ресурсная ошибка (нет памяти) — не «плохие аргументы»: ретраи с просьбой
//! «вызови снова» бессмысленны, отвечаем честно сразу (core §2.2, §1.7.1).

use super::consts::AGENT_ERROR_PREFIX;
use crate::infra::tools::ToolError;

/// Маркер ресурсной ошибки image-preflight (SSOT порогов — плагин,
/// здесь только детект текста для маршрутизации ретраев).
pub fn is_resource_error(output: &str) -> bool {
    output.contains("Недостаточно памяти")
        || output.contains("Недостаточно RAM")
        || output.contains("не стал готов за 300 сек")
}

/// Детерминированная сетап-ошибка media-тула: бандл не скачан/недокачан,
/// движок не установлен, бандла нет в каталоге. Media-тулы выдают её как
/// `ToolError::NotFound`, чей Display начинается с «Не найдено: » —
/// повтор «вызови СНОВА» не может изменить состояние диска, ретрай запрещён.
pub fn is_setup_error(output: &str) -> bool {
    output.contains("Не найдено: ")
}

/// Честный финал при сетап-ошибке: текст уже содержит причину и инструкцию
/// («Активный бандл … скачан не полностью: нет файлов … Скачайте его в
/// Настройки → …»), добавляем только маркер агентской ошибки.
pub fn setup_error_fatal(output: &str) -> String {
    format!("{} {}", AGENT_ERROR_PREFIX, output)
}

/// Ошибка с конкретной причиной в Display (детерминированный отказ или
/// неверные аргументы): агенту отдаём честный текст, маскировать = ложь
/// (core §2.2). Обезличенный текст остаётся только для Io/Timeout —
/// их причины и так пишет лог.
pub fn is_honest_tool_error(e: &ToolError) -> bool {
    matches!(
        e,
        ToolError::NotFound(_) | ToolError::Usage(_) | ToolError::Forbidden(_)
    )
}

/// Честный финал при ресурсной ошибке: без ретраев, с цифрами из preflight.
pub fn resource_error_fatal(agent_id: &str, tool_name: &str, output: &str) -> String {
    format!(
        "{} Нет памяти для '{}' (агент '{}'): {}. Текстовая модель была выгружена, но памяти всё равно не хватило — уменьши контекст или закрой другие GPU-программы.",
        AGENT_ERROR_PREFIX, tool_name, agent_id, output
    )
}

/// Хинт модели после ресурсной ошибки: запрещает повтор тем же способом.
#[allow(dead_code)]
pub fn resource_error_retry_hint(tool_name: &str, output: &str) -> String {
    format!(
        "[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}]:\n{}\n\n⚠️ Ресурсная ошибка: повтор тем же способом бесполезен (памяти не прибавится). НЕ повторяй вызов.",
        tool_name, output
    )
}

pub fn tool_error_to_output(tool_name: &str, e: ToolError) -> String {
    format!("Ошибка '{}': {}", tool_name, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_resource_errors() {
        assert!(is_resource_error("Ошибка 'generate_image': Ошибка ввода/вывода: Недостаточно памяти: минимум ~8.6 ГБ VRAM"));
        assert!(is_resource_error("Недостаточно RAM: нужно ~13 ГБ"));
        assert!(!is_resource_error("Ошибка 'generate_image': Неверные аргументы: prompt_en пуст"));
        assert!(!is_resource_error("Готово за 12.3 сек"));
    }

    #[test]
    fn resource_fatal_is_honest() {
        let msg = resource_error_fatal("image_generator", "generate_image", "Недостаточно памяти: минимум ~8.6 ГБ");
        assert!(msg.contains(AGENT_ERROR_PREFIX));
        assert!(msg.contains("8.6"));
        assert!(!msg.contains("попробуй снова"));
    }

    #[test]
    fn detects_setup_errors() {
        // Точный текст media.rs resolve_bundle_for_tools (ToolError::NotFound → Display).
        let bundle = "Ошибка 'generate_image': Не найдено: Активный бандл «qwen-image-2.1-turbo-viggle-q4km» скачан не полностью: нет файлов Qwen-Image-2.1-viggle-turbo-v0.3-6step-Q4_K_M.gguf. Скачайте его в Настройки → «Наборы моделей» (движок изображений).";
        assert!(is_setup_error(bundle));
        assert!(is_setup_error("Не найдено: Движок изображений не установлен. Откройте Настройки → «Движок изображений»."));
        // Ресурсные/аргументные/системные тексты — НЕ сетап-ошибки.
        assert!(!is_setup_error("Ошибка 'generate_image': Неверные аргументы: prompt_en пуст"));
        assert!(!is_setup_error("Ошибка ввода/вывода: Недостаточно памяти"));
        // «не найден» (тул не найден) ≠ «Не найдено: » (NotFound-причина).
        assert!(!is_setup_error("Ошибка: Инструмент 'generate_image' не найден."));
    }

    #[test]
    fn setup_fatal_keeps_honest_text() {
        let honest = "Ошибка 'generate_image': Не найдено: Активный бандл «x» скачан не полностью: нет файлов a.gguf. Скачайте его в Настройки → «Наборы моделей».";
        let msg = setup_error_fatal(honest);
        assert!(msg.starts_with(AGENT_ERROR_PREFIX));
        assert!(msg.contains("скачан не полностью"));
        assert!(msg.contains("Скачайте его"));
        // Финал — не ретрай-приказ: модель не должна получить «вызови СНОВА».
        assert!(!msg.contains("СНОВА"));
    }

    #[test]
    fn honest_tool_error_classes() {
        use crate::infra::tools::ToolError;
        assert!(is_honest_tool_error(&ToolError::NotFound("скачан не полностью".to_string())));
        assert!(is_honest_tool_error(&ToolError::Usage("prompt_en пуст".to_string())));
        assert!(is_honest_tool_error(&ToolError::Forbidden("запрещено".to_string())));
        assert!(!is_honest_tool_error(&ToolError::Io("spawn упал".to_string())));
        assert!(!is_honest_tool_error(&ToolError::Timeout("таймаут".to_string())));
    }
}
