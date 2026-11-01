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
/// Класс символов `[^<]` — С переносами строк (решение владельца 31.10.2026):
/// черновики по элементам модель пишет списками («Элемент 3: … → false»), и
/// запрет `\n` вынуждал её в одну длинную строку, ухудшая читаемость отчёта.
/// Перенос не несёт риска выхода за границу — счётчик квантификатора считает
/// его как обычный символ; `<` исключён по-прежнему, иначе модель ушла бы в
/// `<think>` внутри `<think>`.
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

    // Свой черновик на каждый обязательный элемент контракта; минимум ОДИН —
    // слот отчёта обязан существовать у любого контракта (инвариант
    // draft_count_for_contract): enum/строковые/pattern-схемы не имеют
    // `required`, и draft{0} структурно убивал отчёт агента — модель шла сразу
    // в конверт (кейс soma_translator, tasks/27.10.26 draft0).
    let drafts = draft_count_for_contract(schema);
    let draft_rule = match think_budget_chars {
        None => "draft ::= \"<think>\" [^<]* \"</think>\"".to_string(),
        Some(budget) => format!("draft ::= \"<think>\" [^<]{{0,{budget}}} \"</think>\""),
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

    // ── Вариант B (слотовые вердикты): e-контракт с булевыми required-полями ──
    // Каждый слот ЗАКАНЧИВАЕТСЯ собственным вердиктом: `"Element k: " <think
    // тело </think> "e<k>": true|false`. Вердикт генерируется СРАЗУ после
    // мысли о своём элементе и обусловлен ею (генерация слева направо);
    // конверт с булями не генерируется вовсе — канал «заполнить слоты
    // декоративно и угадать всё в конце» закрыт грамматически, а значение
    // сигнала собирается из слотов единственным парсером
    // ([`assemble_slot_signal_value`], SSOT: один источник вердикта).
    // Несовпадение пар с e-видом (невозможно при пройденном
    // is_slot_verdict_contract) тихо падает в обычный конвертный путь ниже.
    if let Some(pairs) = slot_verdict_pairs(schema, drafts)
        .filter(|_| is_slot_verdict_contract(schema))
    {
        let body = &draft_rule["draft ::= ".len()..];
        let mut rule = String::from("root ::= drafts-block\ndrafts-block ::=");
        for k in 1..=drafts {
            if k > 1 {
                rule.push_str(" sp");
            }
            rule.push_str(&format!(" draft-{}", k));
        }
        for ((label, key), idx) in pairs.iter().zip(1..=drafts) {
            // Литерал вердикта собран так, чтобы кавычки внутри него остались
            // экранированными В ГРАММАТИКЕ (форма `\"e1\": `), — как и все
            // кавычки-литералы этого файла.
            rule.push_str(&format!(
                "\ndraft-{} ::= \"{}\" {} \"\\\"{}\\\": \" bool",
                idx, label, body, key
            ));
        }
        rule.push_str(&format!("\n{}\n{}", sp, "bool ::= \"true\" | \"false\""));
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
/// Enum/строковые/pattern-схемы (`soma_translator`, `focus_keeper`,
/// `destructor_detector`) не имеют массива `required` — раньше это давало
/// `draft{0}`: грамматика не оставляла выхода для анализа, модель шла сразу
/// в конверт, отчёт агента был пуст (регрессия 93e30c7, 13.10.26).
/// Object-контракты с N обязательными полями сохраняют N черновиков —
/// это существующее поведение валидатора и его слоёв.
pub fn draft_count_for_contract(schema: &serde_json::Value) -> usize {
    required_field_count(schema).max(1)
}

/// Метки якорей слотов отчёта (решение владельца 01.11.2026).
///
/// Возвращает `Some(метки)` только если ВСЕ обязательные поля контракта —
/// элементы вида `e<N>`: тогда слот k обязан начаться с `Element <N>: `,
/// где N — номер из имени поля (слои воронки наследуют те же якоря: у
/// `validator_surface` required = e1,e2,e6,e7 → слоты Element 1/2/6/7).
/// Поля другого вида (`idol`, enum-строки, pattern) — прежний свободный
/// черновик (`None`): им текстуальный якорь не соответствует по смыслу.
///
/// Пустой список невозможен: у object-контракта без `required` якорей нет
/// (`None`), а draft_count_for_contract гарантирует слот ≥ 1 в свободном режиме.
pub fn slot_anchor_labels(schema: &serde_json::Value, drafts: usize) -> Option<Vec<String>> {
    slot_verdict_pairs(schema, drafts).map(|pairs| pairs.into_iter().map(|(l, _)| l).collect())
}

/// Пары (метка якоря, ключ вердикта) для e-контрактов — общая основа якорей
/// слотов и Варианта B (вердикт внутри слота). Пары существуют только если
/// ВСЕ обязательные поля — элементы вида `e<N>`; ключ вердикта слота k — само
/// имя k-го required-поля (SSOT: и якорь, и вердикт происходят из одного
/// списка `required`, дублей нет).
fn slot_verdict_pairs(schema: &serde_json::Value, drafts: usize) -> Option<Vec<(String, String)>> {
    let required = schema.get("required")?.as_array()?;
    if drafts == 0 || required.len() != drafts {
        return None;
    }
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(drafts);
    for field in required {
        let name = field.as_str()?;
        let digits: String = name.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() || digits.len() != name.chars().count().saturating_sub(1)
            || !name.starts_with('e')
        {
            return None;
        }
        pairs.push((format!("Element {}: ", digits), name.to_string()));
    }
    Some(pairs)
}

/// Является ли контракт контрактом **слотовых вердиктов** (Вариант B): все
/// обязательные поля — e-элементы типа boolean. Только у таких вердикты
/// генерируются ВНУТРИ слотов (сразу после дум-тела элемента), а конверт
/// булей не содержит вовсе — канал «угадать всё в конце» закрыт грамматикой.
pub fn is_slot_verdict_contract(schema: &serde_json::Value) -> bool {
    let Some(required) = schema.get("required").and_then(Value::as_array) else {
        return false;
    };
    if required.is_empty() {
        return false;
    }
    required.iter().all(|field| {
        let Some(name) = field.as_str() else { return false };
        name.starts_with('e')
            && schema
                .get("properties")
                .and_then(|p| p.get(name))
                .and_then(|ps| ps.get("type"))
                .and_then(Value::as_str)
                == Some("boolean")
    })
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
/// правило чаннель-глитча). Правится ТОЛЬКО первый класс `[^<]` с его
/// квантификатором, найденный по ASCII-якорю; всё, что в правиле ПОСЛЕ
/// квантификатора (закрывающий think-литерал), переносится как есть:
/// - `[^<]*` → `[^<]{M} [^<]*` — прогон + неограниченный хвост;
/// - `[^<]{0,B}` → `[^<]{M} [^<]{0,B−M}` — прогон + ужатый потолок.
///
/// Отказ от трансформации (правило возвращается как есть): `min_chars == 0`;
/// квантификатор не форма `*`/`{lo,hi}`; потолочная форма `{0,B}` с
/// `M >= B` — нет места под минимум (потолок < 24 симв.).
fn with_slot_min_body(draft_rule: &str, min_chars: usize) -> String {
    if min_chars == 0 {
        return draft_rule.to_string();
    }
    let Some(class_pos) = draft_rule.find("[^<]") else {
        return draft_rule.to_string();
    };
    let quant_start = class_pos + "[^<]".len();
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
        out.push_str("[^<]");
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
    with_min(&format!("[^<]{{0,{}}}{}", hi_num - min_chars, remainder))
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
/// Потолок черновика обязан укладываться в оба: иначе модель, заполнившая
/// черновики до потолка грамматики, структурно не доходит до конверта — тот
/// самый механический отказ, который граница должна была устранить.
/// `reasoning_budget = 0` — флаг не активен (не задан или отключён фолбэком
/// цикла запуска): считаем только от общего бюджета генерации, как стенды.
pub fn auto_think_budget_chars(
    max_gen_tokens: usize,
    reasoning_budget: usize,
    drafts: usize,
) -> Option<usize> {
    let gen_budget = max_gen_tokens.saturating_sub(ENVELOPE_TOKENS.min(max_gen_tokens));
    let think_tokens = if reasoning_budget == 0 {
        gen_budget
    } else {
        reasoning_budget.min(gen_budget)
    };
    let per_draft_tokens = think_tokens / drafts.max(1);
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

/// Вариант B: собирает значение e-контракта из **слотовых вердиктов** —
/// пар `"e<N>": true|false`, которыми заканчивается каждый слот
/// (см. [`is_slot_verdict_contract`]).
///
/// SSOT: слоты — единственный источник вердикта; конверта с булями для таких
/// контрактов не существует (грамматика его не выпускает). Для каждого
/// required-поля берётся ПЕРВОЕ вхождение `"ключ": true|false` в тексте;
/// грамматика гарантирует ровно одно вхождение на ключ (слоты идут по
/// элементам без повторов). Если хоть один ключ не найден — `None`: сигнал
/// не собран, и штатные ретраи формата работают как раньше.
pub fn assemble_slot_signal_value(schema: &Value, text: &str) -> Option<Value> {
    if !is_slot_verdict_contract(schema) {
        return None;
    }
    let required = schema.get("required")?.as_array()?;
    let mut obj = serde_json::Map::new();
    for field in required {
        let name = field.as_str()?;
        let b = find_bool_after_key(text, name)?;
        obj.insert(name.to_string(), Value::Bool(b));
    }
    Some(Value::Object(obj))
}

/// Первое `"ключ": true|false` в тексте после указанной позиции ключа.
/// Сканирует все вхождения литерала `"ключ"`, за каждым — двоеточие и
/// литерал буля; вхождения без буля после двоеточия пропускаются (модельный
/// текст мог упомянуть ключ в мысли), продолжение идентификатора после
/// `true`/`false` отбраковывается (защита от `trueish`).
fn find_bool_after_key(text: &str, key: &str) -> Option<bool> {
    let needle = format!("\"{}\"", key);
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find(&needle) {
        let abs = search_from + rel;
        let after = text[abs + needle.len()..].trim_start();
        if let Some(rest) = after.strip_prefix(':') {
            let v = rest.trim_start();
            for (lit, val) in [("true", true), ("false", false)] {
                if let Some(tail) = v.strip_prefix(lit) {
                    let ident_continues = tail
                        .chars()
                        .next()
                        .map(|c| c.is_alphanumeric() || c == '_')
                        .unwrap_or(false);
                    if !ident_continues {
                        return Some(val);
                    }
                }
            }
        }
        search_from = abs + needle.len();
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
        for id in ["validator", "focus_keeper", "soma_translator"] {
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
            bounded_draft.contains(&format!("[^<]{{{}}} [^<]{{0,{}}}", min, 1400 - min)),
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
            unbounded_draft.contains("[^<]*"),
            "неограниченный хвост потерян"
        );
        assert!(
            unbounded_draft.contains(&format!("[^<]{{{}}}", slot_min_chars(None))),
            "минимум содержания потерян в слоте без формального потолка: {}",
            unbounded_draft
        );
        // Закрывающий литерал слота обязан пережить трансформацию минимума
        // (дефект 01.11.2026: bounded-ветка теряла хвост после квантификатора —
        // think не закрывался, конверт уходил внутрь незакрытой мысли).
        // Литерал НЕ переписывается руками (чаннель-глитч): берётся срезом из
        // неограниченной строки по якорю класса `[^<]*`.
        let close_tail = unbounded_draft
            .split("[^<]*")
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
            draft.contains(&format!("[^<]{{{}}} [^<]{{0,{}}}", min, 300 - min)),
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
            unbounded_draft.contains("[^<]*"),
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
    /// Enum-схемы (soma_translator, focus_keeper) не имеют `required` — прежнее
    /// `draft{0}` убивало отчёт: грамматика не оставляла выхода для анализа,
    /// модель шла сразу в конверт (регрессия 93e30c7; tasks/27.10.26 draft0.md).
    #[test]
    fn enum_contract_gets_one_draft_slot() {
        let contract = SignalContract {
            key: "soma_translator".to_string(),
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
            grammar.contains(&format!("[^<]{{{}}} [^<]*", slot_min_chars(None))),
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
        let bounded = "x ::= [^<]{0,300}";
        assert_eq!(
            with_slot_min_body(bounded, 37),
            "x ::= [^<]{37} [^<]{0,263}"
        );
        // Нет места под минимум (M >= B): форма не тронута.
        assert_eq!(with_slot_min_body(bounded, 300), bounded);
        // Неограниченный хвост: минимум + прежний хвост.
        let star = "x ::= [^<]*";
        assert_eq!(with_slot_min_body(star, 24), "x ::= [^<]{24} [^<]*");
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
        for id in ["soma_translator", "focus_keeper"] {
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
            key: "soma_translator".to_string(),
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
        let enum_contract = load_signal_contract(&dir, "soma_translator").expect("контракт прочитан");
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
            "soma_translator",
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

    /// Слотовым контрактом считается ТОЛЬКО контракт, у которого ВСЕ
    /// обязательные поля — boolean-элементы `e<N>`. Конвертные формы
    /// (enum/pattern/строковые поля) не должны распознаваться как слотовые.
    #[test]
    fn slot_verdict_contract_detection() {
        let slot = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "e1": { "type": "boolean" }, "e2": { "type": "boolean" } },
            "required": ["e1", "e2"]
        });
        assert!(is_slot_verdict_contract(&slot));

        // Строковое поле в required — конвертный контракт.
        let mixed = serde_json::json!({
            "type": "object",
            "properties": { "e1": { "type": "boolean" }, "note": { "type": "string" } },
            "required": ["e1", "note"]
        });
        assert!(!is_slot_verdict_contract(&mixed));

        // Не-e поле — конвертный контракт.
        let not_e = serde_json::json!({
            "type": "object",
            "properties": { "field_a": { "type": "boolean" } },
            "required": ["field_a"]
        });
        assert!(!is_slot_verdict_contract(&not_e));

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
            key: "soma_translator".to_string(),
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