//! API-команды для отката версий (делегируют в инфраструктуру).

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

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

/// Откат к конкретной версии. Бэкапит данные, затем через плагин updater ставит
/// выбранную версию из её `manifest.json` (ассет релиза). На Windows процесс завершается
/// самим установщиком и приложение перезапускается.
#[tauri::command]
pub async fn install_release(app: AppHandle, version: String) -> Result<(), String> {
    // 1. Бэкап данных перед понижением версии.
    backup_before_rollback(&app)?;

    // 2. Манифест выбранной версии (загружается как ассет релиза).
    let manifest_url = format!("https://github.com/{REPO}/releases/download/v{version}/manifest.json")
        .parse::<url::Url>()
        .map_err(|e| e.to_string())?;

    let update = app
        .updater_builder()
        .endpoints(vec![manifest_url])
        .map_err(|e| e.to_string())?
        .version_comparator(|_, _| true)
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;

    let Some(update) = update else {
        return Err(
            "Не удалось получить манифест версии. Возможно, релиз собран без manifest.json \
             (нужен релиз после внедрения отката)."
                .into(),
        );
    };

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}
