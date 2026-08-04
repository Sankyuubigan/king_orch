//! API движка llamacpp: статус, установка, обновление, удаление.
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
    pub message: String,
}

fn engine_dir(app: &AppHandle) -> PathBuf {
    let cfg = crate::infra::load_config(app);
    if let Some(p) = &cfg.llamacpp_dir {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let exe_dir = app
        .path()
        .executable_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    llamacpp_installer::default_dir(&exe_dir)
}

pub fn get_engine_dir(app: &AppHandle) -> PathBuf {
    engine_dir(app)
}

/// Добавляет папку движка в PATH текущего процесса (для загрузки DLL загрузчиком)
pub fn add_to_path(dir: &PathBuf) {
    let dir_str = dir.to_string_lossy().to_string();
    let path_env = std::env::var("PATH").unwrap_or_default();
    let already = path_env.split(';').any(|p| p == dir_str);
    if !already {
        std::env::set_var("PATH", format!("{};{}", dir_str, path_env));
    }
}

#[tauri::command]
pub fn get_engine_status(app: AppHandle) -> EngineStatus {
    let dir = engine_dir(&app);
    let meta = llamacpp_installer::installed_meta(&dir);
    let gpu = gpu_detector::detect_gpu();

    let message = if meta.is_some() {
        format!(
            "Установлен: {} (CUDA {})",
            meta.as_ref().map(|m| m.tag.as_str()).unwrap_or("?"),
            meta.as_ref().map(|m| m.cuda.as_str()).unwrap_or("?")
        )
    } else {
        gpu_detector::describe_gpu(&gpu)
    };

    EngineStatus {
        installed: meta.is_some(),
        tag: meta.as_ref().map(|m| m.tag.clone()),
        cuda: meta.as_ref().map(|m| m.cuda.clone()),
        path: dir.to_string_lossy().to_string(),
        has_nvidia: gpu.has_nvidia,
        requires_driver_update: gpu_detector::requires_driver_update(&gpu),
        cuda_major: gpu.cuda_major,
        cuda_minor: gpu.cuda_minor,
        gpu_name: gpu.gpu_name,
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

    if !gpu.has_nvidia {
        return Err("NVIDIA GPU не обнаружен. Движок CUDA установить нельзя — приложение работает в CPU-режиме.".to_string());
    }
    if gpu_detector::requires_driver_update(&gpu) {
        return Err(format!(
            "Ваш драйвер NVIDIA поддерживает только CUDA {}.{}. Для GPU-ускорения обновите драйвер (нужна версия >= 527.41, CUDA 12+). Пока приложение работает в CPU-режиме.",
            gpu.cuda_major, gpu.cuda_minor
        ));
    }

    let _meta = llamacpp_installer::install(&dir, &log_cb, &progress_cb).await?;
    add_to_path(&dir);
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
