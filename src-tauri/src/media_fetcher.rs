use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::process::Command;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use regex::Regex;
use std::process::Stdio;
use crate::processor::{emit_log, emit_status};

pub fn find_yt_dlp() -> std::path::PathBuf {
    let target = env!("TARGET");
    let dev_name = format!("yt-dlp-{}.exe", target);
    
    let mut paths = vec![std::path::PathBuf::from("yt-dlp.exe")];
    
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        paths.push(exe.join("yt-dlp.exe"));
        paths.push(exe.join(&dev_name));
        paths.push(exe.join("bin").join(&dev_name));
    }
    
    paths.push(std::path::PathBuf::from("bin").join(&dev_name));
    paths.push(std::path::PathBuf::from("src-tauri").join("bin").join(&dev_name));
    
    for p in paths {
        if p.exists() { return p; }
    }
    std::path::PathBuf::from("yt-dlp")
}

pub fn find_portable_node() -> std::path::PathBuf {
    let target = env!("TARGET");
    let dev_name = format!("node-{}.exe", target);
    
    let mut paths = vec![std::path::PathBuf::from("node.exe")];
    
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        paths.push(exe.join("node.exe"));
        paths.push(exe.join(&dev_name));
        paths.push(exe.join("bin").join(&dev_name));
    }
    
    paths.push(std::path::PathBuf::from("bin").join(&dev_name));
    paths.push(std::path::PathBuf::from("src-tauri").join("bin").join(&dev_name));
    
    for p in paths {
        if p.exists() { return p; }
    }
    std::path::PathBuf::from("node")
}

pub fn calculate_hash<T: Hash>(t: &T) -> u64 {
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}

pub async fn fetch_sponsorblock(url: &str) -> Result<Vec<(f64, f64)>, String> {
    let re = Regex::new(r"(?:v=|youtu\.be/|embed/|shorts/)([a-zA-Z0-9_-]{11})").unwrap();
    let video_id = match re.captures(url) {
        Some(cap) => cap[1].to_string(),
        None => return Err("Не удалось найти ID видео в ссылке".to_string()),
    };

    let api_url = format!(
        "https://sponsor.ajay.app/api/skipSegments?videoID={}&categories=[\"sponsor\",\"selfpromo\",\"interaction\"]",
        video_id
    );

    let resp = reqwest::get(&api_url).await.map_err(|e| e.to_string())?;
    
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new()); 
    }
    
    if !resp.status().is_success() {
        return Err(format!("Код ответа: {}", resp.status()));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut segments = Vec::new();

    if let Some(arr) = json.as_array() {
        for item in arr {
            if let Some(seg) = item.get("segment").and_then(|s| s.as_array()) {
                if seg.len() == 2 {
                    let start = seg[0].as_f64().unwrap_or(0.0);
                    let end = seg[1].as_f64().unwrap_or(0.0);
                    segments.push((start, end));
                }
            }
        }
    }

    Ok(segments)
}

pub async fn fetch_media_data(app: &AppHandle, url: &str, cancel_flag: Arc<AtomicBool>) -> Result<(String, String), String> {
    if !url.starts_with("http") {
        emit_log(app, &format!("Локальный файл: {}", url));
        return Ok((url.to_string(), String::new()));
    }

    let app_data_dir = app.path().app_data_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let temp_dir = app_data_dir.join("temp");
    if !temp_dir.exists() {
        let _ = fs::create_dir_all(&temp_dir);
    }
    
    let re = Regex::new(r"(?:v=|youtu\.be/|embed/|shorts/)([a-zA-Z0-9_-]{11})").unwrap();
    let video_id = match re.captures(url) {
        Some(cap) => cap[1].to_string(),
        None => calculate_hash(&url).to_string(),
    };
    let base_name = format!("sub_{}", video_id);

    let mut sub_path_opt: Option<String> = None;
    let mut info_path_opt: Option<String> = None;
    
    if let Ok(entries) = fs::read_dir(&temp_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_string_lossy();
                if name.starts_with(&base_name) && (name.ends_with(".srt") || name.ends_with(".vtt")) {
                    if let Some(ref current) = sub_path_opt {
                        if current.contains(".ru") && !name.contains(".ru") { continue; }
                    }
                    sub_path_opt = Some(path.to_string_lossy().to_string());
                }
                if name.starts_with(&base_name) && name.ends_with(".info.json") {
                    info_path_opt = Some(path.to_string_lossy().to_string());
                }
            }
        }
    }

    if let (Some(sub), Some(info)) = (&sub_path_opt, &info_path_opt) {
        emit_log(app, "✅ Субтитры и метаданные найдены в кэшe.");
        return Ok((sub.clone(), info.clone()));
    }

    emit_status(app, "Получение данных (yt-dlp)...", 2);
    let yt_dlp_path = find_yt_dlp();
    let node_path = find_portable_node();
    
    let js_runtime_arg = format!("node:{}", node_path.to_string_lossy());
    
    let mut args = vec![
        "--write-auto-sub", "--write-subs", "--sub-langs", "ru.*,en.*,ru,en",
        "--convert-subs", "srt", "--write-info-json", "--skip-download",
        "--js-runtimes", &js_runtime_arg, "-4", "--legacy-server-connect",
        "--no-check-certificates", "-o", &base_name,
    ];

    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let cookies_dir = exe_dir.join("cookies");
    
    let mut cookie_file_path = String::new();
    if let Ok(entries) = fs::read_dir(&cookies_dir) {
        for entry in entries.flatten() {
            if entry.path().extension().map_or(false, |e| e == "txt") {
                cookie_file_path = entry.path().to_string_lossy().to_string();
                break;
            }
        }
    }

    if !cookie_file_path.is_empty() {
        args.push("--cookies");
        args.push(&cookie_file_path);
    }

    args.push(url);

    let mut cmd = Command::new(&yt_dlp_path);
    cmd.current_dir(&temp_dir).args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x08000000);
    }

    let mut child = cmd.spawn().map_err(|e| format!("Ошибка запуска yt-dlp: {}", e))?;

    let output = loop {
        if cancel_flag.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            return Err("Скачивание прервано пользователем".to_string());
        }
        if let Ok(Some(_)) = child.try_wait() {
            break child.wait_with_output().await.map_err(|e| format!("Ошибка ожидания yt-dlp: {}", e))?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    };

    let stderr_str = String::from_utf8_lossy(&output.stderr);

    let mut final_sub_path: Option<String> = None;
    let mut final_info_path: Option<String> = None;

    if let Ok(entries) = fs::read_dir(&temp_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_string_lossy();
                if name.starts_with(&base_name) && (name.ends_with(".srt") || name.ends_with(".vtt")) {
                    final_sub_path = Some(path.to_string_lossy().to_string());
                }
                if name.starts_with(&base_name) && name.ends_with(".info.json") {
                    final_info_path = Some(path.to_string_lossy().to_string());
                }
            }
        }
    }

    let info_result = match final_info_path {
        Some(p) => p,
        None => {
            let dummy_path = temp_dir.join(format!("{}.dummy.info.json", base_name));
            let _ = fs::write(&dummy_path, "{}");
            dummy_path.to_string_lossy().to_string()
        }
    };

    if let Some(sub_path) = final_sub_path {
        Ok((sub_path, info_result))
    } else {
        Err(format!("Субтитры не найдены.\nДетали ошибки yt-dlp:\n{}", stderr_str.trim()))
    }
}