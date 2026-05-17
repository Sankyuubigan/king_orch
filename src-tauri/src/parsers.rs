fn extract_json_block(text: &str) -> Option<String> {
    // 1. Сначала ищем четко выделенный блок ```json ... ```
    if let Some(start) = text.find("```json") {
        let content_start = start + 7;
        if let Some(end) = text[content_start..].find("```") {
            return Some(text[content_start..content_start + end].trim().to_string());
        }
    }
    
    // 2. Если блока с тегами нет, пытаемся найти просто первую { и последнюю }
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return Some(text[start..=end].trim().to_string());
        }
    }
    
    None
}

// Возвращает: Имя инструмента, Аргументы, Мысль (thought)
pub fn parse_tool_call(text: &str) -> Option<(String, serde_json::Value, String)> {
    if let Some(json_str) = extract_json_block(text) {
        let parsed = serde_json::from_str::<serde_json::Value>(&json_str)
            .or_else(|_| {
                let cleaned = json_str.replace('\n', " ").replace('\r', "");
                serde_json::from_str::<serde_json::Value>(&cleaned)
            });

        if let Ok(val) = parsed {
            if let Some(tool) = val.get("tool").and_then(|v| v.as_str()) {
                let args = val.get("arguments")
                    .cloned()
                    .unwrap_or_else(|| {
                        val.get("arg").cloned().unwrap_or(serde_json::Value::Null)
                    });
                let thought = val.get("thought").and_then(|v| v.as_str()).unwrap_or("").to_string();
                return Some((tool.to_string(), args, thought));
            }
        }
    }
    None
}

// Возвращает: Confidence, Target, Content, Мысль (thought)
pub fn parse_orchestrator_response(text: &str) -> Option<(f32, String, String, String)> {
    if let Some(json_str) = extract_json_block(text) {
        let parsed = serde_json::from_str::<serde_json::Value>(&json_str)
            .or_else(|_| {
                let cleaned = json_str.replace('\n', " ").replace('\r', "");
                serde_json::from_str::<serde_json::Value>(&cleaned)
            });

        if let Ok(val) = parsed {
            let conf = val.get("confidence_score")
                .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                .unwrap_or(1.0) as f32;
            
            let target = val.get("target").and_then(|v| v.as_str()).unwrap_or("user").to_string();
            
            let content = val.get("task_or_response")
                .or_else(|| val.get("response"))
                .or_else(|| val.get("task"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            
            let thought = val.get("thought").and_then(|v| v.as_str()).unwrap_or("").to_string();

            // Проверяем, что это действительно вызов сабагента (есть target)
            if val.get("target").is_some() {
                return Some((conf, target, content, thought));
            }
        }
    }
    None
}

pub fn extract_state_update(text: &str) -> (Option<String>, String) {
    let start_tag = "<update_state>";
    let end_tag = "</update_state>";
    
    if let Some(start_idx) = text.find(start_tag) {
        if let Some(end_idx) = text.find(end_tag) {
            let state_content = text[start_idx + start_tag.len()..end_idx].trim().to_string();
            
            let mut clean_text = String::new();
            clean_text.push_str(&text[..start_idx]);
            clean_text.push_str(&text[end_idx + end_tag.len()..]);
            
            return (Some(state_content), clean_text.trim().to_string());
        }
    }
    (None, text.to_string())
}