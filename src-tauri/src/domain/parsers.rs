use serde_json;

fn extract_json_block(text: &str) -> Option<String> {
    let text = clean_thought_tags(text);
    let mut first_block: Option<String> = None;
    let mut search_pos = 0;

    while let Some(start) = text[search_pos..].find("```json") {
        let abs_start = search_pos + start + 7;
        if let Some(end) = text[abs_start..].find("```") {
            let block = text[abs_start..abs_start + end].trim().to_string();
            if first_block.is_none() {
                first_block = Some(block.clone());
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

    if let Some(block) = first_block {
        return Some(block);
    }

    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return Some(text[start..=end].trim().to_string());
        }
    }
    None
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
    if let Ok(re) = regex::Regex::new(r"(?s)<\|channel>thought.*?<channel\|>") {
        result = re.replace_all(&result, "").to_string();
    }
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
    result.trim().to_string()
}

pub struct ParsedOrchestratorResponse {
    pub target: String,
    pub content: String,
    pub thought: String,
    pub namespace: Option<String>,
}

pub fn parse_tool_call(text: &str) -> Option<(String, serde_json::Value, String)> {
    if let Some(json_str) = extract_json_block(text) {
        let parsed = serde_json::from_str::<serde_json::Value>(&json_str)
            .or_else(|_| serde_json::from_str(&json_str.replace('\n', " ").replace('\r', "")));
        if let Ok(val) = parsed {
            if let Some(tool) = val.get("tool").and_then(|v| v.as_str()) {
                if !is_valid_tool_name(tool) { return None; }
                let args = val.get("arguments").cloned().unwrap_or_else(|| val.get("arg").cloned().unwrap_or(serde_json::Value::Null));
                let thought = val.get("thought").and_then(|v| v.as_str()).unwrap_or("").to_string();
                return Some((tool.to_string(), args, thought));
            }
        } else {
            let tool_re = regex::Regex::new(r#"(?is)"tool"\s*:\s*"([^"]+)""#).ok()?;
            if let Some(tool_cap) = tool_re.captures(&json_str) {
                let tool = tool_cap.get(1)?.as_str().to_string();
                if !is_valid_tool_name(&tool) { return None; }
                let args_re = regex::Regex::new(r#"(?is)"arguments"\s*:\s*(\{.*?\})"#).ok()?;
                let args_str = args_re.captures(&json_str).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or("{}".to_string());
                let args = serde_json::from_str(&args_str).unwrap_or(serde_json::Value::Null);
                let thought_re = regex::Regex::new(r#"(?is)"thought"\s*:\s*"(.*?)"\s*(?:,|\})"#).ok()?;
                let thought_raw = thought_re.captures(&json_str).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
                return Some((tool, args, decode_json_escapes(&thought_raw)));
            }
        }
    }
    None
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
                let namespace = val.get("namespace").and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty()).map(|s| s.to_string());
                return Some(ParsedOrchestratorResponse { target, content, thought, namespace });
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
                let ns_re = regex::Regex::new(r#"(?is)"namespace"\s*:\s*"([^"]+)""#).ok()?;
                let namespace = ns_re.captures(&json_str).and_then(|c| c.get(1)).map(|m| m.as_str().to_string());
                return Some(ParsedOrchestratorResponse { target, content, thought, namespace });
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