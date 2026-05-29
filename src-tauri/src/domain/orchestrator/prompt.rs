use crate::domain::agent_manager::{build_l0_manifest, AgentProfile};
use std::collections::HashMap;

const TRUTH_PROTOCOL: &str = "YOU SHOULD: Tell the truth; never guess or speculate. Say 'I don't know' or 'I cannot confirm this' when something cannot be verified. Prioritize accuracy over speed. YOU MUST AVOID: Fabricating facts, making confident statements without proof, answering if unsure without disclosing uncertainty.";

pub fn render_session_state_status(
    dossier: &HashMap<String, HashMap<String, String>>,
    namespace: &str,
) -> String {
    if dossier.is_empty() {
        return "[СТАТУС СОСТОЯНИЯ СЕССИИ]: Пусто — данные ещё не собраны.".to_string();
    }

    if namespace == "main" {
        let mut lines = vec!["[СТАТУС СОСТОЯНИЯ СЕССИИ]:".to_string()];
        let mut has_data = false;
        for (ns, agents) in dossier {
            if agents.is_empty() { continue; }
            has_data = true;
            lines.push(format!("=== Контекст: {} ===", ns));
            for (agent_id, value) in agents {
                if value.is_empty() {
                    lines.push(format!("- {}: ❌ Нет данных", agent_id));
                } else if agent_id == "user_query" {
                    lines.push(format!("- user_query: {}", value));
                } else {
                    lines.push(format!("- {}: ✅ Данные собраны", agent_id));
                }
            }
        }
        if !has_data { return "[СТАТУС СОСТОЯНИЯ СЕССИИ]: Пусто — данные ещё не собраны.".to_string(); }
        lines.join("\n")
    } else {
        let mut lines = vec![format!("[СТАТУС СОСТОЯНИЯ СЕССИИ] (Контекст: {}):", namespace)];

        if let Some(main_ns) = dossier.get("main") {
            if let Some(user_query) = main_ns.get("user_query") {
                lines.push(format!("- user_query: {}", user_query));
            }
        }

        match dossier.get(namespace) {
            Some(agents) if !agents.is_empty() => {
                for (agent_id, value) in agents {
                    if value.is_empty() {
                        lines.push(format!("- {}: ❌ Нет данных", agent_id));
                    } else if agent_id == "user_query" {
                        lines.push(format!("- user_query: {}", value));
                    } else {
                        lines.push(format!("- {}: ✅ Данные собраны", agent_id));
                    }
                }
            }
            _ => { lines.push("(Данные ещё не собраны в этом контексте)".to_string()); }
        }
        lines.join("\n")
    }
}

pub fn render_session_state_full(
    dossier: &HashMap<String, HashMap<String, String>>,
    namespace: &str,
) -> String {
    if dossier.is_empty() {
        return "[СОСТОЯНИЕ СЕССИИ]: Пусто.".to_string();
    }

    if namespace == "main" {
        let mut lines = vec!["[СОСТОЯНИЕ СЕССИИ]:".to_string()];
        let mut has_data = false;
        for (ns, agents) in dossier {
            if agents.is_empty() { continue; }
            has_data = true;
            lines.push(format!("=== Контекст: {} ===", ns));
            for (agent_id, value) in agents {
                if value.is_empty() { lines.push(format!("--- {} ---\n(Нет данных)", agent_id)); }
                else { lines.push(format!("--- {} ---\n{}", agent_id, value)); }
            }
        }
        if !has_data { return "[СОСТОЯНИЕ СЕССИИ]: Пусто.".to_string(); }
        lines.join("\n\n")
    } else {
        let mut lines = vec![format!("[СОСТОЯНИЕ СЕССИИ] (Контекст: {}):", namespace)];

        if let Some(main_ns) = dossier.get("main") {
            if let Some(user_query) = main_ns.get("user_query") {
                lines.push(format!("--- user_query ---\n{}", user_query));
            }
        }

        match dossier.get(namespace) {
            Some(agents) if !agents.is_empty() => {
                for (agent_id, value) in agents {
                    if value.is_empty() { lines.push(format!("--- {} ---\n(Нет данных)", agent_id)); }
                    else { lines.push(format!("--- {} ---\n{}", agent_id, value)); }
                }
            }
            _ => { lines.push("(Данные ещё не собраны в этом контексте)".to_string()); }
        }
        lines.join("\n\n")
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_system_prompt(
    agent: &AgentProfile,
    dossier: &HashMap<String, HashMap<String, String>>,
    namespace: &str,
    has_subagents: bool,
    has_tools: bool,
    filtered_agents: &[AgentProfile],
    all_tools: &[(String, String, serde_json::Value)],
) -> String {
    let mut sp = agent.system_prompt.clone();
    sp.push_str("\n\n[ПРОТОКОЛ ЧЕСТНОСТИ]\n");
    sp.push_str(TRUTH_PROTOCOL);

    match agent.mode.as_str() {
        "router" => sp.push_str(&format!("\n\n{}\n", render_session_state_status(dossier, namespace))),
        _ => sp.push_str(&format!("\n\n{}\n", render_session_state_full(dossier, namespace))),
    }

    if agent.mode == "primary" {
        sp.push_str("\n[ИНСТРУКЦИЯ ДЛЯ КОММУНИКАТОРА]\n");
        sp.push_str("Ты общаешься напрямую с пользователем.\n");
        sp.push_str("🚫 КАТЕГОРИЧЕСКИЙ ЗАПРЕТ: НИКОГДА не копируй в свой ответ технические отчёты, JSON, внутренние статусы сабагентов (✅/❌) или фразы типа \"Ответ от сабагента\".\n");
        sp.push_str("✅ ПРАВИЛЬНО: Читай данные из [СОСТОЯНИЕ СЕССИИ] и формулируй краткий, эмпатичный ответ обычным текстом.\n");
        sp.push_str("Если сабагент вернул код диаграммы Mermaid — выведи его, но без технических пояснений.\n");
    }

    if agent.mode == "worker" {
        let has_query = dossier.get("main").and_then(|m| m.get("user_query")).is_some()
            || dossier.get(namespace).and_then(|m| m.get("user_query")).is_some();
        if has_query {
            sp.push_str("\n[ИНСТРУКЦИЯ]: Данные пользователя находятся в `user_query` в [СОСТОЯНИЕ СЕССИИ]. Используй их. НЕ запрашивай повторно.\n");
        }
    }

    if has_subagents || has_tools {
        sp.push_str("\n\n[ПРАВИЛА ВЫЗОВА]\nЕсли нужен сабагент или инструмент — верни ОДИН JSON-блок (```json ... ```).\nВ JSON обязательно поле \"thought\".\n\n⚠️ ВАЖНО: Если задача ВЫПОЛНЕНА — пиши ОБЫЧНЫЙ ТЕКСТ без JSON!\n");
        if has_subagents {
            sp.push_str("\nДля вызова сабагента:\n```json\n{\"thought\": \"...\", \"target\": \"ID\", \"task_or_response\": \"ЗАДАЧА\"}\n```\nС указанием неймспейса (контекста проблемы):\n```json\n{\"thought\": \"...\", \"target\": \"ID\", \"task_or_response\": \"ЗАДАЧА\", \"namespace\": \"problem_1\"}\n```\nЕсли `namespace` не указан — сабагент работает в текущем контексте.\nДля прямого ответа:\n```json\n{\"thought\": \"...\", \"target\": \"reply\", \"task_or_response\": \"ОТВЕТ\"}\n```\n");
        }
        if agent.mode == "router" {
            sp.push_str("\n\n[АБСОЛЮТНОЕ ПРАВИЛО ДЛЯ МАРШРУТИЗАТОРА]\nТы ВСЕГДА отвечаешь в формате JSON.\nНЕ вызывай сабагента, у которого уже ✅ в [СОСТОЯНИЕ СЕССИИ] — он уже отработал!\n");
        }
    }

    if has_subagents {
        sp.push_str("\n\n");
        sp.push_str(&build_l0_manifest(filtered_agents));
    }

    if has_tools {
        let mut td = String::from("[ДОСТУПНЫЕ ИНСТРУМЕНТЫ]\nДля вызова:\n```json\n{\"thought\": \"...\", \"tool\": \"ИМЯ\", \"arguments\": {}}\n```\n\n");
        for (_, name, tool) in all_tools {
            let desc = tool.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let schema = tool.get("inputSchema").cloned().unwrap_or(serde_json::Value::Null);
            td.push_str(&format!("- \"{}\": {} | Параметры: {}\n\n", name, desc, serde_json::to_string(&schema).unwrap_or_default()));
        }
        sp.push_str("\n\n");
        sp.push_str(&td);
    }

    sp
}