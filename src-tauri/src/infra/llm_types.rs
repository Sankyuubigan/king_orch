//! Типы данных LLM и форматирование промптов

use serde::{Deserialize, Serialize};

use super::llm_gguf::extract_string_from_gguf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub tool_name: String,
    pub arguments: String,
    pub result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubCall {
    pub agent_name: String,
    pub prompt: String,
    pub response: String,
    pub time_sec: f32,
    pub tool_calls: Vec<ToolCallInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAttachment {
    pub file_name: String,
    pub mime_type: String,
    pub data_base64: String,
}

/// Лёгкий тип для промпта LLM — только role + content.
/// Используется временно при вызове generate_chat(), не сохраняется в сессию.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
}

/// Извлекает имя файла из полного пути модели (для поля model в ChatMessage).
pub fn extract_model_filename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_calls: Option<Vec<SubCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Добавляет отчёт агента в массив сообщений сессии.
/// Если `single_report == true`, предварительно удаляет все прошлые сообщения
/// того же автора — чтобы в сессии хранился только один (последний) отчёт агента
/// и не раздувался контекст.
pub fn push_report(messages: &mut Vec<ChatMessage>, msg: ChatMessage, single_report: bool) {
    if single_report {
        if let Some(author) = msg.author.clone() {
            messages.retain(|m| {
                // Сигналы — инфраструктурные маркеры, а не отчёты агента.
                // Их нельзя сворачивать single_report, иначе маршрутизаторы
                // (signal_router) теряют эмитнутые сигналы.
                if m.msg_type == "signal" {
                    return true;
                }
                m.author.as_deref() != Some(author.as_str())
            });
        }
    }
    messages.push(msg);
}

impl ChatMessage {
    pub fn llm_role(&self) -> &str {
        match (self.msg_type.as_str(), self.author.as_deref()) {
            ("message", Some("user")) => "user",
            ("message", Some("system")) => "system",
            ("message", Some(_)) => "assistant",
            ("message", None) => "user",
            ("thought", _) => "user",
            _ => "user",
        }
    }

    pub fn to_llm_message(&self) -> LlmMessage {
        LlmMessage {
            role: self.llm_role().to_string(),
            content: self.content.clone(),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum PromptFormat {
    Auto,
    ChatML,
    Gemma,
    Gemma4,
    Llama3,
}

impl PromptFormat {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "gemma" => PromptFormat::Gemma,
            "gemma4" | "gemma-4" => PromptFormat::Gemma4,
            "llama3" | "llama-3" => PromptFormat::Llama3,
            "chatml" => PromptFormat::ChatML,
            _ => PromptFormat::Auto,
        }
    }

    pub fn detect_from_path(path: &str) -> Self {
        let lower = path.to_lowercase();
        if lower.contains("gemma-4") || lower.contains("gemma4") { PromptFormat::Gemma4 }
        else if lower.contains("gemma") { PromptFormat::Gemma }
        else if lower.contains("llama-3") || lower.contains("llama3") { PromptFormat::Llama3 }
        else { PromptFormat::ChatML }
    }

    pub fn detect_from_gguf(path: &str) -> Self {
        if let Some(template) = extract_string_from_gguf(path, "tokenizer.chat_template") {
            if template.contains("<|im_start|>") { return PromptFormat::ChatML; }
            if template.contains("<|start_header_id|>") { return PromptFormat::Llama3; }
            if template.contains("<|turn>") || template.contains("<turn|>") { return PromptFormat::Gemma4; }
            if template.contains("<start_of_turn>") { return PromptFormat::Gemma; }
        }
        Self::detect_from_path(path)
    }

    pub fn format_messages_jinja(template: &str, messages: &[LlmMessage]) -> Option<String> {
        let mut env = minijinja::Environment::new();
        env.add_template("chat", template).ok()?;
        let tmpl = env.get_template("chat").ok()?;

        let mut msgs_val = Vec::new();
        for m in messages {
            msgs_val.push(minijinja::context! {
                role => m.role,
                content => m.content
            });
        }

        tmpl.render(minijinja::context! {
            messages => msgs_val,
            add_generation_prompt => true
        }).ok()
    }

    pub fn format_messages(&self, messages: &[LlmMessage]) -> String {
        let mut full_prompt = String::new();
        match self {
            PromptFormat::ChatML | PromptFormat::Auto => {
                for msg in messages {
                    full_prompt.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", msg.role, msg.content));
                }
                full_prompt.push_str("<|im_start|>assistant\n");
            },
            PromptFormat::Gemma => {
                let mut system_text = String::new();
                for msg in messages {
                    let role = &*msg.role;
                    if role == "system" {
                        if !system_text.is_empty() { system_text.push_str("\n\n"); }
                        system_text.push_str(&msg.content);
                        continue;
                    }
                    let content = if role == "user" && !system_text.is_empty() {
                        let combined = format!("{}\n\n{}", system_text, msg.content);
                        system_text.clear();
                        combined
                    } else {
                        msg.content.clone()
                    };
                    let out_role = if role == "assistant" { "model".to_string() } else { role.to_string() };
                    full_prompt.push_str(&format!("<start_of_turn>{}\n{}<end_of_turn>\n", out_role, content));
                }
                if !system_text.is_empty() {
                    full_prompt.push_str(&format!("<start_of_turn>user\n{}<end_of_turn>\n", system_text));
                }
                full_prompt.push_str("<start_of_turn>model\n");
            },
            PromptFormat::Gemma4 => {
                let mut system_text = String::new();
                for msg in messages {
                    let role = &*msg.role;
                    if role == "system" {
                        if !system_text.is_empty() { system_text.push_str("\n\n"); }
                        system_text.push_str(&msg.content);
                        continue;
                    }
                    let content = if role == "user" && !system_text.is_empty() {
                        let combined = format!("{}\n\n{}", system_text, msg.content);
                        system_text.clear();
                        combined
                    } else {
                        msg.content.clone()
                    };
                    let out_role = if role == "assistant" { "model".to_string() } else { role.to_string() };
                    full_prompt.push_str(&format!("<|turn>{}\n{}<turn|>\n", out_role, content));
                }
                if !system_text.is_empty() {
                    full_prompt.push_str(&format!("<|turn>user\n{}<turn|>\n", system_text));
                }
                full_prompt.push_str("<|turn>model\n");
            },
            PromptFormat::Llama3 => {
                for msg in messages {
                    full_prompt.push_str(&format!("<|start_header_id|>{}<|end_header_id|>\n\n{}<|eot_id|>", msg.role, msg.content));
                }
                full_prompt.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");
            }
        }
        full_prompt
    }

    pub fn get_stop_words(&self) -> Vec<&'static str> {
        match self {
            PromptFormat::ChatML | PromptFormat::Auto => vec!["<|im_end|>", "<|im_start|>"],
            PromptFormat::Gemma => vec!["<end_of_turn>", "<start_of_turn>", "<|turn|>"],
            PromptFormat::Gemma4 => vec!["<turn|>", "<|turn|>"],
            PromptFormat::Llama3 => vec!["<|eot_id|>", "<|start_header_id|>"],
        }
    }
}