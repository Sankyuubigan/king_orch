use super::*;
use std::path::Path;
use std::fs;
use std::io::Write;
use serde_json::Value;
use crate::infra::{ChatMessage, LlmMessage, SubCall, ToolCallInfo, ModelParams, ChatAttachment, LlamaEngine, GrammarSpec, extract_model_filename, push_report};
use crate::domain::agent_manager::AgentProfile;

/// Ключ хранения чек-листа задач агента в сессии (как `thought`-сообщение).
pub(crate) fn todo_store_key(agent_id: &str) -> String {
    format!("todo::{}", agent_id)
}

/// Прочитать чек-лист задач агента из сессии (или пустой список).
pub(crate) fn read_todos(messages: &[ChatMessage], agent_id: &str) -> Vec<(String, bool)> {
    let key = todo_store_key(agent_id);
    for m in messages {
        if m.msg_type == "thought" && m.author.as_deref() == Some(&key) {
            if let Ok(v) = serde_json::from_str::<Vec<(String, bool)>>(&m.content) {
                return v;
            }
        }
    }
    Vec::new()
}

/// Записать/обновить чек-лист задач агента в сессии (персистится, переживает компакцию).
pub(crate) fn write_todos(messages: &mut Vec<ChatMessage>, agent_id: &str, todos: &[(String, bool)]) {
    let key = todo_store_key(agent_id);
    let content = serde_json::to_string(todos).unwrap_or_default();
    for m in messages.iter_mut() {
        if m.msg_type == "thought" && m.author.as_deref() == Some(&key) {
            m.content = content;
            return;
        }
    }
    messages.push(ChatMessage {
        id: None,
        msg_type: "thought".to_string(),
        content,
        sub_calls: None,
        author: Some(key),
        model: None,
    });
}

/// Исполнение туду-инструментов (`todo_write` / `todo_list`).
pub(crate) fn run_todo_tool(
    tool_name: &str,
    arguments: &serde_json::Value,
    messages: &mut Vec<ChatMessage>,
    agent_id: &str,
) -> String {
    let mut todos = read_todos(messages, agent_id);
    match tool_name {
        "todo_list" => {
            if todos.is_empty() {
                return "📋 Список задач пуст.".to_string();
            }
            let mut s = String::from("📋 Список задач:\n");
            for (i, (t, done)) in todos.iter().enumerate() {
                s.push_str(&format!("{}. [{}] {}\n", i + 1, if *done { "x" } else { " " }, t));
            }
            s
        }
        "todo_write" => {
            let action = arguments
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("add");
            match action {
                "list" => run_todo_tool("todo_list", arguments, messages, agent_id),
                "add" => {
                    let title = arguments
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if title.trim().is_empty() {
                        return "❌ Ошибка: для добавления нужен 'title' (текст задачи).".to_string();
                    }
                    todos.push((title.clone(), false));
                    write_todos(messages, agent_id, &todos);
                    format!("✅ Добавлена задача '{}'. Всего задач: {}.", title, todos.len())
                }
                "done" | "remove" => {
                    let idx = resolve_todo_index(arguments, &todos);
                    match idx {
                        Some(i) => {
                            if action == "done" {
                                let t = todos[i].0.clone();
                                todos[i].1 = true;
                                write_todos(messages, agent_id, &todos);
                                format!("✅ Задача '{}' отмечена выполненной.", t)
                            } else {
                                let t = todos.remove(i).0;
                                write_todos(messages, agent_id, &todos);
                                format!("🗑 Удалена задача '{}'.", t)
                            }
                        }
                        None => "❌ Ошибка: укажи 'index' (номер задачи) или 'title'.".to_string(),
                    }
                }
                "clear" => {
                    write_todos(messages, agent_id, &[]);
                    "🗑 Список задач очищен.".to_string()
                }
                _ => "❌ Ошибка: неизвестное действие. Используй add/done/remove/clear/list.".to_string(),
            }
        }
        _ => "❌ Неизвестный todo-инструмент.".to_string(),
    }
}

/// Найти индекс задачи по `index` (1-based) или по `title` (подстрока).
pub(crate) fn resolve_todo_index(
    arguments: &serde_json::Value,
    todos: &[(String, bool)],
) -> Option<usize> {
    if let Some(i) = arguments.get("index").and_then(|v| v.as_u64()) {
        let i = i as usize;
        if i >= 1 && i <= todos.len() {
            return Some(i - 1);
        }
    }
    if let Some(t) = arguments.get("title").and_then(|v| v.as_str()) {
        let t = t.trim().to_lowercase();
        return todos.iter().position(|(title, _)| title.to_lowercase().contains(&t));
    }
    None
}

