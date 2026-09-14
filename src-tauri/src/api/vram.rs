//! 🔔 Форвардинг уведомлений о VRAM в UI.
//!
//! Подписка на in-process шину (`VramNotice`) делается ОДИН раз при старте
//! приложения (setup). Событие форвардится на фронтенд как `vram_notice`,
//! фронт показывает окно-уведомление с одной кнопкой «ОК» (запуск не блокируется).

use tauri::{AppHandle, Emitter};

/// Подписка на шину + форвардинг VramNotice → `vram_notice` (UI).
/// Вызывается один раз в setup.
pub fn init_vram_forwarding(app: &AppHandle) {
    let app = app.clone();
    crate::infra::event_bus::global_bus().subscribe(std::sync::Arc::new(move |event| {
        if let crate::infra::event_bus::AgentEvent::VramNotice { note } = event {
            let _ = app.emit(
                "vram_notice",
                serde_json::json!({
                    "note": note,
                }),
            );
        }
    }));
}