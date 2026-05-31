use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// Converts an OLD-format chat message (with `role`/`agent_name`) to NEW format (`type`/`author`).
/// Returns `true` if any migration was applied.
///
/// # Old format → New format
/// - `role: "user"`              → `type: "message"`, `author: "user"`
/// - `role: "assistant"`          → `type: "message"`, `author: <agent_name> ?? "assistant"`
/// - `role: "thought"`            → `type: "thought"`, `author: <agent_name>`
/// - `role: "system"`             → `type: "message"`, `author: "system"`
/// - standalone `agent_name`      → `author` (copy)
pub fn migrate_legacy_message(msg: &mut Value) -> bool {
    let obj = match msg.as_object_mut() {
        Some(o) => o,
        None => return false,
    };
    let mut changed = false;

    if let Some(role) = obj.remove("role") {
        changed = true;
        if !obj.contains_key("type") {
            let role_str = role.as_str().unwrap_or("user");
            match role_str {
                "user" => {
                    obj.insert("type".into(), Value::String("message".into()));
                    obj.insert("author".into(), Value::String("user".into()));
                }
                "assistant" => {
                    obj.insert("type".into(), Value::String("message".into()));
                    let author = obj
                        .remove("agent_name")
                        .unwrap_or(Value::String("assistant".into()));
                    obj.insert("author".into(), author);
                }
                "thought" => {
                    obj.insert("type".into(), Value::String("thought".into()));
                    if let Some(agent_name) = obj.remove("agent_name") {
                        obj.insert("author".into(), agent_name);
                    }
                }
                "system" => {
                    obj.insert("type".into(), Value::String("message".into()));
                    obj.insert("author".into(), Value::String("system".into()));
                }
                _ => {
                    obj.insert("type".into(), Value::String("message".into()));
                }
            }
        }
    }

    // Standalone agent_name (no role) → author
    if let Some(agent_name) = obj.remove("agent_name") {
        changed = true;
        if !obj.contains_key("author") {
            obj.insert("author".into(), agent_name);
        }
    }

    changed
}

/// Migrates all messages inside a session JSON Value.
/// Returns `true` if at least one message was migrated.
pub fn migrate_session_value(value: &mut Value) -> bool {
    let messages = match value
        .as_object_mut()
        .and_then(|o| o.get_mut("messages"))
    {
        Some(Value::Array(arr)) => arr,
        _ => return false,
    };
    let mut any_migrated = false;
    for msg in messages.iter_mut() {
        if migrate_legacy_message(msg) {
            any_migrated = true;
        }
    }
    any_migrated
}

/// Reads a JSON session file, migrates it in-place if needed, and saves back.
/// Returns `(migrated: bool, value: Value)` where `value` is the (possibly migrated) session.
/// Logs via eprintln when migration occurs.
pub fn migrate_session_file(path: &PathBuf) -> Result<(bool, Value), String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Ошибка чтения {}: {}", path.display(), e))?;

    let mut value: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Ошибка парсинга {}: {}", path.display(), e))?;

    let migrated = migrate_session_value(&mut value);

    if migrated {
        eprintln!(
            "[migration] session {}: обновлён формат (role/agent_name → type/author)",
            path.display()
        );
        let new_content = serde_json::to_string_pretty(&value)
            .map_err(|e| format!("Ошибка сериализации {}: {}", path.display(), e))?;
        fs::write(path, new_content)
            .map_err(|e| format!("Ошибка записи {}: {}", path.display(), e))?;
        eprintln!("[migration] session {}: сохранён", path.display());
    }

    Ok((migrated, value))
}

/// Scans all session JSON files in the given directory and migrates old-format ones.
/// Logs summary at the end. Safe to call repeatedly — skips already-migrated files.
pub fn migrate_all_sessions(sessions_dir: &PathBuf) {
    if !sessions_dir.exists() {
        return;
    }
    let dir = match fs::read_dir(sessions_dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[migration] не удалось прочитать {}: {}", sessions_dir.display(), e);
            return;
        }
    };

    let mut total = 0u32;
    let mut migrated = 0u32;

    for entry in dir.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().map_or(false, |e| e == "json") {
            total += 1;
            match migrate_session_file(&path) {
                Ok((did_migrate, _)) => {
                    if did_migrate {
                        migrated += 1;
                    }
                }
                Err(e) => {
                    eprintln!("[migration] ошибка {}: {}", path.display(), e);
                }
            }
        }
    }

    eprintln!(
        "[migration] завершено: {}/{} сессий обновлено",
        migrated, total
    );
}
