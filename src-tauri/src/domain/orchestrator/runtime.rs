use std::path::{Path, PathBuf};
use crate::infra::McpClient;
use crate::infra::bin_downloader;

pub fn get_mcp_server_path(mcp_servers_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let possible_paths = vec![
        mcp_servers_dir.join(format!("{}.cjs", name)),
        mcp_servers_dir.join(format!("{}.js", name)),
        mcp_servers_dir.join(format!("{}.ts", name)),
        PathBuf::from("src-tauri").join("mcp_servers").join(format!("{}.cjs", name)),
        PathBuf::from("src-tauri").join("mcp_servers").join(format!("{}.js", name)),
        PathBuf::from("src-tauri").join("mcp_servers").join(format!("{}.ts", name)),
    ];
    for path in possible_paths { if path.exists() { return Ok(path); } }
    Err(format!("MCP-сервер {} не найден", name))
}

fn find_or_download_runtime<L: Fn(String) + Clone + Send + Sync>(
    runtime_name: &str, bins_dir: &Path, log_cb: L,
) -> PathBuf {
    let target = env!("TARGET");
    let dev_name = format!("{}-{}.exe", runtime_name, target);
    let exe_name = format!("{}.exe", runtime_name);

    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        for p in vec![
            exe.join(&exe_name),
            exe.join(&dev_name),
            exe.join("bin").join(&dev_name),
            PathBuf::from("bin").join(&dev_name),
        ] {
            if p.exists() { return p; }
        }
    }

    match bin_downloader::ensure_runtime_bin(runtime_name, bins_dir, log_cb.clone()) {
        Ok(path) if path.exists() => return path,
        Ok(_) => {}
        Err(e) => log_cb(format!("❌ Не удалось скачать {}: {}", runtime_name, e)),
    }

    PathBuf::from(runtime_name)
}

pub fn resolve_runtime_and_args<L: Fn(String) + Clone + Send + Sync>(
    log_cb: L, script_path: &Path, bins_dir: &Path,
) -> (PathBuf, Vec<String>) {
    let ext = script_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    if ext == "ts" || ext == "mts" {
        let deno_path = find_or_download_runtime("deno", bins_dir, log_cb.clone());
        log_cb(format!("   🦎 Runtime: Deno | {}", script_path.display()));
        (deno_path, vec!["run".to_string(), "--allow-run".to_string(), "--no-check".to_string(), "--no-config".to_string(), script_path.to_string_lossy().to_string()])
    } else {
        let node_path = find_or_download_runtime("node", bins_dir, log_cb.clone());
        log_cb(format!("   🟢 Runtime: Node | {}", script_path.display()));
        (node_path, vec![script_path.to_string_lossy().to_string()])
    }
}

fn ensure_mcp_deps<L: Fn(String) + Clone + Send + Sync>(
    mcp_name: &str, bins_dir: &Path, log_cb: &L,
) -> Vec<(&'static str, String)> {
    if mcp_name == "youtube_mcp" {
        if let Ok(_yt_path) = bin_downloader::ensure_runtime_bin("yt-dlp", bins_dir, log_cb.clone()) {
            if let Some(bins_str) = bins_dir.to_str() {
                return vec![("KING_ORCH_BINS_DIR", bins_str.to_string())];
            }
        }
    }
    vec![]
}

pub fn load_mcp_servers<L: Fn(String) + Clone + Send + Sync + 'static>(
    log_cb: &L,
    mcp_servers_dir: &Path,
    bins_dir: &Path,
    mcp_names: &[String],
    mcp_clients: &mut std::collections::HashMap<String, McpClient>,
    all_tools: &mut Vec<(String, String, serde_json::Value)>,
) {
    for mcp_name in mcp_names {
        log_cb(format!("⏳ Инициализация MCP: {}", mcp_name));
        match get_mcp_server_path(mcp_servers_dir, mcp_name) {
            Ok(script_path) => {
                let (runtime_path, runtime_args) = resolve_runtime_and_args(log_cb.clone(), &script_path, bins_dir);
                let args_refs: Vec<&str> = runtime_args.iter().map(|s| s.as_str()).collect();
                let envs = ensure_mcp_deps(mcp_name, bins_dir, log_cb);
                let env_refs: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (*k, v.as_str())).collect();
                match McpClient::spawn_stub_with_env(&runtime_path.to_string_lossy(), &args_refs, &env_refs, log_cb.clone()) {
                    Ok(mut client) => {
                        match client.list_tools() {
                            Ok(tools) => {
                                let mut loaded = 0;
                                for tool in &tools {
                                    if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                                        all_tools.push((mcp_name.clone(), name.to_string(), tool.clone()));
                                        loaded += 1;
                                    }
                                }
                                mcp_clients.insert(mcp_name.clone(), client);
                                log_cb(format!("✅ MCP '{}' запущен. Инструментов: {}", mcp_name, loaded));
                            }
                            Err(e) => log_cb(format!("❌ Ошибка списка инструментов у '{}': {}", mcp_name, e))
                        }
                    }
                    Err(e) => log_cb(format!("❌ Критическая ошибка запуска MCP '{}': {}", mcp_name, e)),
                }
            }
            Err(e) => log_cb(format!("❌ Ошибка поиска файла сервера: {}", e)),
        }
    }
}
