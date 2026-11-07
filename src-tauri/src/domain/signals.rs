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
/// think-block ::= "<think>" think-char* "</think>" | ""
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
/// `think-block ::= "<think>" think-char* "</think>"` не ограничивает длину
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
/// Тело слота — правило `think-char` (SSOT в плагине, `THINK_BODY_RULE`),
/// С переносами строк (решение владельца 31.10.2026): черновики по элементам
/// модель пишет списками («Элемент 3: … → false»), и запрет `\n` вынуждал её
/// в одну длинную строку, ухудшая читаемость отчёта. С 02.11.2026 тело
/// разрешает и «<» (стрелки, сравнения): запрещена только закрывающая
/// последовательность `</think` — раньше попытка модели написать «<»
/// маскировала все продолжения, и единственным валидным «<»-ходом был
/// преждевременный `</think>` (обрыв мысли посреди слова, кейс валидатора
/// 02.11.26: 7 из 9 слотов оборвались на строке «- Сверка…»).
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
/// Базовые JSON-правила сигнальных грамматик — общие для конвертного пути и
/// Phase-2 JSON-only. Строковые правила (json-string/string-char/escape) —
/// канонический блок ПЛАГИНА ([`crate::infra::strict_json_string_rules`]):
/// raw «<» в строковых значениях запрещён — think-модель может открыть
/// `<think>` внутри значения, сервер держит дельты до его закрытия
/// (llama.cpp#15000) — «0 токенов, MAX_TOKENS» (лог юзера 01.11.26);
/// легитимный «<» — как escape `\u003c`. Копий строковых правил в хосте нет
/// (SSOT в плагине); число-правила и `bool` (алиас json-bool, на который
/// ссылаются value-грамматики) — локальный поднабор, единственный в файле.
fn json_value_rules() -> String {
    format!(
        "{}{}",
        crate::infra::strict_json_string_rules(),
        concat!(
            "json-number ::= \"-\"? (\"0\" | [1-9] [0-9]*) (\".\" [0-9]+)? ([eE] [-+]? [0-9]+)?\n",
            "json-bool ::= \"true\" | \"false\"\n",
            "bool ::= \"true\" | \"false\"\n",
        )
    )
}

pub fn build_signal_envelope_grammar_with_think_budget(
    contract: &SignalContract,
    think_budget_chars: Option<usize>,
) -> String {
    let schema = &contract.value_schema;

    // Правило пробелов (ограниченное, как в официальном конвертере llama.cpp)
    let sp = "sp ::= \" \" | \"\\n\" [ \\t]{0,4}";

    // Базовые JSON-правила (строковые — канонический блок плагина; их же
    // добирает Вариант B для финальных слотов без enum)
    let json_rules = json_value_rules();

    // Генерируем value-грамматику в зависимости от типа контракта
    let value_grammar = build_value_grammar(schema);

    // Свой черновик на каждый обязательный элемент контракта; минимум ОДИН —
    // слот отчёта обязан существовать у любого контракта (инвариант
    // draft_count_for_contract): enum/строковые/pattern-схемы не имеют
    // `required`, и draft{0} структурно убивал отчёт агента — модель шла сразу
    // в конверт (кейс soma_translator, tasks/27.10.26 draft0).
    let drafts = draft_count_for_contract(schema);
    let draft_rule = match think_budget_chars {
        None => "draft ::= \"<think>\" think-char* \"</think>\"".to_string(),
        Some(budget) => format!("draft ::= \"<think>\" think-char{{0,{budget}}} \"</think>\""),
    };
    // Тело слота теряет нулевую альтернативу (решение владельца 01.11.2026,
    // Вариант A; MIN и гейты зафиксированы ДО прогона в план-файле): в батче
    // 04-23-37 дум-слот был пустым у ВСЕХ моделей (слот = «метка + мгновенный
    // конверт»), вердикт угадывался из вступления. Минимум содержания — форма
    // выхода, а не подсказка: «пустой слот» становится невозможным по
    // грамматике. Think-литералы из исходного правила ниже НЕ пересобираются
    // руками (правило чаннель-глитча, handoff §1): правится только класс с
    // квантификатором по ASCII-якорю.
    let draft_rule = with_slot_min_body(&draft_rule, slot_min_chars(think_budget_chars));

    // ── Вариант B (слотовые вердикты): e-контракт + финальные строковые слоты ──
    // Каждый think-слот ЗАКАНЧИВАЕТСЯ собственным вердиктом: `"Element k: "
    // <think тело </think> "e<k>": true|false`. Вердикт генерируется СРАЗУ
    // после мысли о своём элементе и обусловлен ею (генерация слева направо);
    // конверт с булями не генерируется вовсе — канал «заполнить слоты
    // декоративно и угадать всё в конце» закрыт грамматически, а значение
    // сигнала собирается из слотов единственным парсером
    // ([`assemble_slot_signal_value`], SSOT: один источник вердикта).
    // После think-слотов допустимы ФИНАЛЬНЫЕ строковые слоты (напр.
    // `floor0_type`/`floor1_type` у валидатора) БЕЗ дум-тела: мышление о типе
    // уже сделано в think-слотах элементов, think-бюджет на них не делится
    // (см. [`think_slot_count`]). Неслотовая схема (см. [`slot_verdict_plan`])
    // тихо падает в обычный конвертный путь ниже.
    if let Some(plan) = slot_verdict_plan(schema) {
        let body = &draft_rule["draft ::= ".len()..];
        let mut rule = String::from("root ::= drafts-block\ndrafts-block ::=");
        for k in 1..=plan.e_slots.len() {
            if k > 1 {
                rule.push_str(" sp");
            }
            rule.push_str(&format!(" draft-{}", k));
        }
        for t in 0..plan.trailing.len() {
            rule.push_str(&format!(" sp trail-{}", t));
        }
        let mut needs_string_rules = false;
        for ((label, key, verdict_cap), idx) in plan.e_slots.iter().zip(1..=plan.e_slots.len()) {
            // Литерал вердикта собран так, чтобы кавычки внутри него остались
            // экранированными В ГРАММАТИКЕ (форма `\"e1\": `), — как и все
            // кавычки-литералы этого файла.
            let value_rule = if verdict_cap.is_some() {
                needs_string_rules = true;
                format!("verdict-value-{}", idx)
            } else {
                "bool".to_string()
            };
            rule.push_str(&format!(
                "\ndraft-{} ::= \"{}\" {} \"\\\"{}\\\": \" {}",
                idx, label, body, key, value_rule
            ));
        }
        for (idx, (label, key)) in plan.trailing.iter().enumerate() {
            // Значение финального слота: перечисление из enum схемы (SSOT —
            // литералы не переписываются руками), без enum — свободная
            // json-string (тогда нужны её правила).
            let enum_vals = schema
                .get("properties")
                .and_then(|p| p.get(key.as_str()))
                .and_then(|ps| ps.get("enum"))
                .and_then(Value::as_array);
            let value_rule = if enum_vals.is_some() {
                format!("trail-{}-value", idx)
            } else {
                needs_string_rules = true;
                "json-string".to_string()
            };
            rule.push_str(&format!(
                "\ntrail-{} ::= \"{}\" \"\\\"{}\\\": \" {}",
                idx, label, key, value_rule
            ));
            if let Some(vals) = enum_vals {
                let alternatives: Vec<String> = vals
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|v| format!("\"\\\"{}\\\"\"", v))
                    .collect();
                rule.push_str(&format!(
                    "\ntrail-{}-value ::= {}",
                    idx,
                    alternatives.join(" | ")
                ));
            }
        }
        rule.push_str(&format!(
            "\n{}\n{}\n{}",
            sp,
            crate::infra::THINK_BODY_RULE,
            "bool ::= \"true\" | \"false\""
        ));
        // Вердикт-тексты (05.11.2026): строка ограничена maxLength СХЕМЫ
        // (бюджет конверта зашит в формат, §00.2), класс символов — канонический
        // `string-char` плагина (raw «<» запрещён — легитимный «<» выпускается
        // как `\u003c` и декодируется сборке); альтернатива `false` — при
        // непрошедшем критерии элемента.
        for ((_, key, verdict_cap), idx) in plan.e_slots.iter().zip(1..=plan.e_slots.len()) {
            if let Some(cap) = verdict_cap {
                rule.push_str(&format!(
                    "\nverdict-value-{idx} ::= verdict-string-{idx} | \"false\"\n\
                     verdict-string-{idx} ::= \"\\\"\" string-char{{1,{cap}}} \"\\\"\"",
                ));
            }
        }
        if needs_string_rules {
            rule.push('\n');
            rule.push_str(&crate::infra::strict_json_string_rules());
        }
        return rule;
    }

    let drafts_rule = if let Some(labels) = slot_anchor_labels(schema, drafts) {
        // Якорный режим (решение владельца 01.11.26 — устранение рестарт-петли):
        // слот k обязан НАЧАТЬСЯ с литерала метки своего элемента — его даёт
        // k-е required-поле контракта e-вида. Тело слота берётся из того же
        // правила, что собрано выше для свободного слота (фиксированный
        // префикс "draft ::= " вырезается). Прежний отказ, когда модель
        // переписывала на каждом слоте один и тот же ввод и упиралась в
        // потолок посреди слова (батч 02.11.01: 7/13 моделей), становится
        // структурно невозможным: метка пришита к слоту ДО контента, вердикт
        // не может родиться из одного вступления.
        let body = &draft_rule["draft ::= ".len()..];
        let mut rule = String::from("drafts-block ::=");
        for k in 1..=drafts {
            rule.push_str(&format!(" draft-{}", k));
        }
        for (idx, label) in labels.iter().enumerate() {
            rule.push_str(&format!("\ndraft-{} ::= \"{}\" {}", idx + 1, label, body));
        }
        rule
    } else {
        format!("drafts-block ::= draft{{{}}}", drafts)
    };
    // Экранированный ключ `"value"` внутри грамматики. Вынесен отдельно
    // намеренно: в строке формата это `\"\\\"value\\\"\"`, и одна лишняя
    // `\"` здесь молча ломает разбор грамматики движком (`failed to parse
    // grammar`), не ломая при этом тесты Rust.
    let quoted_value_key = "\"\\\"value\\\"\"";
    let think_body_rule = crate::infra::THINK_BODY_RULE;

    format!(
        "root ::= drafts-block envelope-json\n\
         {drafts_rule}\n\
         {draft_rule}\n\
         \n\
         {think_body_rule}\n\
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

/// Сколько обязательных полей у схемы контракта (длина массива `required`).
/// Число черновиков для грамматики считает `draft_count_for_contract` —
/// оно не может быть нулевым независимо от формы контракта.
pub fn required_field_count(schema: &serde_json::Value) -> usize {
    schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0)
}

/// Число черновиков (слотов отчёта) для контракта.
///
/// Инвариант ≥ 1: слот отчёта обязан существовать у ЛЮБОГО контракта.
/// Enum/строковые/pattern-схемы (`focus_keeper`, `destructor_detector`)
/// не имеют массива `required` — раньше это давало
/// `draft{0}`: грамматика не оставляла выхода для анализа, модель шла сразу
/// в конверт, отчёт агента был пуст (регрессия 93e30c7, 13.10.26).
/// Object-контракты с N обязательными полями сохраняют N черновиков —
/// это существующее поведение валидатора и его слоёв.
pub fn draft_count_for_contract(schema: &serde_json::Value) -> usize {
    required_field_count(schema).max(1)
}

/// Число ДУМ-слотов (think-блоков) в грамматике контракта.
///
/// Для слотовых контрактов (Вариант B) дум-тела есть только у e-пар; финальные
/// строковые слоты (напр. `floor0_type`/`floor1_type` у валидатора) дум-тела
/// не имеют, и делить think-бюджет на них нельзя — иначе потолок занижается
/// всем элементам без причины. Для неслотовых контрактов — [`draft_count_for_contract`].
pub fn think_slot_count(schema: &serde_json::Value) -> usize {
    slot_verdict_plan(schema)
        .map(|p| p.e_slots.len().max(1))
        .unwrap_or_else(|| draft_count_for_contract(schema))
}

/// Метки якорей think-слотов отчёта (решение владельца 01.11.2026).
///
/// Возвращает `Some(метки)` только для слотового контракта
/// ([`slot_verdict_plan`]): think-слот k обязан начаться с `Element <N>: `,
/// где N — номер из имени e-поля (слои воронки наследуют те же якоря: у
/// `validator_surface` required = e1,e2,e6,e7 → слоты Element 1/2/6/7).
/// Финальные строковые слоты (без дум-тела) в списке якорей НЕ участвуют —
/// у валидатора (9 e-слотов + `floor0_type`/`floor1_type`/`target`) якорей
/// 9 при 12 слотах. Поля другого вида
/// (`idol`, enum-строки, pattern) — прежний свободный черновик (`None`):
/// им текстуальный якорь не соответствует по смыслу.
///
/// `drafts` — общее число слотов (дум + финальных); якоря выдаются, только
/// если оно совпадает с планом схемы.
pub fn slot_anchor_labels(schema: &serde_json::Value, drafts: usize) -> Option<Vec<String>> {
    let plan = slot_verdict_plan(schema)?;
    if plan.e_slots.len() + plan.trailing.len() != drafts {
        return None;
    }
    Some(plan.e_slots.into_iter().map(|(l, _, _)| l).collect())
}

/// Форма e-поля схемы: булев вердикт или вердикт-текст — union
/// `anyOf: [string{1..maxLength}, const false]` (05.11.2026: слот пишет,
/// ЧТО именно определено, 1-2 предложения; `false` остаётся при непрошедшем
/// критерии). Без `maxLength` у строковой ветки форма не ограничена схемой —
/// план не строится (честное падение в конвертный путь), а не безлимит.
#[derive(Debug, PartialEq)]
enum EFieldKind {
    Bool,
    Union { max_chars: usize },
}

fn e_field_kind(prop: &Value) -> Option<EFieldKind> {
    if prop.get("type").and_then(Value::as_str) == Some("boolean") {
        return Some(EFieldKind::Bool);
    }
    let variants = prop.get("anyOf")?.as_array()?;
    let mut has_string = false;
    let mut has_false = false;
    let mut max_chars = None;
    for v in variants {
        if v.get("const").and_then(Value::as_bool) == Some(false) {
            has_false = true;
        } else if v.get("type").and_then(Value::as_str) == Some("string") {
            has_string = true;
            max_chars = v.get("maxLength").and_then(Value::as_u64);
        }
    }
    match (has_string, has_false, max_chars) {
        (true, true, Some(n)) if n > 0 => Some(EFieldKind::Union {
            max_chars: n as usize,
        }),
        _ => None,
    }
}

/// План слотовых вердиктов (Вариант B): think-слоты с вердиктами
/// плюс опциональные финальные строковые слоты без дум-тела.
struct SlotPlan {
    /// (метка якоря `Element N: `, ключ вердикта `eN`, форма вердикта:
    /// `None` — булев, `Some(cap)` — вердикт-текст до `cap` символов) —
    /// think-слоты.
    e_slots: Vec<(String, String, Option<usize>)>,
    /// (человеческая метка `Floor0 type: `, ключ `floor0_type`) — финальные
    /// строковые слоты.
    trailing: Vec<(String, String)>,
}

/// План слотов для схемы контракта — общая основа якорей слотов, Варианта B
/// (вердикт внутри слота) и сборки значения. План существует только если:
/// - `required` непуст, а поля контракта есть в `properties`;
/// - e-поля (`e<N>`; булевы или вердикт-тексты — [`EFieldKind`]) идут ПЕРВЫМИ,
///   без перемешивания со строковыми;
/// - после них — финальные поля типа string (`floor0_type`/`floor1_type`/
///   `target` у валидатора);
/// - хотя бы одно e-поле есть (иначе контракт не слотовый).
///
/// Ключ вердикта think-слота k — само имя k-го e-поля (SSOT: и якорь, и
/// вердикт происходят из одного списка `required`, дублей нет).
fn slot_verdict_plan(schema: &serde_json::Value) -> Option<SlotPlan> {
    let required = schema.get("required")?.as_array()?;
    if required.is_empty() {
        return None;
    }
    let props = schema.get("properties")?;
    let mut e_slots: Vec<(String, String, Option<usize>)> = Vec::new();
    let mut trailing: Vec<(String, String)> = Vec::new();
    for field in required {
        let name = field.as_str()?;
        let prop = props.get(name)?;
        let kind = e_field_kind(prop);
        if trailing.is_empty() && name.starts_with('e') {
            // e-слот: булев или вердикт-текст; прочая форма — не слотовый план.
            let verdict_cap = match kind {
                Some(EFieldKind::Bool) => None,
                Some(EFieldKind::Union { max_chars }) => Some(max_chars),
                None => return None,
            };
            // e-слот: имя = ровно "e" + цифры (дублируют якорь Element N).
            let digits: String = name.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() || digits.len() != name.chars().count().saturating_sub(1) {
                return None;
            }
            e_slots.push((format!("Element {}: ", digits), name.to_string(), verdict_cap));
        } else {
            // Финальный слот: только string-поля ПОСЛЕ всех e-полей. Не-e
            // boolean (или e-поле после строкового) — не слотовый план.
            if prop.get("type").and_then(Value::as_str) != Some("string") {
                return None;
            }
            trailing.push((slot_label_for_field(name), name.to_string()));
        }
    }
    if e_slots.is_empty() {
        return None;
    }
    Some(SlotPlan { e_slots, trailing })
}

/// Человеческая метка финального слота из имени поля: `floor0_type` →
/// `Floor0 type: ` (первая буква всей метки заглавная, подчёркивания —
/// пробелами).
fn slot_label_for_field(name: &str) -> String {
    let mut label = name
        .split('_')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    // Заглавная — только первая буква всей метки (`Floor0 type`, а не
    // `Floor0 Type`): грамматика, якорь парсера и промпт держат один литерал.
    // Инплейс по ASCII (имена полей — snake_case); не-ASCII и пустая строка
    // get_mut(0..1) вернёт None — метка останется как есть, без паники.
    if let Some(first) = label.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    label.push_str(": ");
    label
}

/// Является ли контракт контрактом **слотовых вердиктов** (Вариант B):
/// e-поля (булевы или вердикт-тексты; вердикты ВНУТРИ think-слотов, конверт
/// булей не существует) плюс допустимые финальные строковые слоты (напр.
/// `floor0_type`). Форму задаёт [`slot_verdict_plan`] — единственный источник
/// этой классификации.
pub fn is_slot_verdict_contract(schema: &serde_json::Value) -> bool {
    slot_verdict_plan(schema).is_some()
}

/// Облачный промпт-блок сигнала — SSOT root.schema.json.
///
/// Облачные модели не поддерживают локальные грамматики, поэтому форма сигнала
/// задаётся промпт-блоком, собранным ИЗ КОНТРАКТА (`root.schema.json`), а не из
/// `.md` агента: обычный контракт — конверт `emit_signal` с ключом и value-схемой
/// (enum-строки, поля объекта); слотовый контракт (Вариант B) — слотовые
/// вердикты, которые собирает [`assemble_slot_signal_value`]. Механику вызова
/// `.md` агента не дублирует (правило развода — `docs/AGENT_CREATION_GUIDE.md`).
pub fn build_cloud_signal_prompt_block(contract: &SignalContract) -> String {
    let schema = &contract.value_schema;

    if let Some(plan) = slot_verdict_plan(schema) {
        // ── Слотовый контракт (Вариант B): вердикты в слотах, конверта нет ──
        let mut block = String::from(
            "[ТРЕБОВАНИЕ СИГНАЛА (CLOUD MODEL)]\n\
             Ты — сигнальный агент. Облачные модели не поддерживают локальные грамматики, \
             поэтому твой ответ обязан закончиться слотовыми вердиктами в строгом формате — \
             сигнал соберётся из слотов автоматически, инструмент emit_signal вызывать НЕ нужно:\n",
        );
        for (label, key, verdict_cap) in &plan.e_slots {
            let verdict = match verdict_cap {
                Some(cap) => format!(
                    "`\"{key}\": \"вердикт: что именно определено (до {cap} символов)\"` или `\"{key}\": false`"
                ),
                None => format!("`\"{key}\": true` или `\"{key}\": false`"),
            };
            block.push_str(&format!(
                "- строка-якорь `{label}`, затем размышление, затем вердикт {verdict}.\n"
            ));
        }
        for (label, key) in &plan.trailing {
            let enum_note = schema
                .get("properties")
                .and_then(|p| p.get(key.as_str()))
                .and_then(|ps| ps.get("enum"))
                .and_then(Value::as_array)
                .map(|vals| {
                    let list = vals
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|v| format!("\"{v}\""))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(" (одно из: {list})")
                })
                .unwrap_or_default();
            block.push_str(&format!(
                "- финальный слот БЕЗ размышления: `{label}` сразу `\"{key}\": \"…\"{enum_note}.\n"
            ));
        }
        block.push_str(
            "Без слотовых вердиктов сигнал не соберётся, и работа графа будет остановлена с ошибкой.",
        );
        return block;
    }

    // ── Обычный контракт: конверт emit_signal ──
    let mut block = String::from(
        "[ТРЕБОВАНИЕ СИГНАЛА (CLOUD MODEL)]\n\
         Ты — сигнальный агент. Поскольку облачные модели не поддерживают локальные грамматики, \
         в конце своего ответа (после всего разбора) ты ОБЯЗАН вернуть точный JSON-вызов \
         инструмента emit_signal:\n```json\n",
    );
    block.push_str("{\n  \"tool\": \"emit_signal\",\n  \"arguments\": {\n    \"key\": \"");
    block.push_str(&contract.key);
    block.push_str(
        "\",\n    \"value\": <значение по схеме ниже>\n  }\n}\n```\n",
    );
    block.push_str(&describe_value_schema(schema));
    block.push_str("\nБез этого сигнала работа графа будет остановлена с ошибкой.");
    block
}

/// Человекочитаемое описание value-схемы контракта для облачного блока:
/// string-enum — список допустимых строк + `description` из схемы; object —
/// поля с типами/enum/description и отметкой обязательности. SSOT —
/// root.schema.json (смысл значений живёт в схеме, не в .md агента).
fn describe_value_schema(schema: &Value) -> String {
    let desc_note = |ps: &Value| -> String {
        ps.get("description")
            .and_then(Value::as_str)
            .map(|d| format!(" — {d}"))
            .unwrap_or_default()
    };
    match (
        schema.get("type").and_then(Value::as_str),
        schema.get("enum").and_then(Value::as_array),
    ) {
        (Some("string"), Some(vals)) => {
            let list = vals
                .iter()
                .filter_map(Value::as_str)
                .map(|v| format!("\"{v}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Поле `value` — СТРОГО одна из строк: {list}.{}",
                desc_note(schema)
            )
        }
        (Some("object"), _) => {
            let mut s = String::from("Поле `value` — JSON-объект с полями:");
            let required: Vec<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (name, ps) in props {
                    let ptype = match e_field_kind(ps) {
                        Some(EFieldKind::Union { max_chars }) => {
                            format!("вердикт-текст до {max_chars} симв | false")
                        }
                        _ => ps.get("type").and_then(Value::as_str).unwrap_or("any").to_string(),
                    };
                    let enum_note = ps
                        .get("enum")
                        .and_then(Value::as_array)
                        .map(|vals| {
                            let list = vals
                                .iter()
                                .filter_map(Value::as_str)
                                .map(|v| format!("\"{v}\""))
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("; одно из: {list}")
                        })
                        .unwrap_or_default();
                    let req = if required.contains(&name.as_str()) {
                        " [ОБЯЗАТЕЛЬНО]"
                    } else {
                        ""
                    };
                    s.push_str(&format!(
                        "\n  - \"{name}\" ({ptype}{enum_note}){req}{}",
                        desc_note(ps)
                    ));
                }
            }
            s
        }
        (Some("string"), None) => format!("Поле `value` — строка.{}", desc_note(schema)),
        _ => format!(
            "Поле `value` — по JSON-схеме: {}",
            serde_json::to_string(schema).unwrap_or_default()
        ),
    }
}

/// Минимум содержимого ОДНОГО слота отчёта в символах (решение владельца
/// 01.11.2026 — Вариант A): слот «метка + мгновенный конверт» (батч 04-23-37,
/// дум-слои пустые у ВСЕХ моделей, вердикт фактически угадывался из
/// вступления) устраняется ФОРМОЙ, а не подсказкой.
///
/// Формула (зафиксирована ДО прогона в план-файле, не подгоняется):
/// `max(24, потолок/8)`, прижатая к потолку сверху; при потолке < 24 символа
/// минимум физически не входит — он равен потолку и не применяется в
/// [`with_slot_min_body`]. Слот без формального потолка (`None`) получает
/// базовый минимум 24.
pub fn slot_min_chars(think_budget_chars: Option<usize>) -> usize {
    const BASE: usize = 24;
    match think_budget_chars {
        None => BASE,
        Some(budget) => BASE.max(budget / 8).min(budget),
    }
}

/// Вставляет обязательный минимальный прогон содержимого в собранное правило
/// слота: между открывающим литералом и хвостом-потолком.
///
/// Правило приходит собранным (think-литералы уже в нём и здесь их НЕТ в коде —
/// правило чаннель-глитча). Правится ТОЛЬКО первый токен тела `think-char`
/// с его квантификатором, найденный по ASCII-якорю; всё, что в правиле ПОСЛЕ
/// квантификатора (закрывающий think-литерал), переносится как есть:
/// - `think-char*` → `think-char{M} think-char*` — прогон + неограниченный хвост;
/// - `think-char{0,B}` → `think-char{M} think-char{0,B−M}` — прогон + ужатый потолок.
///
/// Отказ от трансформации (правило возвращается как есть): `min_chars == 0`;
/// квантификатор не форма `*`/`{lo,hi}`; потолочная форма `{0,B}` с
/// `M >= B` — нет места под минимум (потолок < 24 симв.).
fn with_slot_min_body(draft_rule: &str, min_chars: usize) -> String {
    if min_chars == 0 {
        return draft_rule.to_string();
    }
    let Some(class_pos) = draft_rule.find("think-char") else {
        return draft_rule.to_string();
    };
    let quant_start = class_pos + "think-char".len();
    let rest = &draft_rule[quant_start..];
    let quant_len = match rest.as_bytes().first() {
        Some(b'*') => 1,
        Some(b'{') => match rest.find('}') {
            Some(close) => close + 1,
            None => return draft_rule.to_string(),
        },
        _ => return draft_rule.to_string(),
    };
    let quant = &rest[..quant_len];
    // Хвост правила ПОСЛЕ квантификатора (закрывающий think-литерал) обязан
    // пережить трансформацию: без него think никогда не закрывается, конверт
    // уходит внутрь незакрытой мысли — отказ «думатель без ответа».
    let remainder = &draft_rule[quant_start + quant_len..];
    let with_min = |tail: &str| {
        let mut out = String::with_capacity(draft_rule.len() + 16);
        out.push_str(&draft_rule[..class_pos]);
        out.push_str("think-char");
        out.push_str(&format!("{{{}}}", min_chars));
        out.push(' ');
        out.push_str(tail);
        out
    };
    if quant == "*" {
        // Неограниченный хвост остаётся как есть.
        return with_min(&draft_rule[class_pos..]);
    }
    let inner = &quant[1..quant.len() - 1];
    let mut parts = inner.splitn(2, ',');
    let lo = parts.next().unwrap_or("").trim();
    let Some(hi_raw) = parts.next() else {
        // {N} — точное число: форма не ожидается в правиле слота, не трогаем.
        return draft_rule.to_string();
    };
    if lo != "0" {
        return draft_rule.to_string();
    }
    let Ok(hi_num) = hi_raw.trim().parse::<usize>() else {
        return draft_rule.to_string();
    };
    if min_chars >= hi_num {
        // Потолок меньше минимума: физически нет места, форма прежняя.
        return draft_rule.to_string();
    }
    with_min(&format!("think-char{{0,{}}}{}", hi_num - min_chars, remainder))
}

/// Константы формулы (`ENVELOPE_TOKENS`, `CHARS_PER_TOKEN`) и сама формула
/// переехали в плагин (`tauri-plugin-llama-engine`, `llm_types.rs`) — SSOT
/// авто-потолка think для ВСЕХ JSON-контрактов: signal-агенты (здесь),
/// узел `llm_fact_extractor` (`fact_extractor.rs`), fused-группы
/// (`groups.rs`), вердикт-стенды (`verdict_contract.rs`). До 01.11.2026
/// формула жила только здесь, и потребители, до которых она не доехала,
/// оставались с неограниченным think (лог юзера 01.11.26: экстрактор,
/// 2×400 с в незакрытом `<think>`). Ниже — реэкспорт под прежним именем:
/// боевой путь сигналов и тесты (332/3000) работают без изменений.

/// Граница ОДНОГО черновика: бюджет генерации делится между черновиками слоя,
/// минус резерв под конверт сигнала.
///
/// Вариант для стендов (`reasoning_budget = 0` — движок думу отдельно не капит):
/// частный случай [`auto_think_budget_chars`], оставлен как отдельное имя,
/// чтобы места вызова читались без уточнения про движковый кап.
///
/// Возвращает `None`, если граница выходит нулевой: такой бюджет не способен
/// вместить даже пустой черновик, и молча ставить `Some(0)` значило бы запретить
/// модели размышлять вовсе. Тогда грамматика строится без границы, и отказ
/// приходит из движка, а не из заведомо непроходимого формата.
pub fn per_draft_budget_chars(max_gen_tokens: usize, drafts: usize) -> Option<usize> {
    auto_think_budget_chars(max_gen_tokens, 0, drafts)
}

/// Резерв конверта/слотов контракта в ТОКЕНАХ — вход формулы авто-потолка
/// think. До 05.11.2026 резервом служила фиксированная константа
/// ENVELOPE_TOKENS=256; контракт валидатора с вердикт-текстами (9 слотов до
/// 200 симв + target до 160) занимает заметно больше, и фиксированная
/// константа занижала резерв: потолки думы не учитывали место под слоты
/// (механический отказ «дума съела конверт» при заполнении по грамматике).
///
/// Оценка — худший case СЛОТОВОГО контракта: Σ(метка якоря + `"ключ": ` +
/// вердикт максимальной формы) по всем слотам плана плюс структурный запас
/// на разделители; результат в токенах (CHARS_PER_TOKEN). Неслотовый
/// контракт — прежняя константа (поведение без изменений). Формула
/// консервативна в бОльшую сторону — перебор резерва лишь занижает потолок
/// думы (не отказ), недобор создаёт непроходимый формат.
pub fn contract_envelope_tokens(schema: &serde_json::Value) -> usize {
    let Some(plan) = slot_verdict_plan(schema) else {
        return crate::infra::ENVELOPE_TOKENS;
    };
    let mut chars = 128usize; // sp-разделители, кавычки, структурный запас
    for (label, key, verdict_cap) in &plan.e_slots {
        chars += label.chars().count();
        chars += key.chars().count() + 5; // `"ключ": ` — кавычки, двоеточие, пробел
        chars += match verdict_cap {
            Some(cap) => cap + 2, // кавычки вердикт-текста
            None => 5,            // «false»/«true»
        };
    }
    for (label, key) in &plan.trailing {
        chars += label.chars().count();
        chars += key.chars().count() + 5;
        // Значение финального слота: enum — самый длинный литерал, свободная
        // строка — maxLength схемы, без него — константа резерва (честный
        // консервативный дефолт для свободных строк без объявленной длины).
        let prop = schema.get("properties").and_then(|p| p.get(key.as_str()));
        let value_cap = prop
            .and_then(|ps| ps.get("enum"))
            .and_then(Value::as_array)
            .map(|vals| {
                vals.iter()
                    .filter_map(Value::as_str)
                    .map(|v| v.chars().count())
                    .max()
                    .unwrap_or(0)
            })
            .or_else(|| {
                prop.and_then(|ps| ps.get("maxLength"))
                    .and_then(Value::as_u64)
                    .map(|n| n as usize)
            })
            .unwrap_or(crate::infra::ENVELOPE_TOKENS);
        chars += 2 + value_cap;
    }
    (chars / crate::infra::CHARS_PER_TOKEN).max(crate::infra::ENVELOPE_TOKENS)
}

/// Авто-потолок ОДНОГО дум-слота для боевого прогона от контракта: формула
/// плагина ([`crate::infra::auto_think_budget_chars_with_envelope`]) от
/// бюджета генерации и РЕЗЕРВА КОНТРАКТА ([`contract_envelope_tokens`]) —
/// место под слоты вычитается из бюджета ДО деления на дум-слоты. Заменяет
/// вызовы `per_draft_budget_chars` там, где есть схема контракта.
pub fn contract_think_budget(max_gen_tokens: usize, schema: &serde_json::Value) -> Option<usize> {
    crate::infra::auto_think_budget_chars_with_envelope(
        max_gen_tokens,
        0,
        think_slot_count(schema),
        contract_envelope_tokens(schema),
    )
}

/// Авто-потолок ОДНОГО черновика для боевого прогона (frontmatter агента без
/// `think_budget:`).
///
/// Формула та же, что у [`per_draft_budget_chars`], но от ЭФФЕКТИВНОГО потолка
/// мышления. Причина: у боевого движка два независимых капа думы.
///
/// 1. `--reasoning-budget N` (N > 0) — жёсткий кап СУММАРНОГО размышления в
///    токенах, enforced парсером reasoning на стороне сервера (дума обрывается,
///    когда токены кончились). Измерено на батче 31.10.2026: LFM2.5-8B в
///    валидаторе сжёг 1491 токен из 1500 — граница движка наступила раньше
///    конверта, ответ ушёл в деградирующие ретраи («отвечай СРАЗУ»), вердикт
///    потерял дисциплину цитат.
/// 2. `n_predict = max_gen_tokens` — общий бюджет генерации, покрывающий
///    думу И конверт вместе.
///
/// Потолок черновика обязан укладываться в оба капа: иначе модель, заполнившая
/// черновики до потолка грамматики, структурно не доходит до конверта — тот
/// самый механический отказ, который граница должна была устранить.
/// `reasoning_budget = 0` — флаг не активен (не задан или отключён фолбэком
/// цикла запуска): считаем только от общего бюджета генерации, как стенды.
///
/// Сама формула — в плагине (SSOT, см. комментарий выше); здесь реэкспорт.
pub use crate::infra::auto_think_budget_chars;

/// Phase 2: JSON-only grammar (без think-block).
/// Модель генерирует ТОЛЬКО JSON envelope — никакого think.
/// Основная грамматика Phase 2 двухфазного режима: размышления уже собраны в Phase 1
/// (без грамматики), здесь нужен только строгий конверт сигнала.
pub fn build_signal_envelope_json_only_grammar(contract: &SignalContract) -> String {
    let schema = &contract.value_schema;

    let sp = "sp ::= \" \" | \"\\n\" [ \\t]{0,4}";

    let json_rules = json_value_rules();

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

            let mut needs_verdict_rule = false;
            let mut value_rule_for = |pschema: &Value| -> String {
                match e_field_kind(pschema) {
                    // Вердикт-union (строка | false, 05.11.2026): JSON-only
                    // retry тоже обязан выпускать `false` при непрошедшем
                    // критерии — чистый json-string запретил бы легитимную
                    // альтернативу и порождал бы неразрешимый ретрай.
                    Some(EFieldKind::Union { .. }) => {
                        needs_verdict_rule = true;
                        "verdict-json".to_string()
                    }
                    _ => match pschema.get("type").and_then(|t| t.as_str()) {
                        Some("boolean") => "bool".to_string(),
                        Some("string") => "json-string".to_string(),
                        Some("number") | Some("integer") => "json-number".to_string(),
                        _ => "json-string".to_string(),
                    },
                }
            };

            // Обязательные ключи через запятую, опциональные — не включаем
            // (генерируем строго фиксированный порядок для required)
            let required_fields: Vec<String> = props
                .keys()
                .filter(|k| required_set.contains(*k))
                .map(|k| format!("\"\\\"{}\\\"\" sp \":\" sp {}", k, value_rule_for(&props[k])))
                .collect();

            let mut out = format!(
                "value-field ::= \"{{\" sp {} sp \"}}\"",
                required_fields.join(" \",\" sp ")
            );
            if needs_verdict_rule {
                out.push_str("\nverdict-json ::= json-string | \"false\"");
            }
            return out;
        }
    }

    // Fallback — любое JSON-значение
    "value-field ::= json-string | json-number | json-bool | \"null\"".to_string()
}

/// Нормализация вердикт-union полей сигнала (06.11.2026, инцидент
/// маршрутизации: `surface_proven=true` на неподтверждённых данных).
///
/// Модель может склеить отказ с пояснением в строку `"false (пояснение)"`
/// вместо литерала `false`: контракт `anyOf [string, false]` строку
/// разрешает, а `is_proven`-потребители (compute_pattern, тул сохранения
/// кейса, пробы) считают любую непустую строку доказанным элементом —
/// пайплайн уходил на лечение недоказанных данных. Отказ устраняется
/// конструктивно (§00.2): неоднозначная форма канонизируется на ГРАНИЦЕ
/// сохранения сигнала — e-поле контракта, чья строка является склеенной
/// формой отказа, заменяется на `false` до валидации и записи в messages[].
/// Единая точка вызова — `handle_emit_signal` (слоты, конверт, облако);
/// вторая линия — `is_proven` в тулах команды (облако/исторические сигналы
/// минуют движок).
///
/// Возвращает нормализованную копию; не-объекты и схемы без union-полей
/// проходят без изменений.
pub fn normalize_signal_value(value_schema: &Value, value: &Value) -> Value {
    if value_schema.get("type").and_then(Value::as_str) == Some("object") {
        if let Some(obj) = value.as_object() {
            let props = value_schema
                .get("properties")
                .and_then(Value::as_object);
            let mut out = obj.clone();
            if let Some(props) = props {
                for (pname, pschema) in props {
                    if matches!(e_field_kind(pschema), Some(EFieldKind::Union { .. })) {
                        if let Some(Value::String(s)) = out.get(pname) {
                            if false_text_form(s) {
                                out.insert(pname.clone(), Value::Bool(false));
                            }
                        }
                    }
                }
            }
            return Value::Object(out);
        }
    }
    value.clone()
}

/// Склеенная форма отказа «false (пояснение)»: после trim строка начинается
/// с литерала «false» (без учёта регистра), за которым НЕ идёт буква/цифра
/// («falsehood» — легитимный вердикт, не форма отказа). Вердикт-тексты —
/// кириллицей, латинская «false» в начале доказанной строки не встречается.
pub(crate) fn false_text_form(s: &str) -> bool {
    let trimmed = s.trim_start();
    let head = match trimmed.get(..5) {
        Some(h) => h,
        None => return false,
    };
    if !head.eq_ignore_ascii_case("false") {
        return false;
    }
    match trimmed[5..].chars().next() {
        None => true,
        Some(c) => !c.is_alphanumeric(),
    }
}

/// ЧЕСТНАЯ валидация значения сигнала против контракта (SSOT).
///
/// Гарантирует, что `value`, присланный эмиттером в `emit_signal`, содержит все
/// ОБЯЗАТЕЛЬНЫЕ поля контракта (`required` — напр. `e1..e9`, этажи у
/// `validator_report`). Если эмиттер исказил форму — возвращаем явную ошибку,
/// которую оркестратор отдаёт эмиттеру на retry. Это исключает тихий обрыв
/// маршрутизации, когда `signal_router` не находит поле и граф завершается,
/// не дойдя до `message`-узла (см. docs/SIGNAL_CONTRACTS.md).
///
/// Проверяется именно `required`, а НЕ все `properties`: properties могут
/// описывать поля, которых у эмиттера нет по архитектуре (03.11.2026:
/// floor2_type валидатора вычисляется нодой compute_pattern графа — тул
/// `tools/compute_pattern_type.ts`; в схеме его больше нет). Требовать от
/// эмиттера поле, которое грамматика контракта выпустить запрещает, —
/// структурно неразрешимый отказ (инцидент 03.11.2026: валидатор
/// перегенерировался 16+ раз по ~26 с). Отсутствие кодо-вычислимых полей
/// маршрутизируется графом как дефицит данных (check_pattern → добор).
/// Если у контракта `required` не объявлен — консервативный фолбэк: проверяем
/// все properties (форма не сужается незаметно для автора схемы).
pub fn validate_signal_value(contract: &SignalContract, value: &Value) -> Result<(), String> {
    let schema = &contract.value_schema;
    if schema.get("type").and_then(|t| t.as_str()) == Some("object") {
        let required_fields: Vec<String> =
            if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
                required
                    .iter()
                    .filter_map(|r| r.as_str().map(|s| s.to_string()))
                    .collect()
            } else if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                props.keys().cloned().collect()
            } else {
                Vec::new()
            };
        let props = schema.get("properties").and_then(Value::as_object);
        let mut missing: Vec<String> = Vec::new();
        for pname in &required_fields {
            if value.get(pname).is_none() {
                missing.push(pname.clone());
                continue;
            }
            // Вердикт-union (e-поле anyOf [string, false], 05.11.2026):
            // значение обязано быть непустой строкой-вердиктом или `false`.
            // Число/null/пустая строка — порча формы: честная ошибка эмиттеру
            // на retry вместо тихого мусора downstream.
            if let Some(ps) = props.as_ref().and_then(|p| p.get(pname.as_str())) {
                if matches!(e_field_kind(ps), Some(EFieldKind::Union { .. })) {
                    let shape_ok = match value.get(pname) {
                        Some(Value::Bool(_)) => true,
                        Some(Value::String(s)) => !s.trim().is_empty(),
                        _ => false,
                    };
                    if !shape_ok {
                        return Err(format!(
                            "Ошибка: поле '{}' контракта '{}' должно быть вердикт-текстом (непустая строка) или false. Исправь и вызови СНОВА.",
                            pname, contract.key
                        ));
                    }
                }
            }
        }
        if !missing.is_empty() {
            return Err(format!(
                "Ошибка: emit_signal для '{}' требует все поля контракта [{}], но в value их нет. Исправь и вызови СНОВА.",
                contract.key, missing.join(", ")
            ));
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
/// - `pattern` вида `^[1-9]$` → первый символ из набора.
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

/// Вариант B: собирает значение слотового контракта из **слотовых вердиктов** —
/// пар `"e<N>": <вердикт>`, которыми заканчивается каждый think-слот, плюс
/// строковых вердиктов финальных слотов (`"floor0_type": "…"`) —
/// см. [`slot_verdict_plan`].
///
/// SSOT: слоты — единственный источник вердикта; конверта для таких
/// контрактов не существует (грамматика его не выпускает). Сборка
/// ПРОГРЕССИРУЮЩИМ курсором (05.11.2026): метка слота ищется только ПОСЛЕ
/// вердикта предыдущего, поэтому цитата формата в дум-теле (раньше — риск
/// перехвата у глобального скана) не может стать якорем или вердиктом.
/// Дума собственного слота исключается якорем `</think>`: тело думы
/// запрещает `</think` ([`crate::infra::THINK_BODY_RULE`]), поэтому первое
/// закрытие после метки — конец думы этого слота, вердикт ищется за ним.
/// У облачных ответов think-тегов нет — вердикт ищется прямо от метки.
/// Если хоть один слот не собран — `None`: сигнал не собран, и штатные
/// ретраи формата работают как раньше.
pub fn assemble_slot_signal_value(schema: &Value, text: &str) -> Option<Value> {
    let plan = slot_verdict_plan(schema)?;
    let mut obj = serde_json::Map::new();
    let mut cursor = 0usize;
    for (label, key, verdict_cap) in &plan.e_slots {
        let label_pos = text[cursor..].find(label.as_str())? + cursor;
        let after_label = label_pos + label.len();
        // Якорь вердикта: конец думы слота (грамматика) или сама метка (облако).
        let anchor = match text[after_label..].find("</think>") {
            Some(rel) => after_label + rel + "</think>".len(),
            None => after_label,
        };
        let (verdict, past_key) = find_verdict_from(text, anchor, key, verdict_cap.is_some())?;
        obj.insert(key.clone(), verdict.into_value());
        cursor = past_key;
    }
    for (label, key) in &plan.trailing {
        let label_pos = text[cursor..].find(label.as_str())? + cursor;
        let after_label = label_pos + label.len();
        let (verdict, past_key) = find_verdict_from(text, after_label, key, true)?;
        obj.insert(key.clone(), verdict.into_value());
        cursor = past_key;
    }
    Some(Value::Object(obj))
}

/// Форма вердикта e-поля: булев литерал или вердикт-текст (union
/// `string|false` из схемы; у булевых слотов текст не допускается).
enum Verdict {
    Bool(bool),
    Text(String),
}

impl Verdict {
    fn into_value(self) -> Value {
        match self {
            Verdict::Bool(b) => Value::Bool(b),
            Verdict::Text(s) => Value::String(s),
        }
    }
}

/// Первое `"ключ": <вердикт>` в тексте с позиции `from` (байтовое смещение).
/// Возвращает значение и позицию ЗА литералом ключа — прогрессирующий курсор
/// сборки продолжает поиск следующего слота отсюда, чтобы цитаты формата в
/// дум-теле не находились повторно. Вхождения без вердикта после двоеточия
/// пропускаются (модельный текст мог упомянуть ключ в мысли);
/// продолжение идентификатора после `true`/`false` отбраковывается
/// (защита от `trueish`). `allow_text = false` — булев слот: строки
/// пропускаются (прежняя семантика `find_bool_after_key`).
fn find_verdict_from(
    text: &str,
    from: usize,
    key: &str,
    allow_text: bool,
) -> Option<(Verdict, usize)> {
    let needle = format!("\"{}\"", key);
    let mut search_from = from;
    while let Some(rel) = text[search_from..].find(&needle) {
        let abs = search_from + rel;
        let after = text[abs + needle.len()..].trim_start();
        if let Some(rest) = after.strip_prefix(':') {
            let v = rest.trim_start();
            if let Some(inner) = v.strip_prefix('"') {
                if allow_text {
                    if let Some(s) = scan_json_string(inner) {
                        return Some((Verdict::Text(s), abs + needle.len()));
                    }
                }
            } else {
                for (lit, val) in [("true", true), ("false", false)] {
                    if let Some(tail) = v.strip_prefix(lit) {
                        let ident_continues = tail
                            .chars()
                            .next()
                            .map(|c| c.is_alphanumeric() || c == '_')
                            .unwrap_or(false);
                        if !ident_continues {
                            return Some((Verdict::Bool(val), abs + needle.len()));
                        }
                    }
                }
            }
        }
        search_from = abs + needle.len();
    }
    None
}

/// Первое `"ключ": true|false` в тексте (легаси-обёртка: юнит-проверки
/// отбраковки идентификаторов и булевых слотов).
fn find_bool_after_key(text: &str, key: &str) -> Option<bool> {
    match find_verdict_from(text, 0, key, false) {
        Some((Verdict::Bool(b), _)) => Some(b),
        _ => None,
    }
}

/// Скан JSON-строки от содержимого ПОСЛЕ открывающей кавычки; возвращает
/// декодированное содержимое до закрывающей кавычки. Экраны декодируются
/// (`\n \t \r \b \f \/ \" \\` и `\uXXXX`): вердикт с «<», выпущенный
/// грамматикой как `\u003c`, обязан дойти до потребителя литералом «<» —
/// иначе serde при сериализации переэкранирует экран повторно и карточка
/// покажет `\u003c`. Незакрытая строка или битый `\u` — `None` (честный
/// отказ сборке, штатный ретрай формата); неизвестный экран сохраняется
/// как есть.
fn scan_json_string(inner: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('b') => out.push('\u{0008}'),
                Some('f') => out.push('\u{000C}'),
                Some('/') => out.push('/'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('u') => {
                    let mut hex = String::new();
                    for _ in 0..4 {
                        hex.push(chars.next()?);
                    }
                    let code = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(code)?);
                }
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => return None,
            },
            '"' => return Some(out),
            _ => out.push(c),
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
        // soma_translator исключён 03.11.2026: пользовательский агент без контракта.
        for id in ["validator", "focus_keeper"] {
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
        // Вердикт-union (05.11.2026): e-поле — вердикт-текст (строка до 200
        // симв) или false; булева форма осталась только у слоёв-стендов
        // (validator_flat/surface/facade/core).
        assert_eq!(
            e_field_kind(&c.value_schema["properties"]["e1"]),
            Some(EFieldKind::Union { max_chars: 200 })
        );
        // 9 вердикт-полей + классификация двух этажей + target (мишень разбора,
        // 05.11.2026). floor2_type из схемы УДАЛЁН — модель его не выдаёт,
        // SAFETY-механика вычисления живёт в туле
        // `agents/psychotherapist/tools/compute_pattern_type.ts` (нода
        // compute_pattern графа); движок агент-агностичен.
        assert_eq!(c.value_schema["required"].as_array().map(|e| e.len()), Some(12));
        assert!(
            c.value_schema["properties"].get("floor2_type").is_none(),
            "floor2_type больше не свойство контракта: его вычисляет нода compute_pattern"
        );
        // target — ПОСЛЕДНИЙ в required (порядок required = порядок слотов
        // грамматики; финальные строковые слоты идут после всех e-слотов).
        assert_eq!(
            c.value_schema["required"]
                .as_array()
                .and_then(|a| a.last())
                .and_then(Value::as_str),
            Some("target"),
            "target обязан быть последним слотом контракта"
        );
        for field in ["floor0_type", "floor1_type"] {
            assert_eq!(
                c.value_schema["properties"][field]["enum"].as_array().map(|e| e.len()),
                Some(9),
                "enum {field} — 9 типов (SSOT для грамматики финального слота)"
            );
        }
        assert!(
            !c.value_schema["required"]
                .as_array()
                .expect("required массив")
                .iter()
                .any(|v| v.as_str() == Some("floor2_type")),
            "floor2_type не в required: модель его не выдаёт, механику считает нода compute_pattern"
        );
    }

    /// Регрессия: soma_translator — ПОЛЬЗОВАТЕЛЬСКИЙ агент без сигнального
    /// контракта (03.11.2026): контракт GBNF-конвертом съедал пользовательскую
    /// расшифровку (ответ агента целиком становился думой+JSON-конвертом),
    /// рукость теперь определяется фактом `handedness` (facts.yaml), а не
    /// сигналом. Записи в схеме быть не должно.
    #[test]
    fn soma_translator_is_not_a_signal_emitter() {
        let text = std::fs::read_to_string(schema_path()).expect("root.schema.json прочитать");
        let root: Value = serde_json::from_str(&text).expect("root.schema.json — валидный JSON");
        let agents = root.get("agents").expect("ключ agents");
        assert!(
            agents.get("soma_translator").is_none(),
            "soma_translator не сигнал-эмиттер: расшифровка идёт в чат, рукость — факт handedness"
        );
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
            .find(|line| line.starts_with("draft-1 ::="))
            .unwrap_or_default();
        // Минимум содержания (решение владельца 01.11.2026, Вариант A): тело
        // слота = минимальный прогон + ужатый потолок, нулевой слот невозможен.
        let min = slot_min_chars(Some(1400));
        assert!(
            min == 175,
            "формула MIN = max(24, B/8) фиксируется до прогона: {}",
            min
        );
        assert!(
            bounded_draft.contains(&format!(
                "think-char{{{}}} think-char{{0,{}}}",
                min,
                1400 - min
            )),
            "в грамматике нет минимума содержания и ужатого потолка: {}",
            bounded_draft
        );
        // Вариант B: слот e-контракта заканчивается собственным вердиктом —
        // токен вердикта стоит СРАЗУ после закрывающего think-литерала.
        assert!(
            bounded_draft.contains(r#""\"e1\": " bool"#),
            "вердикт-литерал слота 1 потерян: {}",
            bounded_draft
        );
        // Агент без объявленного потолка: черновик не ограничен грамматикой,
        // бюджет задаёт движок (`max_gen_tokens` из конфига пользователя);
        // минимум содержания держится и здесь (нулевой слот невозможен при любой
        // конфигурации), хвост остаётся неограниченным.
        let unbounded = build_signal_envelope_grammar(&contract);
        let unbounded_draft = unbounded
            .lines()
            .find(|line| line.starts_with("draft-1 ::="))
            .unwrap_or_default();
        assert!(
            !unbounded_draft.contains("{0,"),
            "грамматика без бюджета не должна получать границу: {}",
            unbounded_draft
        );
        assert!(
            unbounded_draft.contains("think-char*"),
            "неограниченный хвост потерян"
        );
        assert!(
            unbounded_draft.contains(&format!("think-char{{{}}}", slot_min_chars(None))),
            "минимум содержания потерян в слоте без формального потолка: {}",
            unbounded_draft
        );
        // Закрывающий литерал слота обязан пережить трансформацию минимума
        // (дефект 01.11.2026: bounded-ветка теряла хвост после квантификатора —
        // think не закрывался, конверт уходил внутрь незакрытой мысли).
        // Литерал НЕ переписывается руками (чаннель-глитч): берётся срезом из
        // неограниченной строки по якорю тела `think-char*`.
        let close_tail = unbounded_draft
            .split("think-char*")
            .nth(1)
            .expect("хвост неограниченного слота после класса")
            .to_string();
        assert!(
            !close_tail.trim().is_empty() && bounded_draft.ends_with(&close_tail),
            "закрывающий литерал потерян в ограниченном слоте: {}",
            bounded_draft
        );
        // Число черновиков не зависит от потолка: его задаёт контракт.
        // Якорный режим (e-контракт): слоты поимённо через sp-разделитель,
        // потолок меняет только тело слота.
        assert!(
            bounded.contains("drafts-block ::= draft-1 sp draft-2")
                && unbounded.contains("drafts-block ::= draft-1 sp draft-2"),
            "потолок не должен менять число черновиков"
        );
        assert!(
            bounded.contains(r#"draft-1 ::= "Element 1: ""#),
            "якорь слота 1 потерян: {}",
            bounded
        );
        // Остальная форма конверта одинакова — граница меняет только черновик.
        assert_eq!(
            bounded.lines().count(),
            unbounded.lines().count(),
            "структура грамматики разъехалась"
        );
    }

    /// Перенос строки внутри ограниченного черновика РАЗРЕШЁН (решение
    /// владельца 31.10.2026): черновики по элементам модель пишет списками
    /// («Элемент 3: … → false»), запрет `\n` вынуждал её в одну длинную
    /// строку. `<` исключён по-прежнему — иначе модель ушла бы в `<think>`
    /// внутри `<think>`.
    #[test]
    fn bounded_draft_allows_newlines() {
        let contract = SignalContract {
            key: "validator_surface".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "e1": { "type": "boolean" } },
                "required": ["e1"]
            }),
        };
        let bounded = build_signal_envelope_grammar_with_think_budget(&contract, Some(300));
        let draft = bounded
            .lines()
            .find(|line| line.starts_with("draft-1 ::="))
            .unwrap_or_default();
        // Минимум содержания: прогон по формуле max(24, B/8) + ужатый потолок;
        // нулевая альтернатива тела устранена (решение владельца 01.11.2026).
        let min = slot_min_chars(Some(300));
        assert!(
            draft.contains(&format!("think-char{{{}}} think-char{{0,{}}}", min, 300 - min)),
            "минимум содержания/ужатый потолок потеряны: {}",
            draft
        );
        assert!(
            !draft.contains("[^\\n<]"),
            "запрет переноса строки должен быть снят: {}",
            draft
        );
        // Вариант B: слот заканчивается вердиктом, а не конвертом.
        assert!(
            draft.contains(r#""\"e1\": " bool"#),
            "вердикт-литерал слота 1 потерян: {}",
            draft
        );
        // Непрерывный черновик (без границы) остался прежним.
        let unbounded = build_signal_envelope_grammar(&contract);
        let unbounded_draft = unbounded
            .lines()
            .find(|line| line.starts_with("draft-1 ::="))
            .unwrap_or_default();
        assert!(
            unbounded_draft.contains("think-char*"),
            "непрерывный черновик изменён: {}",
            unbounded_draft
        );
    }

    /// Авто-потолок обязан считаться от ЭФФЕКТИВНОГО капа мышления:
    /// min(reasoning_budget, max_gen_tokens − резерв конверта). Измерено на
    /// батче 31.10.2026: кап думы 1500 токенов наступил РАНЬШЕ конверта
    /// (LFM2.5: 1491 токен думы) — потолок от одного max_gen_tokens вернул бы
    /// тот же отказ «думатель без ответа».
    #[test]
    fn auto_think_budget_respects_reasoning_cap() {
        // Боевой случай валидатора: 9 слотов, rb=1500, mgt=2048:
        // min(1500, 2048−256) = 1500 → 1500/9 = 166 ток × 2 симв = 332.
        assert_eq!(auto_think_budget_chars(2048, 1500, 9), Some(332));
        // Сома: один слот — весь эффективный кап думы.
        assert_eq!(auto_think_budget_chars(2048, 1500, 1), Some(3000));
        // rb=0 — флаг не активен: эквивалент стендовой формулы.
        assert_eq!(
            auto_think_budget_chars(2048, 0, 9),
            per_draft_budget_chars(2048, 9),
            "rb=0 обязан совпадать со стендовой формулой"
        );
        assert_eq!(auto_think_budget_chars(2048, 0, 9), Some(398));
        // rb больше бюджета генерации: генерация — единственный кап.
        assert_eq!(
            auto_think_budget_chars(2048, 8000, 1),
            per_draft_budget_chars(2048, 1),
            "rb больше бюджета генерации не должен ужать потолок"
        );
        // rb уже max_gen_tokens: rb — единственный кап.
        assert_eq!(auto_think_budget_chars(2048, 1000, 2), Some(1000));
        // Ничтожный бюджет не даёт непроходимой границы — None.
        assert_eq!(auto_think_budget_chars(64, 0, 9), None);
    }

    /// Извлечь число черновиков из строки `drafts-block ::= draft{N}`
    /// (свободный режим) или `drafts-block ::= draft-1 ... draft-N`
    /// (якорный режим, решение владельца 01.11.2026).
    fn drafts_count_of(grammar: &str) -> usize {
        grammar
            .lines()
            .find(|line| line.starts_with("drafts-block ::= draft{"))
            .and_then(|line| {
                let start = "drafts-block ::= draft{".len();
                let end = line.len().saturating_sub(1);
                line.get(start..end).and_then(|n| n.parse::<usize>().ok())
            })
            .or_else(|| {
                grammar
                    .lines()
                    .find(|line| line.starts_with("drafts-block ::= draft-"))
                    .map(|line| {
                        line.split_whitespace()
                            .filter(|word| word.starts_with("draft-"))
                            .count()
                    })
                    .filter(|count| *count > 0)
            })
            .unwrap_or(0)
    }

    /// Слот отчёта обязан существовать у ЛЮБОГО контракта (инвариант ≥ 1).
    /// Enum-схемы (focus_keeper) не имеют `required` — прежнее
    /// `draft{0}` убивало отчёт: грамматика не оставляла выхода для анализа,
    /// модель шла сразу в конверт (регрессия 93e30c7; tasks/27.10.26 draft0.md).
    #[test]
    fn enum_contract_gets_one_draft_slot() {
        let contract = SignalContract {
            key: "focus_keeper".to_string(),
            value_schema: serde_json::json!({
                "type": "string",
                "enum": ["НУЖНА РУКОСТЬ", "РУКОСТЬ ИЗВЕСТНА"]
            }),
        };
        let grammar = build_signal_envelope_grammar(&contract);
        assert_eq!(
            drafts_count_of(&grammar),
            1,
            "у enum-контракта обязан быть ровно один слот отчёта, grammar:\n{}",
            grammar
        );
        // Строковые правила — канонический блок плагина (strict_json_string_rules):
        // raw «<» в значениях запрещён — think-smuggle в значении невозможен
        // (llama.cpp#15000, лог юзера 01.11.26).
        assert!(
            grammar.contains("string-char ::= [^\"\\\\<] | escape"),
            "конвертная грамматика несёт свободный строковый класс: {}",
            grammar
        );
    }

    /// Object-контракты сохраняют прежнее поведение: один черновик на каждое
    /// обязательное поле (валидатор и его слои).
    #[test]
    fn object_contract_keeps_draft_per_required_field() {
        let contract = SignalContract {
            key: "validator_surface".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "e1": { "type": "boolean" }, "e2": { "type": "boolean" } },
                "required": ["e1", "e2"]
            }),
        };
        let grammar = build_signal_envelope_grammar(&contract);
        assert_eq!(
            drafts_count_of(&grammar),
            2,
            "object-контракт: черновик на каждое required-поле"
        );
        // Якорный режим (e-контракт, решение владельца 01.11.2026): каждый слот
        // обязан начаться с метки своего элемента.
        assert!(
            grammar.contains(r#"draft-1 ::= "Element 1: ""#)
                && grammar.contains(r#"draft-2 ::= "Element 2: ""#),
            "якорные слоты потеряны: {}",
            grammar
        );
        // Минимум содержания в якорном теле (Вариант A): нулевой слот невозможен
        // и без формального потолка (build_signal_envelope_grammar = None).
        assert!(
            grammar.contains(&format!("think-char{{{}}} think-char*", slot_min_chars(None))),
            "минимум содержания в якорном теле слота потерян: {}",
            grammar
        );
    }

    /// Формула MIN и трансформация тела слота (гейты зафиксированы ДО прогона
    /// в план-файле, 01.11.2026): нулевые альтернативы не рождаются, потолок
    /// не роняется, форма без места под минимум не ломается.
    #[test]
    fn slot_min_chars_formula_and_body_transform() {
        assert_eq!(slot_min_chars(None), 24);
        // Боевой валидатор: 9 слотов, rb=1500, mgt=2048 → B=332 → max(24, 41) = 41.
        assert_eq!(slot_min_chars(Some(332)), 41);
        assert_eq!(slot_min_chars(Some(1400)), 175);
        // Потолок < 24: минимум прижат к потолку и в форму не входит.
        assert_eq!(slot_min_chars(Some(20)), 20);
        // Потолочная форма: минимум + ужатый хвост, верх не вырос.
        let bounded = "x ::= think-char{0,300}";
        assert_eq!(
            with_slot_min_body(bounded, 37),
            "x ::= think-char{37} think-char{0,263}"
        );
        // Нет места под минимум (M >= B): форма не тронута.
        assert_eq!(with_slot_min_body(bounded, 300), bounded);
        // Неограниченный хвост: минимум + прежний хвост.
        let star = "x ::= think-char*";
        assert_eq!(with_slot_min_body(star, 24), "x ::= think-char{24} think-char*");
    }

    /// Ни один production-контракт не может остаться без слота отчёта.
    #[test]
    fn all_production_contracts_have_at_least_one_draft() {
        let path = schema_path();
        let dir = path.parent().expect("папка signals");
        let map = load_all_signal_contracts(dir);
        assert!(map.len() > 1, "контракты не прочитаны");
        for (id, contract) in &map {
            let grammar = build_signal_envelope_grammar(contract);
            assert!(
                drafts_count_of(&grammar) >= 1,
                "контракт '{}' остался без слота отчёта (draft{{0}})",
                id
            );
        }
        // soma_translator исключён из контрактов 03.11.2026 (пользовательский
        // агент): enum-пример — focus_keeper.
        for id in ["focus_keeper"] {
            let contract = map.get(id).unwrap_or_else(|| panic!("контракт {} не найден", id));
            assert_eq!(
                drafts_count_of(&build_signal_envelope_grammar(contract)),
                1,
                "контракт {}: enum-схема обязана давать ровно один слот отчёта",
                id
            );
        }
        // Якорный режим (решение владельца 01.11.2026): валидатор обязан иметь
        // 9 слотов с метками Element 1..9, а не 9 свободных черновиков.
        let validator = map.get("validator").expect("контракт validator");
        let validator_grammar = build_signal_envelope_grammar(validator);
        for i in 1..=9 {
            assert!(
                validator_grammar.contains(&format!(r#"draft-{} ::= "Element {}: ""#, i, i)),
                "валидатор: якорь слота {} потерян, grammar:\n{}",
                i,
                validator_grammar
            );
        }
    }

    /// Phase-2 JSON-only rescue-билдер остаётся без слота отчёта: там отчёт не
    /// нужен (размышления Phase 1 уже сохранены thought-сообщением).
    #[test]
    fn json_only_grammar_has_no_drafts_block() {
        let contract = SignalContract {
            key: "focus_keeper".to_string(),
            value_schema: serde_json::json!({
                "type": "string",
                "enum": ["НУЖНА РУКОСТЬ", "РУКОСТЬ ИЗВЕСТНА"]
            }),
        };
        let grammar = build_signal_envelope_json_only_grammar(&contract);
        assert!(
            !grammar.contains("drafts-block"),
            "Phase-2 билдер не должен нести слот отчёта: {}",
            grammar
        );
        // Phase-2/ретрай — прямой наследник канала smuggle: без think-блока
        // свободный «<» в значении позволял открыть <think> и сжечь бюджет
        // (лог юзера 01.11.26). Класс — канонический блок плагина.
        assert!(
            grammar.contains("string-char ::= [^\"\\\\<] | escape"),
            "Phase-2 грамматика несёт свободный строковый класс: {}",
            grammar
        );
    }

    /// Экранирование литералов внутри грамматики обязано остаться GBNF-строкой
    /// вида `"\"e1\": "` / `"\"value\""`. Проверено на движке 11.10.26: при
    /// «голом» `\"value\"` парсер llama.cpp отвечает `failed to parse grammar`,
    /// а все тесты Rust при этом остаются зелёными — грамматику проверяет только
    /// сервер. Сырой литерал здесь обязателен: повторный набор экранирования в
    /// обычной строке и породил тот самый отказ.
    ///
    /// С 01.11.2026 (Вариант B) у e-контрактов конверта нет — тот же класс
    /// опасности переехал в вердикт-литералы слотов, поэтому для слоёв воронки
    /// проверяется экранирование `"\"eN\": "`, а конвертный литерал — на не-e
    /// контракте.
    #[test]
    fn value_key_escaping_survives() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .join("agents")
            .join("psychotherapist")
            .join("signals");
        let layer_keys: &[(&str, &[&str])] = &[
            ("validator_surface", &["e1", "e2", "e6", "e7"]),
            ("validator_facade", &["e4", "e5", "e8", "e9"]),
            ("validator_core", &["e3"]),
        ];
        for (id, keys) in layer_keys {
            let contract = load_signal_contract(&dir, id).expect("контракт прочитан");
            let grammar = build_signal_envelope_grammar(&contract);
            for key in *keys {
                let verdict_literal = format!(r#""\"{}\": ""#, key);
                assert!(
                    grammar.contains(&verdict_literal),
                    "экранирование вердикт-литерала {} сломано в грамматике слоя {}, grammar:\n{}",
                    verdict_literal,
                    id,
                    grammar
                );
            }
        }
        // Конвертная ветка жива для не-e контрактов — там прежний литерал `"\"value\""`.
        // (soma_translator исключён из контрактов 03.11.2026; enum-пример — focus_keeper.)
        let enum_contract = load_signal_contract(&dir, "focus_keeper").expect("контракт прочитан");
        let enum_grammar = build_signal_envelope_grammar(&enum_contract);
        assert!(
            enum_grammar.contains(
                r#"arguments-json ::= "{" sp key-field "," sp "\"value\"" sp ":" sp value-field sp "}""#
            ),
            "экранирование ключа value сломано в конвертной грамматике, grammar:\n{}",
            enum_grammar
        );
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
            // soma_translator исключён 03.11.2026: пользовательский агент
            // (расшифровка в чат), рукость — факт handedness, не сигнал.
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
            key: "focus_keeper".to_string(),
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

        // Поле из properties ВНЕ required (кодо-вычислимое, напр. floor2_type
        // валидатора) может отсутствовать: эмиттер не обязан его выдавать, а
        // грамматика контракта его выпустить запрещает. Требование такого поля
        // порождало структурно неразрешимый ретрай-цикл (инцидент 03.11.2026).
        let c_code_field = SignalContract {
            key: "validator_report".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "e1": { "type": "boolean" },
                    "floor2_type": { "type": "string" }
                },
                "required": ["e1"]
            }),
        };
        assert!(validate_signal_value(&c_code_field, &serde_json::json!({ "e1": true })).is_ok());
        // Обязательное поле отсутствует — по-прежнему ошибка.
        assert!(validate_signal_value(&c_code_field, &serde_json::json!({})).is_err());
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
            grammar.contains("drafts-block ::= draft-1 sp draft-2"),
            "два обязательных поля контракта — значит два слота через sp: {}",
            grammar
        );
        // Конверт больше не несёт поле thought (размышления — Phase 1).
        assert!(!grammar.contains("thought-field"), "поле thought убрано из конверта");
        // Грамматика ОБЯЗАНА содержать ключи сигнала — как вердикт-литералы слотов
        assert!(
            grammar.contains(r#""\"e1\": ""#) && grammar.contains(r#""\"e2\": ""#),
            "вердикт-литералы e1/e2 потеряны: {}",
            grammar
        );
        // Вариант B (01.11.2026): для e-контрактов конверт УСТРАНЁН — канал
        // «угадать всё в конце» закрыт грамматически, значение собирается из
        // слотов кодом. Его отсутствие — конструктивное свойство, а не деталь.
        assert!(
            !grammar.contains("arguments-json") && !grammar.contains("emit_signal"),
            "у e-контракта не должно быть конверта (вердикты живут в слотах): {}",
            grammar
        );
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
    // Вариант B (01.11.2026): слотовые вердикты — детекция и сборка
    // ═══════════════════════════════════════════════════════════════════

    /// Слотовым контрактом считается контракт, у которого обязательные поля —
    /// boolean-элементы `e<N>` подряд, плюс (02.11.2026) допустимые ФИНАЛЬНЫЕ
    /// строковые поля после них. Конвертные формы (enum/pattern, не-e boolean,
    /// перемешанные поля) не должны распознаваться как слотовые.
    #[test]
    fn slot_verdict_contract_detection() {
        let slot = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "e1": { "type": "boolean" }, "e2": { "type": "boolean" } },
            "required": ["e1", "e2"]
        });
        assert!(is_slot_verdict_contract(&slot));

        // e-поле + ФИНАЛЬНОЕ строковое поле (floor0_type/floor1_type у
        // валидатора, 02.11.2026) — слотовый контракт с финальным слотом
        // без дум-тела.
        let mixed = serde_json::json!({
            "type": "object",
            "properties": { "e1": { "type": "boolean" }, "note": { "type": "string" } },
            "required": ["e1", "note"]
        });
        assert!(is_slot_verdict_contract(&mixed));

        // Вердикт-текст (union string{1..maxLength} | false) вместо буля —
        // тоже слотовый контракт (05.11.2026, боевой вид валидатора).
        let union = serde_json::json!({
            "type": "object",
            "properties": {
                "e1": { "anyOf": [{ "type": "string", "minLength": 1, "maxLength": 200 }, { "const": false }] }
            },
            "required": ["e1"]
        });
        assert!(is_slot_verdict_contract(&union));

        // Union без maxLength: форма не ограничена схемой — план не строится
        // (честное падение в конвертный путь), а не безлимитный вердикт.
        let union_unbounded = serde_json::json!({
            "type": "object",
            "properties": {
                "e1": { "anyOf": [{ "type": "string" }, { "const": false }] }
            },
            "required": ["e1"]
        });
        assert!(!is_slot_verdict_contract(&union_unbounded));

        // Не-e поле (boolean) — конвертный контракт.
        let not_e = serde_json::json!({
            "type": "object",
            "properties": { "field_a": { "type": "boolean" } },
            "required": ["field_a"]
        });
        assert!(!is_slot_verdict_contract(&not_e));

        // Перемешивание: строковое поле МЕЖДУ e-полями — конвертный контракт
        // (финальные слоты допустимы только после ВСЕХ e-полей).
        let interleaved = serde_json::json!({
            "type": "object",
            "properties": {
                "e1": { "type": "boolean" },
                "note": { "type": "string" },
                "e2": { "type": "boolean" }
            },
            "required": ["e1", "note", "e2"]
        });
        assert!(!is_slot_verdict_contract(&interleaved));

        // Enum — конвертный.
        let enum_schema = serde_json::json!({ "type": "string", "enum": ["А", "Б"] });
        assert!(!is_slot_verdict_contract(&enum_schema));

        // Пустой required — не слотовый.
        let empty = serde_json::json!({ "type": "object", "required": [] });
        assert!(!is_slot_verdict_contract(&empty));
    }

    /// Сборка значения из слотовых вердиктов: первое `"e<N>": true|false` на
    /// каждый required-ключ; упоминание ключа в мысли без буля после двоеточия
    /// пропускается; недостающий ключ → None (сигнал не собран, работают
    /// штатные ретраи формата).
    #[test]
    fn assemble_slot_signal_value_from_slots() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "e1": { "type": "boolean" }, "e2": { "type": "boolean" } },
            "required": ["e1", "e2"]
        });
        let text = "Element 1: разбор первого элемента, цитата найдена, \"e1\": true\n\
                    Element 2: подтверждений нет, must_not_be совпал, \"e2\": false";
        assert_eq!(
            assemble_slot_signal_value(&schema, text),
            Some(serde_json::json!({ "e1": true, "e2": false }))
        );

        // Упоминание ключа без буля после двоеточия (в prose/мысли) пропускается.
        let single = serde_json::json!({
            "type": "object",
            "properties": { "e1": { "type": "boolean" } },
            "required": ["e1"]
        });
        let with_prose = "Element 1: ключ \"e1\" обсуждается без решения, итог: \"e1\": false";
        assert_eq!(
            assemble_slot_signal_value(&single, with_prose),
            Some(serde_json::json!({ "e1": false }))
        );

        // Второй ключ не найден → None.
        let partial = "Element 1: разбор, \"e1\": true";
        assert_eq!(assemble_slot_signal_value(&schema, partial), None);

        // Мусорный текст → None.
        assert_eq!(
            assemble_slot_signal_value(&schema, "модель ушла в мысли и не ответила"),
            None
        );

        // Не-слотовый контракт из слотов не собирается вовсе.
        let enum_schema = serde_json::json!({ "type": "string", "enum": ["А", "Б"] });
        assert_eq!(
            assemble_slot_signal_value(&enum_schema, "\"e1\": true \"e2\": false"),
            None
        );
    }

    /// Финальные слоты (floor0_type/floor1_type, 02.11.2026): строковые
    /// вердикты собираются ПОСЛЕ якорей меток `Floor0 type: `/`Floor1 type: ` —
    /// дум-тело, цитирующее формат `"floor0_type": "…"`, не должно
    /// перехватывать парсер. Пропуск вердикта (грамматика не применялась) → None.
    #[test]
    fn assemble_slot_signal_value_with_trailing_floor_types() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "e1": { "type": "boolean" },
                "e2": { "type": "boolean" },
                "floor0_type": { "type": "string", "enum": ["1", "2", "3"] },
                "floor1_type": { "type": "string", "enum": ["1", "2", "3"] }
            },
            "required": ["e1", "e2", "floor0_type", "floor1_type"]
        });
        let text = "Element 1: разбор, цитата найдена, \"e1\": true\n\
                    Element 2: разбор, \"e2\": false\n\
                    Шаг 2: думаю про floor1_type, вот формат: \"floor1_type\": \"9\"\n\
                    Floor0 type: \"floor0_type\": \"1\"\n\
                    Floor1 type: \"floor1_type\": \"7\"";
        assert_eq!(
            assemble_slot_signal_value(&schema, text),
            Some(serde_json::json!(
                { "e1": true, "e2": false, "floor0_type": "1", "floor1_type": "7" }
            )),
            "вердикты берутся из финальных слотов по своим меткам, а не из цитат в дум-теле"
        );

        // Вердикта финального слота нет → None (штатный ретрай формата).
        let no_trailing = "Element 1: разбор, \"e1\": true\nElement 2: разбор, \"e2\": false";
        assert_eq!(assemble_slot_signal_value(&schema, no_trailing), None);
    }

    /// Вердикт-тексты (05.11.2026): e-слот union думает, закрывает `</think>`
    /// и пишет строковый вердикт или `false`. Сборка берёт вердикт ПОСЛЕ
    /// закрытия думы своего слота (цитата формата `"e1": true` внутри думы
    /// перехватчика не даёт); экран `\u003c` декодируется до литерала «<»
    /// (грамматика запрещает raw «<» в строках).
    #[test]
    fn assemble_slot_signal_value_union_verdict_texts() {
        let union_field = serde_json::json!({
            "anyOf": [{ "type": "string", "minLength": 1, "maxLength": 200 }, { "const": false }]
        });
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "e1": union_field, "e2": union_field },
            "required": ["e1", "e2"]
        });
        let text = "Element 1: <think>проверка критерия; формат: \"e1\": true — так НЕ отвечать</think> \"e1\": \"Спазм лба и висков: зажим держит тревогу без разрядки\"\n\
                    Element 2: <think>повтора нет</think> \"e2\": false";
        assert_eq!(
            assemble_slot_signal_value(&schema, text),
            Some(serde_json::json!({
                "e1": "Спазм лба и висков: зажим держит тревогу без разрядки",
                "e2": false
            })),
            "вердикт-текст берётся после закрытия думы своего слота; цитата формата в думе не перехватывает"
        );

        // Экран `\u003c` декодируется до литерала «<». Сборка тотальна по
        // плану слотов (required e1+e2) — вход обязан содержать оба слота.
        let escaped = "Element 1: <think>разбор</think> \"e1\": \"вдох \\u003c выдох по длительности\"\n\
                       Element 2: <think>повтора нет</think> \"e2\": false";
        assert_eq!(
            assemble_slot_signal_value(&schema, escaped),
            Some(serde_json::json!({ "e1": "вдох < выдох по длительности", "e2": false })),
            "экран строки обязан декодироваться, а не доходить до карточки литералом"
        );

        // Прогрессирующий курсор: цитата ЧУЖОГО слота целиком (метка + дума +
        // вердикт) внутри думы слота 1 не перехватывает сборку — поиск слота 2
        // начинается после вердикта слота 1.
        let quoted_next = "Element 1: <think>цитата: Element 2: <think>fake</think> \"e2\": true</think> \"e1\": \"вердикт один\"\n\
                           Element 2: <think>разбор</think> \"e2\": false";
        assert_eq!(
            assemble_slot_signal_value(&schema, quoted_next),
            Some(serde_json::json!({ "e1": "вердикт один", "e2": false })),
            "цитата чужого слота в думе не должна становиться якорем или вердиктом"
        );
    }

    /// Резерв конверта из контракта (05.11.2026): вердикт-тексты занимают
    /// заметно больше фикс-константы ENVELOPE_TOKENS=256, поэтому потолки
    /// думы обязаны считать место под слоты из схемы — иначе заполнение по
    /// грамматике упирается в max_gen_tokens («дума съела конверт»).
    #[test]
    fn contract_envelope_tokens_scales_with_verdict_texts() {
        let path = schema_path();
        let dir = path.parent().expect("папка signals");
        let validator = load_signal_contract(dir, "validator").expect("контракт валидатора");
        let reserve = contract_envelope_tokens(&validator.value_schema);
        assert!(
            reserve > crate::infra::ENVELOPE_TOKENS,
            "вердикт-тексты (9×200 симв + target 160) обязаны дать резерв больше фикс-константы: {}",
            reserve
        );
        // Боевой потолок думы слота от резерва КОНТРАКТА строже прежней
        // формулы от константы: место под слоты вычитается до деления.
        let budget = contract_think_budget(2048, &validator.value_schema);
        let legacy = per_draft_budget_chars(2048, 9);
        assert!(
            budget.is_some() && budget < legacy,
            "бюджет с резервом контракта ({:?}) обязан быть строже прежнего {:?}",
            budget,
            legacy
        );
        // Булев слотовый контракт (слои-стенды) — прежняя константа: короткие
        // вердикты `false`/`true` не раздувают резерв.
        let flat = SignalContract {
            key: "validator_flat".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "properties": { "e1": { "type": "boolean" } },
                "required": ["e1"]
            }),
        };
        assert_eq!(
            contract_envelope_tokens(&flat.value_schema),
            crate::infra::ENVELOPE_TOKENS,
            "булев слотовый контракт не должен раздувать резерв"
        );
        // Неслотовый контракт — константа (поведение без изменений).
        let enum_schema = serde_json::json!({ "type": "string", "enum": ["А", "Б"] });
        assert_eq!(contract_envelope_tokens(&enum_schema), crate::infra::ENVELOPE_TOKENS);
    }

    /// Вердикт-union (05.11.2026): непустая строка или `false` проходит;
    /// число/null/пустая строка — честная ошибка эмиттеру (порча формы),
    /// а не тихий мусор downstream.
    #[test]
    fn validate_signal_value_rejects_malformed_verdict_union() {
        let c = SignalContract {
            key: "validator_report".to_string(),
            value_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "e1": { "anyOf": [{ "type": "string", "minLength": 1, "maxLength": 200 }, { "const": false }] }
                },
                "required": ["e1"]
            }),
        };
        assert!(validate_signal_value(&c, &serde_json::json!({ "e1": "Спазм лба: зажим" })).is_ok());
        assert!(validate_signal_value(&c, &serde_json::json!({ "e1": false })).is_ok());
        assert!(validate_signal_value(&c, &serde_json::json!({ "e1": 42 })).is_err());
        assert!(validate_signal_value(&c, &serde_json::json!({ "e1": "" })).is_err());
        assert!(validate_signal_value(&c, &serde_json::json!({ "e1": null })).is_err());
    }

    /// Нормализация вердикт-union (06.11.2026, инцидент маршрутизации:
    /// «false (…)»-строки дали surface_proven=true на пустых данных и повели
    /// пайплайн на shadow_worker): склеенная форма отказа канонизируется в
    /// `false` на границе сохранения сигнала; легитимные вердикты, не-union
    /// поля и не-объекты проходят без изменений.
    #[test]
    fn normalize_signal_value_canonizes_false_text_form() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "e1": { "anyOf": [{ "type": "string", "minLength": 1, "maxLength": 200 }, { "const": false }] },
                "target": { "type": "string" }
            },
            "required": ["e1", "target"]
        });
        // Склеенная форма отказа (с лид-пробелами и хвостом) → false.
        let out = normalize_signal_value(
            &schema,
            &serde_json::json!({
                "e1": "  false (физическая боль без описания ментального тупика).\n\n",
                "target": "Спазм"
            }),
        );
        assert_eq!(out["e1"], serde_json::json!(false));
        // Легитимный вердикт-текст проходит без изменений.
        let out = normalize_signal_value(
            &schema,
            &serde_json::json!({ "e1": "  пользователь называл себя обязанным всем помогать" }),
        );
        assert_eq!(
            out["e1"],
            serde_json::json!("  пользователь называл себя обязанным всем помогать")
        );
        // «falsehood …» — латинское слово, не форма отказа.
        let out = normalize_signal_value(&schema, &serde_json::json!({ "e1": "falsehood фильма" }));
        assert_eq!(out["e1"], serde_json::json!("falsehood фильма"));
        // Не-union поле (target) не нормализуется, даже начинаясь с «false».
        let out = normalize_signal_value(
            &schema,
            &serde_json::json!({ "e1": "верлибр", "target": "false (…)" }),
        );
        assert_eq!(out["target"], serde_json::json!("false (…)"));
        // Литерал false не трогается; не-объект проходит насквозь.
        let out = normalize_signal_value(
            &schema,
            &serde_json::json!({ "e1": false, "target": "t" }),
        );
        assert_eq!(out["e1"], serde_json::json!(false));
        assert_eq!(
            normalize_signal_value(&schema, &serde_json::json!("скаляр")),
            serde_json::json!("скаляр")
        );
    }

    /// Грамматика боевого контракта валидатора (12 слотов, 05.11.2026):
    /// 9 think-слотов с вердиктами-текстами e1..e9 (union `string{1..200} |
    /// false` — потолок из maxLength схемы) + финальные `Floor0 type: `/`Floor1
    /// type: ` (enum из схемы) и `Target: ` (свободная строка, целевой якорь
    /// разбора); конверта с булями нет (Вариант B).
    #[test]
    fn validator_contract_builds_twelve_slot_grammar() {
        let path = schema_path();
        let dir = path.parent().expect("папка signals");
        let c = load_signal_contract(dir, "validator").expect("контракт валидатора");
        let grammar = build_signal_envelope_grammar(&c);

        // Дум-слоты с якорями и вердиктами-union.
        for i in 1..=9 {
            assert!(
                grammar.contains(&format!(r#"draft-{} ::= "Element {}: ""#, i, i)),
                "дум-слот {} потерян, grammar:\n{}",
                i,
                grammar
            );
            assert!(
                grammar.contains(&format!(r#""\"e{}\": " verdict-value-{}"#, i, i)),
                "вердикт e{} потерян, grammar:\n{}",
                i,
                grammar
            );
            assert!(
                grammar.contains(&format!(
                    r#"verdict-value-{i} ::= verdict-string-{i} | "false""#
                )),
                "альтернатива false у вердикта e{} потеряна, grammar:\n{}",
                i,
                grammar
            );
            assert!(
                grammar.contains(&format!(r#"verdict-string-{i} ::= "\"" string-char{{1,200}} "\"""#)),
                "потолок вердикт-строки e{} (maxLength схемы) потерян, grammar:\n{}",
                i,
                grammar
            );
        }
        // Финальные слоты без дум-тела: метка + ключ + enum/строка из схемы (SSOT).
        assert!(
            grammar.contains(r#"trail-0 ::= "Floor0 type: " "\"floor0_type\": " trail-0-value"#),
            "финальный слот Floor0 type потерян, grammar:\n{}",
            grammar
        );
        assert!(
            grammar.contains(r#"trail-1 ::= "Floor1 type: " "\"floor1_type\": " trail-1-value"#),
            "финальный слот Floor1 type потерян, grammar:\n{}",
            grammar
        );
        assert!(
            grammar.contains(r#"trail-2 ::= "Target: " "\"target\": " json-string"#),
            "финальный слот Target (свободная строка) потерян, grammar:\n{}",
            grammar
        );
        for v in 1..=9 {
            assert!(
                grammar.contains(&format!(r#""\"{}\"""#, v)),
                "enum-литерал {} отсутствует в грамматике финального слота, grammar:\n{}",
                v,
                grammar
            );
        }
        // Класс string-char — канонический блок плагина (raw «<» запрещён).
        assert!(
            grammar.contains("string-char ::= [^\"\\\\<] | escape"),
            "вердикт-строки обязаны держать класс канонических строковых правил, grammar:\n{}",
            grammar
        );
        // Конверта нет (Вариант B), границы размышления в дум-слотах есть.
        assert!(!grammar.contains("envelope-json"), "Вариант B не должен выпускать конверт, grammar:\n{}", grammar);
        assert!(grammar.contains("draft-1 ::=") && grammar.contains("<think>"), "дум-слоты обязаны нести think-тело, grammar:\n{}", grammar);

        // Числа слотов: всего 12, дум-слотов 9 (бюджет делится по дум-слотам).
        assert_eq!(draft_count_for_contract(&c.value_schema), 12);
        assert_eq!(think_slot_count(&c.value_schema), 9);
        assert_eq!(slot_anchor_labels(&c.value_schema, 12).map(|l| l.len()), Some(9));
    }

    /// Защита от продолжения идентификатора: `trueish`/`true_x`/`falsehood` —
    /// не булевы литералы и вердиктом служить не могут.
    #[test]
    fn find_bool_after_key_rejects_identifiers() {
        assert_eq!(find_bool_after_key("\"e1\": trueish", "e1"), None);
        assert_eq!(find_bool_after_key("\"e1\": true_x", "e1"), None);
        assert_eq!(find_bool_after_key("\"e1\": falsehood", "e1"), None);
        assert_eq!(find_bool_after_key("\"e1\": true", "e1"), Some(true));
        assert_eq!(find_bool_after_key("\"e1\": false", "e1"), Some(false));
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

    /// Гипотеза 3 (обновлена 01.11.2026, Вариант B): для e-контрактов «полной
    /// формой» стали слоты с вердиктами — конверт (tool/arguments/key) устранён,
    /// значение собирается кодом из слотов. Конверт остаётся только у не-e
    /// контрактов (enum/pattern/object-строки).
    #[test]
    fn hypothesis_full_envelope_structure() {
        let e_contract = SignalContract {
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
        let grammar = build_signal_envelope_grammar(&e_contract);

        // Слотовая форма: якорь → think → вердикт-токен; правила root/drafts-block/sp/bool.
        assert!(grammar.contains("drafts-block"), "должно быть правило drafts-block");
        assert!(grammar.contains(r#"draft-1 ::= "Element 1: ""#), "должен быть якорь слота 1");
        assert!(grammar.contains(r#""\"e1\": " bool"#), "должен быть вердикт-токен e1");
        assert!(
            grammar.lines().any(|l| l.trim_start().starts_with("bool ::=")),
            "должно быть определение bool"
        );
        // Конверта у e-контракта больше нет — канал «угадать всё в конце» закрыт.
        assert!(
            !grammar.contains("envelope-json") && !grammar.contains("emit_signal"),
            "у e-контракта конверт устранён: {}",
            grammar
        );

        // Не-e контракт (enum) по-прежнему идёт через полный конверт.
        let enum_contract = SignalContract {
            key: "focus_keeper".to_string(),
            value_schema: serde_json::json!({
                "type": "string",
                "enum": ["НУЖНА РУКОСТЬ", "РУКОСТЬ ИЗВЕСТНА"]
            }),
        };
        let enum_grammar = build_signal_envelope_grammar(&enum_contract);
        assert!(
            enum_grammar.contains("envelope-json") && enum_grammar.contains("emit_signal"),
            "у не-e контракта конверт обязан остаться: {}",
            enum_grammar
        );
    }

    /// Гипотеза 4: Enum-контракт — должен содержать альтернативы строк
    #[test]
    fn hypothesis_enum_contract_variants() {
        let c = SignalContract {
            key: "focus_keeper".to_string(),
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

    /// Гипотеза 5: Pattern-контрат стринг — должен содержать цифры 1-9
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