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
}
