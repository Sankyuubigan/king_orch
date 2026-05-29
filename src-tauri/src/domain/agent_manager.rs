use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use regex::Regex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub is_hidden: bool,
    pub mode: String,
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
            format!("\n<file path=\"{}\">\n<file_content>\n{}\n</file_content>\n</file>\n", rel_path, file_content)
        } else {
            format!("\n<error>Файл {} не найден по пути {}</error>\n", rel_path, full_path.display())
        }
    }).to_string()
}

pub fn load_agents(agents_dir: &Path) -> Result<Vec<AgentProfile>, String> {
    let mut agents = Vec::new();
    if !agents_dir.exists() { return Ok(agents); }
    let mut md_files = Vec::new();
    collect_md_files(agents_dir, &mut md_files);
    for path in md_files {
        if let Some(agent) = parse_agent_file(&path) { agents.push(agent); }
    }
    Ok(agents)
}

fn parse_agent_file(path: &Path) -> Option<AgentProfile> {
    if let Ok(content) = fs::read_to_string(path) {
        let base_dir = path.parent().unwrap_or_else(|| Path::new(""));
        let processed_content = process_includes(base_dir, &content);
        if let Some(mut agent) = parse_agent_markdown(&processed_content) {
            agent.id = path.file_stem().unwrap().to_string_lossy().to_string();
            agent.is_hidden = agent.mode != "primary";
            return Some(agent);
        }
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
            let mut mode = String::from("worker");
            let mut mcp_servers = Vec::new();
            let mut subagents = Vec::new();
            for line in frontmatter.lines() {
                let line = line.trim();
                if line.starts_with("name:") { name = line["name:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string(); }
                else if line.starts_with("description:") { description = line["description:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string(); }
                else if line.starts_with("mode:") { mode = line["mode:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string(); if mode == "subagent" { mode = "worker".to_string(); } }
                else if line.starts_with("mcp_servers:") { if let Ok(parsed) = serde_json::from_str::<Vec<String>>(line["mcp_servers:".len()..].trim()) { mcp_servers = parsed; } }
                else if line.starts_with("subagents:") { if let Ok(parsed) = serde_json::from_str::<Vec<String>>(line["subagents:".len()..].trim()) { subagents = parsed; } }
            }
            if !name.is_empty() { return Some(AgentProfile { id: String::new(), name, description, system_prompt, is_hidden: false, mode, mcp_servers, subagents }); }
        }
    }
    None
}

pub fn build_l0_manifest(agents: &[AgentProfile]) -> String {
    if agents.is_empty() { return String::from("У тебя нет доступных сабагентов. Всегда отвечай пользователю напрямую."); }
    let mut manifest = String::from("ДОСТУПНЫЕ САБАГЕНТЫ (Твоя команда):\n");
    for agent in agents { manifest.push_str(&format!("- ID: \"{}\" | Имя: {} | Роль: {}\n", agent.id, agent.name, agent.description)); }
    manifest.push_str("\nЕсли задача требует специфических навыков, вызови нужного сабагента по его ID.");
    manifest
}