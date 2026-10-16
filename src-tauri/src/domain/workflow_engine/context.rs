use crate::domain::orchestrator::prompt::sanitize_model_visible_text;
use crate::domain::orchestrator::RequestMedia;
use crate::infra::{llm_history, ChatMessage};
use std::collections::HashMap;
use std::sync::Arc;

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
    /// Заполняется ТОЛЬКО из pending_signal вызывающего (нативный emit_signal,
    /// сигнал сабагента, текстовый fallback — nodes.rs:518 / dispatch.rs:887).
    /// При создании контекста ПУСТОЙ: сигналы прошлых ходов из messages[] сюда
    /// не гидрируются (это не SSOT-заявление про messages, а про доставку).
    /// SSOT для SignalRouter/ConditionRouter и {{ signals }} шаблона.
    pub signals: HashMap<String, serde_json::Value>,
    /// Группы нод, уже выполненные склеенным вызовом в ЭТОМ ходе.
    ///
    /// Инвариант фьюжинга: группа исполняется один раз за ход (`groups::fused_group_for`).
    /// Повторный вход в ту же группу идёт по её детерминированным нодам — сигналы
    /// уже лежат в `signals`, второй вызов LLM дал бы тот же результат за те же деньги.
    pub executed_groups: std::collections::HashSet<String>,
    /// Режим исполнения, выбранный пользователем В ЧАТЕ для этой сессии.
    ///
    /// Перекрывает `strategy:` файла графа, но только когда пользователь сделал
    /// явный выбор (тумблер в чате). `None` = «не переопределять»: файл главный,
    /// и UI не имеет права молча увести разговор в другой режим.
    pub strategy_override: Option<super::groups::ExecutionStrategy>,
    /// Лог последней смены режима — чтобы вкладка «Логи» показала, что режим
    /// взят из чата, а не из файла (§2.2: UI не должен молчать о подмене).
    pub strategy_source: &'static str,
    pub image_candidates: String,
    pub request_media: Arc<RequestMedia>,
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
            executed_groups: std::collections::HashSet::new(),
            strategy_override: None,
            strategy_source: "файл графа",
            image_candidates: String::new(),
            request_media: Arc::new(RequestMedia::empty()),
        }
    }

    pub fn with_image_candidates(mut self, candidates: String) -> Self {
        self.image_candidates = candidates;
        self
    }

    pub fn with_request_media(mut self, rm: Arc<RequestMedia>) -> Self {
        self.request_media = rm;
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
