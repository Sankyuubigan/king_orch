//! 🚪 API-слой — Tauri команды
//! main.rs регистрирует команды ТОЛЬКО через этот фасад

pub mod config;
pub mod sessions;
pub mod agents;
pub mod chat;
pub mod graph;
pub mod test;
pub mod coding_test;
pub mod file_utils;
pub mod permissions;
pub mod translate;
pub mod updater;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Глобальное состояние приложения
pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>,
    /// Счётчики обращений к облачной LLM: `session_id` → число выполненных
    /// HTTP-запросов к роутеру за эту сессию (сумма за весь чат, а не за прогон).
    ///
    /// Ключ — id сессии: у каждой чат-вкладки своя сессия, поэтому вкладки не
    /// мешают друг другу автоматически. Значение — общий `Arc<AtomicU64>`,
    /// который инкрементит сам `CloudEngine` (infra не знает про AppState и
    /// получает счётчик на руки), — двойного учёта быть не может.
    pub cloud_requests: Mutex<HashMap<String, Arc<AtomicU64>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            cancel_flag: Arc::new(AtomicBool::new(false)),
            cloud_requests: Mutex::new(HashMap::new()),
        }
    }

    /// Счётчик сессии (создаётся с нуля при первом обращении). Пустой id не
    /// считается: без сессии счётчик некуда привязать, и сумма «повисла» бы.
    pub fn cloud_counter(&self, session_id: &str) -> Arc<AtomicU64> {
        if session_id.trim().is_empty() {
            return Arc::new(AtomicU64::new(0));
        }
        let mut map = self.cloud_requests.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(session_id.to_string())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)))
            .clone()
    }

    /// Сколько запросов к облачной LLM израсходовано этой сессией.
    pub fn cloud_total(&self, session_id: &str) -> u64 {
        if session_id.trim().is_empty() {
            return 0;
        }
        let map = self.cloud_requests.lock().unwrap_or_else(|e| e.into_inner());
        map.get(session_id)
            .map(|counter| counter.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Поднять счётчик до значения из файла сессии (после перезапуска
    /// приложения). Только вверх: иначе открытый старый чат откатил бы счётчик
    /// и «съел» бы уже оплаченные запросы.
    pub fn prime_cloud_total(&self, session_id: &str, persisted: u64) -> u64 {
        let counter = self.cloud_counter(session_id);
        counter.fetch_max(persisted, Ordering::Relaxed);
        counter.load(Ordering::Relaxed)
    }

    /// Забыть счётчик удалённой сессии — иначе записи копятся в памяти.
    pub fn forget_cloud_counter(&self, session_id: &str) {
        let mut map = self.cloud_requests.lock().unwrap_or_else(|e| e.into_inner());
        if map.remove(session_id).is_some() {
            log::info!("[cloud] Счётчик запросов сессии {} удалён", session_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        AppState::new()
    }

    #[test]
    fn cloud_counter_accumulates_per_session() {
        let state = state();
        let a = state.cloud_counter("s1");
        a.fetch_add(3, Ordering::Relaxed);
        a.fetch_add(1, Ordering::Relaxed);
        let b = state.cloud_counter("s2");
        b.fetch_add(7, Ordering::Relaxed);

        assert_eq!(state.cloud_total("s1"), 4);
        assert_eq!(state.cloud_total("s2"), 7);
    }

    #[test]
    fn cloud_counter_is_shared_between_acquisitions() {
        let state = state();
        let first = state.cloud_counter("s1");
        let second = state.cloud_counter("s1");
        first.fetch_add(2, Ordering::Relaxed);
        assert_eq!(second.load(Ordering::Relaxed), 2);
        assert_eq!(state.cloud_total("s1"), 2);
    }

    #[test]
    fn empty_session_id_is_not_counted() {
        let state = state();
        let counter = state.cloud_counter("");
        counter.fetch_add(5, Ordering::Relaxed);
        assert_eq!(state.cloud_total(""), 0);
        assert_eq!(state.cloud_total("missing"), 0);
    }

    #[test]
    fn prime_never_lowers_the_counter() {
        let state = state();
        state.cloud_counter("s1").fetch_add(9, Ordering::Relaxed);
        assert_eq!(state.prime_cloud_total("s1", 4), 9);
        assert_eq!(state.prime_cloud_total("s1", 40), 40);
    }

    #[test]
    fn forget_drops_the_session_counter() {
        let state = state();
        state.cloud_counter("s1").fetch_add(3, Ordering::Relaxed);
        state.forget_cloud_counter("s1");
        assert_eq!(state.cloud_total("s1"), 0);
    }
}