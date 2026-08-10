use crate::domain::agent_manager::AgentProfile;
use crate::infra::ChatMessage;
use chrono::Datelike;

const TRUTH_PROTOCOL: &str = "ОТВЕЧАЙ ТОЛЬКО ПРАВДУ. Если не знаешь, скажи 'я не знаю'. Запрещено выдумывать факты, давать ложные утверждения или строить догадки. Приоритет — точность, а не скорость.\nRespond strictly based on verified facts. If you do not have sufficient information to answer confidently, you must output exactly 'я не знаю' or 'I lack the data' without any guessing.";

/// Глобальное ограничение для всех агентов (единый источник — SSOT).
/// Дублируется на этапе сборки worst-case промпта для оценки контекста.
pub const CRITICAL_LIMIT_BLOCK: &str = "Твой максимальный лимит генерации строго ограничен. Твои внутренние размышления должны состоять максимум из 3-4 предложений, после чего сразу должен идти финальный ответ или вызов.";

/// Блок текущей даты — инжектится в промпт агентов с флагом `current_date: true`.
pub fn current_date_block() -> String {
    const WEEKDAYS: [&str; 7] = ["понедельник", "вторник", "среда", "четверг", "пятница", "суббота", "воскресенье"];
    const MONTHS: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
    let now = chrono::Local::now();
    let weekday = WEEKDAYS[now.weekday().num_days_from_monday() as usize];
    let month = MONTHS[now.month0() as usize];
    format!(
        "[ТЕКУЩАЯ ДАТА]\nСегодня {}, {} {} {}. Текущее местное время: {}.\nЭто ЕДИНСТВЕННЫЙ источник истины о текущей дате и времени. Не полагайся на свои внутренние «знания» о том, какой сейчас год/месяц — они устарели. При оценке актуальности информации, расчёте дат и упоминании «сегодня» ориентируйся строго на эту дату.",
        weekday,
        now.day(),
        month,
        now.year(),
        now.format("%H:%M"),
    )
}

pub fn build_system_prompt(
    agent: &AgentProfile,
    _messages: &[ChatMessage],
    has_tools: bool,
    all_tools: &[(String, String, serde_json::Value)],
    max_gen_tokens: usize,
) -> String {
    let mut sp = agent.system_prompt.clone();

    // Агентам с флагом current_date: true инжектим актуальную дату/время при каждом вызове.
    if agent.current_date {
        sp = format!("{}\n\n{}", current_date_block(), sp);
    }
    
    // ДОБАВЛЯЕМ ЛИМИТ ГЕНЕРАЦИИ ДЛЯ ЗАЩИТЫ ОТ ОБРЫВОВ
    sp.push_str(&format!("\n\n[ЛИМИТ ОТВЕТА]\nТвой жесткий лимит генерации — {} токенов. Строй свой ответ так, чтобы гарантированно успеть завершить мысль. Писать длинно НЕ обязательно. Если можешь ответить кратко — отвечай кратко.", max_gen_tokens));
    
    sp.push_str("\n\n[ПРОТОКОЛ ЧЕСТНОСТИ]\n");
    sp.push_str(TRUTH_PROTOCOL);
    sp.push_str("\n\n⚠️ ВАЖНО: ОТВЕЧАЙ НА ТОМ ЖЕ ЯЗЫКЕ, ЧТО И ПОЛЬЗОВАТЕЛЬ.");

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