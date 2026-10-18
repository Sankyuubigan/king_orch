//! 🔒 Контракт «сообщение чата» — SSOT принадлежит ХОСТУ.
//!
//! Доменные типы сессии (`ChatMessage`, `SubCall`, `ToolCallInfo`) и её
//! списковые операции (`push_report`, `message_phase`, `llm_history`) раньше
//! жили в плагине `tauri-plugin-llama-engine` — доменное знание во внешнем
//! инфраструктурном крейте. С 18.10.2026 контракт определяется здесь:
//! плагин хранит только LLM-внутренности (`LlmMessage`, грамматики,
//! `GenerationResult`) и API-тип вложений `ChatAttachment` (вход
//! generate-вызовов движка), на который ссылается поле `attachments`.
//!
//! Сериализованная форма не изменилась ни в одном поле — сессии
//! (`app_data_dir/sessions/*.json`) совместимы со старыми.

use serde::{Deserialize, Serialize};

use tauri_plugin_llama_engine::engine::{ChatAttachment, LlmMessage};

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
    /// Размышления агента (Phase 1) — для отчёта сабагента. None у старых сессий.
    pub thinking: Option<String>,
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
    /// Длительность генерации ответа (секунды). Вычисляется на фронте
    /// и сохраняется в JSON сессии для отображения при повторном открытии.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_sec: Option<f64>,
    /// Вложения пользовательского сообщения (картинки/файлы). Хранятся в JSON
    /// сессии для отображения в чате; в промпт модели НЕ попадают (llm_history
    /// берёт только content). Текущий ход передаёт их отдельным аргументом.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<ChatAttachment>>,
    /// Фаза двухфазного вызова агента: `1` — свободные размышления (think),
    /// `2` — финальный ответ/сигнал. Отсутствие поля = однофазный/legacy режим.
    /// Нужна для раздельной инжекции `inject_thoughts` (фаза 1) и `inject_response`
    /// (фаза 2), а также чтобы `replace_report` не стирал размышления фазы 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<u8>,
    /// Живое имя автора для отображения фронтендом.
    ///
    /// `author` — МАШИННЫЙ ключ (id агента/группы): по нему фильтруют отчёты
    /// (`inject_response`), сигнальные правила и `replace_report`, поэтому
    /// загрязнять его текстом с пробелами нельзя. Ставится только когда
    /// отображаемое имя нельзя достать из каталога по `author` (например,
    /// у явной fused-группы). Фронт: `author_name` есть → показать его,
    /// иначе резолвить `author` через каталог агентов.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
}

/// Возвращает фазу сообщения (1 = размышления, 2 = ответ, None = legacy).
pub fn message_phase(msg: &ChatMessage) -> Option<u8> {
    msg.phase
        .or_else(|| if msg.msg_type == "signal" { Some(2) } else { None })
}

/// Добавляет отчёт агента в массив сообщений сессии.
/// Если `replace_report == true`, предварительно удаляет прошлые отчёты того же автора
/// (обе фазы прошлого вызова), оставляя только текущий полный отчёт и сигналы.
/// Сообщения фазы 1 текущего отчёта НЕ удаляются при сохранении фазы 2 —
/// иначе двухфазный разбор (размышления агента) пропадал бы из сессии.
///
/// `phase` — фаза сохраняемого сообщения (`Some(1)` = размышления, `Some(2)` = ответ).
/// Фаза 1 сохраняется с заменой прошлых отчётов, фаза 2 — только хвост собственной фазы.
pub fn push_report(messages: &mut Vec<ChatMessage>, msg: ChatMessage, replace_report: bool, phase: Option<u8>) {
    if replace_report {
        if let Some(author) = msg.author.clone() {
            messages.retain(|m| {
                // Сигналы — инфраструктурные маркеры, а не отчёты агента.
                // Их нельзя сворачивать replace_report, иначе маршрутизаторы
                // (signal_router) теряют эмитнутые сигналы.
                if m.msg_type == "signal" {
                    return true;
                }
                if m.author.as_deref() != Some(author.as_str()) {
                    return true;
                }
                // Сохраняя фазу 2, не вычищаем размышления (фаза 1) ТЕКУЩЕГО отчёта.
                // Иначе двухфазный агент теряет свой разбор из сессии.
                if phase == Some(2) && message_phase(m) == Some(1) {
                    return true;
                }
                false
            });
        }
    }
    messages.push(msg);
}

/// Единое правило «что такое история для LLM»: из сессии в промпт модели
/// попадают только не-thought и не-signal сообщения, и только их `content`
/// (без `sub_calls` — это UI-метаданные, а не переписка). Используется и при
/// инжекции истории агентам, и в шаблонах workflow (`{{ messages }}`).
///
/// Системные сообщения (`author == "system"`) НЕ попадают в промпт — они нужны
/// только юзеру (хранятся в JSON сессии) и лишь зря жрут контекст модели.
///
/// Сигналы (`msg_type == "signal"`) НЕ попадают в промпт — это артефакты данных,
/// а не сообщения диалога. Сигналы доступны через `{{ signals }}` шаблон
/// (WorkflowContext) или прямой фильтрацию `messages` по `msg_type`.
pub fn llm_history(messages: &[ChatMessage]) -> Vec<&ChatMessage> {
    messages
        .iter()
        .filter(|m| {
            m.msg_type != "thought"
                && m.msg_type != "signal"
                && m.author.as_deref() != Some("system")
        })
        .collect()
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
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, msg_type: &str, author: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: Some(id.to_string()),
            msg_type: msg_type.to_string(),
            content: content.to_string(),
            sub_calls: None,
            author: Some(author.to_string()),
            model: None,
            time_sec: None,
            attachments: None,
            phase: None,
            author_name: None,
        }
    }

    #[test]
    fn llm_history_skips_thoughts_and_signals() {
        let msgs = vec![
            msg("msg_0", "message", "user", "привет"),
            msg("msg_1", "thought", "агент", "внутренняя мысль"),
            msg("msg_2", "signal", "агент", "{\"key\": \"value\"}"),
            msg("msg_3", "message", "агент", "ответ"),
            msg("msg_4", "message", "system", "системное уведомление только для юзера"),
        ];
        let history = llm_history(&msgs);
        // message(user) + message(агент) = 2 сообщения
        assert_eq!(history.len(), 2);
        assert!(history.iter().all(|m| m.msg_type != "thought"));
        assert!(history.iter().all(|m| m.msg_type != "signal"));
        assert!(history.iter().all(|m| m.author.as_deref() != Some("system")));
    }

    #[test]
    fn llm_history_keeps_content_references_without_sub_calls() {
        let with_sub_calls = ChatMessage {
            sub_calls: Some(vec![SubCall {
                agent_name: "агент".to_string(),
                prompt: "полный системный промпт — не должен попадать в историю".to_string(),
                response: "ответ".to_string(),
                time_sec: 1.0,
                tool_calls: vec![],
                thinking: None,
            }]),
            time_sec: None,
            ..msg("msg_5", "message", "grounder", "содержимое")
        };
        let msgs = [with_sub_calls];
        let history = llm_history(&msgs);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "содержимое");
    }

    #[test]
    fn author_name_serializes_compactly_and_loads_legacy_sessions() {
        // С `author_name`: поле попадает в JSON; без — serde его вырезает.
        // Старые сессии (без поля) читаются благодаря `#[serde(default)]`.
        let with = ChatMessage { author_name: Some("Декомпозиция и синтез".into()), ..msg("m", "message", "group_1", "текст") };
        let v = serde_json::to_value(&with).unwrap();
        assert_eq!(v["author_name"], "Декомпозиция и синтез");
        let without = ChatMessage { author_name: None, ..msg("m2", "message", "decomposer", "текст") };
        let v2 = serde_json::to_value(&without).unwrap();
        assert!(v2.get("author_name").is_none(), "поле должно не сериализоваться, когда None");
        let legacy: ChatMessage = serde_json::from_str(r#"{"type":"message","content":"старое сообщение","author":"decomposer"}"#).unwrap();
        assert_eq!(legacy.author.as_deref(), Some("decomposer"));
        assert_eq!(legacy.author_name, None);
    }
}
