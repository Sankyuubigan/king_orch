use tauri::{AppHandle, Manager, Emitter};

use crate::domain;

#[tauri::command]
pub fn get_agents(app: AppHandle) -> Vec<domain::AgentProfile> {
    let agents_dir = find_agents_dir(&app);
    let _ = app.emit("log", &format!("🔍 Поиск агентов в: {}", agents_dir.display()));
    match domain::load_agents(&agents_dir) {
        Ok(agents) => {
            let _ = app.emit("log", &format!("✅ Загружено агентов: {}", agents.len()));
            agents
        }
        Err(e) => {
            let _ = app.emit("log", &format!("❌ Ошибка загрузки агентов: {}", e));
            vec![]
        }
    }
}

fn find_agents_dir(app: &AppHandle) -> std::path::PathBuf {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    for dir in [
        exe_dir.join("agents"),
        resource_dir.join("agents"),
        std::path::PathBuf::from("agents"),
        exe_dir.join("..").join("..").join("agents"),
    ] {
        if dir.exists() {
            return dir;
        }
    }
    let default = exe_dir.join("agents");
    let _ = std::fs::create_dir_all(&default);
    default
}