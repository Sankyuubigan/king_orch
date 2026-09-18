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

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Глобальное состояние приложения
pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>,
}