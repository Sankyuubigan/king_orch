//! 🚪 API-слой — Tauri команды
//! main.rs регистрирует команды ТОЛЬКО через этот фасад

pub mod config;
pub mod sessions;
pub mod models;
pub mod agents;
pub mod chat;
pub mod graph;
pub mod test;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Глобальное состояние приложения
pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>,
}