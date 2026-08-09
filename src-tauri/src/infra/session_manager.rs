use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

use crate::infra::llm::ChatMessage;

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub updated_at: u64,
    pub created_at: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub updated_at: u64,
    #[serde(default)]
    pub created_at: Option<u64>,
    #[serde(default)]
    pub draft: String,
    #[serde(default)]
    pub title_manual: bool,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    pub messages: Vec<ChatMessage>,
}

pub fn sessions_dir(app: &AppHandle) -> PathBuf {
    let base = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let path = base.join("sessions");
    if !path.exists() {
        let _ = fs::create_dir_all(&path);
    }
    path
}

/// Reads a session file, deserializes to ChatSession.
fn load_session(path: &PathBuf) -> Result<(Value, ChatSession), String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Ошибка чтения сессии: {}", e))?;
    let value: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Ошибка парсинга сессии: {}", e))?;
    let session: ChatSession = serde_json::from_value(value.clone())
        .map_err(|e| format!("Ошибка парсинга сессии: {}", e))?;
    Ok((value, session))
}

pub fn get_sessions(app: &AppHandle) -> Vec<SessionMeta> {
    let mut sessions = Vec::new();
    let dir = sessions_dir(app);
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |e| e == "json") {
                if let Ok((_, session)) = load_session(&path) {
                    let created_at = session.created_at.unwrap_or(session.updated_at);
                    sessions.push(SessionMeta {
                        id: session.id,
                        title: session.title,
                        updated_at: session.updated_at,
                        created_at,
                    });
                }
            }
        }
    }
    sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    sessions
}

pub fn get_session(app: &AppHandle, id: &str) -> Result<ChatSession, String> {
    let path = sessions_dir(app).join(format!("{}.json", id));
    let (_, session) = load_session(&path)?;
    Ok(session)
}

pub fn save_session(
    app: &AppHandle,
    id: &str,
    title: &str,
    messages: Vec<ChatMessage>,
    draft: String,
    model: Option<String>,
    agent: Option<String>,
) -> Result<(), String> {
    let path = sessions_dir(app).join(format!("{}.json", id));
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let mut session_created_at = now;
    let mut old_session: Option<ChatSession> = None;

    if path.exists() {
        if let Ok((_, loaded)) = load_session(&path) {
            session_created_at = loaded.created_at.unwrap_or(loaded.updated_at);
            old_session = Some(loaded);
        }
    }

    let session = ChatSession {
        id: id.to_string(),
        title: resolve_title(old_session.as_ref(), title),
        updated_at: now,
        created_at: Some(session_created_at),
        draft,
        title_manual: old_session.as_ref().map_or(false, |s| s.title_manual),
        model: model.or_else(|| old_session.as_ref().and_then(|s| s.model.clone())),
        agent: agent.or_else(|| old_session.as_ref().and_then(|s| s.agent.clone())),
        messages,
    };
    save_session_raw(&path, &session)?;

    // Save a debug copy to test/last_session.json
    let last_path = PathBuf::from("test").join("last_session.json");
    let _ = fs::create_dir_all("test");
    save_session_raw(&last_path, &session)
}

/// Решает, какое название сохранить. Если пользователь вручную переименовал
/// сессию (title_manual == true) — ручное имя неприкосновенно. Иначе —
/// авто-имя (обычно из первого сообщения пользователя).
fn resolve_title(existing: Option<&ChatSession>, computed: &str) -> String {
    match existing {
        Some(s) if s.title_manual => s.title.clone(),
        _ => computed.to_string(),
    }
}

pub fn delete_session(app: &AppHandle, id: &str) -> Result<(), String> {
    let path = sessions_dir(app).join(format!("{}.json", id));
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("Ошибка удаления сессии: {}", e))?;
    }
    Ok(())
}

fn save_session_raw(path: &PathBuf, session: &ChatSession) -> Result<(), String> {
    let content =
        serde_json::to_string_pretty(&session).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| format!("Ошибка сохранения сессии: {}", e))
}

pub fn rename_session(app: &AppHandle, id: &str, new_title: &str) -> Result<(), String> {
    let path = sessions_dir(app).join(format!("{}.json", id));
    if !path.exists() {
        return Err("Сессия не найдена".to_string());
    }
    let (mut value, _) = load_session(&path)?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("title".to_string(), Value::String(new_title.to_string()));
        // Пользователь поставил имя вручную — авто-переименование больше
        // не должно трогать title при последующих save_session.
        obj.insert(
            "title_manual".to_string(),
            Value::Bool(!new_title.trim().is_empty()),
        );
    }
    let content =
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn open_session_folder(app: &AppHandle, _id: &str) -> Result<(), String> {
    let dir = sessions_dir(app);
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn sample_session(title: &str, title_manual: bool) -> ChatSession {
    ChatSession {
        id: "test".to_string(),
        title: title.to_string(),
        updated_at: 1000,
        created_at: Some(1000),
        draft: String::new(),
        title_manual,
        model: None,
        agent: None,
        messages: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_title_uses_computed_for_new_session() {
        assert_eq!(resolve_title(None, "Вопрос юзера..."), "Вопрос юзера...");
    }

    #[test]
    fn resolve_title_keeps_manual_title() {
        let existing = sample_session("Мой любимый чат", true);
        assert_eq!(resolve_title(Some(&existing), "Вопрос юзера..."), "Мой любимый чат");
    }

    #[test]
    fn resolve_title_replaces_auto_title() {
        let existing = sample_session("Новая сессия", false);
        assert_eq!(resolve_title(Some(&existing), "Вопрос юзера..."), "Вопрос юзера...");
    }
}
