use crate::domain::agent_manager::AgentProfile;
use crate::infra::ChatMessage;

const TRUTH_PROTOCOL: &str = "YOU SHOULD: Tell the truth; never guess or speculate. Say 'I don't know' or 'I cannot confirm this' when something cannot be verified. Prioritize accuracy over speed. YOU MUST AVOID: Fabricating facts, making confident statements without proof, answering if unsure without disclosing uncertainty.";

fn get_user_query_from_messages(messages: &[ChatMessage]) -> Option<&str> {
    messages.iter()
        .find(|m| m.msg_type == "message" && m.author.as_deref() == Some("user") && !m.content.trim().is_empty())
        .map(|m| m.content.as_str())
}

fn render_report_tool_instruction() -> String {
    r#"
[ИНСТРУМЕНТ batch_get_agent_report]
Для получения одного или нескольких отчётов за один вызов:
```json
{"thought": "...", "tool": "batch_get_agent_report", "arguments": {"queries": [{"author": "ИМЯ_АГЕНТА", "namespace": "problem_1"}, {"author": "user", "namespace": "main"}]}}
```
author='user' для запроса пользователя. Инструмент вернёт самый свежий отчёт каждого указанного агента в указанном неймспейсе. Если нужен всего один отчёт — передай один запрос в queries.
"#.to_string()
}

pub fn build_system_prompt(
    agent: &AgentProfile,
    messages: &[ChatMessage],
    _namespace: &str,
    has_tools: bool,
    all_tools: &[(String, String, serde_json::Value)],
) -> String {
    let mut sp = agent.system_prompt.clone();
    sp.push_str("\n\n[ПРОТОКОЛ ЧЕСТНОСТИ]\n");
    sp.push_str(TRUTH_PROTOCOL);

    // Запрос пользователя — для контекста всем агентам
    if let Some(uq) = get_user_query_from_messages(messages) {
        sp.push_str(&format!("\n\n[ЗАПРОС ПОЛЬЗОВАТЕЛЯ]\n{}\n", uq));
    }

    // Инструкция по batch_get_agent_report — для всех агентов
    sp.push_str(&render_report_tool_instruction());

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