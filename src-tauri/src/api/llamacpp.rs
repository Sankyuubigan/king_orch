//! API движка llama.cpp: статус, установка, обновление, удаление.
//! Новая архитектура: движок — отдельный процесс `llama-server.exe` (полный релиз),
//! инференс ТОЛЬКО через него. Приложение не линкует llama.cpp нативно.
//! Весь прогресс дублируется в событие "log" (вкладка «Логи»).

use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

use crate::infra::{gpu_detector, llamacpp_installer};

#[derive(Serialize, Clone)]
pub struct EngineStatus {
    pub installed: bool,
    pub tag: Option<String>,
    pub cuda: Option<String>,
    pub path: String,
    pub has_nvidia: bool,
    pub requires_driver_update: bool,
    pub cuda_major: u32,
    pub cuda_minor: u32,
    pub gpu_name: String,
    /// Compute capability вида "12.0" (пусто, если не определена)
    pub compute_cap: String,
    /// Какой вариант движка нужен этой машине: "cuda-12.4" / "cuda-13.3" / "cpu"
    pub required_variant: String,
    pub message: String,
}

fn engine_dir(app: &AppHandle) -> PathBuf {
    let cfg = crate::infra::load_config(app);
    if let Some(p) = &cfg.llamacpp_dir {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let exe_dir = match std::env::current_exe() {
        Ok(p) => p
            .parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".")),
        Err(_) => app
            .path()
            .executable_dir()
            .unwrap_or_else(|_| PathBuf::from(".")),
    };
    llamacpp_installer::default_dir(&exe_dir)
}

pub fn get_engine_dir(app: &AppHandle) -> PathBuf {
    engine_dir(app)
}

#[tauri::command]
pub fn get_engine_status(app: AppHandle) -> EngineStatus {
    let dir = engine_dir(&app);
    let meta = llamacpp_installer::installed_meta(&dir);
    let gpu = gpu_detector::detect_gpu();

    let compute_cap = if gpu.compute_major > 0 {
        format!("{}.{}", gpu.compute_major, gpu.compute_minor)
    } else {
        String::new()
    };
    let required_variant = llamacpp_installer::select_variant();

    let message = if let Some(m) = &meta {
        format!("Установлен: {} (вариант: {})", m.tag, m.variant)
    } else if gpu.has_nvidia {
        "Движок llama.cpp не установлен — инференс недоступен. Установите движок ниже.".to_string()
    } else {
        gpu_detector::describe_gpu(&gpu)
    };

    EngineStatus {
        installed: meta.is_some(),
        tag: meta.as_ref().map(|m| m.tag.clone()),
        cuda: meta.as_ref().map(|m| m.variant.clone()),
        path: dir.to_string_lossy().to_string(),
        has_nvidia: gpu.has_nvidia,
        requires_driver_update: gpu_detector::requires_driver_update(&gpu),
        cuda_major: gpu.cuda_major,
        cuda_minor: gpu.cuda_minor,
        gpu_name: gpu.gpu_name,
        compute_cap,
        required_variant,
        message,
    }
}

#[tauri::command]
pub async fn install_llamacpp(app: AppHandle) -> Result<EngineStatus, String> {
    let app_log = app.clone();
    let log_cb = move |msg: String| {
        let _ = app_log.emit("log", &msg);
    };
    let app_prog = app.clone();
    let progress_cb = move |downloaded: u64, total: u64| {
        let _ = app_prog.emit(
            "engine_progress",
            serde_json::json!({ "downloaded": downloaded, "total": total }),
        );
    };

    let dir = engine_dir(&app);

    let gpu = gpu_detector::detect_gpu();
    log_cb(gpu_detector::describe_gpu(&gpu));
    // Вариант движка (cpu / cuda-12.4) выбирается автоматически внутри инсталлера.
    let variant = llamacpp_installer::select_variant();
    log_cb(format!("Вариант движка: {}", variant));

    let _meta = llamacpp_installer::install(&dir, &variant, &log_cb, &progress_cb).await?;
    log_cb(format!("📂 Папка движка: {}", dir.display()));

    Ok(get_engine_status(app))
}

#[tauri::command]
pub async fn check_engine_update(app: AppHandle) -> Result<Option<String>, String> {
    let dir = engine_dir(&app);
    let app_log = app.clone();
    let log_cb = move |msg: String| {
        let _ = app_log.emit("log", &msg);
    };
    llamacpp_installer::check_update(&dir, &log_cb).await
}

/// Обновление = переустановка с той же папкой (старая версия удаляется после успеха)
#[tauri::command]
pub async fn install_engine_update(app: AppHandle) -> Result<EngineStatus, String> {
    install_llamacpp(app).await
}

#[tauri::command]
pub fn remove_engine(app: AppHandle) -> Result<EngineStatus, String> {
    let dir = engine_dir(&app);
    let app_log = app.clone();
    let log_cb = move |msg: String| {
        let _ = app_log.emit("log", &msg);
    };
    llamacpp_installer::remove(&dir, &log_cb)?;
    Ok(get_engine_status(app))
}

#[tauri::command]
pub fn set_engine_dir(app: AppHandle, path: String) -> Result<EngineStatus, String> {
    let mut cfg = crate::infra::load_config(&app);
    cfg.llamacpp_dir = Some(path);
    crate::infra::save_config(&app, &cfg);
    Ok(get_engine_status(app))
}
