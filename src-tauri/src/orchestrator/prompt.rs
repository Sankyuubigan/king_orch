use crate::agent_manager::{build_l0_manifest, AgentProfile};
use std::collections::HashMap;

const TRUTH_PROTOCOL: &str = "YOU SHOULD: Tell the truth; never guess or speculate. Say 'I don't know' or 'I cannot confirm this' when something cannot be verified. Prioritize accuracy over speed. YOU MUST AVOID: Fabricating facts, making confident statements without proof, answering if unsure without disclosing uncertainty.";

pub fn render_session_state_status(dossier: &HashMap<String, String>) -> String {
    if dossier.is_empty() { return "[СТАТУС СОСТОЯНИЯ СЕССИИ]: Пусто — данные ещё не собраны.".to_string(); }
    let mut lines = vec!["[СТАТУС СОСТОЯНИЯ СЕССИИ]:".to_string()];
    for (key, value) in dossier {
        if value.is_empty() { lines.push(format!("- {}: ❌ Нет данных", key)); }
        else { lines.push(format!("- {}: ✅ Данные собраны", key)); }
    }
    lines.join("\n")
}

pub fn render_session_state_full(dossier: &HashMap<String, String>) -> String {
    if dossier.is_empty() { return "[СОСТОЯНИЕ СЕССИИ]: Пусто.".to_string(); }
    let mut lines = vec!["[СОСТОЯНИЕ СЕССИИ]:".to_string()];
    for (key, value) in dossier {
        if value.is_empty() { lines.push(format!("--- {} ---\n(Нет данных)", key)); }
        else { lines.push(format!("--- {} ---\n{}", key, value)); }
    }
    lines.join("\n\n")
}

#[allow(clippy::too_many_arguments)]
pub fn build_system_prompt(
    agent: &AgentProfile, dossier: &HashMap<String, String>,
    has_subagents: bool, has_tools: bool,
    filtered_agents: &[AgentProfile],
    all_tools: &[(String, String, serde_json::Value)],
) -> String {
    let mut sp = agent.system_prompt.clone();
    sp.push_str("\n\n[ПРОТОКОЛ ЧЕСТНОСТИ]\n");
    sp.push_str(TRUTH_PROTOCOL);

    match agent.mode.as_str() {
        "router" => sp.push_str(&format!("\n\n{}\n", render_session_state_status(dossier))),
        _ => sp.push_str(&format!("\n\n{}\n", render_session_state_full(dossier))),
    }

    if agent.mode == "worker" && dossier.contains_key("user_query") && !dossier["user_query"].is_empty() {
        sp.push_str("\n[ИНСТРУКЦИЯ]: Данные пользователя находятся в `user_query` в [СОСТОЯНИЕ СЕССИИ]. Используй их. НЕ запрашивай повторно.\n");
    }

    if has_subagents || has_tools {
        sp.push_str("\n\n[ПРАВИЛА ВЫЗОВА]\nЕсли нужен сабагент или инструмент — верни ОДИН JSON-блок (```json ... ```).\nВ JSON обязательно поле \"thought\".\n\n⚠️ ВАЖНО: Если задача ВЫПОЛНЕНА — пиши ОБЫЧНЫЙ ТЕКСТ без JSON!\n");
        if has_subagents {
            sp.push_str("\nДля вызова сабагента:\n```json\n{\"thought\": \"...\", \"target\": \"ID\", \"task_or_response\": \"ЗАДАЧА\"}\n```\nДля прямого ответа:\n```json\n{\"thought\": \"...\", \"target\": \"reply\", \"task_or_response\": \"ОТВЕТ\"}\n```\n");
        }
        if agent.mode == "router" {
            sp.push_str("\n\n[АБСОЛЮТНОЕ ПРАВИЛО ДЛЯ МАРШРУТИЗАТОРА]\nТы ВСЕГДА отвечаешь в формате JSON.\n");
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