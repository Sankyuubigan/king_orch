use tauri::{AppHandle, Emitter};

use crate::domain;
use crate::infra;

#[tauri::command]
pub fn get_agents(app: AppHandle) -> Vec<domain::AgentEntry> {
    let agents_dir = infra::find_agents_dir(&app);
    let _ = app.emit("log", &format!("🔍 Поиск entry points в: {}", agents_dir.display()));
    let entries = domain::load_entry_points(&agents_dir);
    let _ = app.emit("log", &format!("✅ Загружено entry points: {}", entries.len()));
    entries
}