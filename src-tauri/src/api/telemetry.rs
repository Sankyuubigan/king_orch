//! 🚪 API-команды телеметрии для фронтенда.
//!
//! Фронтенд не имеет App Key и не знает про Aptabase — он лишь передаёт
//! параметры ошибки, а бэкенд отправляет их в Error Reporting API
//! (раздел «Errors» в дашборде).

use crate::infra::telemetry::{ErrorReport, track_error_report};

/// Отправить отчёт об ошибке (вызывается из telemetry.ts).
///
/// - `error_type` — группа ошибки: "Frontend Error", "UI Error", …
/// - `severity`: "fatal" | "error"
/// - `kind`: "crash" | "unhandled" | "taskException" | "handled"
#[tauri::command]
pub fn track_error(
    error_type: String,
    message: String,
    stack: Option<String>,
    severity: Option<String>,
    kind: Option<String>,
) {
    track_error_report(&ErrorReport {
        error_type,
        message,
        stack,
        severity: severity.unwrap_or_else(|| "error".to_string()),
        kind: kind.unwrap_or_else(|| "handled".to_string()),
    });
}