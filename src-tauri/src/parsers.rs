use regex::Regex;

pub fn parse_tool_call(text: &str) -> Option<(String, String)> {
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            let json_str = &text[start..=end];
            let cleaned = json_str.replace('\n', "\\n").replace('\r', "");
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&cleaned) {
                if let Some(tool) = val.get("tool").and_then(|v| v.as_str()) {
                    let arg = val.get("arg").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    return Some((tool.to_string(), arg));
                }
            }
        }
    }
    None
}

pub fn parse_orchestrator_response(text: &str) -> Option<(f32, String, String)> {
    // 1. Попытка распарсить как идеальный JSON (если LLM справилась и заэкранировала кавычки)
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            let json_str = &text[start..=end];
            let cleaned = json_str.replace('\n', "\\n").replace('\r', "");
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&cleaned) {
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

    // 2. Резервный пуленепробиваемый парсер (если LLM сломала JSON неэкранированными кавычками внутри текста)
    
    // Ищем confidence
    let re_conf = Regex::new(r#""confidence_score"\s*:\s*([0-9.]+)"#).unwrap();
    let conf = re_conf.captures(text).and_then(|c| c[1].parse::<f32>().ok()).unwrap_or(1.0);

    // Ищем target
    let re_target = Regex::new(r#""target"\s*:\s*"([^"]+)""#).unwrap();
    let target = re_target.captures(text).map(|c| c[1].to_string()).unwrap_or_else(|| "user".to_string());

    // Ищем content: берем всё от начала ключа до последней кавычки перед закрывающей скобкой }
    let keys = ["\"task_or_response\"", "\"response\"", "\"task\""];
    for key in keys {
        if let Some(start_idx) = text.find(key) {
            let after_key = &text[start_idx + key.len()..];
            if let Some(colon_idx) = after_key.find(':') {
                let after_colon = &after_key[colon_idx + 1..];
                if let Some(quote_start) = after_colon.find('"') {
                    let val_str = &after_colon[quote_start + 1..];
                    
                    // Ищем последнюю кавычку в строке, которая идет перед }
                    if let Some(last_quote) = val_str.rfind('"') {
                        let content = val_str[..last_quote].to_string();
                        // Убираем возможные ручные экранирования, чтобы текст выглядел чисто
                        let final_content = content.replace("\\n", "\n").replace("\\\"", "\"");
                        return Some((conf, target, final_content));
                    }
                }
            }
        }
    }

    None
}