//! 🚪 Инфраструктурный слой — публичный контракт.
//! Доменный и API слои импортируют инфраструктуру ТОЛЬКО через этот фасад.
//!
//! ## Движок llama.cpp — переиспользуемый плагин (SSOT)
//! Вся движковая логика вынесена в `tauri-plugin-llama-engine` и доступна здесь
//! через glob: `crate::infra::llm::*`, `llamacpp_installer`, `process_util`,
//! `vram_estimate`, каталог моделей и т.д. Хост оставляет за собой полный
//! `AppConfig`, сессии, MCP, инструменты и пр.

pub mod config;
pub mod event_bus;
pub mod session_manager;
pub mod mcp_client;
pub mod plugins;
pub mod bin_downloader;
pub mod tools;
pub mod permissions;
pub mod lsp;
pub mod updater_rollback;
pub mod system_proxy;
pub mod network_diagnostics;

// Движок llama.cpp — переиспользуемый плагин (SSOT). Глоб-реэкспорт тенят
// явные `pub use`/`pub mod` хоста ниже (explicit item > glob import).
pub use tauri_plugin_llama_engine::engine::*;

// Хелпер хоста (setup): папка движка `<exe>/llamacpp` (или из конфига).
pub use tauri_plugin_llama_engine::get_engine_dir;

// ─── Публичные типы ───
pub use config::{AppConfig, SamplingPresets, TabState};
pub use session_manager::{ChatSession, SessionMeta};
pub use mcp_client::{McpClient, McpPool, SharedMcpClient};

// ─── Публичные функции ───
pub use config::{
    load_config, load_config_early, save_config, load_sampling_presets,
    find_agents_dir, find_mcp_servers_dir, find_coding_tests_dir,
};
pub use session_manager::{get_session, get_sessions, save_session, delete_session, rename_session, open_session_folder};
pub use permissions::{PermissionApprover, GrantDecision, global_approver, test_approver};
pub use tools::{Tool, ToolCtx, ToolError, WriteOutside, tool_schemas, execute_tool, all_tools};