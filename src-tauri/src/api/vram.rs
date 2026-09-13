//! 🎛 Команда ответа на диалог pre-flight VRAM + форвардинг событий в UI.
//!
//! Подписка на in-process шину (`VramConfirmRequest`) делается ОДИН раз при
//! старте приложения (setup). Событие форвардится на фронтенд как
//! `vram_confirm_request`, фронт показывает диалог «Отмена / Запустить с
//! выгрузкой в ОЗУ» и вызывает `respond_vram_choice`, которая разрешает
//! ожидание pre-flight в агентском цикле.

use tauri::{AppHandle, Emitter};
use crate::infra::{VramChoice, global_vram_approver};

/// Разрешить ожидание pre-flight VRAM: `request_id` из события
/// `vram_confirm_request`, `decision` — "cancel" | "proceed".
#[tauri::command]
pub fn respond_vram_choice(request_id: String, decision: String) -> Result<(), String> {
    let choice = VramChoice::from_str(&decision)
        .ok_or_else(|| format!("Неизвестное решение '{}' (ожидается cancel/proceed)", decision))?;
    let ok = global_vram_approver().resolve(&request_id, choice);
    if !ok {
        return Err(format!("Запрос VRAM-решения '{}' не найден или уже истёк", request_id));
    }
    Ok(())
}

/// Подписка на шину + форвардинг VramConfirmRequest → `vram_confirm_request` (UI).
/// Вызывается один раз в setup.
pub fn init_vram_forwarding(app: &AppHandle) {
    let app = app.clone();
    crate::infra::event_bus::global_bus().subscribe(std::sync::Arc::new(move |event| {
        if let crate::infra::event_bus::AgentEvent::VramConfirmRequest {
            request_id,
            note,
        } = event
        {
            let _ = app.emit(
                "vram_confirm_request",
                serde_json::json!({
                    "request_id": request_id,
                    "note": note,
                }),
            );
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vram_choice_parses_from_frontend() {
        assert_eq!(VramChoice::from_str("cancel"), Some(VramChoice::Cancel));
        assert_eq!(VramChoice::from_str("proceed"), Some(VramChoice::ProceedWithRam));
        assert!(VramChoice::from_str("run_anyway").is_none());
    }
}