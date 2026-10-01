use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use regex::Regex;

use super::workflow_engine::parser::load_workflows;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub mode: String,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default)]
    pub subagents: Vec<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub replace_report: bool,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub current_date: bool,
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Зрение: агент получает изображения текущего запроса в LLM-вызов.
    /// Дефолт `false` — vision не включается неявно «просто потому что прикрепили
    /// файл»: это поднимает расход VRAM, роняет llama-server на не-vision моделях
    /// и неявно связывает LLM-вызов с наличием вложений. Включается вручную
    /// во frontmatter (`vision: true`) только тем агентам, которым пиксели
    /// действительно нужны.
    #[serde(default)]
    pub vision: bool,
}

/// Единая точка входа в UI — может быть .md агентом или YAML графом.
///
/// Каталог НЕ фильтруется по видимости: показывать или нет entry point —
/// решение пользователя, хранится в конфиге (`agent_visibility`).
/// Здесь лежит только факт «что вообще существует на диске».
#[derive(Debug, Clone, Serialize)]
pub struct AgentEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub entry_type: String,
    pub folder: Option<String>,
    /// Путь относительно `agents/` (например `coder/primary_coder.md`).
    /// Позволяет UI показать, где лежит агент, и отличить воркер от точки входа.
    pub rel_path: String,
    /// Роль агента: `"graph"` — узел workflow-графа, `"agent"` — самостоятельный
    /// legacy-агент (в т.ч. корень `agents/*.md`, запускается напрямую).
    pub role: String,
}

/// Имя папки, содержимое которой — мёртвый код и НЕ должно попадать
/// ни в каталог, ни в загрузку агентов. Исключение на уровне сканера,
/// а не на уровне UI: иначе архивные агенты остались бы «невидимыми,
/// но исполняемыми» через прямой `agent_id` (это ложь в UI, §2.2 core/rules).
const ARCHIVE_DIR: &str = "archive";

/// Относительный путь файла внутри `agents/` через `/` (кроссплатформенно).
fn rel_path_of(path: &Path, agents_dir: &Path) -> String {
    path.strip_prefix(agents_dir)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// Первый сегмент относительного пути — команда (`coder`, `psychotherapist`…),
/// `None` для файлов в корне `agents/`.
fn team_folder_of(path: &Path, agents_dir: &Path) -> Option<String> {
    let rel = path.strip_prefix(agents_dir).ok()?;
    let parent = rel.parent()?;
    parent
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
}

fn collect_md_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if is_archive_dir(&path) {
                    log::info!("[agents] Пропущен архив: {}", path.display());
                    continue;
                }
                collect_md_files(&path, files);
            } else if path.extension().map_or(false, |e| e == "md") {
                files.push(path);
            }
        }
    }
}

/// Папка `archive/` в любой вложенности — мёртвый код, исключается из обхода.
pub(crate) fn is_archive_dir(path: &Path) -> bool {
    path.file_name()
        .map_or(false, |n| n == std::ffi::OsStr::new(ARCHIVE_DIR))
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
        if let Some(agent) = parse_agent_file(&path, agents_dir) { agents.push(agent); }
    }
    Ok(agents)
}

fn parse_agent_file(path: &Path, agents_dir: &Path) -> Option<AgentProfile> {
    if let Ok(content) = fs::read_to_string(path) {
        let base_dir = path.parent().unwrap_or_else(|| Path::new(""));
        let processed_content = process_includes(base_dir, &content);
        if let Some(mut agent) = parse_agent_markdown(&processed_content) {
            agent.id = path.file_stem().unwrap().to_string_lossy().to_string();
            agent.folder = team_folder_of(path, agents_dir);
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
            let mut replace_report = false;
            let mut current_date = false;
            let mut temperature: Option<f32> = None;
            let mut vision = false;
            let mut mcp_servers = Vec::new();
            let mut tools = Vec::new();
            let frontmatter_lines: Vec<&str> = frontmatter.lines().collect();
            let mut i = 0;
            while i < frontmatter_lines.len() {
                let line = frontmatter_lines[i].trim();
                if line.starts_with("name:") { name = line["name:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string(); }
                else if line.starts_with("description:") { description = line["description:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string(); }
                else if line.starts_with("visible:") { log::warn!("[agents] Поле `visible:` в frontmatter проигнорировано — видимостью управляет пользователь в конфиге (agent_visibility)"); }
                else if line.starts_with("replace_report:") { replace_report = line["replace_report:".len()..].trim().parse().unwrap_or(false); }
                else if line.starts_with("single_report:") { replace_report = line["single_report:".len()..].trim().parse().unwrap_or(false); }
                else if line.starts_with("current_date:") { current_date = line["current_date:".len()..].trim().parse().unwrap_or(false); }
                else if line.starts_with("temperature:") {
                    temperature = line["temperature:".len()..].trim().parse::<f32>().ok();
                }
                else if line.starts_with("vision:") {
                    vision = line["vision:".len()..].trim().parse().unwrap_or(false);
                }
                else if line.starts_with("mcp_servers:") {
                    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(line["mcp_servers:".len()..].trim()) { mcp_servers = parsed; }
                }
                else if line.starts_with("tools:") {
                    // Предпочтение: JSON-массив имён (`tools: ["code_write"]`).
                    let inline = line["tools:".len()..].trim();
                    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(inline) {
                        tools = parsed;
                    } else {
                        // Robustness: legacy YAML-мапа `tools:\n  write: true\n  bash: true`.
                        // Сопоставляем булевы ключи с мета-наборами / именами тулов.
                        let mut j = i + 1;
                        while j < frontmatter_lines.len() && (frontmatter_lines[j].starts_with(' ') || frontmatter_lines[j].starts_with('\t')) {
                            let kv = frontmatter_lines[j].trim();
                            if let Some((k, v)) = kv.split_once(':') {
                                if let Ok(enabled) = v.trim().parse::<bool>() {
                                    if enabled {
                                        match k.trim() {
                                            "write" | "edit" => tools.push("code_write".to_string()),
                                            "read" => tools.push("code_read".to_string()),
                                            "bash" => tools.push("bash".to_string()),
                                            other => tools.push(other.to_string()),
                                        }
                                    }
                                }
                            }
                            j += 1;
                        }
                        i = j - 1;
                    }
                }
                i += 1;
            }
            if !name.is_empty() { return Some(AgentProfile { id: String::new(), name, description, system_prompt, mode: "worker".to_string(), mcp_servers, subagents: Vec::new(), folder: None, replace_report, tools, current_date, temperature, vision }); }
        }
    }
    None
}

/// Каталог всех существующих entry points для UI: `.md` агенты + YAML-графы.
///
/// ВОЗВРАЩАЕТ ВСЁ, БЕЗ ФИЛЬТРАЦИИ ПО ВИДИМОСТИ. Показом управляет
/// пользователь через `agent_visibility` в конфиге (см. `store.agentVisibility`
/// и `utils/agent-visibility.ts`).
pub fn load_entry_points(agents_dir: &Path) -> Vec<AgentEntry> {
    let mut entries = Vec::new();

    // .md агенты
    if let Ok(agents) = load_agents(agents_dir) {
        for a in agents {
            let path = agents_dir.join(
                a.folder
                    .as_ref()
                    .map(|f| PathBuf::from(f).join(&a.id))
                    .unwrap_or_else(|| PathBuf::from(&a.id))
                    .with_extension("md"),
            );
            entries.push(AgentEntry {
                id: a.id,
                name: a.name,
                description: a.description,
                entry_type: "agent".to_string(),
                folder: a.folder.clone(),
                rel_path: rel_path_of(&path, agents_dir),
                role: "agent".to_string(),
            });
        }
    }

    // YAML графы
    if let Ok(workflows) = load_workflows(agents_dir) {
        for wf in &workflows {
            let path = PathBuf::from(&wf.parent_dir).join(format!("{}.yaml", wf.file_stem));
            entries.push(AgentEntry {
                id: wf.file_stem.clone(),
                name: wf.name.clone(),
                description: wf.description.clone().unwrap_or_default(),
                entry_type: "workflow".to_string(),
                folder: team_folder_of(&path, agents_dir),
                rel_path: rel_path_of(&path, agents_dir),
                role: "graph".to_string(),
            });
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(content: &str) -> AgentProfile {
        parse_agent_markdown(content).expect("агент должен распарситься")
    }

    #[test]
    fn tools_json_array_parsed() {
        let a = parse("---\nname: Test\ntools: [\"code_write\"]\n---\nbody\n");
        assert_eq!(a.tools, vec!["code_write"]);
    }

    #[test]
    fn tools_yaml_map_parsed_as_legacy_robustness() {
        // Legacy формат: YAML-мапа вместо JSON-массива.
        let a = parse("---\nname: Test\ntools:\n  write: true\n  bash: true\n  read: false\n---\nbody\n");
        assert_eq!(a.tools, vec!["code_write", "bash"], "write→code_write, bash→bash, read:false игнорируется");
    }

    #[test]
    fn tools_after_yaml_map_not_consumed() {
        // Ключи после блока мапы (например description ниже tools) парсятся корректно.
        let a = parse("---\nname: Test\ntools:\n  read: true\ndescription: d\n---\nbody\n");
        assert_eq!(a.tools, vec!["code_read"]);
        assert_eq!(a.description, "d");
    }

        #[test]
    fn vision_defaults_to_false_and_is_opt_in() {
        // Дефолт OFF: зрение не должно включаться неявно «просто потому что
        // юзер прикрепил файл» (это роняло llama-server на не-vision моделях).
        let default_agent = parse("---\nname: Default Agent\n---\nbody\n");
        assert!(!default_agent.vision, "зрение по умолчанию должно быть выключено");

        let explicit_off = parse("---\nname: Off\nvision: false\n---\nbody\n");
        assert!(!explicit_off.vision);

        let vision_agent = parse("---\nname: Vision Agent\nvision: true\n---\nbody\n");
        assert!(vision_agent.vision, "vision: true должен включать зрение");
    }

    #[test]
    fn temperature_parsed_from_frontmatter() {
        let a = parse("---\nname: Primary Coder\ntemperature: 0.1\n---\nbody\n");
        assert_eq!(a.temperature, Some(0.1));

        let b = parse("---\nname: Default Agent\n---\nbody\n");
        assert_eq!(b.temperature, None);
    }
}
