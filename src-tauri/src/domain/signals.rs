//! 🚦 Контракт сигналов emit_signal (SSOT).
//!
//! Единый источник правды для сигналов: файл `signals/root.schema.json` рядом
//! с командами агентов (`agents/<команда>/signals/root.schema.json`). Структура:
//!
//! ```json
//! { "agents": { "<agent_id>": { "key": "<сигнальный key>", "value": { ... JSON Schema value ... } } } }
//! ```
//!
//! Агент, у которого есть контракт, вызывает `emit_signal` как инструмент
//! внутри своего обычного ответа (ОДИН вызов LLM, второго вызова нет).
//! Оркестратор парсит конверт и сохраняет сигнал. Форма конверта:
//! `{"tool":"emit_signal","arguments":{"key":..,"value":..}}; допускается
//! сплющенная форма без обёртки `arguments` — парсер нормализует её.
//! (`$ref` в схемах value не используется — вложенные `$ref` сломаны в
//! llama.cpp, ggml-org/llama.cpp#8073.)

use std::collections::HashMap;
use std::path::Path;
use serde_json::Value;

/// Контракт одного агента: сигнальный ключ + JSON Schema значения.
#[derive(Debug, Clone)]
pub struct SignalContract {
    pub key: String,
    pub value_schema: Value,
}

/// Загружает контракт конкретного агента из `root.schema.json`.
/// Возвращает `None`, если контракта для агента нет или файл отсутствует/битый.
/// Папку `signals` оркестратор берёт РЯДОМ с грамматиками (родитель grammars_dir),
/// чтобы не плодить второй резолвер путей: signals///<папка сигналов> = <grammars>/../signals.
pub fn load_signal_contract(signals_dir: &Path, agent_id: &str) -> Option<SignalContract> {
    let path = signals_dir.join("root.schema.json");
    let text = std::fs::read_to_string(&path).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    let agent = root.get("agents")?.get(agent_id)?;
    let key = agent.get("key")?.as_str()?;
    let value_schema = agent.get("value")?;
    if value_schema.is_object() || value_schema.is_string() {
        Some(SignalContract {
            key: key.to_string(),
            value_schema: value_schema.clone(),
        })
    } else {
        None
    }
}

/// Собирает JSON Schema конверта emit_signal для ПЕРЕДАЧИ движку.
/// Без `$ref` (сломаны в llama.cpp #8073): value-схема встраивается инлайн.
/// Требования движка: `additionalProperties` по умолчанию `false`,
/// `pattern` обязан начинаться `^` и заканчиваться `$`.
pub fn build_signal_envelope_schema(contract: &SignalContract) -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "thought": { "type": "string" },
            "tool": { "type": "string", "const": "emit_signal" },
            "arguments": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "const": contract.key },
                    "value": contract.value_schema
                },
                "required": ["key", "value"],
                "additionalProperties": false
            }
        },
        "required": ["tool", "arguments"],
        "additionalProperties": false
    })
}

/// Собирает гибридную GBNF-грамматику (Method 3 из docs/gbnf.md):
/// модель ОБЯЗАНА сначала сгенерировать `<think>...</think>` (свободные
/// рассуждения), и ТОЛЬКО ПОТОМ JSON-объект сигнала. Грамматика физически
/// не пустит модель к JSON, пока не встретит `</think>`.
///
/// Формат как в docs/gbnf.md строка 33:
/// ```text
/// root ::= think-block envelope-json (Method 3 из docs/gbnf.md)
/// thought-content ::= [^<]*
/// ```
pub fn build_signal_envelope_grammar(contract: &SignalContract) -> String {
    let schema = &contract.value_schema;

    // Правило пробелов (ограниченное, как в официальном конвертере llama.cpp)
    let sp = "sp ::= \" \" | \"\\n\" [ \\t]{0,4}";

    // Базовые JSON-правила
    let json_rules = concat!(
        "json-string ::= \"\\\"\" string-char* \"\\\"\"\n",
        "string-char ::= [^\"\\\\] | escape\n",
        "escape ::= \"\\\\\" (\"\\\\\" | \"\\\"\" | \"n\" | \"t\" | \"r\" | \"b\" | \"f\" | \"u\" [0-9a-f] [0-9a-f] [0-9a-f] [0-9a-f])\n",
        "json-number ::= \"-\"? (\"0\" | [1-9] [0-9]*) (\".\" [0-9]+)? ([eE] [-+]? [0-9]+)?\n",
        "json-bool ::= \"true\" | \"false\"\n",
        "bool ::= \"true\" | \"false\"\n",
    );

    // Генерируем value-грамматику в зависимости от типа контракта
    let value_grammar = build_value_grammar(schema);

    // Method 3 из docs/gbnf.md: гибридная GBNF с think-block.
    // root ::= think-block envelope-json
    // think-block ::= "<think>" [^<]* "</think>" | ""  — опциональный thinking.
    // Thinking-модели генерируют <think>...</think>, non-thinking — сразу JSON.
    // Без disable_reasoning: модель сама решает, думать или нет.
    format!(
        "root ::= think-block envelope-json\n\
         think-block ::= \"<think>\" [^<]* \"</think>\" | \"\"\n\
         \n\
         envelope-json ::= \"{{\" sp thought-field \",\" sp tool-field \",\" sp arguments-field sp \"}}\"\n\
         thought-field ::= \"\\\"thought\\\"\" sp \":\" sp json-string\n\
         tool-field ::= \"\\\"tool\\\"\" sp \":\" sp \"\\\"emit_signal\\\"\"\n\
         arguments-field ::= \"\\\"arguments\\\"\" sp \":\" sp arguments-json\n\
         arguments-json ::= \"{{\" sp key-field \",\" sp \"\\\"value\\\"\" sp \":\" sp value-field sp \"}}\"\n\
         key-field ::= \"\\\"key\\\"\" sp \":\" sp \"\\\"{}\\\"\"\n\
         {}\n\
         {}\n\
         {}",
        contract.key,
        value_grammar,
        sp,
        json_rules,
    )
}

/// Генерирует GBNF-грамматику для value-поля в зависимости от типа контракта.
fn build_value_grammar(schema: &serde_json::Value) -> String {
    // 1. Enum (массив строк) — альтернативы
    if let Some(enum_vals) = schema.get("enum").and_then(|e| e.as_array()) {
        let variants: Vec<String> = enum_vals
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| format!("\"\\\"{}\\\"\"", s))
            .collect();
        return format!("value-field ::= {}", variants.join(" | "));
    }

    // 2. Pattern одиночного символа (^[1-9]$) — список альтернатив
    if let Some(pat) = schema.get("pattern").and_then(|p| p.as_str()) {
        if let Some(inner) = pat.strip_prefix('^').and_then(|p| p.strip_suffix('$')) {
            if inner.starts_with('[') && inner.ends_with(']') {
                let chars = &inner[1..inner.len() - 1];
                // Обрабатываем диапазон X-Y (напр. "1-9")
                if let Some((lo, hi)) = chars.split_once('-') {
                    if let (Some(lo_ch), Some(hi_ch)) = (lo.chars().next(), hi.chars().next()) {
                        let variants: Vec<String> = (lo_ch as u8..=hi_ch as u8)
                            .map(|b| format!("\"\\\"{}\\\"\"", b as char))
                            .collect();
                        return format!("value-field ::= {}", variants.join(" | "));
                    }
                }
                // Обрабатываем список символов
                let variants: Vec<String> = chars
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .map(|c| format!("\"\\\"{}\\\"\"", c))
                    .collect();
                return format!("value-field ::= {}", variants.join(" | "));
            }
        }
    }

    // 3. Object со свойствами — генерируем фиксированные ключи
    if schema.get("type").and_then(|t| t.as_str()) == Some("object") {
        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
            let required_set: std::collections::HashSet<String> = schema
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let mut fields: Vec<String> = Vec::new();
            for (name, pschema) in props {
                let typ = pschema.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let value_rule = match typ {
                    "boolean" => "bool".to_string(),
                    "string" => "json-string".to_string(),
                    "number" | "integer" => "json-number".to_string(),
                    _ => "json-string".to_string(),
                };
                fields.push(format!(
                    "\"\\\"{}\\\"\" sp \":\" sp {}",
                    name, value_rule
                ));
            }

            // Обязательные ключи через запятую, опциональные — не включаем
            // (генерируем строго фиксированный порядок для required)
            let required_fields: Vec<String> = props
                .keys()
                .filter(|k| required_set.contains(*k))
                .map(|k| {
                    let pschema = &props[k];
                    let typ = pschema.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    let value_rule = match typ {
                        "boolean" => "bool".to_string(),
                        "string" => "json-string".to_string(),
                        "number" | "integer" => "json-number".to_string(),
                        _ => "json-string".to_string(),
                    };
                    format!("\"\\\"{}\\\"\" sp \":\" sp {}", k, value_rule)
                })
                .collect();

            return format!("value-field ::= \"{{\" sp {} sp \"}}\"", required_fields.join(" \",\" sp "));
        }
    }

    // Fallback — любое JSON-значение
    "value-field ::= json-string | json-number | json-bool | \"null\"".to_string()
}

/// ЧЕСТНАЯ валидация значения сигнала против контракта (SSOT).
///
/// Гарантирует, что `value`, присланный моделью в `emit_signal`, содержит все
/// обязательные поля контракта (напр. `element_1_mental` у `validator_report`). Если модель
/// исказила форму — возвращаем явную ошибку, которую оркестратор отдаёт модели на retry.
/// Это исключает тихий обрыв маршрутизации, когда `signal_router` не находит поле и граф
/// завершается, не дойдя до `message`-узла (см. docs/SIGNAL_CONTRACTS.md). Не является
/// костылём: это обычная валидация входа, а не «умолчание по дефолту».
pub fn validate_signal_value(contract: &SignalContract, value: &Value) -> Result<(), String> {
    let schema = &contract.value_schema;
    if schema.get("type").and_then(|t| t.as_str()) == Some("object") {
        // Проверяем ВСЕ properties контракта (а не только required).
        // Это гарантирует, что signal_router найдёт каждое поле (element_X_mental и т.д.).
        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
            let mut missing: Vec<String> = Vec::new();
            for (pname, _) in props {
                if value.get(pname).is_none() {
                    missing.push(pname.clone());
                }
            }
            if !missing.is_empty() {
                return Err(format!(
                    "Ошибка: emit_signal для '{}' требует все поля контракта [{}], но в value их нет. Исправь и вызови СНОВА.",
                    contract.key, missing.join(", ")
                ));
            }
        }
    }
    Ok(())
}

/// Детерминированное извлечение значения сигнала из ТЕКСТА ответа агента по
/// допустимым значениям контракта. НЕ делает LLM-вызов — просто ищет подстроку
/// или токен. Используется как fallback, когда агент не вызвал `emit_signal` нативно.
///
/// Поддерживаемые виды `value_schema`:
/// - `enum` (массив строк) → первая совпавшая строка;
/// - `object` со свойствами-`enum` → объект из совпавших свойств;
/// - `pattern` вида `^[1-9]$` → первый символ из набора (synthesizer: destructor_type).
/// Возвращает `None`, если ничего не нашлось (тогда сигнал не сохраняется).
pub fn extract_signal_value_from_text(contract: &SignalContract, text: &str) -> Option<Value> {
    let schema = &contract.value_schema;

    // 1) enum (массив строк)
    if let Some(enum_vals) = schema.get("enum").and_then(|e| e.as_array()) {
        for v in enum_vals {
            if let Some(s) = v.as_str() {
                if !s.is_empty() && text.contains(s) {
                    return Some(Value::String(s.to_string()));
                }
            }
        }
        return None;
    }

    // 2) object со свойствами-enum
    if schema.get("type").and_then(|t| t.as_str()) == Some("object") {
        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
            let mut obj = serde_json::Map::new();
            let mut matched = false;
            for (pname, pschema) in props {
                if let Some(enum_vals) = pschema.get("enum").and_then(|e| e.as_array()) {
                    for v in enum_vals {
                        if let Some(s) = v.as_str() {
                            if !s.is_empty() && text.contains(s) {
                                obj.insert(pname.clone(), Value::String(s.to_string()));
                                matched = true;
                                break;
                            }
                        }
                    }
                }
            }
            if matched {
                return Some(Value::Object(obj));
            }
        }
        return None;
    }

    // 3) pattern одиночного символа (напр. ^[1-9]$)
    if let Some(pat) = schema.get("pattern").and_then(|p| p.as_str()) {
        if let Some(inner) = pat.strip_prefix('^').and_then(|p| p.strip_suffix('$')) {
            if inner.starts_with('[') && inner.ends_with(']') {
                let set: Vec<char> = inner[1..inner.len() - 1].chars().collect();
                // Диапазон вида X-Y (напр. "1-9")
                if set.len() == 3 && set[1] == '-' {
                    let (lo, hi) = (set[0], set[2]);
                    for ch in text.chars() {
                        if ch >= lo && ch <= hi {
                            return Some(Value::String(ch.to_string()));
                        }
                    }
                } else {
                    for ch in text.chars() {
                        if set.contains(&ch) {
                            return Some(Value::String(ch.to_string()));
                        }
                    }
                }
            }
        }
    }

    None
}

/// Полностью инлайн-схемы агентов (карта agent_id → контракт) для отладки/тестов.
pub fn load_all_signal_contracts(signals_dir: &Path) -> HashMap<String, SignalContract> {
    let mut map = HashMap::new();
    let path = signals_dir.join("root.schema.json");
    let Ok(text) = std::fs::read_to_string(&path) else { return map };
    let Ok(root) = serde_json::from_str::<Value>(&text) else { return map };
    let Some(agents) = root.get("agents").and_then(|a| a.as_object()) else { return map };
    for (agent_id, agent) in agents {
        let Some(key) = agent.get("key").and_then(Value::as_str) else { continue };
        let Some(value_schema) = agent.get("value") else { continue };
        map.insert(
            agent_id.clone(),
            SignalContract { key: key.to_string(), value_schema: value_schema.clone() },
        );
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn schema_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../agents/psychotherapist/signals/root.schema.json")
    }

    #[test]
    fn root_schema_parses_and_has_all_emitters() {
        let text = std::fs::read_to_string(schema_path()).expect("root.schema.json прочитать");
        let root: Value = serde_json::from_str(&text).expect("root.schema.json — валидный JSON");
        let agents = root.get("agents").expect("ключ agents");
        for id in ["validator", "cluster_checker", "soma_translator", "synthesizer"] {
            let a = agents.get(id).unwrap_or_else(|| panic!("агент {id} в контракте"));
            assert!(a.get("key").is_some(), "{id}: key");
            assert!(a.get("value").is_some(), "{id}: value");
        }
    }

    #[test]
    fn load_contract_pulls_key_and_value() {
        let path = schema_path();
        let dir = path.parent().expect("папка signals");
        let c = load_signal_contract(dir, "validator").expect("контракт валидатора");
        assert_eq!(c.key, "validator_report");
        assert_eq!(c.value_schema["properties"]["element_1_mental"]["type"].as_str(), Some("boolean"));
        assert_eq!(c.value_schema["required"].as_array().map(|e| e.len()), Some(9));
    }

    /// Регрессия: soma_translator обязан уметь сообщить, что рукость
    /// ИЗВЕСТНА — иначе вопрос пользователю задаётся даже при наличии данных
    /// (контракт с единственным значением «НУЖНА РУКОСТЬ» = принудительный вопрос).
    #[test]
    fn soma_translator_contract_allows_known_handedness() {
        let path = schema_path();
        let dir = path.parent().expect("папка signals");
        let c = load_signal_contract(dir, "soma_translator").expect("контракт soma_translator");
        assert_eq!(c.key, "soma_translator");
        let enum_vals = c.value_schema["enum"].as_array().expect("enum в контракте soma");
        assert!(enum_vals.iter().any(|v| v.as_str() == Some("НУЖНА РУКОСТЬ")));
        assert!(enum_vals.iter().any(|v| v.as_str() == Some("РУКОСТЬ ИЗВЕСТНА")));
    }

    #[test]
    fn unknown_agent_returns_none() {
        let path = schema_path();
        let dir = path.parent().expect("папка signals");
        assert!(load_signal_contract(dir, "no_such_agent").is_none());
    }

    #[test]
    fn envelope_schema_is_inline_and_strict() {
        let contract = SignalContract {
            key: "destructor_type".to_string(),
            value_schema: serde_json::json!({ "type": "string", "pattern": "^[1-9]$" }),
        };
        let schema = build_signal_envelope_schema(&contract);
        assert_eq!(schema["properties"]["tool"]["const"], serde_json::json!("emit_signal"));
        assert_eq!(schema["properties"]["arguments"]["properties"]["key"]["const"], serde_json::json!("destructor_type"));
        assert_eq!(schema["properties"]["arguments"]["properties"]["value"]["pattern"], serde_json::json!("^[1-9]$"));
        // Без $ref — вложенные $ref ломаются в llama.cpp (#8073)
        assert!(serde_json::to_string(&schema).unwrap().contains("$ref") == false);
        assert_eq!(schema["required"], serde_json::json!(["tool", "arguments"]));
        assert_eq!(schema["properties"]["arguments"]["required"], serde_json::json!(["key", "value"]));
    }

    #[test]
    fn all_contracts_are_loadable() {
        let path = schema_path();
        let dir = path.parent().expect("папка signals");
        let map = load_all_signal_contracts(dir);
        assert_eq!(map.len(), 5, "все 5 сигналов");
        assert!(map.contains_key("validator"));
    }

    #[test]
    fn extract_signal_value_from_text_enum() {
        let c = SignalContract {
            key: "soma_translator".to_string(),
            value_schema: serde_json::json!({ "type": "string", "enum": ["НУЖНА РУКОСТЬ", "РУКОСТЬ ИЗВЕСТНА"] }),
        };
        // Точное совпадение enum-значения в тексте → извлекается.
        assert_eq!(
            extract_signal_value_from_text(&c, "разбор готов. РУКОСТЬ ИЗВЕСТНА, правая сторона — Мать"),
            Some(serde_json::json!("РУКОСТЬ ИЗВЕСТНА"))
        );
        // Нет точного совпадения enum → None (fallback не спасает).
        assert_eq!(
            extract_signal_value_from_text(&c, "биологический левша, анализ завершён"),
            None
        );
        assert_eq!(
            extract_signal_value_from_text(&c, "просто текст без маркеров"),
            None
        );
    }

    #[test]
    fn extract_signal_value_from_text_object_enum() {
        let c = SignalContract {
            key: "validator_report".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "missing_data": { "type": "string", "enum": ["не найдено", "есть"] },
                    "element_1_mental": { "type": "boolean" }
                }
            }),
        };
        // Текст содержит "не найдено" → missing_data извлекается (enum-свойство внутри объекта).
        assert_eq!(
            extract_signal_value_from_text(&c, "Дефицит: элемент 3 не найдено"),
            Some(serde_json::json!({ "missing_data": "не найдено" }))
        );
    }

    #[test]
    fn extract_signal_value_from_text_pattern() {
        let c = SignalContract {
            key: "destructor_type".to_string(),
            value_schema: serde_json::json!({ "type": "string", "pattern": "^[1-9]$" }),
        };
        assert_eq!(
            extract_signal_value_from_text(&c, "тип деструктора равен 7, глубинный"),
            Some(serde_json::json!("7"))
        );
        assert_eq!(
            extract_signal_value_from_text(&c, "нет цифр"),
            None
        );
    }

    #[test]
    fn validate_signal_value_rejects_missing_required_fields() {
        let c = SignalContract {
            key: "validator_report".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "element_1_mental": { "type": "boolean" },
                    "element_2_mental": { "type": "boolean" }
                },
                "required": ["element_1_mental", "element_2_mental"]
            }),
        };
        // Искажённая форма (старое поле verdict вместо element_X_mental) — ошибка.
        assert!(validate_signal_value(&c, &serde_json::json!({ "verdict": "ДАННЫХ ДОСТАТОЧНО" })).is_err());
        // Пустое значение без обязательных полей — тоже ошибка.
        assert!(validate_signal_value(&c, &serde_json::Value::Object(serde_json::Map::new())).is_err());
        // Корректная форма проходит.
        assert!(validate_signal_value(&c, &serde_json::json!({ "element_1_mental": true, "element_2_mental": false })).is_ok());
    }

    #[test]
    fn envelope_schema_is_inline_and_requires_all_fields() {
        let c = SignalContract {
            key: "validator_report".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "element_1_mental": { "type": "boolean" },
                    "element_2_mental": { "type": "boolean" }
                },
                "required": ["element_1_mental", "element_2_mental"]
            }),
        };
        let schema = build_signal_envelope_schema(&c);
        // Допускается свободный текст в thought + защищённый value с обязательными полями.
        assert_eq!(schema["properties"]["tool"]["const"], serde_json::json!("emit_signal"));
        assert_eq!(
            schema["properties"]["arguments"]["properties"]["value"]["required"],
            serde_json::json!(["element_1_mental", "element_2_mental"])
        );
        assert_eq!(
            schema["properties"]["arguments"]["properties"]["value"]["additionalProperties"],
            serde_json::json!(false)
        );
    }

    #[test]
    fn hybrid_grammar_contains_think_tags_and_json() {
        let c = SignalContract {
            key: "validator_report".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "element_1_mental": { "type": "boolean" },
                    "element_2_mental": { "type": "boolean" },
                    "missing_data": { "type": "string" }
                },
                "required": ["element_1_mental", "element_2_mental"]
            }),
        };
        let grammar = build_signal_envelope_grammar(&c);
        // Грамматика ОБЯЗАНА содержать think-теги как в docs/gbnf.md (строка 33)
        assert!(grammar.contains("\"azaar\\n\""), "грамматика должна содержать \"azaar\\n\"");
        assert!(grammar.contains("thought-content"), "грамматика должна содержать правило thought-content");
        // Грамматика ОБЯЗАНА содержать ключи сигнала
        assert!(grammar.contains("element_1_mental"), "грамматика должна содержать element_1_mental");
        assert!(grammar.contains("element_2_mental"), "грамматика должна содержать element_2_mental");
        // Грамматика ОБЯЗАНА содержать полный конверт
        assert!(grammar.contains("emit_signal"), "грамматика должна содержать emit_signal");
        assert!(grammar.contains("validator_report"), "грамматика должна содержать ключ validator_report");
    }

    #[test]
    fn hybrid_grammar_for_validator_contract() {
        let path = schema_path();
        let dir = path.parent().expect("папка signals");
        let c = load_signal_contract(dir, "validator").expect("контракт валидатора");
        let grammar = build_signal_envelope_grammar(&c);
        // Все 9 элементов должны быть в грамматике
        for i in 1..=9 {
            let key = format!("element_{}_mental", i);
            assert!(grammar.contains(&key), "грамматика должна содержать {}", key);
        }
        // think-теги как в docs/gbnf.md обязательны
        assert!(grammar.contains("\"azaar\\n\""), "должен быть \"azaar\\n\"");
    }

    /// Регрессия: грамматика для boolean-полей (element_X_mental) ОБЯЗАНА
    /// содержать определение правила `bool`, иначе llama-server падает с
    /// "Undefined rule identifier 'bool'" (HTTP 400).
    #[test]
    fn validator_grammar_defines_bool_rule() {
        let path = schema_path();
        let dir = path.parent().expect("папка signals");
        let c = load_signal_contract(dir, "validator").expect("контракт валидатора");
        let grammar = build_signal_envelope_grammar(&c);
        // Правило `bool` ДОЛЖНО быть определено (а не только json-bool)
        assert!(
            grammar.contains("bool ::="),
            "грамматика должна содержать определение правила `bool`, grammar:\n{}",
            grammar
        );
        // При этом `bool` не должен быть частью другого имени правила (json-bool и т.д.)
        // Проверяем что `bool ::=` стоит отдельной(rule definition)
        assert!(
            grammar.lines().any(|l| l.trim_start().starts_with("bool ::=")),
            "правило `bool` должно быть отдельной строкой (rule definition), grammar:\n{}",
            grammar
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // ТЕСТЫ ГИПОТЕЗ: формат GBNF для <think> тегов
    // ═══════════════════════════════════════════════════════════════════

    /// Гипотеза 1: Токенный синтаксис <thingk> / <end_of_think> (канонический из GBNF README llama.cpp)
    /// Проверяет что грамматика содержит токенные ссылки, а НЕ строковые литералы.
    #[test]
    fn hypothesis_token_syntax_no_string_literals() {
        let c = SignalContract {
            key: "test_key".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "field_a": { "type": "boolean" }
                },
                "required": ["field_a"]
            }),
        };
        let grammar = build_signal_envelope_grammar(&c);
        // Токенный синтаксис как в docs/gbnf.md: "azaar\n" + [^<]*
        assert!(grammar.contains("\"azaar\\n\""), "должен быть строковый литерал \"azaar\\n\", grammar:\n{}", grammar);
        assert!(grammar.contains("thought-content"), "должно быть правило thought-content, grammar:\n{}", grammar);
        // Не должно быть токенного синтаксиса (не работает с gemma-4)
        assert!(!grammar.contains("<thingk>"), "не должно быть токена <thingk>");
        assert!(!grammar.contains("<end_of_think>"), "не должно быть токена <end_of_think>");
    }

    /// Строковый литерал "azaar\n" как в docs/gbnf.md — проверяем что \n в строковом литерале
    #[test]
    fn hypothesis_string_literal_without_newline() {
        let c = SignalContract {
            key: "test_key".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "field_a": { "type": "boolean" }
                },
                "required": ["field_a"]
            }),
        };
        let grammar = build_signal_envelope_grammar(&c);
        // Должен быть строковый литерал "azaar\n" как в docs/gbnf.md
        assert!(grammar.contains("\"azaar\\n\""), "должен быть строковый литерал \"azaar\\n\", grammar:\n{}", grammar);
        // Корень правила должен начинаться с root ::=
        let root_line = grammar.lines().next().unwrap_or("");
        assert!(root_line.starts_with("root ::="), "корень должен быть root ::=");
    }

    /// Гипотеза 3: Полный конверт — грамматика должна содержать tool, arguments, key
    #[test]
    fn hypothesis_full_envelope_structure() {
        let c = SignalContract {
            key: "validator_report".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "element_1_mental": { "type": "boolean" }
                },
                "required": ["element_1_mental"]
            }),
        };
        let grammar = build_signal_envelope_grammar(&c);

        // Проверяем наличие ключевых частей конверта
        assert!(grammar.contains("thought"), "должно быть поле thought");
        assert!(grammar.contains("emit_signal"), "должно быть emit_signal");
        assert!(grammar.contains("arguments"), "должно быть arguments");
        assert!(grammar.contains("key"), "должно быть поле key");
        assert!(grammar.contains("validator_report"), "должен быть ключ validator_report");
        assert!(grammar.contains("element_1_mental"), "должен быть element_1_mental");
        assert!(grammar.contains("thought-content"), "должно быть правило thought-content");
        assert!(grammar.contains("envelope-json"), "должно быть правило envelope-json");
    }

    /// Гипотеза 4: Enum-контракт (soma_translator) — должен содержать альтернативы строк
    #[test]
    fn hypothesis_enum_contract_variants() {
        let c = SignalContract {
            key: "soma_translator".to_string(),
            value_schema: serde_json::json!({
                "type": "string",
                "enum": ["НУЖНА РУКОСТЬ", "РУКОСТЬ ИЗВЕСТНА"]
            }),
        };
        let grammar = build_signal_envelope_grammar(&c);

        // Enum-контракт должен содержать альтернативы
        assert!(grammar.contains("НУЖНА РУКОСТЬ"), "должен быть вариант НУЖНА РУКОСТЬ");
        assert!(grammar.contains("РУКОСТЬ ИЗВЕСТНА"), "должен быть вариант РУКОСТЬ ИЗВЕСТНА");
        // Не должно быть пустого bool_members
        assert!(!grammar.contains("bool_members ::="), "не должно быть пустого bool_members");
    }

    /// Гипотеза 5: Pattern-контракт (synthesizer) — должен содержать цифры 1-9
    #[test]
    fn hypothesis_pattern_contract_digits() {
        let c = SignalContract {
            key: "destructor_type".to_string(),
            value_schema: serde_json::json!({
                "type": "string",
                "pattern": "^[1-9]$"
            }),
        };
        let grammar = build_signal_envelope_grammar(&c);
        // Pattern ^[1-9]$ должен содержать цифры
        for d in 1..=9 {
            let needle = format!("\"\\\"{}\\\"\"", d);
            assert!(grammar.contains(&needle), "должна быть цифра {} (ищи {}), grammar:\n{}", d, needle, grammar);
        }
    }

    /// Гипотеза 6: destructor_detector — object с 5 строковыми ключами
    #[test]
    fn hypothesis_destructor_detector_object() {
        let c = SignalContract {
            key: "diagnostic_report".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "idol": { "type": "string" },
                    "threat": { "type": "string" },
                    "destructor": { "type": "string" },
                    "pleasure_ban": { "type": "string" },
                    "paradoxical_loop": { "type": "string" }
                },
                "required": ["idol", "threat", "destructor", "pleasure_ban", "paradoxical_loop"]
            }),
        };
        let grammar = build_signal_envelope_grammar(&c);

        // Должны быть все ключи
        for key in &["idol", "threat", "destructor", "pleasure_ban", "paradoxical_loop"] {
            assert!(grammar.contains(key), "грамматика должна содержать {}", key);
        }
        // Не должно быть пустых правил
        assert!(!grammar.contains("bool_members ::="), "не должно быть пустого bool_members");
    }

}