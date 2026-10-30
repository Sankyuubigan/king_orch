//! 🚪 Инфраструктурный слой — публичный контракт.
//! Доменный и API слои импортируют инфраструктуру ТОЛЬКО через этот фасад.
//!
//! ## Движок llama.cpp — переиспользуемый плагин (SSOT)
//! Вся движковая логика вынесена в `tauri-plugin-llama-engine` и доступна здесь
//! через glob: `crate::infra::llm::*`, `llamacpp_installer`, `process_util`,
//! `vram_estimate`, каталог моделей и т.д. Хост оставляет за собой полный
//! `AppConfig`, сессии, MCP, инструменты и пр.

pub mod config;
pub mod data_dir;
pub mod llm_backend;
pub mod event_bus;
pub mod messages;
pub mod session_manager;
pub mod storage;
pub mod mcp_client;
pub mod plugins;
pub mod bin_downloader;
pub mod tools;
pub mod permissions;
pub mod system_proxy;
pub mod network_diagnostics;

// Движок llama.cpp — переиспользуемый плагин (SSOT). Глоб-реэкспорт тенят
// явные `pub use`/`pub mod` хоста ниже (explicit item > glob import).
pub use tauri_plugin_llama_engine::engine::*;
// Замок image-бандла (F5, 29.10.26): домен собирает маркер `[АКТИВНЫЙ БОНДЛ]`
// из этого SSOT (image_models_catalog.json + app_config.json) только через
// эту дверь — прямых обращений домена к крейту плагина быть не должно.
pub use tauri_plugin_image_engine::engine::models_catalog::{
    bundle_by_name, default_bundle_entry, ImageBundleEntry, RefEditMode,
};
pub use tauri_plugin_image_engine::engine::{bundle_dir_early, load_image_config_early};

// Хелпер хоста (setup): папка движка `<exe>/llamacpp` (или из конфига).
pub use tauri_plugin_llama_engine::get_engine_dir;

// ─── Публичные типы ───
pub use config::{AppConfig, SamplingPresets, TabState};
pub use llm_backend::{
    CloudEndpoint, LlmEngine, is_cloud_cancelled_error, is_cloud_empty_stream_error,
    is_cloud_retryable_transport_error, is_cloud_timeout_error, is_cloud_truncated_error,
};
pub use messages::{
    attachment_line, attachment_lines, content_with_attachments, llm_history, message_phase,
    push_report, ChatAttachment, ChatMessage, SubCall, ToolCallInfo,
};
pub use session_manager::{ChatSession, SessionMeta};
pub use mcp_client::{McpClient, McpPool, SharedMcpClient};

// ─── Публичные функции ───
pub use config::{
    load_config, load_config_early, save_config, load_sampling_presets,
    set_engine_last_model, set_engine_dirs,
    find_agents_dir, find_mcp_servers_dir, find_coding_tests_dir,
};
pub use session_manager::{get_session, get_sessions, save_session, delete_session, rename_session, open_session_folder, reveal_path};
pub use permissions::{PermissionApprover, GrantDecision, global_approver, test_approver};
pub use tools::{Tool, ToolCtx, ToolError, WriteOutside, tool_schemas, execute_tool, all_tools};
pub use tools::media::{
    attach_saved_images_from_sub_calls, execute_image_tool, image_tool_schemas,
    ImageArtifactRegistry,
};
pub use tools::video::{
    attach_saved_videos_from_sub_calls, execute_video_tool, video_image_gate_error,
    video_tool_schemas,
};