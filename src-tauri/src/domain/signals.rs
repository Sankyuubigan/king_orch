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
/// think-block ::= "<think>" [^<]* "</think>" | ""
/// ```
///
/// Размышление здесь **не ограничено** — историческое поведение всех
/// сигнальных агентов. Агент, которому бюджет размышления задан во
/// frontmatter (`think_budget:`), собирает грамматику через
/// [`build_signal_envelope_grammar_with_think_budget`].
pub fn build_signal_envelope_grammar(contract: &SignalContract) -> String {
    build_signal_envelope_grammar_with_think_budget(contract, None)
}

/// Гибридная грамматика конверта с **границей размышления**.
///
/// # Зачем граница (измерено, `tasks/09.10.26 …md` §7)
///
/// `think-block ::= "<think>" [^<]* "</think>"` не ограничивает длину
/// размышления, поэтому модель, ушедшая в длинный разбор, выжигает весь
/// `n_predict` в think и **JSON-фаза становится недостижимой**: конверта нет,
/// сигнала нет, воронка останавливается. На 63 кейсах это воспроизвелось
/// на `somatic_throat_fever` (слой 1, 2048 токенов размышления, 0 символов
/// ответа) — механический отказ, а не качество модели.
///
/// Граница зашивается в ФОРМАТ, а не в `max_tokens` и не в ретрай
/// (AGENTS.md §00.2): после неё грамматика принудительно закрывает `</think>`,
/// поэтому JSON-фаза структурно достижима всегда. Число символов — то же, что
/// у измеренного контракта вердиктов
/// ([`verdict_contract::DEFAULT_THINK_BUDGET_CHARS`], свип 500 / 1200 / без
/// потолка: 73.9% / **74.8%** / 73.9% при хвосте 121 с против 11.9 с).
///
/// Класс символов `[^\n<]` — как в `verdict_contract`: перевод строки внутри
/// think невозможен, поэтому граница наступает раньше, чем модель его написала
/// бы сама.
/// Гибридная грамматика конверта сигнала: **свой черновик на каждый элемент**.
///
/// # Что изменилось и почему
///
/// Раньше на слой был ОДИН черновик на все его элементы (`think-block`,
/// слои `validator_surface` / `validator_facade` — по четыре элемента на один
/// блок). Модель писала один проход по четырём элементам сразу, и на замере
/// 11.10.26 элементы 6 («Триггер») и 8 («Иллюзии») не двигались ни при какой
/// правке формулировок: 19 и 13 промахов соответственно. Гипотеза в том, что
/// делить один черновик — значит отдавать четверть внимания каждому элементу.
///
/// Теперь число черновиков равно числу обязательных полей контракта, то есть
/// числу элементов в слое. Оно НЕ задаётся руками: считается из контракта,
/// поэтому слой и его контракт не могут разойтись (SSOT).
///
/// # Про потолок
///
/// `think_budget_chars = None` — черновик не ограничен грамматикой, бюджет
/// задаёт движок (`max_gen_tokens` из конфига пользователя). Это осознанный
/// выбор замера: 1400 символов в заголовках агентов брались из свипа на
/// gemma-4-12B для ОДНОГО черновика (`tasks/02.10.26 …md`, строка 237) и к
/// этой конфигурации отношения не имели.
pub fn build_signal_envelope_grammar_with_think_budget(
    contract: &SignalContract,
    think_budget_chars: Option<usize>,
) -> String {
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

    // Свой черновик на каждый обязательный элемент контракта.
    let drafts = required_field_count(schema);
    let draft_rule = match think_budget_chars {
        None => "draft ::= \"<think>\" [^<]* \"</think>\"".to_string(),
        Some(budget) => format!("draft ::= \"<think>\" [^\\n<]{{0,{budget}}} \"</think>\""),
    };
    let drafts_rule = format!("drafts-block ::= draft{{{}}}", drafts);
    // Экранированный ключ `"value"` внутри грамматики. Вынесен отдельно
    // намеренно: в строке формата это `\"\\\"value\\\"\"`, и одна лишняя
    // `\"` здесь молча ломает разбор грамматики движком (`failed to parse
    // grammar`), не ломая при этом тесты Rust.
    let quoted_value_key = "\"\\\"value\\\"\"";

    format!(
        "root ::= drafts-block envelope-json\n\
         {drafts_rule}\n\
         {draft_rule}\n\
         \n\
         envelope-json ::= \"{{\" sp tool-field \",\" sp arguments-field sp \"}}\"\n\
         tool-field ::= \"\\\"tool\\\"\" sp \":\" sp \"\\\"emit_signal\\\"\"\n\
         arguments-field ::= \"\\\"arguments\\\"\" sp \":\" sp arguments-json\n\
         arguments-json ::= \"{{\" sp key-field \",\" sp {quoted_value_key} sp \":\" sp value-field sp \"}}\"\n\
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

/// Сколько черновиков требует контракт: столько же, сколько у него
/// обязательных полей. Ноль полов у контракта означал бы, что модель не может
/// перейти к конверту вовсе, поэтому такое значение — ошибка контракта.
pub fn required_field_count(schema: &serde_json::Value) -> usize {
    schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0)
}

/// Токены, оставаемые конверту `emit_signal`.
///
/// Конверт жёстко задан грамматикой: имя ключа, четыре (или один) булевых поля,
/// скобки — около 150 символов, то есть порядка 50 токенов. Здесь взят запас
/// примерно пятикратный: резервировать ровно минимум нельзя, любая ошибка в
/// оценке съедает конверт и возвращает отказ «слой не ответил».
const ENVELOPE_TOKENS: usize = 256;

/// Символов на токен для русского текста, **взято с запасом вниз**.
///
/// Грамматика режет черновик в символах, а движок — в токенах, поэтому перевод
/// нужен в любую сторону. Замер 11.10.26 на этом же стенде при
/// `max_gen_tokens = 2048`: ответы, дошедшие до конверта, не длиннее 5443
/// символов, ответы без конверта — от 6851. Значит бюджет в 2048 токенов
/// вмещает от 2.7 до 3.3 символа на токен. Берётся 2.0: при меньшем
/// коэффициенте граница черновика наступает раньше, чем кончится бюджет.
/// Цена — черновики короче измеренного среднего; выигрыш — «не дошёл до
/// конверта» становится структурно невозможен.
const CHARS_PER_TOKEN: usize = 2;

/// Граница ОДНОГО черновика: бюджет генерации делится между черновиками слоя,
/// минус резерв под конверт сигнала.
///
/// Возвращает `None`, если граница выходит нулевой: такой бюджет не способен
/// вместить даже пустой черновик, и молча ставить `Some(0)` значило бы запретить
/// модели размышлять вовсе. Тогда грамматика строится без границы, и отказ
/// приходит из движка, а не из заведомо непроходимого формата.
pub fn per_draft_budget_chars(max_gen_tokens: usize, drafts: usize) -> Option<usize> {
    let for_drafts = max_gen_tokens.saturating_sub(ENVELOPE_TOKENS.min(max_gen_tokens));
    let per_draft_tokens = for_drafts / drafts.max(1);
    let chars = per_draft_tokens.saturating_mul(CHARS_PER_TOKEN);
    if chars == 0 {
        None
    } else {
        Some(chars)
    }
}

/// Phase 2: JSON-only grammar (без think-block).
/// Модель генерирует ТОЛЬКО JSON envelope — никакого think.
/// Основная грамматика Phase 2 двухфазного режима: размышления уже собраны в Phase 1
/// (без грамматики), здесь нужен только строгий конверт сигнала.
pub fn build_signal_envelope_json_only_grammar(contract: &SignalContract) -> String {
    let schema = &contract.value_schema;

    let sp = "sp ::= \" \" | \"\\n\" [ \\t]{0,4}";

    let json_rules = concat!(
        "json-string ::= \"\\\"\" string-char* \"\\\"\"\n",
        "string-char ::= [^\"\\\\] | escape\n",
        "escape ::= \"\\\\\" (\"\\\\\" | \"\\\"\" | \"n\" | \"t\" | \"r\" | \"b\" | \"f\" | \"u\" [0-9a-f] [0-9a-f] [0-9a-f] [0-9a-f])\n",
        "json-number ::= \"-\"? (\"0\" | [1-9] [0-9]*) (\".\" [0-9]+)? ([eE] [-+]? [0-9]+)?\n",
        "json-bool ::= \"true\" | \"false\"\n",
        "bool ::= \"true\" | \"false\"\n",
    );

    let value_grammar = build_value_grammar(schema);

    format!(
        "root ::= envelope-json\n\
         \n\
         envelope-json ::= \"{{\" sp tool-field \",\" sp arguments-field sp \"}}\"\n\
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
/// обязательные поля контракта (напр. `e1` у `validator_report`). Если модель
/// исказила форму — возвращаем явную ошибку, которую оркестратор отдаёт модели на retry.
/// Это исключает тихий обрыв маршрутизации, когда `signal_router` не находит поле и граф
/// завершается, не дойдя до `message`-узла (см. docs/SIGNAL_CONTRACTS.md). Не является
/// костылём: это обычная валидация входа, а не «умолчание по дефолту».
pub fn validate_signal_value(contract: &SignalContract, value: &Value) -> Result<(), String> {
    let schema = &contract.value_schema;
    if schema.get("type").and_then(|t| t.as_str()) == Some("object") {
        // Проверяем ВСЕ properties контракта (а не только required).
        // Это гарантирует, что signal_router найдёт каждое поле (eX и т.д.).
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
        for id in ["validator", "focus_keeper", "soma_translator", "synthesizer"] {
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
        assert_eq!(c.value_schema["properties"]["e1"]["type"].as_str(), Some("boolean"));
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

    /// Граница размышления обязана быть В ГРАММАТИКЕ: без неё модель выжигает
    /// весь `n_predict` в think и не возвращает конверт вообще (воспроизведено
    /// на `somatic_throat_fever`, `tasks/09.10.26 …md` §7).
    #[test]
    fn think_budget_is_written_into_grammar() {
        let contract = SignalContract {
            key: "validator_surface".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "e1": { "type": "boolean" }, "e2": { "type": "boolean" } },
                "required": ["e1", "e2"]
            }),
        };
        let bounded = build_signal_envelope_grammar_with_think_budget(&contract, Some(1400));
        let bounded_draft = bounded
            .lines()
            .find(|line| line.starts_with("draft ::="))
            .unwrap_or_default();
        assert!(
            bounded_draft.contains("{0,1400}"),
            "в грамматике нет границы черновика: {}",
            bounded_draft
        );
        // Агент без объявленного потолка: черновик не ограничен грамматикой,
        // бюджет задаёт движок (`max_gen_tokens` из конфига пользователя).
        let unbounded = build_signal_envelope_grammar(&contract);
        let unbounded_draft = unbounded
            .lines()
            .find(|line| line.starts_with("draft ::="))
            .unwrap_or_default();
        assert!(
            !unbounded_draft.contains("{0,"),
            "грамматика без бюджета не должна получать границу: {}",
            unbounded_draft
        );
        assert!(unbounded_draft.contains("[^<]*"), "исторический черновик потерян");
        // Число черновиков не зависит от потолка: его задаёт контракт.
        assert!(
            bounded.contains("drafts-block ::= draft{2}") && unbounded.contains("drafts-block ::= draft{2}"),
            "потолок не должен менять число черновиков"
        );
        // Остальная форма конверта одинакова — граница меняет только черновик.
        assert_eq!(
            bounded.lines().count(),
            unbounded.lines().count(),
            "структура грамматики разъехалась"
        );
    }

    /// Экранирование ключа `"value"` внутри грамматики обязано остаться GBNF-строкой
    /// `"\"value\""`. Проверено на движке 11.10.26: при «голом» `\"value\"` парсер
    /// llama.cpp отвечает `failed to parse grammar`, а все тесты Rust при этом
    /// остаются зелёными — грамматику проверяет только сервер. Сырой литерал
    /// здесь обязателен: повторный набор экранирования в обычной строке и породил
    /// тот самый отказ.
    #[test]
    fn value_key_escaping_survives() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .join("agents")
            .join("psychotherapist")
            .join("signals");
        for id in ["validator_surface", "validator_facade", "validator_core"] {
            let contract = load_signal_contract(&dir, id).expect("контракт прочитан");
            let grammar = build_signal_envelope_grammar(&contract);
            assert!(
                grammar.contains(
                    r#"arguments-json ::= "{" sp key-field "," sp "\"value\"" sp ":" sp value-field sp "}""#
                ),
                "экранирование ключа value сломано в грамматике слоя {}, grammar:\n{}",
                id,
                grammar
            );
        }
    }

    /// Слои воронки НЕ объявляют потолок черновика — по решению владельца
    /// бюджет берётся из конфига пользователя (`max_gen_tokens`), и модель
    /// должна уложиться в него сама. Прежняя цифра 1400 пришла из свипа на
    /// gemma-4-12B для ОДНОГО черновика на слой (`tasks/02.10.26 …md`, строка
    /// 237) и к этой конфигурации отношения не имела.
    ///
    /// Явный тест, а не «молча убрали»: если кто-то вернёт цифру в заголовок,
    /// грамматика снова начнёт резать черновик не по бюджету пользователя.
    #[test]
    fn funnel_layer_agents_declare_no_think_budget() {
        let agents_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .join("agents");
        let agents = crate::domain::agent_manager::load_agents(&agents_dir).expect("агенты прочитаны");
        // Плоская форма — та же граница из конфига, что и у слоёв: иначе плоский
        // вариант мерился бы на другом бюджете и сравнение было бы бессмысленным.
        for id in ["validator_surface", "validator_facade", "validator_core", "validator_flat"] {
            let agent = agents
                .iter()
                .find(|agent| agent.id == id)
                .unwrap_or_else(|| panic!("агент слоя {} не найден", id));
            assert_eq!(
                agent.think_budget_chars, None,
                "у слоя {} в заголовке появился think_budget: {:?} — бюджет должен идти \
                 из конфига пользователя, иначе грамматика режет черновик не по нему",
                id,
                agent.think_budget_chars
            );
        }
        // Плоский валидатор боя не трогаем этой правкой.
        let plain = agents.iter().find(|agent| agent.id == "validator");
        if let Some(plain) = plain {
            assert_eq!(
                plain.think_budget_chars, None,
                "think_budget у плоского валидатора не менялся этой правкой"
            );
        }
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
        // Число контрактов растёт вместе с агентами: проверяем не количество, а
        // то, что каждый объявленный агент читается. Счётчик «5 сигналов» устарел
        // с появлением трёх слоёв воронки и молчал бы о битом контракте.
        for agent in [
            "validator",
            "validator_flat",
            "validator_surface",
            "validator_facade",
            "validator_core",
            "focus_keeper",
            "soma_translator",
            "synthesizer",
            "destructor_detector",
        ] {
            assert!(
                map.contains_key(agent),
                "контракт агента '{}' не читается из {}",
                agent,
                path.display()
            );
        }
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
                    "e1": { "type": "boolean" }
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
                    "e1": { "type": "boolean" },
                    "e2": { "type": "boolean" }
                },
                "required": ["e1", "e2"]
            }),
        };
        // Искажённая форма (старое поле verdict вместо eX) — ошибка.
        assert!(validate_signal_value(&c, &serde_json::json!({ "verdict": "ДАННЫХ ДОСТАТОЧНО" })).is_err());
        // Пустое значение без обязательных полей — тоже ошибка.
        assert!(validate_signal_value(&c, &serde_json::Value::Object(serde_json::Map::new())).is_err());
        // Корректная форма проходит.
        assert!(validate_signal_value(&c, &serde_json::json!({ "e1": true, "e2": false })).is_ok());
    }

    #[test]
    fn envelope_schema_is_inline_and_requires_all_fields() {
        let c = SignalContract {
            key: "validator_report".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "e1": { "type": "boolean" },
                    "e2": { "type": "boolean" }
                },
                "required": ["e1", "e2"]
            }),
        };
        let schema = build_signal_envelope_schema(&c);
        // Конверт больше не несёт поле thought (размышления — в Phase 1);
        // value защищён обязательными полями контракта.
        assert_eq!(schema["properties"]["tool"]["const"], serde_json::json!("emit_signal"));
        assert!(schema["properties"].get("thought").is_none(), "поле thought убрано из конверта");
        assert_eq!(
            schema["properties"]["arguments"]["properties"]["value"]["required"],
            serde_json::json!(["e1", "e2"])
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
                    "e1": { "type": "boolean" },
                    "e2": { "type": "boolean" },
                    "missing_data": { "type": "string" }
                },
                "required": ["e1", "e2"]
            }),
        };
        let grammar = build_signal_envelope_grammar(&c);
        // Грамматика ОБЯЗАНА содержать черновик (Method 3 из docs/gbnf.md), и черновик
// теперь СВОЙ НА КАЖДЫЙ обязательный элемент: контракт с двумя полями требует
// двух слотов, а не одного на весь слой.
        assert!(grammar.contains("<think>"), "грамматика должна содержать <think>");
        assert!(grammar.contains("</think>"), "грамматика должна содержать </think>");
        assert!(grammar.contains("drafts-block"), "грамматика должна содержать правило drafts-block");
        assert!(
            grammar.contains("drafts-block ::= draft{2}"),
            "два обязательных поля контракта — значит два черновика: {}",
            grammar
        );
        // Конверт больше не несёт поле thought (размышления — Phase 1).
        assert!(!grammar.contains("thought-field"), "поле thought убрано из конверта");
        // Грамматика ОБЯЗАНА содержать ключи сигнала
        assert!(grammar.contains("e1"), "грамматика должна содержать e1");
        assert!(grammar.contains("e2"), "грамматика должна содержать e2");
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
            let key = format!("e{}", i);
            assert!(grammar.contains(&key), "грамматика должна содержать {}", key);
        }
        // think-блок (Method 3) обязателен
        assert!(grammar.contains("<think>") && grammar.contains("</think>"), "должен быть think-блок");
    }

    /// Регрессия: грамматика для boolean-полей (eX) ОБЯЗАНА
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
        // Гибридный черновик (Method 3 из docs/gbnf.md): <think> ... </think>
        assert!(grammar.contains("<think>"), "должен быть <think>, grammar:\n{}", grammar);
        assert!(grammar.contains("</think>"), "должен быть </think>, grammar:\n{}", grammar);
        assert!(grammar.contains("drafts-block"), "должно быть правило drafts-block, grammar:\n{}", grammar);
        // Не должно быть токенного синтаксиса (не работает с gemma-4)
        assert!(!grammar.contains("<thingk>"), "не должно быть токена <thingk>");
        assert!(!grammar.contains("<end_of_think>"), "не должно быть токена <end_of_think>");
    }

    /// think-блок (Method 3) как в docs/gbnf.md — проверяем его присутствие
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
        // Должен быть think-блок как в docs/gbnf.md (Method 3)
        assert!(grammar.contains("<think>") && grammar.contains("</think>"), "должен быть think-блок, grammar:\n{}", grammar);
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
                    "e1": { "type": "boolean" }
                },
                "required": ["e1"]
            }),
        };
        let grammar = build_signal_envelope_grammar(&c);

        // Конверт без свободного thought: только tool + arguments (размышления — Phase 1).
        assert!(!grammar.contains("thought-field"), "поле thought убрано из конверта");
        assert!(grammar.contains("emit_signal"), "должно быть emit_signal");
        assert!(grammar.contains("arguments"), "должно быть arguments");
        assert!(grammar.contains("key"), "должно быть поле key");
        assert!(grammar.contains("validator_report"), "должен быть ключ validator_report");
        assert!(grammar.contains("e1"), "должен быть e1");
        assert!(grammar.contains("drafts-block"), "должно быть правило drafts-block");
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