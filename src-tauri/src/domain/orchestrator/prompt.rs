use crate::domain::agent_manager::AgentProfile;
use crate::domain::signals::{build_cloud_signal_prompt_block, SignalContract};
use crate::infra::ChatMessage;
use chrono::Datelike;

const TRUTH_PROTOCOL: &str = "ОТВЕЧАЙ ТОЛЬКО ПРАВДУ. Если не знаешь, скажи 'я не знаю'. Запрещено выдумывать факты, давать ложные утверждения или строить догадки. Приоритет — точность, а не скорость.\nRespond strictly based on verified facts. If you do not have sufficient information to answer confidently, you must output exactly 'я не знаю' or 'I lack the data' without any guessing.";

/// Глобальное ограничение для всех агентов (единый источник — SSOT).
/// Дублируется на этапе сборки worst-case промпта для оценки контекста.
pub const CRITICAL_LIMIT_BLOCK: &str = "Твой максимальный лимит генерации строго ограничен. Твои внутренние размышления должны состоять максимум из 3-4 предложений, после чего сразу должен идти финальный ответ или вызов.";

/// Блок текущей даты — инжектится в промпт агентов с флагом `current_date: true`.
pub fn current_date_block() -> String {
    const WEEKDAYS: [&str; 7] = [
        "понедельник",
        "вторник",
        "среда",
        "четверг",
        "пятница",
        "суббота",
        "воскресенье",
    ];
    const MONTHS: [&str; 12] = [
        "января",
        "февраля",
        "марта",
        "апреля",
        "мая",
        "июня",
        "июля",
        "августа",
        "сентября",
        "октября",
        "ноября",
        "декабря",
    ];
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
    messages: &[ChatMessage],
    has_tools: bool,
    all_tools: &[(String, String, serde_json::Value)],
    max_gen_tokens: usize,
    signal_contract: Option<&SignalContract>,
    is_native: bool,
    is_hybrid: bool,
    is_cloud: bool,
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
    sp.push_str(&format!("\n\n{}", language_directive(messages)));

    // Native (coder/research): тул-схемы инжектся чат-шаблоном (--jinja), а markdown-секции
    // [ПРАВИЛА ВЫЗОВА ИНСТРУМЕНТОВ]/[ДОСТУПНЫЕ ИНСТРУМЕНТЫ] противоречат нативному
    // протоколу и дублируют схемы → для native-агентов НЕ добавляются.
    if is_native {
        sp.push_str("\n\n[NATIVE TOOL CALLING]\nТы вызываешь инструменты через родной протокол (function calling): когда нужен инструмент — объявляй вызов функции с аргументами строго по JSON-schema тула. Результаты инструментов приходят отдельными сообщениями. Только после фактических результатов инструментов дай финальный ответ ОБЫЧНЫМ ТЕКСТОМ — БЕЗ JSON-конвертов и фигурных разметок вызова инструментов.");
        if is_hybrid {
            sp.push_str("\nФинальный вердикт по задаче запрашивается отдельным шагом в строгом JSON по твоей роли — НЕ оформляй вердикт в этом ходе, сначала собери факты инструментами.");
        }
    }

    // Для Method 3-агентов грамматика сама принуждает JSON через emit_signal
    // (локальный движок), поэтому секции [ПРАВИЛА ВЫЗОВА ИНСТРУМЕНТОВ] и
    // [ДОСТУПНЫЕ ИНСТРУМЕНТЫ] не добавляются — они противоречат грамматике
    // и тратят когнитивную энергию модели. Облачным моделям грамматика
    // недоступна — форму сигнала задаёт промпт-блок, собранный ИЗ КОНТРАКТА
    // (root.schema.json): .md агента механику вызова не дублирует
    // (правило развода — docs/AGENT_CREATION_GUIDE.md).
    if let Some(contract) = signal_contract {
        if is_cloud {
            let block = build_cloud_signal_prompt_block(contract);
            sp.push_str("\n\n");
            sp.push_str(&block);
        }
    }

    if has_tools && signal_contract.is_none() && !is_native {
        sp.push_str("\n\n[ПРАВИЛА ВЫЗОВА ИНСТРУМЕНТОВ]\nЕсли нужен инструмент — верни ОДИН JSON-блок (```json ... ```).\nВ JSON обязательно поле \"thought\".\n\n⚠️ ВАЖНО: Если задача ВЫПОЛНЕНА — пиши ОБЫЧНЫЙ ТЕКСТ без JSON!\n");
    }

    if has_tools && signal_contract.is_none() && !is_native {
        let mut td = String::new();
        for (_, name, tool) in all_tools {
            let desc = tool
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            td.push_str(&format!("- \"{}\": {}\n", name, desc));
            if let Some(input_schema) = tool.get("inputSchema") {
                let type_name = input_schema
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("object");
                td.push_str(&format!("  Тип: {}\n", type_name));
                if let Some(props) = input_schema.get("properties").and_then(|p| p.as_object()) {
                    td.push_str("  Параметры (arguments):\n  {\n");
                    let required = input_schema
                        .get("required")
                        .and_then(|r| r.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                        .unwrap_or_default();
                    for (prop_name, prop_schema) in props {
                        let prop_type = prop_schema
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("any");
                        let prop_desc = prop_schema
                            .get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or("");
                        let is_required = if required.contains(&prop_name.as_str()) {
                            " [ОБЯЗАТЕЛЬНО]"
                        } else {
                            ""
                        };
                        td.push_str(&format!(
                            "    \"{}\" (type: {}){} - {}\n",
                            prop_name, prop_type, is_required, prop_desc
                        ));
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

/// Финальная сборка системного промпта агента: плагины → инжект отчётов
/// коллег → блок критического лимита.
///
/// Вынесено отдельной функцией, потому что **стенды замеров обязаны видеть тот же
/// текст, что и боевая нода**. Пока сборка жила внутри `run_agent_node`, стенд
/// вынужден был повторять её у себя, и замерял бы промпт без блока лимита и без
/// правок плагинов — то есть другой промпт при той же модели.
///
/// Порядок частей сохранён как был: плагины видели промпт уже с медиа-блоком, а
/// критический лимит оставался последним разделом.
pub fn finalize_system_prompt(
    base: &str,
    agent_id: &str,
    injected_reports: &str,
) -> String {
    let mut sp = base.to_string();
    crate::infra::plugins::global_plugins().on_system_prompt(agent_id, &mut sp);

    if !injected_reports.is_empty() {
        sp.push_str("\n\n");
        sp.push_str(injected_reports);
    }

    // Глобальное правило для всех мыслящих моделей, чтобы не пробивали лимит ответа.
    sp.push_str("\n\n[КРИТИЧЕСКОЕ ОГРАНИЧЕНИЕ]\n");
    sp.push_str(CRITICAL_LIMIT_BLOCK);
    sp
}

/// Строгая инструкция по языку ответа, основанная на языке последнего
/// сообщения пользователя. Слабые модели игнорируют общие фразы вроде
/// «отвечай на языке пользователя» и сползают на английский по умолчанию,
/// поэтому инструкция формулируется ЯВНО и (для русского) двуязычно.
pub(crate) fn language_directive(messages: &[ChatMessage]) -> String {
    let text: String = messages
        .iter()
        .rev()
        .filter(|m| m.author.as_deref() == Some("user") && !m.content.trim().is_empty())
        .take(5)
        .map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join(" ");

    let cyr = text
        .chars()
        .filter(|c| ('\u{0400}'..='\u{04FF}').contains(c))
        .count();
    let lat = text.chars().filter(|c| c.is_ascii_alphabetic()).count();

    if cyr > lat {
        "⚠️ ВАЖНО: ОТВЕЧАЙ ИСКЛЮЧИТЕЛЬНО НА РУССКОМ ЯЗЫКЕ (RESPOND IN RUSSIAN). Не переключайся на английский и другие языки, даже если в промпте или истории встречается другой язык.".to_string()
    } else {
        "IMPORTANT: RESPOND STRICTLY IN THE SAME LANGUAGE AS THE USER'S LAST MESSAGE (mirror it). Do not switch to another language.".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::ChatMessage;

    fn user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            id: None,
            msg_type: "message".to_string(),
            content: content.to_string(),
            sub_calls: None,
            author: Some("user".to_string()),
            model: None,
            time_sec: None,
            attachments: None,
            phase: None,
            author_name: None,
        }
    }

    #[test]
    fn language_directive_russian() {
        let msgs = vec![user_msg("привет, как у тебя дела?")];
        let d = language_directive(&msgs);
        assert!(
            d.contains("РУССКОМ"),
            "ожидается русская инструкция, получено: {}",
            d
        );
    }

    #[test]
    fn language_directive_latin_mirrors() {
        let msgs = vec![user_msg("hello, how are you?")];
        let d = language_directive(&msgs);
        assert!(
            d.contains("SAME LANGUAGE"),
            "ожидается mirror-инструкция, получено: {}",
            d
        );
    }

    #[test]
    fn language_directive_empty_mirrors() {
        let msgs: Vec<ChatMessage> = vec![];
        let d = language_directive(&msgs);
        assert!(
            d.contains("SAME LANGUAGE"),
            "при пустых сообщениях — mirror (англ. fallback), получено: {}",
            d
        );
    }

    /// Порядок частей системного промпта зафиксирован: инжект идёт ДО блока
    /// критического лимита. Если блок переедет выше, модель прочитает «не
    /// пробивай лимит» как часть отчёта коллеги, а не как правило себе.
    #[test]
    fn finalize_keeps_inject_before_critical_limit() {
        let prompt = finalize_system_prompt("БАЗА", "agent_x", "ИНЖЕКТ");
        let base = prompt.find("БАЗА").expect("база на месте");
        let injected = prompt.find("ИНЖЕКТ").expect("инжект на месте");
        let limit = prompt
            .find("[КРИТИЧЕСКОЕ ОГРАНИЧЕНИЕ]")
            .expect("блок лимита на месте");
        assert!(base < injected, "инжект должен идти после базового промпта");
        assert!(
            injected < limit,
            "блок лимита обязан оставаться последним разделом"
        );
    }

    /// Пустой инжект не оставляет пустого раздела: лишний перевод строки перед
    /// блоком лимита меняет текст, который видят и бой, и стенды.
    #[test]
    fn finalize_without_inject_has_no_empty_section() {
        let prompt = finalize_system_prompt("БАЗА", "agent_x", "");
        assert!(!prompt.contains("\n\n\n"), "пустой инжект оставил пустой раздел");
    }

    // ── Контракт-рендеринг облачного блока сигнала (SSOT root.schema.json) ──

    fn make_agent(system: &str) -> AgentProfile {
        AgentProfile {
            id: "test_agent".to_string(),
            name: "Тест".to_string(),
            description: String::new(),
            system_prompt: system.to_string(),
            mode: "worker".to_string(),
            mcp_servers: Vec::new(),
            subagents: Vec::new(),
            folder: None,
            replace_report: false,
            tools: Vec::new(),
            current_date: false,
            temperature: None,
            vision: false,
            think_budget_chars: None,
            disable_reasoning: false,
            require_tool_call: false,
            visible: true,
        }
    }

    /// Облачный блок для обычного контракта рендерит key и enum значений
    /// из root.schema.json — .md агента механику вызова не дублирует.
    #[test]
    fn cloud_signal_block_renders_contract_key_and_enum() {
        let agent = make_agent("переводчик симптомов");
        let contract = SignalContract {
            key: "soma_translator".to_string(),
            value_schema: serde_json::json!({
                "type": "string",
                "enum": ["НУЖНА РУКОСТЬ", "РУКОСТЬ ИЗВЕСТНА"]
            }),
        };
        let sp = build_system_prompt(&agent, &[], false, &[], 2048, Some(&contract), false, false, true);
        assert!(
            sp.contains("[ТРЕБОВАНИЕ СИГНАЛА (CLOUD MODEL)]"),
            "облачному контракт-агенту блок сигнала обязан приходить"
        );
        assert!(
            sp.contains("\"key\": \"soma_translator\""),
            "ключ контракта попадает в блок из root.schema.json"
        );
        assert!(
            sp.contains("НУЖНА РУКОСТЬ") && sp.contains("РУКОСТЬ ИЗВЕСТНА"),
            "enum значения из схемы попадают в блок"
        );
        assert!(
            !sp.contains("[ДОСТУПНЫЕ ИНСТРУМЕНТЫ]"),
            "сигнальному агенту legacy-секции тулов не выдаются"
        );
    }

    /// Облачный блок для слотового контракта описывает слотовый формат
    /// (конверт emit_signal слотовому контракту не показывается) — тот же
    /// формат, что собирает assemble_slot_signal_value.
    #[test]
    fn cloud_slot_contract_instructs_slots_not_envelope() {
        let agent = make_agent("валидатор");
        let contract = SignalContract {
            key: "validator_report".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "e1": { "type": "boolean" },
                    "pattern_type": { "type": "string", "enum": ["1", "2"] }
                },
                "required": ["e1", "pattern_type"]
            }),
        };
        let sp = build_system_prompt(&agent, &[], false, &[], 2048, Some(&contract), false, false, true);
        assert!(sp.contains("[ТРЕБОВАНИЕ СИГНАЛА (CLOUD MODEL)]"));
        assert!(
            sp.contains("`Element 1: `"),
            "якорь слота из плана попадает в облачный блок"
        );
        assert!(
            sp.contains("\"e1\": true"),
            "формат вердикта слота описан в блоке"
        );
        assert!(
            sp.contains("pattern_type") && sp.contains("\"1\""),
            "enum финального слота попадает в блок"
        );
        assert!(
            !sp.contains("\"tool\": \"emit_signal\""),
            "слотовому контракту конверт emit_signal не показывается"
        );
    }

    /// На локальном движке блок сигнала не добавляется: форму держит
    /// грамматика (build_signal_envelope_grammar*), промпт остаётся прежним.
    #[test]
    fn local_contract_agent_has_no_cloud_block() {
        let agent = make_agent("валидатор");
        let contract = SignalContract {
            key: "validator_report".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "properties": { "e1": { "type": "boolean" } },
                "required": ["e1"]
            }),
        };
        let sp = build_system_prompt(&agent, &[], false, &[], 2048, Some(&contract), false, false, false);
        assert!(
            !sp.contains("[ТРЕБОВАНИЕ СИГНАЛА"),
            "на локальном движке облачный блок сигнала не добавляется"
        );
    }
}
