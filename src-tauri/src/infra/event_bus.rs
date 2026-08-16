//! 📡 In-process шина событий агентов.
//!
//! Позволяет модулям (оркестратор, MCP-серверы, будущий Tauri-слой) публиковать
//! и подписываться на события жизненного цикла агентов без жёсткой связности.
//! Аналог фронтенд-EventBus, но на бэкенде (Слой 4.2 по мотивам deepseek-harness:
//! агенты сообщают о статусе через шину, а не через возврат значения).
//!
//! Глобальный экземпляр доступен через [`global_bus()`]; подписка — через
//! [`EventBus::subscribe`]. Без подписчиков публикация бессмысленна, но безопасна.

use std::sync::{Arc, Mutex, OnceLock};

/// Событие жизненного цикла агента.
#[derive(Clone, Debug)]
pub enum AgentEvent {
    /// Агент начал выполнение.
    Spawned {
        agent: String,
        namespace: String,
    },
    /// Агент завершил выполнение (успешно или с ошибкой — см. поле error).
    Finished {
        agent: String,
        namespace: String,
        ms: u128,
        error: Option<String>,
    },
    /// Агент вызвал инструмент.
    ToolCall {
        agent: String,
        tool: String,
    },
}

type Listener = Arc<dyn Fn(&AgentEvent) + Send + Sync>;

/// Простая шина pub/sub на основе списка подписчиков.
#[derive(Default)]
pub struct EventBus {
    listeners: Mutex<Vec<Listener>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Добавить подписчика. Вызывается для каждого опубликованного события.
    pub fn subscribe(&self, listener: Listener) {
        self.listeners.lock().unwrap().push(listener);
    }

    /// Опубликовать событие всем подписчикам. Порядок доставки не гарантирован.
    pub fn publish(&self, event: AgentEvent) {
        for listener in self.listeners.lock().unwrap().iter() {
            listener(&event);
        }
    }
}

static GLOBAL_BUS: OnceLock<Arc<EventBus>> = OnceLock::new();

/// Глобальный экземпляр шины (лениво инициализируется один раз).
pub fn global_bus() -> Arc<EventBus> {
    GLOBAL_BUS
        .get_or_init(|| Arc::new(EventBus::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_to_all_subscribers() {
        let bus = EventBus::new();
        let received = Arc::new(Mutex::new(Vec::new()));
        let r1 = received.clone();
        bus.subscribe(Arc::new(move |e| {
            if let AgentEvent::Spawned { agent, .. } = e {
                r1.lock().unwrap().push(agent.clone());
            }
        }));
        let r2 = received.clone();
        bus.subscribe(Arc::new(move |e| {
            if let AgentEvent::Spawned { agent, .. } = e {
                r2.lock().unwrap().push(agent.clone());
            }
        }));

        bus.publish(AgentEvent::Spawned {
            agent: "therapist".to_string(),
            namespace: "main".to_string(),
        });

        let got = received.lock().unwrap();
        assert_eq!(got.len(), 2, "оба подписчика должны получить событие");
        assert!(got.iter().all(|a| a == "therapist"));
    }

    #[test]
    fn global_bus_is_singleton() {
        let a = global_bus();
        let b = global_bus();
        assert!(Arc::ptr_eq(&a, &b), "global_bus должен возвращать тот же экземпляр");
    }
}
