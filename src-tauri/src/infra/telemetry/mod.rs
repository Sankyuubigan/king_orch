//! 🛰️ Слой телеметрии — тонкий фасад над сервисом сбора ошибок.
//!
//! Всё приложение работает ТОЛЬКО через этот модуль:
//! `track_event` / `track_error` / `is_enabled` / `install_plugin`.
//! Никто (кроме `aptabase.rs`) не знает, какой сервис используется внутри.
//!
//! 🔄 Смена провайдера (например, с Aptabase на другой сервис):
//! 1. Пишем новый файл `telemetry/<service>.rs` с `impl TelemetryBackend`
//!    и своей сборкой плагина;
//! 2. Меняем `aptabase::` → `новый_модуль::` в `init()` и `install_plugin()`.
//! Остальное приложение не трогаем вообще.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tauri::{AppHandle, Runtime, plugin::TauriPlugin};

mod aptabase;

/// Абстрактный бэкенд телеметрии. Реализации: `aptabase.rs` (или любой другой).
/// Ожидается, что очередь событий сам выгружается в фоне (плагин делает это
/// на завершении приложения), поэтому отдельный flush наружу не нужен.
pub trait TelemetryBackend: Send + Sync {
    /// Отправить событие. Не должна паниковать и надолго блокировать.
    fn track_event(&self, name: &str, props: Value);
}

/// Реальный бэкенд, установленный в `init()`. До этого — его нет.
static BACKEND: OnceLock<Box<dyn TelemetryBackend>> = OnceLock::new();

/// Разрешена ли отправка телеметрии (управляется настройкой пользователя).
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Включена ли отправка телеметрии (настройка «анонимные отчёты»).
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Принудительно включить/выключить отправку (переключение галочки в настройках).
/// Отключение действует мгновенно; включение — со следующего запуска, если
/// плагин не был зарегистрирован при старте.
pub fn set_enabled(v: bool) {
    ENABLED.store(v, Ordering::Relaxed);
    if v && BACKEND.get().is_none() {
        crate::infra::startup_log::append(
            "INFO",
            "Телеметрия включена вручную — заработает со следующего запуска приложения",
        );
    }
}

/// Зарегистрировать реальный бэкенд. Вызывается в `setup()` main.rs
/// ТОЛЬКО если юзер не отключил анонимные отчёты.
pub fn init(app: &AppHandle) {
    let backend = aptabase::backend(app);
    let _ = BACKEND.set(backend);
    ENABLED.store(true, Ordering::Relaxed);
    crate::infra::startup_log::append("INFO", "Телеметрия: бэкенд инициализирован");
}

/// Собрать Tauri-плагин телеметрии (регистрируется в main.rs условно).
pub fn install_plugin<R: Runtime>() -> TauriPlugin<R> {
    aptabase::install_plugin()
}

/// Отправить событие. Тихий no-op, если телеметрия отключена/не инициализирована.
pub fn track_event(name: &str, props: Value) {
    if !is_enabled() {
        return;
    }
    if let Some(backend) = BACKEND.get() {
        backend.track_event(name, props);
    }
}

/// Отправить критическую ошибку бэкенда как событие `Backend Error`.
pub fn track_error(source: &str, message: &str) {
    track_event(
        "Backend Error",
        json!({ "message": format!("[{}] {}", source, message) }),
    );
}
