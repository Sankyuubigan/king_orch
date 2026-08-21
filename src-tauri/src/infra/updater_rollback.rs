//! Откат к предыдущим версиям: бэкап пользовательских данных перед понижением версии.

use std::fs;
use std::path::Path;
use tauri::{AppHandle, Manager};

/// Копирует `app_config.json` и папку `sessions/` в `rollback_backup/` рядом с данными
/// приложения. Гарантирует, что при даунгрейде старая версия сможет прочитать данные,
/// либо пользователь сможет вручную вернуть бэкап, если схема данных новее нечитаема.
pub fn backup_before_rollback(app: &AppHandle) -> Result<(), String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    if !data_dir.exists() {
        return Ok(());
    }

    let backup_dir = data_dir.join("rollback_backup");
    fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;

    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();

    let cfg = data_dir.join("app_config.json");
    if cfg.exists() {
        let dst = backup_dir.join(format!("app_config_{ts}.json"));
        let _ = fs::copy(&cfg, &dst);
    }

    let sessions = data_dir.join("sessions");
    if sessions.exists() {
        let dst = backup_dir.join(format!("sessions_{ts}"));
        let _ = copy_dir(&sessions, &dst);
    }

    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}
