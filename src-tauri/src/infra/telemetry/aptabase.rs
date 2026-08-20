//! Реализация телеметрии через **Aptabase** (`tauri-plugin-aptabase`).
//!
//! ЕДИНСТВЕННЫЙ файл проекта, который знает про Aptabase:
//! здесь лежат app key, сборка плагина, panic-хук и отправка событий.
//! Смена сервиса = замена содержимого этого файла (см. `mod.rs`).
//!
//! Два канала отправки:
//! - **События** (аналитика) — через плагин (`/api/v0/events`);
//! - **Ошибки** — напрямую через Error Reporting API (`/api/v0/error`,
//!   раздел «Errors» в дашборде). Плагин этот endpoint не умеет.

use serde_json::{json, Value};
use std::time::Duration;
use tauri::{AppHandle, Runtime, plugin::TauriPlugin};
use tauri_plugin_aptabase::{Builder as AptabaseBuilder, EventTracker};

use super::{ErrorReport, TelemetryBackend};

/// App Key приложения в Aptabase. Это публичный идентификатор (не секрет).
const APP_KEY: &str = "A-EU-5188621951";

/// Базовый URL ошибок определяется регионом из App Key (EU/US).
fn error_api_url() -> String {
    let region = APP_KEY.split('-').nth(1).unwrap_or("EU");
    let base = match region {
        "US" => "https://us.aptabase.com",
        _ => "https://eu.aptabase.com",
    };
    format!("{}/api/v0/error", base)
}

/// Собрать тело запроса ErrorBody (см. error-api-openapi.yaml).
fn build_error_body(report: &ErrorReport) -> Value {
    json!({
        "errorMessage": report.message,
        "errorType": report.error_type,
        "stackTrace": report.stack.clone().unwrap_or_default(),
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "severity": report.severity,
        "kind": report.kind,
        "isDebug": cfg!(debug_assertions),
    })
}

/// Отправить отчёт об ошибке в Error Reporting API (fire-and-forget).
fn send_error(report: &ErrorReport) {
    let body = build_error_body(report);
    let url = error_api_url();
    let app_key = APP_KEY.to_string();
    tauri::async_runtime::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let resp = client
            .post(&url)
            .header("App-Key", app_key)
            .json(&body)
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => crate::infra::startup_log::append(
                "WARN",
                &format!("[telemetry] error API: HTTP {}", r.status()),
            ),
            Err(e) => crate::infra::startup_log::append(
                "WARN",
                &format!("[telemetry] error API: запрос не удался: {}", e),
            ),
        }
    });
}

/// Бэкенд поверх `AppHandle`: плагин хранит свой клиент в Tauri-state,
/// а мы обращаемся к нему через трейт `EventTracker`.
pub struct AptabaseBackend {
    app: AppHandle,
}

impl TelemetryBackend for AptabaseBackend {
    fn track_event(&self, name: &str, props: Value) {
        let props = if props.is_null() { None } else { Some(props) };
        // Правило «не врать» (core/rules.md §2.2): сбой отправки не глотаем
        // молча — пишем в лог-файл (в UI он попадает через вкладку «Логи»).
        if let Err(e) = self.app.track_event(name, props) {
            crate::infra::startup_log::append(
                "WARN",
                &format!("[telemetry] track_event({}) не удалось: {}", name, e),
            );
        }
    }

    fn track_error(&self, report: &ErrorReport) {
        send_error(report);
    }
}

/// Создать бэкенд для глобальной инициализации (`telemetry::init`).
pub fn backend(app: &AppHandle) -> Box<dyn TelemetryBackend> {
    Box::new(AptabaseBackend { app: app.clone() })
}

/// Собрать Tauri-плагин с panic-хуком.
///
/// Как это работает: плагин в своём `setup` забирает текущий hook
/// (наш `startup_log::install_panic_hook`) как default, ставит свой hook,
/// а тот в конце вызывает default. Поэтому паника по-прежнему пишется
/// в `king_orch.log`, а дополнительно уходит в Aptabase в раздел «Errors»
/// как `Backend Panic` (Error Reporting API).
/// Хук проверяет глобальный флаг `is_enabled()`: если юзер снял галочку —
/// паника в Aptabase НЕ отправляется (приватность соблюдена).
pub fn install_plugin<R: Runtime>() -> TauriPlugin<R> {
    AptabaseBuilder::new(APP_KEY)
        .with_panic_hook(Box::new(|_client, info, msg| {
            if !super::is_enabled() {
                return;
            }
            let location = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            send_error(&ErrorReport {
                error_type: "Backend Panic".to_string(),
                message: msg,
                stack: if location.is_empty() {
                    None
                } else {
                    Some(location)
                },
                severity: "fatal".to_string(),
                kind: "crash".to_string(),
            });
        }))
        .build()
}
