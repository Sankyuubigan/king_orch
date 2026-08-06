//! Реализация телеметрии через **Aptabase** (`tauri-plugin-aptabase`).
//!
//! ЕДИНСТВЕННЫЙ файл проекта, который знает про Aptabase:
//! здесь лежат app key, сборка плагина, panic-хук и отправка событий.
//! Смена сервиса = замена содержимого этого файла (см. `mod.rs`).

use serde_json::{json, Value};
use tauri::{AppHandle, Runtime, plugin::TauriPlugin};
use tauri_plugin_aptabase::{Builder as AptabaseBuilder, EventTracker};

use super::TelemetryBackend;

/// App Key приложения в Aptabase. Это публичный идентификатор (не секрет).
const APP_KEY: &str = "A-EU-5188621951";

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
/// в `king_orch.log`, а дополнительно уходит в Aptabase как событие.
/// Хук проверяет глобальный флаг `is_enabled()`: если юзер снял галочку —
/// паника в Aptabase НЕ отправляется (приватность соблюдена).
pub fn install_plugin<R: Runtime>() -> TauriPlugin<R> {
    AptabaseBuilder::new(APP_KEY)
        .with_panic_hook(Box::new(|client, info, msg| {
            if !super::is_enabled() {
                return;
            }
            let location = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            let _ = client.track_event(
                "Backend Panic",
                Some(json!({ "message": msg, "location": location })),
            );
        }))
        .build()
}
