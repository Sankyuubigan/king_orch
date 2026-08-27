//! API-команды для отката версий (делегируют в инфраструктуру).

use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;
use tauri::AppHandle;
use tokio::time::{sleep, Duration};

use crate::infra::updater_rollback::backup_before_rollback;

const REPO: &str = "Sankyuubigan/king_orch";

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    pub version: String,
    pub pub_date: Option<String>,
    pub notes: Option<String>,
    pub download_url: String,
    pub is_current: bool,
}

/// Список доступных релизов через GitHub Releases REST API.
#[tauri::command]
pub async fn get_release_history(_app: AppHandle) -> Result<Vec<ReleaseInfo>, String> {
    let client = reqwest::Client::builder()
        .user_agent("KingOrch-Rollback")
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases?per_page=100"
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API error: {}", resp.status()));
    }

    let releases: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let current = env!("CARGO_PKG_VERSION").to_string();

    let mut out: Vec<ReleaseInfo> = Vec::new();
    let arr = releases.as_array().ok_or("Некорректный ответ GitHub API")?;

    for rel in arr {
        let tag = rel
            .get("tag_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let version = tag.trim_start_matches('v').to_string();
        if version.is_empty() {
            continue;
        }

        let pub_date = rel
            .get("published_at")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let notes = rel
            .get("body")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut download_url = String::new();
        if let Some(assets) = rel.get("assets").and_then(|v| v.as_array()) {
            if let Some(a) = assets.iter().find(|a| {
                let n = a
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                n.ends_with("-setup.exe") && !n.ends_with(".sig")
            }) {
                download_url = a
                    .get("browser_download_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
            }
        }
        if download_url.is_empty() {
            continue;
        }

        out.push(ReleaseInfo {
            version: version.clone(),
            pub_date,
            notes,
            download_url,
            is_current: version == current,
        });
    }

    Ok(out)
}

/// Откат к конкретной версии.
///
/// Единственный источник правды — GitHub Releases. Фронтенд передаёт сюда реальный
/// URL установщика (`download_url`, полученный из GitHub API в `get_release_history`),
/// мы качаем ровно этот ассет и запускаем NSIS-инсталлер (тихо, с даунгрейдом —
/// `allowDowngrades` включён в `tauri.conf.json`). Так откат не зависит от отдельных
/// ассетов `manifest.json`, которые могли оказаться устаревшими/битыми (причина бага).
#[tauri::command]
pub async fn install_release(app: AppHandle, download_url: String) -> Result<(), String> {
    // 1. Бэкап данных перед понижением версии.
    backup_before_rollback(&app)?;

    // 2. Имя установщика из URL.
    let file_name = download_url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Некорректный URL установщика".to_string())?
        .to_string();

    // 3. Скачивание установщика во временную папку.
    let client = reqwest::Client::builder()
        .user_agent("KingOrch-Rollback")
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("Ошибка загрузки установщика: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "Не удалось скачать установщик (HTTP {}). Возможно, релиз недоступен или был удалён.",
            resp.status()
        ));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Ошибка чтения установщика: {}", e))?;

    let installer_path: PathBuf = std::env::temp_dir().join(&file_name);
    std::fs::write(&installer_path, &bytes)
        .map_err(|e| format!("Ошибка записи установщика на диск: {}", e))?;

    // 4. Запуск инсталлера в тихом режиме и авто-перезапуск приложения после
    //    переустановки. NSIS в режиме /S НЕ перезапускает приложение сам, поэтому
    //    запускаем отсоединённый cmd, который дожидается завершения инсталлера
    //    (start /wait) и затем сам запускает обновлённый exe (start "" <exe>).
    let app_exe = std::env::current_exe()
        .map_err(|e| format!("Не удалось получить путь к exe: {}", e))?;
    let installer_str = installer_path.display().to_string();
    let app_str = app_exe.display().to_string();

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let relaunch_script = format!(
            "start \"\" /wait \"{}\" /S & start \"\" \"{}\"",
            installer_str, app_str
        );
        Command::new("cmd")
            .args(["/c", &relaunch_script])
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
            .spawn()
            .map_err(|e| format!("Не удалось запланировать перезапуск: {}", e))?;
    }
    #[cfg(not(windows))]
    {
        let relaunch_script = format!("\"{}\" /S; \"{}\"", installer_str, app_str);
        Command::new("sh")
            .args(["-c", &relaunch_script])
            .spawn()
            .map_err(|e| format!("Не удалось запланировать перезапуск: {}", e))?;
    }

    // Завершаем текущий процесс, чтобы он не держал заблокированным свой exe
    // (иначе тихая переустановка не сможет заменить файлы). Перезапуск выполнит
    // отсоединённый релаунчер выше.
    std::process::exit(0);
}
