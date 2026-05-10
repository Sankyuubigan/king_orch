use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use regex::Regex;
use crate::processor::emit_log;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub is_hidden: bool,
    pub mode: String, // "primary" или "subagent"
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

/// Умный инжектор файлов. Ищет <<INCLUDE: path/to/file.md>> и заменяет содержимым.
/// Использует индустриальный стандарт XML-тегов для разграничения контекста.
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
    
    emit_log(app, "🔍 Начинаю сканирование агентов...");

    // 1. Загрузка видимых агентов (пользовательских)
    let agents_dir = exe_dir.join("agents");
    if !agents_dir.exists() {
        let _ = fs::create_dir_all(&agents_dir);
    }

    // Создаем дефолтного Оркестратора (теперь это обычный md-файл)
    let orch_path = agents_dir.join("orchestrator.md");
    if !orch_path.exists() {
        let default_orch = "---\nname: Orchestrator\ndescription: Главный маршрутизатор задач\nmode: primary\n---\nТы — Главный Оркестратор. Твоя задача — решить запрос пользователя, делегируя задачи профильным сабагентам, если это необходимо.";
        let _ = fs::write(orch_path, default_orch);
    }

    // Создаем демо-кодера
    let coder_path = agents_dir.join("coder.md");
    if !coder_path.exists() {
        let demo_agent = "---\nname: Coder\ndescription: Вызывай меня для написания кода, скриптов или исправления багов.\nmode: subagent\n---\nТы — Senior Software Engineer. Твоя задача писать чистый, оптимизированный и рабочий код.\nОтвечай только кодом с краткими комментариями. Не пиши лишних рассуждений.";
        let _ = fs::write(coder_path, demo_agent);
    }

    let mut md_files = Vec::new();
    collect_md_files(&agents_dir, &mut md_files);

    for path in md_files {
        if let Some(agent) = parse_agent_file(app, &path, false) {
            agents.push(agent);
        }
    }

    // 2. Загрузка теневых агентов (категорий / клаудовских сабагентов)
    let categories_dir = exe_dir.join("categories");
    if categories_dir.exists() {
        let mut cat_files = Vec::new();
        collect_md_files(&categories_dir, &mut cat_files);
        for path in cat_files {
            if let Some(agent) = parse_agent_file(app, &path, true) {
                agents.push(agent);
            }
        }
    }
    
    emit_log(app, &format!("✅ Сканирование завершено. Успешно загружено агентов: {}", agents.len()));
    agents
}

fn parse_agent_file(app: &AppHandle, path: &Path, is_hidden: bool) -> Option<AgentProfile> {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    
    if let Ok(content) = fs::read_to_string(path) {
        let base_dir = path.parent().unwrap_or_else(|| Path::new(""));
        let processed_content = process_includes(base_dir, &content);

        if let Some(mut agent) = parse_agent_markdown(&processed_content) {
            agent.id = path.file_stem().unwrap().to_string_lossy().to_string();
            agent.is_hidden = is_hidden;
            return Some(agent);
        } else {
            emit_log(app, &format!("⚠️ Пропущен файл {}: Не найден или некорректен блок Frontmatter (--- name: ... description: ... ---)", file_name));
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
            let mut mode = String::from("subagent"); // По умолчанию
            
            for line in frontmatter.lines() {
                let line = line.trim();
                if line.starts_with("name:") {
                    name = line["name:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string();
                } else if line.starts_with("description:") {
                    description = line["description:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string();
                } else if line.starts_with("mode:") {
                    mode = line["mode:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string();
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
                });
            }
        }
    }
    None
}

pub fn build_l0_manifest(agents: &[AgentProfile]) -> String {
    if agents.is_empty() {
        return String::from("У тебя нет доступных сабагентов. Всегда отвечай пользователю напрямую (target: \"user\").");
    }
    
    let mut manifest = String::from("ДОСТУПНЫЕ САБАГЕНТЫ (Твоя команда):\n");
    for agent in agents {
        manifest.push_str(&format!("- ID: \"{}\" | Имя: {} | Роль: {}\n", agent.id, agent.name, agent.description));
    }
    manifest.push_str("\nЕсли задача требует специфических навыков, вызови нужного сабагента по его ID. Иначе отвечай пользователю (target: \"user\").");
    manifest
}