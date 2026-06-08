#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Тонкий бутстраппер — только подключает слои и запускает Tauri.
//! Вся логика изолирована в слоях: api, domain, infra.

mod api;
mod domain;
mod infra;

use api::AppState;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState {
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })
        .setup(|app| {
            let app_handle = app.handle();
            let sessions_dir = infra::session_manager::sessions_dir(&app_handle);
            infra::migration::migrate_all_sessions(&sessions_dir);
            api::chat::init_log_file();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::config::get_config,
            api::config::set_config_value,
            api::config::set_last_model,
            api::config::set_theme,
            api::config::set_prompt_format,
            api::agents::get_agents,
            api::sessions::get_sessions,
            api::sessions::load_session,
            api::sessions::save_session,
            api::sessions::delete_session,
            api::sessions::rename_session,
            api::sessions::open_session_folder,
            api::models::get_models_catalog,
            api::models::get_model_params,
            api::models::set_model_params,
            api::models::reset_model_params,
            api::models::add_model,
            api::models::set_mmproj_path,
            api::models::get_mmproj_path,
            api::chat::chat_request,
            api::chat::stop_processing,
            api::graph::get_workflow_graphs,
            api::test::run_iterative_test,
            api::test::read_test_file,
            api::test::write_test_results,
            infra::downloader::download_model,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}