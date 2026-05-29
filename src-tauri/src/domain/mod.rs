//! 🚪 Доменный слой — публичный контракт
//! API-слой импортирует домен ТОЛЬКО через этот фасад.
//! Доменный слой НЕ зависит от Tauri — чистая бизнес-логика.

pub mod orchestrator;
pub mod parsers;
pub mod agent_manager;

// ─── Публичные типы ───
pub use agent_manager::AgentProfile;

// ─── Публичные функции ───
pub use orchestrator::run_chat;
pub use agent_manager::load_agents;