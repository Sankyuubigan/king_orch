//! 🔌 Плагин-слои (Слой 4.5 по мотивам deepseek-harness): точки расширения
//! (hooks) в ядре агентского цикла, где внешние модули подмешивают поведение,
//! не правя оркестратор. Аналог middleware/патчей: фичи (логирование, модерация,
//! кастомные преобразования промпта/результатов) добавляются плагином, а не форком.
//!
//! Поддерживаются три точки:
//!   • `on_system_prompt` — мутация системного промпта агента перед генерацией;
//!   • `on_tool_result`   — мутация результата инструмента до показа модели;
//!   • `on_agent_finish`  — уведомление о завершении агента (успех/ошибка).
//!
//! По умолчанию зарегистрированных плагинов нет → все хуки pass-through,
//! поведение ядра не меняется. Зарегистрировать плагин: `global_plugins().register(arc)`.

use std::sync::{Arc, Mutex, OnceLock};

/// Трейт плагина. Переопределяй только нужные методы (остальные — no-op).
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str {
        "unnamed"
    }
    /// Мутирует системный промпт агента перед генерацией.
    fn on_system_prompt(&self, _agent_id: &str, _prompt: &mut String) {}
    /// Мутирует результат инструмента перед тем, как он попадёт модели.
    fn on_tool_result(&self, _agent_id: &str, _tool: &str, _result: &mut String) {}
    /// Вызывается при завершении агента (успех или ошибка).
    fn on_agent_finish(&self, _agent_id: &str, _final_response: &str) {}
}

/// Реестр плагинов с потокобезопасной регистрацией и вызовом хуков.
#[derive(Default)]
pub struct PluginManager {
    plugins: Mutex<Vec<Arc<dyn Plugin>>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Зарегистрировать плагин (вызывается один раз при инициализации приложения).
    pub fn register(&self, plugin: Arc<dyn Plugin>) {
        self.plugins.lock().unwrap().push(plugin);
    }

    pub fn on_system_prompt(&self, agent_id: &str, prompt: &mut String) {
        for p in self.plugins.lock().unwrap().iter() {
            p.on_system_prompt(agent_id, prompt);
        }
    }

    pub fn on_tool_result(&self, agent_id: &str, tool: &str, result: &mut String) {
        for p in self.plugins.lock().unwrap().iter() {
            p.on_tool_result(agent_id, tool, result);
        }
    }

    pub fn on_agent_finish(&self, agent_id: &str, final_response: &str) {
        for p in self.plugins.lock().unwrap().iter() {
            p.on_agent_finish(agent_id, final_response);
        }
    }
}

static MANAGER: OnceLock<Arc<PluginManager>> = OnceLock::new();

/// Глобальный реестр плагинов (лениво инициализируется один раз).
pub fn global_plugins() -> Arc<PluginManager> {
    MANAGER
        .get_or_init(|| Arc::new(PluginManager::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AppendPlugin {
        marker: String,
    }
    impl Plugin for AppendPlugin {
        fn name(&self) -> &str {
            "append"
        }
        fn on_system_prompt(&self, _id: &str, prompt: &mut String) {
            prompt.push_str(&self.marker);
        }
    }

    #[test]
    fn plugin_mutates_prompt_via_manager() {
        let mgr = PluginManager::new();
        mgr.register(Arc::new(AppendPlugin {
            marker: " [PLUGIN_OK]".to_string(),
        }));
        let mut p = "SYS".to_string();
        mgr.on_system_prompt("agent_x", &mut p);
        assert!(p.contains("PLUGIN_OK"), "плагин должен дописать маркер в промпт");
    }

    #[test]
    fn no_plugins_is_passthrough() {
        let mgr = PluginManager::new();
        let mut p = "SYS".to_string();
        mgr.on_system_prompt("agent_x", &mut p);
        assert_eq!(p, "SYS", "без плагинов промпт не меняется");
    }
}
