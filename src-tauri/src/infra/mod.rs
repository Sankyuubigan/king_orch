//! 🚪 Инфраструктурный слой — публичный контракт
//! Доменный и API слои импортируют инфраструктуру ТОЛЬКО через этот фасад

pub mod config;
pub mod llm;
pub mod llm_types;
pub mod llm_gguf;
pub mod llm_multimodal;
pub mod session_manager;
pub mod mcp_client;
pub mod downloader;
pub mod bin_downloader;

// ─── Публичные типы ───
pub use config::{AppConfig, CatalogEntry, ModelParams};
pub use llm::{ChatMessage, ChatAttachment, LlamaEngine, SubCall, ToolCallInfo, push_report};
pub use session_manager::{ChatSession, SessionMeta};
pub use mcp_client::McpClient;

// ─── Публичные функции ───
pub use config::{load_config, save_config, load_catalog, auto_detect_mmproj, find_agents_dir, find_mcp_servers_dir};
pub use llm::{extract_f32_from_gguf, extract_u32_from_gguf};
pub use session_manager::{get_session, get_sessions, save_session, delete_session, rename_session, open_session_folder};