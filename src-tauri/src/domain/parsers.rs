use serde_json;

fn extract_json_block(text: &str) -> Option<String> {
    let text = clean_thought_tags(text);
    let mut first_fenced: Option<String> = None;
    let mut search_pos = 0;

    while let Some(start) = text[search_pos..].find("```json") {
        let abs_start = search_pos + start + 7;
        if let Some(end) = text[abs_start..].find("```") {
            let block = text[abs_start..abs_start + end].trim().to_string();
            if first_fenced.is_none() {
                first_fenced = Some(block.clone());
            }
            // Prefer blocks that contain a target or tool action
            if block.contains("\"target\"") || block.contains("\"tool\"") {
                return Some(block);
            }
            search_pos = abs_start + end + 3;
        } else {
            break;
        }
    }

    if let Some(block) = first_fenced {
        return Some(block);
    }

    // Не-fenced: собираем все кандидаты {…} с учётом вложенности (скобки внутри
    // JSON-строк не считаются) и берём ПОСЛЕДНИЙ валидный JSON. Итоговый JSON
    // модели идёт в конце ответа, а первая {…} из прозы/размышлений может быть
    // мусором (например, при парсинге склеенного continuation_raw + нового ответа).
    let mut candidates: Vec<(usize, usize)> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i;
            let mut depth = 0u32;
            let mut in_str = false;
            let mut esc = false;
            let mut closed_at: Option<usize> = None;
            let mut j = i;
            while j < bytes.len() {
                let b = bytes[j];
                if in_str {
                    if esc {
                        esc = false;
                    } else if b == b'\\' {
                        esc = true;
                    } else if b == b'"' {
                        in_str = false;
                    }
                } else {
                    match b {
                        b'"' => in_str = true,
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                closed_at = Some(j);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                j += 1;
            }
            match closed_at {
                Some(end) => {
                    candidates.push((start, end));
                    i = end + 1;
                    continue;
                }
                // Незакрытая скобка: дальше корректных кандидатов не будет
                None => break,
            }
        }
        i += 1;
    }

    for (start, end) in candidates.iter().rev() {
        let candidate = &text[*start..=*end];
        if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
            return Some(candidate.trim().to_string());
        }
    }
    None
}

pub fn is_valid_json_action(text: &str) -> bool {
    if let Some(json_str) = extract_json_block(text) {
        serde_json::from_str::<serde_json::Value>(&json_str)
            .or_else(|_| serde_json::from_str(&json_str.replace('\n', " ").replace('\r', "")))
            .is_ok()
    } else {
        false
    }
}

fn is_valid_tool_name(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    if lower.is_empty() { return false; }
    let invalid = ["none", "null", "n/a", "reply", "нет", "no", "nobody", "nothing", "undefined"];
    !invalid.contains(&lower.as_str())
}

pub fn extract_think_content(text: &str) -> Vec<String> {
    let mut thoughts = Vec::new();
    if let Ok(re) = regex::Regex::new(r"(?s)<think[^>]*>(.*?)</think\s*>") {
        for cap in re.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                let c = m.as_str().trim().to_string();
                if !c.is_empty() { thoughts.push(c); }
            }
        }
    }
    if thoughts.is_empty() {
        if let Ok(re) = regex::Regex::new(r"(?s)<think[^>]*>\s*(.+)$") {
            if let Some(cap) = re.captures(text) {
                if let Some(m) = cap.get(1) {
                    let c = m.as_str().trim().to_string();
                    if !c.is_empty() { thoughts.push(c); }
                }
            }
        }
    }
    thoughts
}

pub fn extract_thought_from_partial_json(text: &str) -> Option<String> {
    if let Some(json_str) = extract_json_block(text) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str)
            .or_else(|_| serde_json::from_str(&json_str.replace('\n', " ").replace('\r', ""))) 
        {
            if let Some(thought) = val.get("thought").and_then(|v| v.as_str()) {
                return Some(thought.to_string());
            }
        }
    }
    if let Ok(re) = regex::Regex::new(r#"(?is)"thought"\s*:\s*"(.*?)(?:"\s*(?:\}|,)|$)"#) {
        if let Some(cap) = re.captures(text) {
            if let Some(m) = cap.get(1) {
                let thought = decode_json_escapes(m.as_str());
                if !thought.trim().is_empty() { return Some(thought); }
            }
        }
    }
    None
}

pub fn clean_thought_tags(text: &str) -> String {
    let mut result = text.to_string();
    // Реальный формат Gemma: `<|channel>KIND\n` открывает канал, `<channel|>`
    // (или `</|channel>`) закрывает его и переключает на контент. KIND — до
    // первого `\n` или `<` (модель часто не ставит `>` после KIND).
    // Вырезаем весь канал целиком вместе с содержимым.
    if let Ok(re) = regex::Regex::new(r"(?s)<\|channel>[^\n<]*[\s\S]*?(?:<channel\|>|</\|channel>[^\n<]*>)") {
        result = re.replace_all(&result, "").to_string();
    }
    // Открывающий маркер без закрытия (в конце потока)
    result = result.replace("<|channel>", "");
    // Закрывающие маркеры (оба варианта)
    result = result.replace("<channel|>", "").replace("</|channel>", "");
    // Маркер поворота
    result = result.replace("<|turn>", "");
    if let Ok(re) = regex::Regex::new(r"(?s)<think[^>]*>.*?</think\s*>") {
        result = re.replace_all(&result, "").to_string();
    }
    if let Ok(re) = regex::Regex::new(r"(?s)<think[^>]*>.*$") {
        result = re.replace_all(&result, "").to_string();
    }
    if let Ok(re) = regex::Regex::new(r"<think\s*/>") {
        result = re.replace_all(&result, "").to_string();
    }
    result = result.replace("</start_of_turn>", "").replace("<start_of_turn>", "");
    result = result.replace("<audio|>", "").replace("<video|>", "").replace("<image|>", "");
    // Артефакт: слово "thought"/"json", оставшееся в начале после вырезки
    // тегов (модель иногда пишет его как обычный текст).
    if let Ok(re) = regex::Regex::new(r"(?i)^\s*thought\s*\n?") {
        result = re.replace(&result, "").to_string();
    }
    if let Ok(re) = regex::Regex::new(r"(?i)^\s*json\s*\n?") {
        result = re.replace(&result, "").to_string();
    }
    result.trim().to_string()
}

pub struct ParsedOrchestratorResponse {
    pub target: String,
    pub content: String,
    pub thought: String,
}

fn parse_tool_call_from_json(json_str: &str) -> Option<(String, serde_json::Value, String)> {
    let parsed = serde_json::from_str::<serde_json::Value>(json_str)
        .or_else(|_| serde_json::from_str(&json_str.replace('\n', " ").replace('\r', "")));
    if let Ok(val) = parsed {
        if let Some(tool) = val.get("tool").and_then(|v| v.as_str()) {
            if !is_valid_tool_name(tool) { return None; }
            let args = val
                .get("arguments")
                .cloned()
                .or_else(|| val.get("arg").cloned())
                .unwrap_or_else(|| {
                    if let Some(obj) = val.as_object() {
                        let mut m = obj.clone();
                        m.remove("tool");
                        m.remove("thought");
                        if m.is_empty() {
                            serde_json::Value::Null
                        } else {
                            serde_json::Value::Object(m)
                        }
                    } else {
                        serde_json::Value::Null
                    }
                });
            let thought = val.get("thought").and_then(|v| v.as_str()).unwrap_or("").to_string();
            return Some((tool.to_string(), args, thought));
        }
    } else {
        let tool_re = regex::Regex::new(r#"(?is)"tool"\s*:\s*"([^"]+)""#).ok()?;
        if let Some(tool_cap) = tool_re.captures(json_str) {
            let tool = tool_cap.get(1)?.as_str().to_string();
            if !is_valid_tool_name(&tool) { return None; }
            let args_re = regex::Regex::new(r#"(?is)"arguments"\s*:\s*(\{.*?\})"#).ok()?;
            let args_str = args_re.captures(json_str).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or("{}".to_string());
            let args = serde_json::from_str(&args_str).unwrap_or(serde_json::Value::Null);
            let thought_re = regex::Regex::new(r#"(?is)"thought"\s*:\s*"(.*?)"\s*(?:,|\})"#).ok()?;
            let thought_raw = thought_re.captures(json_str).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
            return Some((tool, args, decode_json_escapes(&thought_raw)));
        }
    }
    None
}

pub fn parse_tool_call(text: &str) -> Option<(String, serde_json::Value, String)> {
    if let Some(json_str) = extract_json_block(text) {
        return parse_tool_call_from_json(&json_str);
    }
    None
}

pub fn strip_tool_call(text: &str) -> (String, Option<(String, serde_json::Value, String)>) {
    if let Some(json_str) = extract_json_block(text) {
        if let Some(tool_info) = parse_tool_call_from_json(&json_str) {
            let cleaned = remove_fenced_json_blocks(text);
            // Если весь вывод модели — JSON-конверт инструмента (без окружающего
            // свободного текста), анализом считаем пустоту. Свободный текст агента в
            // этом случае живёт в поле `thought` конверта, и оркестратор (dispatch.rs)
            // корректно подхватит его как `final_response`. Это чинит баг, когда
            // сигнальный агент под грамматикой контракта отдавал голый конверт вместо
            // аналитического отчёта (см. docs/SIGNAL_CONTRACTS.md: свободный текст в
            // поле `thought`, грамматику отключать запрещено).
            let cleaned_trim = cleaned.trim();
            let analysis = if cleaned_trim.is_empty() || cleaned_trim == json_str.trim() {
                String::new()
            } else {
                cleaned
            };
            return (analysis, Some(tool_info));
        }
    }
    (text.to_string(), None)
}

fn remove_fenced_json_blocks(text: &str) -> String {
    if let Ok(re) = regex::Regex::new(r"(?s)```json\s*.*?```") {
        re.replace_all(text, "").to_string().trim().to_string()
    } else {
        text.to_string()
    }
}

pub fn parse_orchestrator_response(text: &str) -> Option<ParsedOrchestratorResponse> {
    if let Some(json_str) = extract_json_block(text) {
        let parsed = serde_json::from_str::<serde_json::Value>(&json_str)
            .or_else(|_| serde_json::from_str(&json_str.replace('\n', " ").replace('\r', "")));
        if let Ok(val) = parsed {
            if val.get("target").is_some() {
                let target = val.get("target").and_then(|v| v.as_str()).unwrap_or("user").to_string();
                let content = val.get("task_or_response").or_else(|| val.get("response"))
                    .or_else(|| val.get("task")).or_else(|| val.get("message")).or_else(|| val.get("content"))
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();
                let thought = val.get("thought").and_then(|v| v.as_str()).unwrap_or("").to_string();
                return Some(ParsedOrchestratorResponse { target, content, thought });
            }
        } else {
            let target_re = regex::Regex::new(r#"(?is)"target"\s*:\s*"([^"]+)""#).ok()?;
            if let Some(target_cap) = target_re.captures(&json_str) {
                let target = target_cap.get(1)?.as_str().to_string();
                let task_re = regex::Regex::new(r#"(?s)"task_or_response"\s*:\s*"(.*)"\s*(?:\}|,)"#).ok()?;
                let content_raw = task_re.captures(&json_str).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
                let content = decode_json_escapes(&content_raw);
                let thought_re = regex::Regex::new(r#"(?is)"thought"\s*:\s*"(.*?)"\s*(?:,|\})"#).ok()?;
                let thought_raw = thought_re.captures(&json_str).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
                let thought = decode_json_escapes(&thought_raw);
                return Some(ParsedOrchestratorResponse { target, content, thought });
            }
        }
    }
    None
}

pub fn has_incomplete_json_action(text: &str) -> bool {
    if let Some(json_str) = extract_json_block(text) {
        let parsed = serde_json::from_str::<serde_json::Value>(&json_str)
            .or_else(|_| serde_json::from_str(&json_str.replace('\n', " ").replace('\r', "")));
        if let Ok(val) = parsed {
            let has_thought = val.get("thought").is_some();
            let has_target = val.get("target").is_some();
            let has_tool = val.get("tool").is_some();
            return has_thought && !has_target && !has_tool;
        }
    }
    false
}

fn decode_json_escapes(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\\"", "\"").replace("\\t", "\t").replace("\\\\", "\\")
}

// ─── Разделение размышлений и ответа (thinking mode) ───
//
// Поддерживаем два формата «мышления вслух», которые модели используют до ответа:
//  1. Gemma4-каналы:   `<|channel>thought\n...<channel|>\n<|channel>response\n...`
//  2. Qwen/R1/Bonsai thinking-режим: маркер `思考`/`thinking` ... закрытие ` 响应`/` response`
//     (блок может быть незакрытым — генерация оборвалась лимитом токенов).

/// Контент Gemma4-канала `<|channel>KIND ... <channel|>` (или незакрытого).
fn extract_gemma_channel(raw: &str, kind: &str) -> Option<String> {
    let mut search = 0usize;
    while let Some(rel) = raw[search..].find("<|channel>") {
        let abs = search + rel;
        let after_open = &raw[abs + "<|channel>".len()..];
        let kind_end = after_open.find(['\n', '<']).unwrap_or(after_open.len());
        if after_open[..kind_end].trim() == kind {
            let content = &after_open[kind_end..];
            let end = content
                .find("<channel|>")
                .or_else(|| content.find("</|channel>"))
                .unwrap_or(content.len());
            let c = content[..end].trim().to_string();
            if !c.is_empty() {
                return Some(c);
            }
        }
        search = abs + "<|channel>".len();
    }
    None
}

/// Разделяет сырой ответ LLM на (размышления, финальный ответ).
/// Если маркеры размышлений не распознаны — всё считается ответом.
pub fn split_thinking_and_answer(raw: &str) -> (String, String) {
    if raw.contains("<|channel>") {
        let thinking = extract_gemma_channel(raw, "thought")
            .or_else(|| extract_gemma_channel(raw, "thinking"));
        let answer = extract_gemma_channel(raw, "response")
            .or_else(|| extract_gemma_channel(raw, "content"));
        if thinking.is_some() || answer.is_some() {
            return (thinking.unwrap_or_default(), answer.unwrap_or_default());
        }
    }

    // Самозакрывающийся `<think/>` — размышлений нет, всё после — ответ
    if let Some(pos) = raw.find("<think/>").or_else(|| raw.find("<think />")) {
        let after = raw[pos..].find('>').map(|i| pos + i + 1).unwrap_or(raw.len());
        return (String::new(), raw[after..].trim().to_string());
    }

    // `<think>...</think>` теги (или незакрытый `<think>` в конце генерации)
    if let Some(start) = raw.find("<think") {
        let after_tag = raw[start..]
            .find('>')
            .map(|i| start + i + 1)
            .unwrap_or(start + "<think>".len());
        if let Some(close_rel) = raw[after_tag..].find("</think") {
            let close_start = after_tag + close_rel;
            let close_end = raw[close_start..]
                .find('>')
                .map(|i| close_start + i + 1)
                .unwrap_or(raw.len());
            return (
                raw[after_tag..close_start].trim().to_string(),
                raw[close_end..].trim().to_string(),
            );
        }
        return (raw[after_tag..].trim().to_string(), String::new());
    }

    // Qwen/R1 thinking-режим: ответ начинается с маркера размышлений.
    // Закрывающий маркер опционален (модель могла оборваться в середине).
    const OPEN_ZH: &str = "\u{601d}\u{8003}"; // 思考
    const OPEN_EN: &str = "thinking";
    const CLOSE_ZH: &str = "\u{54cd}\u{5e94}"; // 响应
    const CLOSE_EN: &str = "response";

    let trimmed = raw.trim_start();
    let body_start = if trimmed.starts_with(OPEN_ZH) {
        OPEN_ZH.len()
    } else if trimmed.starts_with(OPEN_EN) {
        OPEN_EN.len()
    } else {
        return (String::new(), raw.to_string());
    };

    // Позиция в исходном тексте сразу после открывающего маркера
    let full_start = raw.len() - trimmed.len() + body_start;
    let body = &raw[full_start..];

    let close_zh = body.find(CLOSE_ZH);
    let close_en = body.find(CLOSE_EN);
    match (close_zh, close_en) {
        (Some(p), _) => {
            let thinking = body[..p].trim().to_string();
            let answer = body[p + CLOSE_ZH.len()..].trim().to_string();
            (thinking, answer)
        }
        (None, Some(p)) => {
            let thinking = body[..p].trim().to_string();
            let answer = body[p + CLOSE_EN.len()..].trim().to_string();
            (thinking, answer)
        }
        (None, None) => (body.trim().to_string(), String::new()),
    }
}

/// True, если генерация оборвалась ВНУТРИ размышлений и финального ответа ещё нет.
/// Используется, чтобы продолжить генерацию с места обрыва, а не генерировать ЗАНОВО.
pub fn is_thinking_truncated(raw: &str) -> bool {
    if raw.trim().is_empty() {
        return false;
    }
    let (thinking, answer) = split_thinking_and_answer(raw);
    if !thinking.trim().is_empty() && answer.trim().is_empty() {
        return true;
    }
    // Незакрытый `<think ... ` (без ` response`/`</think>`)
    if raw.contains("<think") && !raw.contains("</think") && !raw.contains(" thought") {
        return true;
    }
    false
}

/// Нужна ли «докачка» оборванной генерации: размышления не завершены, а причина
/// остановки — обрыв, а не патология (зацикливание) или отмена пользователем.
/// Решение принимается по СОДЕРЖИМОМУ ответа (незакрытый думатель), а не по
/// stop_reason: обрыв случается и по лимиту токенов, и по стоп-слову, и по EOS
/// в середине думателя. LOOP_DETECTED/CANCELLED докачку не запускают.
pub fn needs_cutoff_continuation(raw: &str, stop_reason: &str) -> bool {
    if matches!(stop_reason, "CANCELLED" | "LOOP_DETECTED") {
        return false;
    }
    is_thinking_truncated(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoff_continuation_unclosed_think_any_stop() {
        let raw = "<think\nразмышления оборваны";
        assert!(needs_cutoff_continuation(raw, "STOP_WORD"));
        assert!(needs_cutoff_continuation(raw, "MAX_TOKENS"));
        assert!(needs_cutoff_continuation(raw, "EOS"));
    }

    #[test]
    fn cutoff_continuation_closed_think_no_answer() {
        let raw = " 思考\nмысли без ответа\n 响应";
        assert!(needs_cutoff_continuation(raw, "STOP_WORD"));
    }

    #[test]
    fn cutoff_continuation_plain_and_empty() {
        assert!(!needs_cutoff_continuation("", "STOP_WORD"));
        assert!(!needs_cutoff_continuation("обычный ответ.", "STOP_WORD"));
        assert!(!needs_cutoff_continuation("<|channel>response\nответ", "STOP_WORD"));
        assert!(!needs_cutoff_continuation(" 思考\nмысли\n 响应\nИтоговый ответ.", "MAX_TOKENS"));
    }

    #[test]
    fn cutoff_continuation_not_for_pathology() {
        let raw = "<think\nразмышления";
        assert!(!needs_cutoff_continuation(raw, "LOOP_DETECTED"));
        assert!(!needs_cutoff_continuation(raw, "CANCELLED"));
    }

    #[test]
    fn split_qwen_thinking_closed() {
        let raw = " 思考\nПроанализирую задачу.\n 响应\nИтоговый ответ.";
        let (t, a) = split_thinking_and_answer(raw);
        assert_eq!(t, "Проанализирую задачу.");
        assert_eq!(a, "Итоговый ответ.");
    }

    #[test]
    fn split_qwen_thinking_english_markers() {
        let raw = " thinking\nSome reasoning here.\n response\nFinal answer.";
        let (t, a) = split_thinking_and_answer(raw);
        assert_eq!(t, "Some reasoning here.");
        assert_eq!(a, "Final answer.");
    }

    #[test]
    fn split_qwen_thinking_truncated() {
        let raw = " 思考\nРазмышления оборваны в середине предложе";
        let (t, a) = split_thinking_and_answer(raw);
        assert_eq!(a, "");
        assert!(!t.is_empty());
        assert!(is_thinking_truncated(raw));
    }

    #[test]
    fn split_plain_text_is_answer() {
        let raw = "Обычный ответ без размышлений.";
        let (t, a) = split_thinking_and_answer(raw);
        assert_eq!(t, "");
        assert_eq!(a, raw);
        assert!(!is_thinking_truncated(raw));
    }

    #[test]
    fn split_gemma4_channels() {
        let raw = "<|channel>thought\nвнутренние размышления<channel|>\n<|channel>response\nфинальный ответ";
        let (t, a) = split_thinking_and_answer(raw);
        assert_eq!(t, "внутренние размышления");
        assert_eq!(a, "финальный ответ");
    }

    #[test]
    fn split_gemma4_thought_only() {
        let raw = "<|channel>thought\nмысли без ответа";
        let (t, a) = split_thinking_and_answer(raw);
        assert_eq!(t, "мысли без ответа");
        assert_eq!(a, "");
        assert!(is_thinking_truncated(raw));
    }

    #[test]
    fn split_think_tag_closed() {
        let raw = "<think>размышления</think>\nОтвет после тега.";
        let (t, a) = split_thinking_and_answer(raw);
        assert_eq!(t, "размышления");
        assert_eq!(a, "Ответ после тега.");
    }

    #[test]
    fn is_agent_error_detects_prefix() {
        assert!(crate::domain::orchestrator::is_agent_error("⚠️ ОШИБКА_АГЕНТА: Агент 'x' не смог"));
        assert!(!crate::domain::orchestrator::is_agent_error("нормальный ответ агента"));
        assert!(!crate::domain::orchestrator::is_agent_error(""));
    }

    #[test]
    fn extract_json_prefers_last_valid_over_garbage_prefix() {
        // Мусорная {…} из прозы/размышлений в начале + реальный JSON в конце:
        // раньше first-{ … last-} склеивал оба и парсинг падал (баг emit_signal).
        let text = "thinking\nРазмышления {в фигурных скобках без смысла} и текст\n\
            {\"arguments\": {\"key\": \"soma_translator\", \"value\": \"РУКОСТЬ ИЗВЕСТНА\"}, \"tool\": \"emit_signal\"}";
        let blk = extract_json_block(text).expect("должен найти валидный JSON в конце");
        let val: serde_json::Value = serde_json::from_str(&blk).expect("найденный блок — валидный JSON");
        assert_eq!(val["tool"], "emit_signal");
        assert_eq!(val["arguments"]["key"], "soma_translator");
    }

    #[test]
    fn extract_json_handles_nested_braces_and_quotes() {
        // Скобки внутри JSON-строк и вложенные объекты должны учитываться.
        let text = "перед {\"a\": \"текст {скобка} внутри\", \"nested\": {\"b\": [1, 2]}} после";
        let blk = extract_json_block(text).expect("должен извлечь полный объект");
        let val: serde_json::Value = serde_json::from_str(&blk).expect("валидный JSON");
        assert_eq!(val["nested"]["b"][1], 2);
    }

    #[test]
    fn parse_tool_call_from_emit_signal_envelope() {
        let parsed = parse_tool_call("{\"arguments\": {\"key\": \"soma_translator\", \"value\": \"РУКОСТЬ ИЗВЕСТНА\"}, \"tool\": \"emit_signal\"}")
            .expect("конверт emit_signal распознаётся");
        assert_eq!(parsed.0, "emit_signal");
        assert_eq!(parsed.1["key"], "soma_translator");
    }

    #[test]
    fn parse_tool_call_accepts_flattened_emit_signal_envelope() {
        // qwen3.8 (и слабые модели) эмитят конверт БЕЗ обёртки arguments:
        // {"tool":"emit_signal","key":..,"value":..}. Парсер должен нормализовать.
        let parsed = parse_tool_call("{\"tool\": \"emit_signal\", \"key\": \"soma_translator\", \"value\": {\"handness\": \"левша\"}}")
            .expect("сплющенный конверт emit_signal распознаётся");
        assert_eq!(parsed.0, "emit_signal");
        assert_eq!(parsed.1["key"], "soma_translator");
        assert_eq!(parsed.1["value"]["handness"], "левша");
    }

    #[test]
    fn extract_json_returns_none_without_valid_json() {
        assert!(extract_json_block("просто текст {без json}").is_none());
        assert!(extract_json_block("{broken \"key\": }").is_none());
    }

    #[test]
    fn extract_json_fenced_preferred() {
        let text = "слова\n```json\n{\"tool\": \"write\", \"arguments\": {}}\n```\nконец {мусор}";
        let blk = extract_json_block(text).expect("fenced-блок приоритетнее");
        assert!(blk.contains("\"tool\""));
    }
}
