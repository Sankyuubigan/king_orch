//! 🔐 Команда ответа на плашку разрешений + форвардинг событий в UI.
//!
//! Подписка на in-process шину (`PermissionRequest`) делается ОДИН раз при
//! старте приложения (setup). Событие форвардится на фронтенд как
//! `tool_permission_request`, фронт показывает диалог с 3 кнопками и вызывает
//! `respond_permission`, которая разрешает ожидание агентского цикла.

use tauri::{AppHandle, Emitter};
use crate::infra::{GrantDecision, global_approver};

/// Разрешить ожидание агента: `request_id` из события `tool_permission_request`,
/// `decision` — "deny" | "allow_once" | "allow_session".
#[tauri::command]
pub fn respond_permission(request_id: String, decision: String) -> Result<(), String> {
    let decision = GrantDecision::from_str(&decision)
        .ok_or_else(|| format!("Неизвестное решение '{}' (ожидается deny/allow_once/allow_session)", decision))?;
    let ok = global_approver().resolve(&request_id, decision);
    if !ok {
        return Err(format!("Запрос разрешения '{}' не найден или уже истёк", request_id));
    }
    Ok(())
}

/// Подписка на шину + форвардинг PermissionRequest → `tool_permission_request` (UI).
/// Вызывается один раз в setup.
pub fn init_permission_forwarding(app: &AppHandle) {
    let app = app.clone();
    crate::infra::event_bus::global_bus().subscribe(std::sync::Arc::new(move |event| {
        if let crate::infra::event_bus::AgentEvent::PermissionRequest {
            request_id,
            agent,
            tool,
            path,
        } = event
        {
            let _ = app.emit(
                "tool_permission_request",
                serde_json::json!({
                    "request_id": request_id,
                    "agent": agent,
                    "tool": tool,
                    "path": path,
                }),
            );
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_decision_parses_from_frontend() {
        assert_eq!(GrantDecision::from_str("deny"), Some(GrantDecision::Deny));
        assert_eq!(GrantDecision::from_str("allow_session"), Some(GrantDecision::AllowSession));
        assert!(GrantDecision::from_str("delete_all").is_none());
    }
}