//! 🚪 API-слой — Tauri команды
//! main.rs регистрирует команды ТОЛЬКО через этот фасад

pub mod config;
pub mod sessions;
pub mod models;
pub mod agents;
pub mod chat;
pub mod graph;
pub mod test;
pub mod version;
pub mod coding_test;
pub mod updater;
pub mod file_utils;
pub mod llamacpp;
pub mod permissions;
pub mod telemetry;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Глобальное состояние приложения
pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>,
}