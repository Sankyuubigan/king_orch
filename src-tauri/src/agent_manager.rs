use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use regex::Regex;
use crate::emit_log;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub is_hidden: bool,
    pub mode: String,
    #[serde(default)]
    pub can_update_state: bool,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default)]
    pub subagents: Vec<String>,
}

fn collect_md_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_md_files(&path, files); 
            } else if path.extension().map_or(false, |e| e == "md") {
                files.push(path); 
            }
        }
    }
}

fn process_includes(base_path: &Path, content: &str) -> String {
    let re = Regex::new(r"<<INCLUDE:\s*(.+?)\s*>>").unwrap();
    re.replace_all(content, |caps: &regex::Captures| {
        let rel_path = caps.get(1).unwrap().as_str().trim();
        let full_path = base_path.join(rel_path);
        if let Ok(file_content) = fs::read_to_string(&full_path) {
            format!(
                "\n<file path=\"{}\">\n<file_content>\n{}\n</file_content>\n</file>\n", 
                rel_path, file_content
            )
        } else {
            format!("\n<error>Файл {} не найден по пути {}</error>\n", rel_path, full_path.display())
        }
    }).to_string()
}

pub fn load_agents(app: &AppHandle) -> Vec<AgentProfile> {
    let mut agents = Vec::new();
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::from("."));
    
    emit_log(app, "🔍 Начинаю сканирование агентов...");

    let possible_agents_dirs = vec![
        exe_dir.join("agents"),
        resource_dir.join("agents"),
        PathBuf::from("agents"), // Текущая рабочая директория
        exe_dir.join("..").join("..").join("agents"), // Корень проекта при запуске из target/release
    ];
    
    let default_agents_dir = exe_dir.join("agents");
    let agents_dir = possible_agents_dirs.into_iter().find(|p| p.exists()).unwrap_or(default_agents_dir);

    if !agents_dir.exists() {
        let _ = fs::create_dir_all(&agents_dir);
    }

    // БЛОК АВТОСОЗДАНИЯ ОРКЕСТРАТОРА УБРАН ПО ПРОСЬБЕ ПОЛЬЗОВАТЕЛЯ

    let mut md_files = Vec::new();
    collect_md_files(&agents_dir, &mut md_files);

    let possible_categories_dirs = vec![
        exe_dir.join("categories"),
        resource_dir.join("categories"),
        PathBuf::from("categories"),
        exe_dir.join("..").join("..").join("categories"),
    ];
    
    if let Some(cat_dir) = possible_categories_dirs.into_iter().find(|p| p.exists()) {
        collect_md_files(&cat_dir, &mut md_files);
    }

    for path in md_files {
        if let Some(agent) = parse_agent_file(app, &path) {
            agents.push(agent);
        }
    }
    
    emit_log(app, &format!("✅ Сканирование завершено. Успешно загружено агентов: {}", agents.len()));
    agents
}

fn parse_agent_file(app: &AppHandle, path: &Path) -> Option<AgentProfile> {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    
    if let Ok(content) = fs::read_to_string(path) {
        let base_dir = path.parent().unwrap_or_else(|| Path::new(""));
        let processed_content = process_includes(base_dir, &content);

        if let Some(mut agent) = parse_agent_markdown(&processed_content) {
            agent.id = path.file_stem().unwrap().to_string_lossy().to_string();
            agent.is_hidden = agent.mode == "subagent";
            return Some(agent);
        } else {
            emit_log(app, &format!("⚠️ Пропущен файл {}: Не найден или некорректен блок Frontmatter", file_name));
        }
    } else {
        emit_log(app, &format!("❌ Ошибка чтения файла {}", file_name));
    }
    None
}

fn parse_agent_markdown(content: &str) -> Option<AgentProfile> {
    let text = content.trim_start_matches('\u{feff}').trim();
    
    if text.starts_with("---") {
        if let Some(end_idx) = text[3..].find("---") {
            let frontmatter = &text[3..end_idx + 3];
            let system_prompt = text[end_idx + 6..].trim().to_string();
            
            let mut name = String::new();
            let mut description = String::new();
            let mut mode = String::from("subagent");
            let mut can_update_state = false;
            let mut mcp_servers = Vec::new();
            let mut subagents = Vec::new();
            
            for line in frontmatter.lines() {
                let line = line.trim();
                if line.starts_with("name:") {
                    name = line["name:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string();
                } else if line.starts_with("description:") {
                    description = line["description:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string();
                } else if line.starts_with("mode:") {
                    mode = line["mode:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string();
                } else if line.starts_with("can_update_state:") {
                    can_update_state = line["can_update_state:".len()..].trim().parse().unwrap_or(false);
                } else if line.starts_with("mcp_servers:") {
                    let list_str = line["mcp_servers:".len()..].trim();
                    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(list_str) {
                        mcp_servers = parsed;
                    }
                } else if line.starts_with("subagents:") {
                    let list_str = line["subagents:".len()..].trim();
                    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(list_str) {
                        subagents = parsed;
                    }
                } else if line.starts_with("tools:") {
                    let tools_str = line["tools:".len()..].trim();
                    let tools_list: Vec<&str> = tools_str.split(',').map(|s| s.trim()).collect();
                    for t in tools_list {
                        match t.to_lowercase().as_str() {
                            "websearch" => if !mcp_servers.contains(&"ddg_search".to_string()) { mcp_servers.push("ddg_search".to_string()); },
                            "webfetch" => if !mcp_servers.contains(&"docs_fetcher".to_string()) { mcp_servers.push("docs_fetcher".to_string()); },
                            "read" | "write" | "grep" | "glob" | "ls" | "edit" => {
                                if !mcp_servers.contains(&"filesystem".to_string()) { mcp_servers.push("filesystem".to_string()); }
                            },
                            _ => {} 
                        }
                    }
                }
            }
            
            if !name.is_empty() {
                return Some(AgentProfile {
                    id: String::new(), 
                    name,
                    description,
                    system_prompt,
                    is_hidden: false,
                    mode,
                    can_update_state,
                    mcp_servers,
                    subagents,
                });
            }
        }
    }
    None
}

pub fn build_l0_manifest(agents: &[AgentProfile]) -> String {
    if agents.is_empty() {
        return String::from("У тебя нет доступных сабагентов. Всегда отвечай пользователю напрямую.");
    }
    
    let mut manifest = String::from("ДОСТУПНЫЕ САБАГЕНТЫ (Твоя команда):\n");
    for agent in agents {
        manifest.push_str(&format!("- ID: \"{}\" | Имя: {} | Роль: {}\n", agent.id, agent.name, agent.description));
    }
    manifest.push_str("\nЕсли задача требует специфических навыков, вызови нужного сабагента по его ID.");
    manifest
}