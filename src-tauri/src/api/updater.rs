//! Резервная проверка/установка обновлений через GitHub Releases API.
//!
//! Основной путь обновления — tauri-plugin-updater (эндпоинт raw.githubusercontent.com).
//! У части провайдеров этот хост заблокирован, тогда как api.github.com доступен
//! (проверка обновления движка llama.cpp через api.github.com у таких юзеров работает).
//! Этот модуль даёт fallback: опрашивает api.github.com и, при наличии новой версии,
//! скачивает установщик напрямую (как при откате версий, docs rules.md §3.8).

use serde::Serialize;
use tauri::AppHandle;

#[derive(Serialize)]
pub struct GithubUpdateInfo {
    pub version: String,
    pub url: String,
    pub notes: String,
}

/// Побитовое сравнение версий вида "26.8.165" / "v26.8.165".
fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(v: &str) -> Vec<u32> {
        v.trim_start_matches('v')
            .split('.')
            .filter_map(|s| s.parse::<u32>().ok())
            .collect()
    }
    let l = parse(latest);
    let c = parse(current);
    if l.len() != c.len() {
        // Разное число компонент — сравниваем как сумму (грубый, но безопасный фолбек).
        return l.iter().map(|&x| x as u64).sum::<u64>() > c.iter().map(|&x| x as u64).sum::<u64>();
    }
    for (a, b) in l.iter().zip(c.iter()) {
        if a > b {
            return true;
        }
        if a < b {
            return false;
        }
    }
    false
}

#[tauri::command]
pub async fn check_github_release_update(app: AppHandle) -> Result<Option<GithubUpdateInfo>, String> {
    let current = app.package_info().version.to_string();
    let client = reqwest::Client::builder()
        .user_agent("king-orch-app/1.0")
        .build()
        .map_err(|e| e.to_string())?;

    let url = "https://api.github.com/repos/Sankyuubigan/king_orch/releases/latest";
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Ошибка запроса GitHub: {}", crate::infra::llm::chain_err(&e, 3)))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API вернул HTTP {}", resp.status()));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let tag = json.get("tag_name").and_then(|v| v.as_str()).unwrap_or("");
    let latest = tag.trim_start_matches('v');

    let assets = json
        .get("assets")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    let asset = assets.iter().find(|a| {
        a.get("name")
            .and_then(|n| n.as_str())
            .map(|n| n.ends_with("-setup.exe"))
            .unwrap_or(false)
    });
    let download_url = match asset.and_then(|a| a.get("browser_download_url").and_then(|u| u.as_str())) {
        Some(u) => u.to_string(),
        None => return Ok(None),
    };

    if is_newer(latest, &current) {
        Ok(Some(GithubUpdateInfo {
            version: latest.to_string(),
            url: download_url,
            notes: json
                .get("body")
                .and_then(|b| b.as_str())
                .unwrap_or("")
                .to_string(),
        }))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn install_update_from_github(
    app: AppHandle,
    url: String,
    version: String,
) -> Result<(), String> {
    log::info!("[updater] резервная установка релиза {} из {}", version, url);

    // Бэкап пользовательских данных. Для обновления это best-effort (схема
    // данных не меняется), но ошибку не глотаем молча — пишем в лог (core §2.2).
    if let Err(e) = tauri_plugin_about_updates::backup_before_rollback(&app) {
        log::error!("[updater] бэкап перед обновлением не удался: {}", e);
    }

    // Скачивание, запуск NSIS (/P /UPDATE /R), перезапуск приложения и выход —
    // ровно тот же конвейер, что у отката версий (SSOT, tauri-plugin-about-updates).
    // Раньше здесь был второй, отдельный вызов установщика без `/R`:
    // после резервного обновления приложение не перезапускалось вообще.
    tauri_plugin_about_updates::installer::run_install(
        &app,
        tauri_plugin_about_updates::install_report::InstallKind::Update,
        &version,
        &url,
    )
    .await
}
