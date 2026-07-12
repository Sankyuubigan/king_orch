//! 🚪 Доменный слой — публичный контракт
//! API-слой импортирует домен ТОЛЬКО через этот фасад.
//! Доменный слой НЕ зависит от Tauri — чистая бизнес-логика.

pub mod orchestrator;
pub mod parsers;
pub mod agent_manager;
pub mod workflow_engine;

// ─── Публичные типы ───
pub use agent_manager::AgentEntry;
pub use agent_manager::AgentProfile;

// ─── Публичные функции ───
pub use orchestrator::run_chat;
pub use orchestrator::prompt::build_system_prompt;
pub use agent_manager::load_agents;
pub use agent_manager::load_entry_points;
pub use workflow_engine::{find_workflow_by_stem, load_workflows, NodeType, WorkflowDef};