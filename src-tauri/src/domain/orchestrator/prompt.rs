use crate::domain::agent_manager::{build_l0_manifest, AgentProfile};
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

#[allow(clippy::too_many_arguments)]
pub fn build_system_prompt(
    agent: &AgentProfile,
    messages: &[ChatMessage],
    _namespace: &str,
    has_subagents: bool,
    has_tools: bool,
    filtered_agents: &[AgentProfile],
    all_tools: &[(String, String, serde_json::Value)],
) -> String {
    let mut sp = agent.system_prompt.clone();
    sp.push_str("\n\n[ПРОТОКОЛ ЧЕСТНОСТИ]\n");
    sp.push_str(TRUTH_PROTOCOL);

    if agent.mode == "primary" {
        sp.push_str("\n\n[ИНСТРУКЦИЯ ДЛЯ КОММУНИКАТОРА]\n");
        sp.push_str("Ты общаешься напрямую с пользователем.\n");
        sp.push_str("Для получения данных сабагентов используй `batch_get_agent_report`.\n");
        sp.push_str("Если сабагент вернул код диаграммы Mermaid — выведи его, но без технических пояснений.\n");
        sp.push_str(&render_report_tool_instruction());
    }

    if agent.mode == "worker" {
        if let Some(uq) = get_user_query_from_messages(messages) {
            sp.push_str(&format!("\n\n[ЗАПРОС ПОЛЬЗОВАТЕЛЯ]\n{}\n", uq));
        }
        sp.push_str("\nДля получения связанных отчётов других агентов используй `batch_get_agent_report`.\n");
    }

    if agent.mode == "router" {
        sp.push_str(&render_report_tool_instruction());
    }

    if has_subagents || has_tools {
        sp.push_str("\n\n[ПРАВИЛА ВЫЗОВА]\nЕсли нужен сабагент или инструмент — верни ОДИН JSON-блок (```json ... ```).\nВ JSON обязательно поле \"thought\".\n\n⚠️ ВАЖНО: Если задача ВЫПОЛНЕНА — пиши ОБЫЧНЫЙ ТЕКСТ без JSON!\n");
        if has_subagents {
            sp.push_str("\nДля вызова сабагента:\n```json\n{\"thought\": \"...\", \"target\": \"ID\", \"task_or_response\": \"ЗАДАЧА\"}\n```\nС указанием неймспейса (контекста проблемы):\n```json\n{\"thought\": \"...\", \"target\": \"ID\", \"task_or_response\": \"ЗАДАЧА\", \"namespace\": \"problem_1\"}\n```\nЕсли `namespace` не указан — сабагент работает в текущем контексте.\nДля прямого ответа:\n```json\n{\"thought\": \"...\", \"target\": \"reply\", \"task_or_response\": \"ОТВЕТ\"}\n```\n\n⚠️ Вызывай сабагентов ТОЛЬКО через \"target\". \"tool\" — только для batch_get_agent_report и MCP-инструментов. Никогда не используй \"tool\" для вызова сабагента.\n");
        }
        if agent.mode == "router" {
            sp.push_str("\n\n[АБСОЛЮТНОЕ ПРАВИЛО ДЛЯ МАРШРУТИЗАТОРА]\nТы ВСЕГДА отвечаешь в формате JSON.\nНЕ вызывай сабагента, у которого уже есть отчёт (проверь через `batch_get_agent_report`) — он уже отработал!\n");
            let agent_ids: Vec<&str> = filtered_agents.iter().map(|a| a.id.as_str()).collect();
            if !agent_ids.is_empty() {
                sp.push_str(&format!("Вызывай ТОЛЬКО сабагентов из списка выше. Доступные ID: {}. Не придумывай других — их не существует.\n", agent_ids.join(", ")));
            }
        }
    }

    if has_subagents {
        sp.push_str("\n\n");
        sp.push_str(&build_l0_manifest(filtered_agents));
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