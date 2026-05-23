fn extract_json_block(text: &str) -> Option<String> {
    let text = clean_thought_tags(text);
    
    if let Some(start) = text.find("```json") {
        let content_start = start + 7;
        if let Some(end) = text[content_start..].find("```") {
            return Some(text[content_start..content_start + end].trim().to_string());
        }
    }
    
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return Some(text[start..=end].trim().to_string());
        }
    }
    
    None
}

/// Очистка от технической разметки LLM (<|channel>thought...<channel|>, <think...>...</think...>)
pub fn clean_thought_tags(text: &str) -> String {
    let mut result = text.to_string();
    
    if let Ok(re) = regex::Regex::new(r"(?s)<\|channel\>thought.*?<channel\|>") {
        result = re.replace_all(&result, "").to_string();
    }
    
    if let Ok(re) = regex::Regex::new(r"(?s)<think[^>]*>.*?</think\s*>") {
        result = re.replace_all(&result, "").to_string();
    }
    
    if let Ok(re) = regex::Regex::new(r"<think\s*/>") {
        result = re.replace_all(&result, "").to_string();
    }
    
    result.trim().to_string()
}

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
        } else {
            let tool_re = regex::Regex::new(r#"(?is)"tool"\s*:\s*"([^"]+)""#).ok()?;
            if let Some(tool_cap) = tool_re.captures(&json_str) {
                let tool = tool_cap.get(1)?.as_str().to_string();
                
                let args_re = regex::Regex::new(r#"(?is)"arguments"\s*:\s*(\{.*?\})"#).ok()?;
                let args_str = args_re.captures(&json_str).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or("{}".to_string());
                let args = serde_json::from_str(&args_str).unwrap_or(serde_json::Value::Null);
                
                let thought_re = regex::Regex::new(r#"(?is)"thought"\s*:\s*"(.*?)"\s*(?:,|\})"#).ok()?;
                let thought_raw = thought_re.captures(&json_str).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
                let thought = decode_json_escapes(&thought_raw);

                return Some((tool, args, thought));
            }
        }
    }
    None
}

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

            if val.get("target").is_some() {
                return Some((conf, target, content, thought));
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

                return Some((1.0, target, content, thought));
            }
        }
    }
    None
}

fn decode_json_escapes(s: &str) -> String {
    s.replace("\\n", "\n")
     .replace("\\\"", "\"")
     .replace("\\t", "\t")
     .replace("\\\\", "\\")
}