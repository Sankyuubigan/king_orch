// Tauri-команды coding-бенчмарка LLM (Кодинг тест моделей).
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::api::AppState;
use crate::domain;
use crate::infra;

#[derive(Serialize, Deserialize, Clone)]
pub struct ModelSelection {
    pub path: String,
    pub name: String,
}

/// Список наборов задач (с языком, категориями и числом задач) для UI.
#[tauri::command]
pub fn get_coding_bench_info(app: AppHandle) -> Result<Vec<domain::coding_bench::SuiteInfo>, String> {
    let tasks_dir = infra::find_coding_tests_dir(&app);
    domain::coding_bench::list_suites(&tasks_dir)
}

/// Запуск бенчмарка: выбранные модели × наборы задач.
/// Прогресс эмитится событиями coding_status / coding_progress, финал — coding_done.
#[tauri::command]
pub async fn run_coding_bench(
    app: AppHandle,
    state: State<'_, AppState>,
    config: infra::AppConfig,
    models: Vec<ModelSelection>,
    suites: Vec<String>,
    quick_per_suite: Option<usize>,
    vr_budget_mb: u64,
) -> Result<(), String> {
    let engine_dir = crate::api::llamacpp::get_engine_dir(&app);
    let tasks_dir = infra::find_coding_tests_dir(&app);
    let bins_dir = infra::bin_downloader::get_bins_dir(
        &app.path().app_data_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
    );

    let cancel_flag = state.cancel_flag.clone();
    let status_app = app.clone();
    let status_cb = move |msg: String, progress: u8| {
        let _ = status_app.emit("coding_status", msg);
        let _ = status_app.emit("coding_progress", progress);
    };
    let log_app = app.clone();
    let log_cb = move |msg: String| {
        let _ = log_app.emit("log", format!("[coding_bench] {}", msg));
    };

    let options = domain::coding_bench::CodingBenchOptions {
        engine_dir,
        bins_dir,
        tasks_dir,
        models: models
            .into_iter()
            .map(|m| domain::coding_bench::ModelToRun { path: m.path, name: m.name })
            .collect(),
        suites,
        quick_per_suite,
        vr_budget_mb,
        config: &config,
    };

    let report = domain::coding_bench::run_coding_bench(&options, cancel_flag, status_cb, log_cb)?;
    let _ = app.emit(
        "coding_done",
        serde_json::json!({
            "report_file": report.report_file,
            "artifacts_dir": report.artifacts_dir,
        }),
    );
    Ok(())
}