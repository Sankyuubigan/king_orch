use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use crate::llm::ChatMessage;

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub updated_at: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub updated_at: u64,
    #[serde(default)]
    pub state_markdown: String, // Внешняя память сессии (State)
    pub messages: Vec<ChatMessage>,
}

fn get_sessions_dir(app: &AppHandle) -> PathBuf {
    let base = app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."));
    let path = base.join("sessions");
    if !path.exists() {
        let _ = fs::create_dir_all(&path);
    }
    path
}

pub fn get_sessions(app: &AppHandle) -> Vec<SessionMeta> {
    let mut sessions = Vec::new();
    let dir = get_sessions_dir(app);
    
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |e| e == "json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(session) = serde_json::from_str::<ChatSession>(&content) {
                        sessions.push(SessionMeta {
                            id: session.id,
                            title: session.title,
                            updated_at: session.updated_at,
                        });
                    }
                }
            }
        }
    }
    
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

pub fn get_session(app: &AppHandle, id: &str) -> Result<ChatSession, String> {
    let path = get_sessions_dir(app).join(format!("{}.json", id));
    let content = fs::read_to_string(path).map_err(|e| format!("Ошибка чтения сессии: {}", e))?;
    serde_json::from_str(&content).map_err(|e| format!("Ошибка парсинга сессии: {}", e))
}

pub fn save_session(app: &AppHandle, id: &str, title: &str, messages: Vec<ChatMessage>, state_markdown: String) -> Result<(), String> {
    let updated_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let session = ChatSession {
        id: id.to_string(),
        title: title.to_string(),
        updated_at,
        state_markdown,
        messages,
    };
    
    let path = get_sessions_dir(app).join(format!("{}.json", id));
    let content = serde_json::to_string_pretty(&session).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| format!("Ошибка сохранения сессии: {}", e))
}

pub fn delete_session(app: &AppHandle, id: &str) -> Result<(), String> {
    let path = get_sessions_dir(app).join(format!("{}.json", id));
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("Ошибка удаления сессии: {}", e))?;
    }
    Ok(())
}