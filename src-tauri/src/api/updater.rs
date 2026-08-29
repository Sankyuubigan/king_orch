//! Резервная проверка/установка обновлений через GitHub Releases API.
//!
//! Основной путь обновления — tauri-plugin-updater (эндпоинт raw.githubusercontent.com).
//! У части провайдеров этот хост заблокирован, тогда как api.github.com доступен
//! (проверка обновления движка llama.cpp через api.github.com у таких юзеров работает).
//! Этот модуль даёт fallback: опрашивает api.github.com и, при наличии новой версии,
//! скачивает установщик напрямую (как при откате версий, docs rules.md §3.8).

use serde::Serialize;
use tauri::AppHandle;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

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
        .map_err(|e| format!("Ошибка запроса GitHub: {}", e))?;
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
pub async fn install_update_from_github(url: String, app: AppHandle) -> Result<(), String> {
    // Бэкап пользовательских данных (best-effort), как при откате версий.
    let _ = crate::infra::updater_rollback::backup_before_rollback(&app);

    let client = reqwest::Client::builder()
        .user_agent("king-orch-app/1.0")
        .build()
        .map_err(|e| e.to_string())?;

    let tmp = std::env::temp_dir().join(format!(
        "king_orch_update_{}.exe",
        chrono::Local::now().timestamp()
    ));

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Ошибка скачивания установщика: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Скачивание установщика: HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        use std::io::Write;
        f.write_all(&bytes).map_err(|e| e.to_string())?;
    }

    // Тихая установка NSIS; после запуска освобождаем свои файлы (exit),
    // чтобы инсталлер мог перезаписать exe приложения.
    #[cfg(windows)]
    let mut cmd = std::process::Command::new(&tmp);
    #[cfg(windows)]
    cmd.args(["/S"]).creation_flags(0x08000000);
    #[cfg(not(windows))]
    let mut cmd = std::process::Command::new(&tmp);

    match cmd.spawn() {
        Ok(_) => {
            std::process::exit(0);
        }
        Err(e) => Err(format!("Не удалось запустить установщик: {}", e)),
    }
}
