use crate::domain::agent_manager::AgentProfile;
use crate::infra::ChatMessage;

const TRUTH_PROTOCOL: &str = "ОТВЕЧАЙ ТОЛЬКО ПРАВДУ. Если не знаешь — скажи 'я не знаю'. Запрещено выдумывать факты, давать ложные утверждения или строить догадки. Приоритет — точность, а не скорость.";

fn get_user_query_from_messages(messages: &[ChatMessage]) -> Option<&str> {
    messages.iter()
        .find(|m| m.msg_type == "message" && m.author.as_deref() == Some("user") && !m.content.trim().is_empty())
        .map(|m| m.content.as_str())
}

pub fn build_system_prompt(
    agent: &AgentProfile,
    messages: &[ChatMessage],
    has_tools: bool,
    all_tools: &[(String, String, serde_json::Value)],
) -> String {
    let mut sp = agent.system_prompt.clone();
    sp.push_str("\n\n[ПРОТОКОЛ ЧЕСТНОСТИ]\n");
    sp.push_str(TRUTH_PROTOCOL);
    sp.push_str("\n\n⚠️ ВАЖНО: ОТВЕЧАЙ НА ТОМ ЖЕ ЯЗЫКЕ, ЧТО И ПОЛЬЗОВАТЕЛЬ.");

    // Запрос пользователя — для контекста всем агентам
    if let Some(uq) = get_user_query_from_messages(messages) {
        sp.push_str(&format!("\n\n[ЗАПРОС ПОЛЬЗОВАТЕЛЯ]\n{}\n", uq));
    }

    if has_tools {
        sp.push_str("\n\n[ПРАВИЛА ВЫЗОВА ИНСТРУМЕНТОВ]\nЕсли нужен инструмент — верни ОДИН JSON-блок (```json ... ```).\nВ JSON обязательно поле \"thought\".\n\n⚠️ ВАЖНО: Если задача ВЫПОЛНЕНА — пиши ОБЫЧНЫЙ ТЕКСТ без JSON!\n");
    }

    if has_tools {
        let mut td = String::new();
        for (_, name, tool) in all_tools {
            let desc = tool.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let schema = tool.get("inputSchema").cloned().unwrap_or(serde_json::Value::Null);
            td.push_str(&format!("- \"{}\": {} | Параметры: {}\n\n", name, desc, serde_json::to_string(&schema).unwrap_or_default()));
        }
        if !td.is_empty() {
            sp.push_str("\n\n[ДОСТУПНЫЕ ИНСТРУМЕНТЫ]\nДля вызова:\n```json\n{\"thought\": \"...\", \"tool\": \"ИМЯ\", \"arguments\": {}}\n```\n\n");
            sp.push_str(&td);
        }
    }

    sp
}