use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Manager};

fn get_tools_dir(app: &AppHandle) -> PathBuf {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let dev_tools = current_dir.join("tools");
    if dev_tools.exists() {
        return dev_tools;
    }
    
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));
    exe_dir.join("tools")
}

fn collect_tool_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_tool_files(&path, files);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
}

pub fn get_available_tools(app: &AppHandle) -> Vec<String> {
    let mut tools = Vec::new();
    let tools_dir = get_tools_dir(app);
    
    if tools_dir.exists() {
        let mut tool_files = Vec::new();
        collect_tool_files(&tools_dir, &mut tool_files);
        
        for path in tool_files {
            if let Some(stem) = path.file_stem() {
                let name = stem.to_string_lossy().to_string();
                let ext = path.extension().unwrap_or_default().to_string_lossy().to_lowercase();
                if name.to_lowercase() == "readme" || ext == "json" { continue; }
                if !tools.contains(&name) { tools.push(name); }
            }
        }
    }
    tools
}

pub fn execute_tool(app: &AppHandle, tool_name: &str, arg: &str) -> Result<String, String> {
    let tools_dir = get_tools_dir(app);
    
    let mut target_file = None;
    if tools_dir.exists() {
        let mut tool_files = Vec::new();
        collect_tool_files(&tools_dir, &mut tool_files);
        
        for path in tool_files {
            if let Some(stem) = path.file_stem() {
                if stem.to_string_lossy() == tool_name {
                    target_file = Some(path);
                    break;
                }
            }
        }
    }

    if let Some(file_path) = target_file {
        let ext = file_path.extension().unwrap_or_default().to_string_lossy().to_lowercase();

        if ext == "md" || ext == "txt" {
            return fs::read_to_string(&file_path).map_err(|e| format!("Ошибка чтения файла: {}", e));
        }

        let mut cmd = match ext.as_str() {
            "js" => {
                let node_path = crate::media_fetcher::find_portable_node();
                let mut c = Command::new(node_path);
                c.arg(&file_path);
                c
            },
            "py" => {
                let mut c = Command::new("python");
                c.arg(&file_path);
                c
            },
            "bat" | "cmd" => {
                let mut c = Command::new("cmd");
                c.arg("/C").arg(&file_path);
                c
            },
            "sh" => {
                let mut c = Command::new("bash");
                c.arg(&file_path);
                c
            },
            _ => Command::new(&file_path)
        };

        if !arg.is_empty() { cmd.arg(arg); }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        let output = cmd.output().map_err(|e| format!("Ошибка запуска процесса: {}", e))?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            return Ok(stdout);
        } else {
            return Err(format!("{}\n{}", stdout, stderr));
        }
    }

    Err(format!("Инструмент '{}' не найден.", tool_name))
}