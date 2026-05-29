use tauri::AppHandle;

use crate::infra;

#[tauri::command]
pub fn get_sessions(app: AppHandle) -> Vec<infra::SessionMeta> {
    infra::get_sessions(&app)
}

#[tauri::command]
pub fn load_session(app: AppHandle, id: String) -> Result<infra::ChatSession, String> {
    infra::get_session(&app, &id)
}

#[tauri::command]
pub fn save_session(
    app: AppHandle,
    id: String,
    messages: Vec<infra::ChatMessage>,
    dossier: infra::Dossier,
    draft: String,
) -> Result<(), String> {
    let title = messages
        .iter()
        .find(|m| m.role == "user" && !m.content.trim().is_empty())
        .map(|m| {
            let text = m.content.replace('\n', " ");
            if text.chars().count() > 35 {
                format!("{}...", text.chars().take(32).collect::<String>())
            } else {
                text
            }
        })
        .unwrap_or_else(|| "Новая сессия".to_string());
    infra::save_session(&app, &id, &title, messages, dossier, draft)
}

#[tauri::command]
pub fn delete_session(app: AppHandle, id: String) -> Result<(), String> {
    infra::delete_session(&app, &id)
}

#[tauri::command]
pub fn rename_session(app: AppHandle, id: String, new_title: String) -> Result<(), String> {
    infra::rename_session(&app, &id, &new_title)
}

#[tauri::command]
pub fn open_session_folder(app: AppHandle, id: String) -> Result<(), String> {
    infra::open_session_folder(&app, &id)
}