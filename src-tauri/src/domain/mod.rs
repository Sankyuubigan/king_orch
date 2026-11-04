//! 🚪 Доменный слой — публичный контракт
//! API-слой импортирует домен ТОЛЬКО через этот фасад.
//! Доменный слой НЕ зависит от Tauri — чистая бизнес-логика.

pub mod orchestrator;
pub mod parsers;
pub mod agent_manager;
pub mod catalog_cache;
pub mod signals;
pub mod workflow_engine;
pub mod coding_bench;
pub mod pipeline_test;
pub mod triage;

// ─── Публичные типы ───
pub use agent_manager::AgentEntry;

// ─── Публичные функции ───
pub use orchestrator::run_chat;
pub use orchestrator::build_worst_agent_prompt;
pub use orchestrator::RequestMedia;
pub use orchestrator::agent_consumes_image_catalog;
pub use orchestrator::agent_receives_bundle_marker;
pub use orchestrator::active_image_bundle_facts;
pub use orchestrator::active_image_bundle_marker;
pub use orchestrator::full_media_prompt_block;
pub use orchestrator::StreamMeta;
pub use orchestrator::prompt::build_system_prompt;
pub use orchestrator::builtin_tools;
pub use agent_manager::load_agents;
pub use agent_manager::load_entry_points;
pub use catalog_cache::{catalog_snapshot, invalidate_catalog_cache};
pub use workflow_engine::{
    analyze_workflow_fidelity, find_workflow_by_stem, load_workflows, resolve_entry,
    separate_top_level_fields, EdgeDef, FactsFile, GraphDiagnostic, GroupDef, NodeDef,
    WorkflowConfig, WorkflowDef,
};