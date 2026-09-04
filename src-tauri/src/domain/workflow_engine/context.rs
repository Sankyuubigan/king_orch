use crate::infra::{ChatMessage, llm_history};
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
}

impl WorkflowContext {
    pub fn new(
        user_message: String,
        messages: Vec<ChatMessage>,
        history: Vec<ChatMessage>,
    ) -> Self {
        // Заполняем signal bus из messages (сигналы с предыдущих итераций)
        let mut signals = HashMap::new();
        for msg in &messages {
            if msg.msg_type == "signal" {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&msg.content) {
                    if let Some(obj) = val.as_object() {
                        for (k, v) in obj {
                            signals.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
        Self {
            user_message,
            node_outputs: HashMap::new(),
            messages,
            history,
            output_emitted: false,
            signals,
        }
    }

    /// Добавляет сигнал в bus (вызывается из dispatch.rs после emit_signal).
    pub fn insert_signal(&mut self, key: String, value: serde_json::Value) {
        self.signals.insert(key, value);
    }

    /// Разрешает шаблонные переменные вида `{{ nodes.X.output.Y }}` и `{{ user_message }}`
    pub fn resolve_template(&self, template: &str) -> String {
        let mut result = template.to_string();

        // {{ user_message }}
        result = result.replace("{{ user_message }}", &self.user_message);

        // {{ signals }} — JSON-объект из signal bus (ключ → значение)
        if result.contains("{{ signals }}") {
            let signals_json = serde_json::to_string(&self.signals).unwrap_or_else(|_| "{}".to_string());
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
                        "content": m.content
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
