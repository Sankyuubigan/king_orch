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
            td.push_str(&format!("- \"{}\": {}\n", name, desc));
            if let Some(input_schema) = tool.get("inputSchema") {
                let type_name = input_schema.get("type").and_then(|t| t.as_str()).unwrap_or("object");
                td.push_str(&format!("  Тип: {}\n", type_name));
                if let Some(props) = input_schema.get("properties").and_then(|p| p.as_object()) {
                    td.push_str("  Параметры (arguments):\n  {\n");
                    let required = input_schema.get("required")
                        .and_then(|r| r.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                        .unwrap_or_default();
                    for (prop_name, prop_schema) in props {
                        let prop_type = prop_schema.get("type").and_then(|t| t.as_str()).unwrap_or("any");
                        let prop_desc = prop_schema.get("description").and_then(|d| d.as_str()).unwrap_or("");
                        let is_required = if required.contains(&prop_name.as_str()) { " [ОБЯЗАТЕЛЬНО]" } else { "" };
                        td.push_str(&format!("    \"{}\" (type: {}){} - {}\n", prop_name, prop_type, is_required, prop_desc));
                    }
                    td.push_str("  }\n");
                }
            }
            td.push('\n');
        }
        if !td.is_empty() {
            sp.push_str("\n\n[ДОСТУПНЫЕ ИНСТРУМЕНТЫ]\nДля вызова:\n```json\n{\"thought\": \"...\", \"tool\": \"ИМЯ\", \"arguments\": {}}\n```\n\n");
            sp.push_str(&td);
        }
    }

    sp
}