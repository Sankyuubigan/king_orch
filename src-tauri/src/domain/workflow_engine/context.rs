use crate::domain::orchestrator::prompt::sanitize_model_visible_text;
use crate::infra::{llm_history, ChatMessage};
use std::collections::HashMap;

/// Контекст выполнения workflow — передаётся между узлами
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WorkflowContext {
    /// Оригинальный запрос пользователя
    pub user_message: String,
    /// Выводы всех узлов (id узла → JSON значение)
    pub node_outputs: HashMap<String, serde_json::Value>,
    /// Все сообщения сессии
    pub messages: Vec<ChatMessage>,
    /// История сообщений (только от пользователя и ассистента)
    pub history: Vec<ChatMessage>,
    /// Флаг: финальный узел workflow уже сохранил результат как message
    pub output_emitted: bool,
    /// Явный signal bus: ключ сигнала → JSON значение.
    /// Заполняется из messages[] при создании контекста и при каждом emit_signal.
    /// SSOT для SignalRouter/ConditionRouter и {{ signals }} шаблона.
    pub signals: HashMap<String, serde_json::Value>,
    pub image_candidates: String,
}

impl WorkflowContext {
    pub fn new(
        user_message: String,
        messages: Vec<ChatMessage>,
        history: Vec<ChatMessage>,
    ) -> Self {
        Self {
            user_message,
            node_outputs: HashMap::new(),
            messages,
            history,
            output_emitted: false,
            signals: HashMap::new(),
            image_candidates: String::new(),
        }
    }

    pub fn with_image_candidates(mut self, candidates: String) -> Self {
        self.image_candidates = candidates;
        self
    }

    /// Добавляет сигнал в bus (вызывается из dispatch.rs после emit_signal).
    pub fn insert_signal(&mut self, key: String, value: serde_json::Value) {
        self.signals.insert(key, value);
    }

    /// Разрешает шаблонные переменные вида `{{ nodes.X.output.Y }}` и `{{ user_message }}`
    pub fn resolve_template(&self, template: &str) -> String {
        let mut result = template.to_string();

        // {{ user_message }}
        result = result.replace(
            "{{ user_message }}",
            &sanitize_model_visible_text(&self.user_message),
        );
        result = result.replace("{{ image_candidates }}", &self.image_candidates);

        // {{ signals }} — JSON-объект из signal bus (ключ → значение)
        if result.contains("{{ signals }}") {
            let signals_json =
                serde_json::to_string(&self.signals).unwrap_or_else(|_| "{}".to_string());
            result = result.replace("{{ signals }}", &signals_json);
        }

        // {{ messages }} — история сессии для LLM: только не-thought и не-signal сообщения
        // и только их content (без sub_calls — это UI-метаданные, а не переписка).
        if result.contains("{{ messages }}") {
            let history: Vec<serde_json::Value> = llm_history(&self.messages)
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "type": m.msg_type,
                        "author": m.author,
                        "content": sanitize_model_visible_text(&m.content)
                    })
                })
                .collect();
            let msg_json = serde_json::to_string(&history).unwrap_or_else(|_| "[]".to_string());
            result = result.replace("{{ messages }}", &msg_json);
        }

        // {{ nodes.X.output }} и {{ nodes.X.output.Y }}
        for (node_id, output) in &self.node_outputs {
            let placeholder = format!("{{{{ nodes.{}.output }}}}", node_id);
            let output_str = serde_json::to_string(output).unwrap_or_default();
            result = result.replace(&placeholder, &output_str);

            if let Some(obj) = output.as_object() {
                for (key, val) in obj {
                    let key_placeholder = format!("{{{{ nodes.{}.output.{} }}}}", node_id, key);
                    let val_str = match val {
                        serde_json::Value::String(s) => s.clone(),
                        _ => serde_json::to_string(val).unwrap_or_default(),
                    };
                    result = result.replace(&key_placeholder, &val_str);
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(content: &str) -> ChatMessage {
        ChatMessage {
            id: Some("msg_1".to_string()),
            msg_type: "message".to_string(),
            content: content.to_string(),
            sub_calls: None,
            author: Some("user".to_string()),
            model: None,
            time_sec: None,
            attachments: None,
            phase: None,
        }
    }

    #[test]
    fn templates_hide_filesystem_paths() {
        let context = WorkflowContext::new(
            "open D:\\private\\image.png".to_string(),
            vec![message("saved /home/user/image.png")],
            Vec::new(),
        );
        let resolved = context.resolve_template("{{ user_message }} {{ messages }}");
        assert!(!resolved.contains("D:\\private"));
        assert!(!resolved.contains("/home/user"));
        assert_eq!(resolved.matches("[filesystem path omitted]").count(), 2);
    }
}
