//! API движка llama.cpp: статус, установка, обновление, удаление, выбор бекенда.
//! Новая архитектура: движок — отдельный процесс `llama-server.exe` (полный релиз),
//! инференс ТОЛЬКО через него. Приложение не линкует llama.cpp нативно.
//! Несколько бекендов (cpu / cuda-12.4 / cuda-13.3 / vulkan / hip-radeon) могут
//! быть установлены одновременно в `backends/<variant>/` — переключение мгновенное.
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
    /// Какой вариант движка нужен этой машине по авто-подбору: "cuda-12.4" / "cuda-13.3" / "cpu"
    pub required_variant: String,
    /// Выбор юзера из конфига: "auto" или конкретный вариант
    pub selected_variant: String,
    /// Реально используемый вариант (auto → сработан по GPU)
    pub resolved_variant: String,
    /// Установленные на диске варианты
    pub installed_variants: Vec<String>,
    /// Все варианты для дропдауна (с подписями и статусом установки)
    pub available_variants: Vec<llamacpp_installer::VariantInfo>,
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

/// Выбор бекенда из конфига юзера ("auto" по умолчанию)
fn preferred_variant(app: &AppHandle) -> String {
    let cfg = crate::infra::load_config(app);
    cfg.engine_variant.clone().unwrap_or_else(|| llamacpp_installer::VARIANT_AUTO.to_string())
}

/// Плавная миграция старого формата движка (корень папки) → backends/<variant>/
fn ensure_migrated(app: &AppHandle) {
    let dir = engine_dir(app);
    if let Ok(Some(variant)) = llamacpp_installer::migrate_legacy_layout(&dir) {
        crate::infra::startup_log::append(
            "INFO",
            &format!("Миграция движка в новый формат завершена: backends/{}", variant),
        );
    }
}

#[tauri::command]
pub fn get_engine_status(app: AppHandle) -> EngineStatus {
    let dir = engine_dir(&app);
    ensure_migrated(&app);

    let gpu = gpu_detector::detect_gpu();
    let selected = preferred_variant(&app);
    let resolved = llamacpp_installer::resolve_variant(Some(&selected));
    let meta = llamacpp_installer::installed_meta(&dir, &resolved);
    let installed_variants = llamacpp_installer::list_installed_variants(&dir);
    let available = llamacpp_installer::available_variants(&dir);

    let compute_cap = if gpu.compute_major > 0 {
        format!("{}.{}", gpu.compute_major, gpu.compute_minor)
    } else {
        String::new()
    };
    let required_variant = llamacpp_installer::select_variant();

    let message = if let Some(m) = &meta {
        format!(
            "Установлен: {} (вариант: {})",
            m.tag,
            llamacpp_installer::variant_label(&m.variant)
        )
    } else if installed_variants.is_empty() {
        if gpu.has_nvidia {
            "Движок llama.cpp не установлен — инференс недоступен. Установите движок ниже.".to_string()
        } else {
            gpu_detector::describe_gpu(&gpu)
        }
    } else {
        format!(
            "Выбран вариант «{}», но он ещё не установлен (установлены: {}). Нажмите «Установить».",
            llamacpp_installer::variant_label(&resolved),
            installed_variants
                .iter()
                .map(|v| llamacpp_installer::variant_label(v))
                .collect::<Vec<_>>()
                .join(", ")
        )
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
        selected_variant: selected,
        resolved_variant: resolved,
        installed_variants,
        available_variants: available,
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
    ensure_migrated(&app);

    let gpu = gpu_detector::detect_gpu();
    log_cb(gpu_detector::describe_gpu(&gpu));
    // Вариант бекенда: выбор юзера из конфига, "auto" → подбор по GPU.
    let selected = preferred_variant(&app);
    let variant = llamacpp_installer::resolve_variant(Some(&selected));
    log_cb(format!(
        "Вариант бекенда: {} ({})",
        variant,
        llamacpp_installer::variant_label(&variant)
    ));

    let _meta = llamacpp_installer::install(&dir, &variant, &log_cb, &progress_cb).await?;
    log_cb(format!("📂 Папка движка: {}", dir.display()));

    Ok(get_engine_status(app))
}

/// Смена бекенда: сохраняет выбор юзера в конфиг; если вариант ещё не установлен —
/// скачивает его. Уже установленные варианты не трогаются.
#[tauri::command]
pub async fn set_engine_variant(app: AppHandle, variant: String) -> Result<EngineStatus, String> {
    let valid = variant == llamacpp_installer::VARIANT_AUTO
        || llamacpp_installer::is_known_variant(&variant);
    if !valid {
        return Err(format!("Неизвестный вариант бекенда: {}", variant));
    }

    let mut cfg = crate::infra::load_config(&app);
    cfg.engine_variant = if variant == llamacpp_installer::VARIANT_AUTO {
        None
    } else {
        Some(variant.clone())
    };
    crate::infra::save_config(&app, &cfg);

    let dir = engine_dir(&app);
    let resolved = llamacpp_installer::resolve_variant(Some(&variant));
    if !llamacpp_installer::is_installed(&dir, &resolved) {
        // Устанавливаем выбранный бекенд (с прогрессом в те же события)
        install_llamacpp(app.clone()).await?;
    } else {
        let app_log = app.clone();
        let log_cb = move |msg: String| {
            let _ = app_log.emit("log", &msg);
        };
        log_cb(format!(
            "⚙️ Выбран бекенд: {} (уже установлен — переключение мгновенное).",
            llamacpp_installer::variant_label(&resolved)
        ));
    }

    Ok(get_engine_status(app))
}

#[tauri::command]
pub async fn check_engine_update(app: AppHandle) -> Result<Option<String>, String> {
    let dir = engine_dir(&app);
    let variant = llamacpp_installer::resolve_variant(Some(&preferred_variant(&app)));
    let app_log = app.clone();
    let log_cb = move |msg: String| {
        let _ = app_log.emit("log", &msg);
    };
    llamacpp_installer::check_update(&dir, &variant, &log_cb).await
}

/// Обновление = переустановка выбранного варианта (старая версия удаляется после успеха)
#[tauri::command]
pub async fn install_engine_update(app: AppHandle) -> Result<EngineStatus, String> {
    install_llamacpp(app).await
}

#[tauri::command]
pub fn remove_engine(app: AppHandle) -> Result<EngineStatus, String> {
    let dir = engine_dir(&app);
    let variant = llamacpp_installer::resolve_variant(Some(&preferred_variant(&app)));
    let app_log = app.clone();
    let log_cb = move |msg: String| {
        let _ = app_log.emit("log", &msg);
    };
    llamacpp_installer::remove(&dir, &variant, &log_cb)?;
    Ok(get_engine_status(app))
}

#[tauri::command]
pub fn set_engine_dir(app: AppHandle, path: String) -> Result<EngineStatus, String> {
    let mut cfg = crate::infra::load_config(&app);
    cfg.llamacpp_dir = Some(path);
    crate::infra::save_config(&app, &cfg);
    Ok(get_engine_status(app))
}
