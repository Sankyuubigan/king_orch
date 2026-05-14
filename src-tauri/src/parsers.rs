pub fn parse_tool_call(text: &str) -> Option<(String, serde_json::Value)> {
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            let json_str = &text[start..=end];
            
            // Сначала пробуем распарсить как есть
            let parsed = serde_json::from_str::<serde_json::Value>(json_str)
                .or_else(|_| {
                    // Если не вышло (например, из-за неэкранированных переносов строк в значениях),
                    // заменяем переносы на пробелы, чтобы не ломать синтаксис JSON
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
                    return Some((tool.to_string(), args));
                }
            }
        }
    }
    None
}

// Парсит JSON вызова сабагента. Если текст не является валидным JSON, возвращает None.
pub fn parse_orchestrator_response(text: &str) -> Option<(f32, String, String)> {
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            let json_str = &text[start..=end];
            
            let parsed = serde_json::from_str::<serde_json::Value>(json_str)
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
                
                return Some((conf, target, content));
            }
        }
    }
    None
}

// Извлекает содержимое из тега <update_state> и возвращает (Новое_Состояние, Текст_Без_Тега)
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