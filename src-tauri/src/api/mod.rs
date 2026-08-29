//! 🚪 API-слой — Tauri команды
//! main.rs регистрирует команды ТОЛЬКО через этот фасад

pub mod config;
pub mod sessions;
pub mod models;
pub mod agents;
pub mod chat;
pub mod graph;
pub mod test;
pub mod coding_test;
pub mod file_utils;
pub mod llamacpp;
pub mod permissions;
pub mod telemetry;
pub mod translate;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Локальный захват ошибок/событий фронтенда.
/// Фронтенд дёргает эту команду из своих глобальных ловушек ошибок, чтобы
/// реальные JS-ошибки и зависания попадали в локальный лог (king_orch.log и
/// test/last_logs.txt), а не только в Aptabase (которая требует согласия юзера).
#[tauri::command]
pub fn log_frontend_event(level: String, msg: String) {
    crate::infra::startup_log::append(&level, &msg);
}

/// Глобальное состояние приложения
pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>,
}