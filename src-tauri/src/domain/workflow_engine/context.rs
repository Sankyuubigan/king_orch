use crate::infra::ChatMessage;
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
        }
    }

    /// Разрешает шаблонные переменные вида `{{ nodes.X.output.Y }}` и `{{ user_message }}`
    pub fn resolve_template(&self, template: &str) -> String {
        let mut result = template.to_string();

        // {{ user_message }}
        result = result.replace("{{ user_message }}", &self.user_message);

        // {{ signals }} — JSON-массив signal-сообщений из сессии
        if result.contains("{{ signals }}") {
            let signals: Vec<&ChatMessage> = self.messages
                .iter()
                .filter(|m| m.msg_type == "signal")
                .collect();
            let signals_json = serde_json::to_string(&signals).unwrap_or_else(|_| "[]".to_string());
            result = result.replace("{{ signals }}", &signals_json);
        }

        // {{ messages }} — сериализованный JSON последних сообщений
        if result.contains("{{ messages }}") {
            let msg_json = serde_json::to_string(&self.messages).unwrap_or_else(|_| "[]".to_string());
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
