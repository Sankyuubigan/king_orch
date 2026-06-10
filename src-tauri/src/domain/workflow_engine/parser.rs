use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Определение workflow — YAML граф маршрутизации
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub name: String,
    /// Показывать ли граф в UI как точку входа
    #[serde(default)]
    pub visible: bool,
    /// Имя файла без расширения (заполняется при загрузке, не из YAML)
    #[serde(skip)]
    pub file_stem: String,
    #[serde(default)]
    pub config: Option<WorkflowConfig>,
    pub nodes: Vec<NodeDef>,
    pub edges: Vec<EdgeDef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowConfig {
    #[serde(default)]
    pub statuses: Vec<StatusDef>,
    /// Путь к внешнему файлу со статусами (относительно папки workflow)
    #[serde(default)]
    pub statuses_file: Option<String>,
    /// Кастомный системный промпт для llm_classifier
    #[serde(default)]
    pub classifier_prompt: Option<String>,
}

/// Структура внешнего файла статусов
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusesFile {
    #[serde(default)]
    pub classifier_prompt: Option<String>,
    #[serde(default)]
    pub statuses: Vec<StatusDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusDef {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub criteria: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDef {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    /// Для llm_worker / llm_classifier
    #[serde(default)]
    pub agent: Option<String>,
    /// Для llm_worker — задача
    #[serde(default)]
    pub task: Option<String>,
    /// Для llm_classifier / llm_worker — входные данные
    #[serde(default)]
    pub input: Option<String>,
    /// Для llm_classifier — статусы (ссылка на config.statuses)
    #[serde(default)]
    pub statuses: Option<serde_yaml::Value>,
    /// Для system_condition — действие
    #[serde(default)]
    pub action: Option<String>,
    /// Для system_condition — список required агентов
    #[serde(default)]
    pub required: Option<Vec<String>>,
    /// Для sub_workflow — имя файла workflow
    #[serde(default)]
    pub workflow: Option<String>,
    /// Для switch — входное значение
    #[serde(default)]
    pub cases: Option<HashMap<String, String>>,
    /// Для switch — default маршрут, если ни один case не совпал
    #[serde(default)]
    pub default: Option<String>,
    /// Неймспейс для работы (пробрасывается во все дочерние вызовы)
    #[serde(default)]
    pub namespace: Option<String>,
    /// Проблемы (для triage check_all_analyzed)
    #[serde(default)]
    pub problems: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    LlmWorker,
    LlmClassifier,
    LlmFreeform,
    SystemCondition,
    SubWorkflow,
    Switch,
    Return,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeDef {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub case: Option<String>,
}

/// Загружает все YAML workflow файлы из папки workflows/ внутри teams
pub fn load_workflows(agents_dir: &Path) -> Result<Vec<WorkflowDef>, String> {
    let mut workflows = Vec::new();
    let mut yaml_files = Vec::new();
    collect_yaml_files(agents_dir, &mut yaml_files);
    for path in yaml_files {
        match parse_workflow_file(&path) {
            Ok(wf) => workflows.push(wf),
            Err(e) => eprintln!("[workflow_engine] Ошибка загрузки {}: {}", path.display(), e),
        }
    }
    Ok(workflows)
}

fn collect_yaml_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_yaml_files(&path, files);
            } else if path.extension().map_or(false, |e| e == "yaml" || e == "yml") {
                files.push(path);
            }
        }
    }
}

fn parse_workflow_file(path: &Path) -> Result<WorkflowDef, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Не удалось прочитать {}: {}", path.display(), e))?;
    let file_stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut wf: WorkflowDef = serde_yaml::from_str(&content)
        .map_err(|e| format!("Ошибка парсинга YAML {}: {}", path.display(), e))?;
    wf.file_stem = file_stem;

    // Загружаем внешний файл статусов, если указан
    if let Some(ref statuses_file) = wf.config.as_ref().and_then(|c| c.statuses_file.clone()) {
        let parent_dir = path.parent().unwrap_or(Path::new("."));
        let ext_path = parent_dir.join(&statuses_file);
        let ext_content = fs::read_to_string(&ext_path)
            .map_err(|e| format!("Не удалось прочитать статусы {}: {}", ext_path.display(), e))?;
        let ext: StatusesFile = serde_yaml::from_str(&ext_content)
            .map_err(|e| format!("Ошибка парсинга статусов {}: {}", ext_path.display(), e))?;

        if let Some(ref mut config) = wf.config {
            // Статусы из внешнего файла заменяют inline-статусы
            if !ext.statuses.is_empty() {
                config.statuses = ext.statuses;
            }
            // Приоритет у classifier_prompt из внешнего файла
            if let Some(prompt) = ext.classifier_prompt {
                config.classifier_prompt = Some(prompt);
            }
        }
    }

    Ok(wf)
}

/// Находит workflow по его file_stem
pub fn find_workflow_by_stem<'a>(
    workflows: &'a [WorkflowDef],
    stem: &str,
) -> Option<&'a WorkflowDef> {
    workflows.iter().find(|wf| wf.file_stem == stem)
}
