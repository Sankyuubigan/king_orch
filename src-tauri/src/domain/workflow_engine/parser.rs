use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Определение workflow — YAML граф маршрутизации
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub name: String,
    #[serde(default)]
    pub visible: bool,
    #[serde(skip)]
    pub file_stem: String,
    #[serde(default)]
    pub config: Option<WorkflowConfig>,
    pub nodes: Vec<NodeDef>,
    pub edges: Vec<EdgeDef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowConfig {
    // --- Новый паттерн: Fact Extractor ---
    #[serde(default)]
    pub facts: Vec<FactDef>,
    #[serde(default)]
    pub facts_file: Option<String>,
    #[serde(default)]
    pub extractor_prompt: Option<String>,

    // --- Старый паттерн: Status Classifier (для обратной совместимости) ---
    #[serde(default)]
    pub statuses: Vec<StatusDef>,
    #[serde(default)]
    pub statuses_file: Option<String>,
    #[serde(default)]
    pub classifier_prompt: Option<String>,
}

/// Внешний файл фактов (facts.yaml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactsFile {
    #[serde(default)]
    pub extractor_prompt: Option<String>,
    #[serde(default)]
    pub facts: Vec<FactDef>,
}

/// Внешний файл статусов (statuses.yaml) — для обратной совместимости
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusesFile {
    #[serde(default)]
    pub classifier_prompt: Option<String>,
    #[serde(default)]
    pub statuses: Vec<StatusDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactDef {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub criteria: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusDef {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub criteria: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityCase {
    pub key: String,
    #[serde(rename = "to")]
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDef {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub input: Option<String>,
    #[serde(default)]
    pub statuses: Option<serde_yaml::Value>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub required: Option<Vec<String>>,
    #[serde(default)]
    pub workflow: Option<String>,
    #[serde(default)]
    pub cases: Option<HashMap<String, String>>,
    #[serde(default)]
    pub default: Option<String>,
    /// Для switch с приоритетной маршрутизацией (первый true = переход)
    #[serde(default)]
    pub cases_priority: Option<Vec<PriorityCase>>,
    /// Для switch — ссылка на JSON-объект ({{ nodes.X.output }})
    #[serde(default)]
    pub input_object: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub problems: Option<String>,
    #[serde(default)]
    pub output_type: Option<String>,
    /// Визуальные координаты для редактора графов (x, y)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_pos: Option<HashMap<String, i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    LlmWorker,
    LlmClassifier,
    LlmFactExtractor,
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

    let parent_dir = path.parent().unwrap_or(Path::new("."));

    // Загружаем внешний файл фактов (facts.yaml), если указан
    if let Some(ref facts_file) = wf.config.as_ref().and_then(|c| c.facts_file.clone()) {
        let ext_path = parent_dir.join(&facts_file);
        let ext_content = fs::read_to_string(&ext_path)
            .map_err(|e| format!("Не удалось прочитать факты {}: {}", ext_path.display(), e))?;
        let ext: FactsFile = serde_yaml::from_str(&ext_content)
            .map_err(|e| format!("Ошибка парсинга фактов {}: {}", ext_path.display(), e))?;

        if let Some(ref mut config) = wf.config {
            if !ext.facts.is_empty() {
                config.facts = ext.facts;
            }
            if let Some(prompt) = ext.extractor_prompt {
                config.extractor_prompt = Some(prompt);
            }
        }
    }

    // Загружаем внешний файл статусов (statuses.yaml) — обратная совместимость
    if let Some(ref statuses_file) = wf.config.as_ref().and_then(|c| c.statuses_file.clone()) {
        let ext_path = parent_dir.join(&statuses_file);
        let ext_content = fs::read_to_string(&ext_path)
            .map_err(|e| format!("Не удалось прочитать статусы {}: {}", ext_path.display(), e))?;
        let ext: StatusesFile = serde_yaml::from_str(&ext_content)
            .map_err(|e| format!("Ошибка парсинга статусов {}: {}", ext_path.display(), e))?;

        if let Some(ref mut config) = wf.config {
            if !ext.statuses.is_empty() {
                config.statuses = ext.statuses;
            }
            if let Some(prompt) = ext.classifier_prompt {
                config.classifier_prompt = Some(prompt);
            }
        }
    }

    Ok(wf)
}

pub fn find_workflow_by_stem<'a>(
    workflows: &'a [WorkflowDef],
    stem: &str,
) -> Option<&'a WorkflowDef> {
    workflows.iter().find(|wf| wf.file_stem == stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_main_conversation_flow() {
        let path_str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../agents/psychotherapist/transitions/main_conversation_flow.yaml"
        );
        let path = Path::new(path_str);
        assert!(path.exists(), "Файл не найден: {:?}", path);
        let wf = parse_workflow_file(path).expect("Парсинг YAML не удался");
        assert_eq!(wf.name, "Therapist");
        assert!(wf.visible);

        // 12 узлов
        assert_eq!(wf.nodes.len(), 12);
        // 12 рёбер
        assert_eq!(wf.edges.len(), 12);

        // LLM_FACT_EXTRACTOR
        let ef = wf.nodes.iter().find(|n| n.id == "extract_facts").unwrap();
        assert_eq!(ef.node_type, NodeType::LlmFactExtractor);
        assert_eq!(ef.input.as_deref(), Some("{{ user_message }}"));

        // SWITCH с cases_priority (5 кейсов + default)
        let pr = wf.nodes.iter().find(|n| n.id == "priority_router").unwrap();
        assert_eq!(pr.node_type, NodeType::Switch);
        assert!(pr.cases_priority.is_some());
        assert_eq!(pr.cases_priority.as_ref().unwrap().len(), 4);
        assert_eq!(pr.default.as_deref(), Some("freestyle"));

        // SWITCH с обычными cases
        let pr2 = wf.nodes.iter().find(|n| n.id == "protocol_router").unwrap();
        assert_eq!(pr2.node_type, NodeType::Switch);
        assert!(pr2.cases.is_some());
        let cases = pr2.cases.as_ref().unwrap();
        assert_eq!(cases.get("need_more_data").map(String::as_str), Some("ask_missing"));
        assert_eq!(cases.get("ready").map(String::as_str), Some("start_datamining"));

        // LLM_WORKER с agent + task
        let curator = wf.nodes.iter().find(|n| n.id == "call_curator").unwrap();
        assert_eq!(curator.node_type, NodeType::LlmWorker);
        assert_eq!(curator.agent.as_deref(), Some("curator"));
        assert_eq!(curator.output_type.as_deref(), Some("message"));

        // SYSTEM_CONDITION с action + required
        let cond = wf.nodes.iter().find(|n| n.id == "check_protocol").unwrap();
        assert_eq!(cond.node_type, NodeType::SystemCondition);
        assert_eq!(cond.action.as_deref(), Some("check_protocol_state"));
        assert!(cond.required.is_some());
        assert_eq!(cond.required.as_ref().unwrap().len(), 1);

        // SUB_WORKFLOW
        let sub = wf.nodes.iter().find(|n| n.id == "start_datamining").unwrap();
        assert_eq!(sub.node_type, NodeType::SubWorkflow);
        assert_eq!(sub.workflow.as_deref(), Some("treatment_flow.yaml"));

        // LLM_FREEFORM
        let ff = wf.nodes.iter().find(|n| n.id == "freestyle").unwrap();
        assert_eq!(ff.node_type, NodeType::LlmFreeform);

        // Рёбра с case
        let case_edge_1 = wf.edges.iter().find(|e| e.from == "protocol_router" && e.case.as_deref() == Some("need_more_data")).unwrap();
        assert_eq!(case_edge_1.to, "ask_missing");
        let case_edge_2 = wf.edges.iter().find(|e| e.from == "protocol_router" && e.case.as_deref() == Some("ready")).unwrap();
        assert_eq!(case_edge_2.to, "start_datamining");

        // Проверка, что facts.yaml подгрузился
        assert!(wf.config.is_some());
        let cfg = wf.config.as_ref().unwrap();
        assert!(!cfg.facts.is_empty(), "facts.yaml должен был загрузиться");
    }
}
