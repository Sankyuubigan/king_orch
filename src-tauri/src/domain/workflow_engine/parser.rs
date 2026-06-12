use indexmap::IndexMap;
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
    #[serde(skip)]
    pub parent_dir: String,
    #[serde(default)]
    pub config: Option<WorkflowConfig>,
    pub nodes: Vec<NodeDef>,
    pub edges: Vec<EdgeDef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowConfig {
    // --- Новый паттерн: Fact Extractor ---
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<FactDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facts_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractor_prompt: Option<String>,

}

/// Внешний файл фактов (facts.yaml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactsFile {
    #[serde(default)]
    pub extractor_prompt: Option<String>,
    #[serde(default)]
    pub facts: Vec<FactDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactDef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_object: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cases: Option<IndexMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cases_priority: Option<Vec<PriorityCase>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statuses: Option<serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problems: Option<String>,
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
    LlmSequentialSwitch,
    Return,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeDef {
    #[serde(default)]
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<String>,
    #[serde(default)]
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
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
    let parent_dir = path.parent().unwrap_or(Path::new("."));
    let mut wf: WorkflowDef = serde_yaml::from_str(&content)
        .map_err(|e| format!("Ошибка парсинга YAML {}: {}", path.display(), e))?;
    wf.file_stem = file_stem;
    wf.parent_dir = parent_dir.to_string_lossy().to_string();

    // Внешние facts.yaml / statuses.yaml НЕ вливаются в config.facts при парсинге.
    // Они загружаются лениво в fact_extractor::build_extractor_prompt() при выполнении.
    // Это гарантирует, что save_workflow() не запишет дублированные facts в workflow YAML.

    Ok(wf)
}

pub fn find_workflow_by_stem<'a>(
    workflows: &'a [WorkflowDef],
    stem: &str,
) -> Option<&'a WorkflowDef> {
    workflows.iter().find(|wf| wf.file_stem == stem)
}

/// Пост-обработка сериализованного YAML: добавляет 2-пробельный отступ для block sequence
/// (serde_yaml выводит `- item` на том же уровне, что и ключ, что неудобно читать).
///
/// Запускается итеративно до стабилизации, чтобы корректно обработать вложенные sequence.
pub fn indent_block_sequences(yaml: &str) -> String {
    let lines: Vec<&str> = yaml.lines().collect();
    let mut out = String::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        if !trimmed.starts_with('-') && trimmed.ends_with(':') {
            let base_indent = line.len() - trimmed.len();
            if let Some(next) = lines.get(i + 1) {
                let next_trim = next.trim_start();
                if next_trim.starts_with("- ") && (next.len() - next_trim.len()) == base_indent {
                    out.push_str(line);
                    out.push('\n');
                    i += 1;
                    while i < lines.len() {
                        let sub = lines[i];
                        let sub_trim = sub.trim_start();
                        let sub_indent = sub.len() - sub_trim.len();
                        if sub_indent < base_indent
                            || (sub_indent == base_indent && !sub_trim.starts_with('-'))
                        {
                            break;
                        }
                        if sub_trim.is_empty() {
                            out.push('\n');
                        } else {
                            out.push_str("  ");
                            out.push_str(sub);
                            out.push('\n');
                        }
                        i += 1;
                    }
                    continue;
                }
            }
        }

        out.push_str(line);
        out.push('\n');
        i += 1;
    }
    out
}

/// Обёртка: итеративно применяет indent_block_sequences до стабилизации,
/// чтобы корректно выровнять вложенные block sequence (например, cases_priority внутри узла).
pub fn indent_yaml(yaml: &str) -> String {
    let mut current = yaml.to_string();
    loop {
        let next = indent_block_sequences(&current);
        if next == current {
            return current;
        }
        current = next;
    }
}

/// Добавляет переносы строк между полями первого уровня YAML
/// для визуального удобства чтения (поля не слипаются).
pub fn separate_top_level_fields(yaml: &str) -> String {
    let lines: Vec<&str> = yaml.lines().collect();
    let mut result = String::new();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let is_top_level = !line.is_empty()
            && !line.starts_with(' ')
            && !line.starts_with('\t')
            && trimmed.contains(':');

        if i > 0 && is_top_level {
            result.push('\n');
        }

        result.push_str(line);
        result.push('\n');
    }

    result
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

        // 10 узлов
        assert_eq!(wf.nodes.len(), 10);
        // 8 рёбер
        assert_eq!(wf.edges.len(), 8);

        // LLM_FACT_EXTRACTOR
        let ef = wf.nodes.iter().find(|n| n.id == "extract_facts").unwrap();
        assert_eq!(ef.node_type, NodeType::LlmFactExtractor);
        assert_eq!(ef.input.as_deref(), Some("{{ user_message }}"));

        // SWITCH с cases_priority (3 кейса + default)
        let pr = wf.nodes.iter().find(|n| n.id == "priority_router").unwrap();
        assert_eq!(pr.node_type, NodeType::Switch);
        assert!(pr.cases_priority.is_some());
        assert_eq!(pr.cases_priority.as_ref().unwrap().len(), 3);
        assert_eq!(pr.default.as_deref(), Some("freestyle"));

        // SWITCH с пустыми cases (old format)
        let pr2 = wf.nodes.iter().find(|n| n.id == "first_barrier_checker").unwrap();
        assert_eq!(pr2.node_type, NodeType::Switch);
        assert!(pr2.cases.is_some());
        assert!(pr2.cases.as_ref().unwrap().is_empty());

        // LLM_WORKER с agent + task
        let curator = wf.nodes.iter().find(|n| n.id == "call_curator").unwrap();
        assert_eq!(curator.node_type, NodeType::LlmWorker);
        assert_eq!(curator.agent.as_deref(), Some("curator"));
        assert_eq!(curator.output_type.as_deref(), Some("message"));

        // SUB_WORKFLOW
        let sub = wf.nodes.iter().find(|n| n.id == "start_datamining").unwrap();
        assert_eq!(sub.node_type, NodeType::SubWorkflow);
        assert_eq!(sub.workflow.as_deref(), Some("treatment_flow.yaml"));

        // LLM_FREEFORM
        let ff = wf.nodes.iter().find(|n| n.id == "freestyle").unwrap();
        assert_eq!(ff.node_type, NodeType::LlmFreeform);

        // Рёбра с case
        let case_edge_1 = wf.edges.iter().find(|e| e.from == "priority_router" && e.case.as_deref() == Some("user_doesnt_agree")).unwrap();
        assert_eq!(case_edge_1.to, "call_curator");
        let case_edge_2 = wf.edges.iter().find(|e| e.from == "priority_router" && e.case.as_deref() == Some("has_somatic")).unwrap();
        assert_eq!(case_edge_2.to, "call_soma_translator");
        let case_edge_3 = wf.edges.iter().find(|e| e.from == "priority_router" && e.case.as_deref() == Some("not_enough_data")).unwrap();
        assert_eq!(case_edge_3.to, "check_has_grounding");

        // Проверка конфига
        assert!(wf.config.is_some());
        let cfg = wf.config.as_ref().unwrap();
        // facts_file должен быть, но facts теперь не вливаются (ленивая загрузка)
        assert_eq!(cfg.facts_file.as_deref(), Some("facts.yaml"));
        assert!(cfg.facts.is_empty(), "facts больше не вливаются при парсинге");
    }

    /// Тест круглого стола YAML: читаем → десериализуем → сериализуем → показываем разницу.
    /// Этот тест наглядно демонстрирует, какие поля вылезают в YAML после save_workflow().
    #[test]
    fn test_yaml_roundtrip_serialization() {
        let path_str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../agents/psychotherapist/transitions/main_conversation_flow.yaml"
        );
        let path = Path::new(path_str);
        assert!(path.exists(), "Файл не найден: {:?}", path);

        // Читаем оригинальный YAML как строку
        let original_yaml = std::fs::read_to_string(path)
            .expect("Не удалось прочитать файл");

        // 1) Десериализуем напрямую через serde_yaml (БЕЗ parse_workflow_file,
        //    чтобы не было подмешивания external facts.yaml в config.facts)
        let wf: WorkflowDef = serde_yaml::from_str(&original_yaml)
            .expect("Ошибка парсинга YAML через serde_yaml");

        // 2) Сериализуем обратно в YAML
        let serialized = serde_yaml::to_string(&wf)
            .expect("Ошибка сериализации YAML");

        // 3) Применяем indent_yaml для человекочитаемого вывода
        let indented = indent_yaml(&serialized);

        // 4) Печатаем оба варианта
        eprintln!("\n========== ОРИГИНАЛЬНЫЙ YAML ==========");
        eprintln!("{}", original_yaml);
        eprintln!("\n========== ПОСЛЕ СЕРИАЛИЗАЦИИ (indented) =========");
        eprintln!("{}", indented);
        eprintln!("======================================\n");

        // 5) Проверяем «свежий» парсинг indented — не должны потеряться данные
        let wf2: WorkflowDef = serde_yaml::from_str(&indented)
            .unwrap_or_else(|e| panic!("Свежий парсинг упал: {}", e));

        // Сравниваем ключевые поля
        assert_eq!(wf.name, wf2.name, "name различается после round-trip");
        assert_eq!(wf.visible, wf2.visible, "visible различается после round-trip");
        assert_eq!(wf.nodes.len(), wf2.nodes.len(), "количество nodes различается");
        assert_eq!(wf.edges.len(), wf2.edges.len(), "количество edges различается");

        // Проверяем, что каждый узел сохранил id и type
        for n in &wf.nodes {
            let n2 = wf2.nodes.iter().find(|x| x.id == n.id)
                .unwrap_or_else(|| panic!("Узел {} пропал после round-trip", n.id));
            assert_eq!(n.node_type, n2.node_type, "type узла {} изменился", n.id);
            assert_eq!(n.agent, n2.agent, "agent узла {} изменился", n.id);
            assert_eq!(n.task, n2.task, "task узла {} изменился", n.id);
            assert_eq!(n.input, n2.input, "input узла {} изменился", n.id);
            assert_eq!(n.action, n2.action, "action узла {} изменился", n.id);
            assert_eq!(n.workflow, n2.workflow, "workflow узла {} изменился", n.id);
            assert_eq!(n.default, n2.default, "default узла {} изменился", n.id);
            assert_eq!(n.output_type, n2.output_type, "output_type узла {} изменился", n.id);
            assert_eq!(n.cases_priority, n2.cases_priority, "cases_priority узла {} изменился", n.id);
            assert_eq!(n.cases, n2.cases, "cases узла {} изменился", n.id);
        }

        // 6) Проверяем, что нет null/[]/~ мусора
        let null_count = indented.matches(": null").count();
        let empty_list_count = indented.matches(": []").count();
        let tilde_count = indented.matches(": ~").count();
        let total_garbage = null_count + empty_list_count + tilde_count;
        eprintln!(
            "🧹 null: {}, []: {}, ~: {} (всего мусора: {})",
            null_count, empty_list_count, tilde_count, total_garbage
        );
        assert_eq!(
            total_garbage, 0,
            "В indented YAML найден мусор (null/[]/~).\n\
             Это значит, что где-то в struct-ах не хватает skip_serializing_if.\n\
             null={}, []= {}, ~={}",
            null_count, empty_list_count, tilde_count
        );

        // 7) Проверяем indentation block sequence (nodes/edges имеют 2-space indent)
        assert!(
            indented.contains("nodes:\n  - id:"),
            "nodes block sequence должен быть indented на 2 пробела"
        );
        assert!(
            indented.contains("edges:\n  - from:"),
            "edges block sequence должен быть indented на 2 пробела"
        );
        assert!(
            indented.contains("cases_priority:\n      - key:"),
            "nested block sequence cases_priority должен быть indented"
        );
    }

    /// Тест round-trip через parse_workflow_file — проверяет, что внешние facts
    /// больше не вливаются в config.facts и не дублируются в сохранённом YAML.
    #[test]
    fn test_yaml_roundtrip_with_external_facts() {
        let path_str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../agents/psychotherapist/transitions/main_conversation_flow.yaml"
        );
        let path = Path::new(path_str);
        assert!(path.exists());

        let wf = parse_workflow_file(path).expect("Парсинг не удался");
        let serialized = serde_yaml::to_string(&wf).expect("Сериализация не удалась");
        let indented = indent_yaml(&serialized);

        eprintln!("\n========== ПОСЛЕ parse_workflow_file + indent =========");
        eprintln!("{}", indented);

        // facts_file должен быть, но facts не должен дублироваться
        assert!(
            !indented.contains("\nfacts:"),
            "config.facts не должен появляться в сохранённом YAML!\n\
             Внешние facts должны оставаться только в facts.yaml."
        );

        // Проверяем что нет null мусора
        let null_count = indented.matches(": null").count();
        assert_eq!(null_count, 0, "В сериализованном YAML есть null поля!");

        // Проверяем что parent_dir не попал в YAML
        assert!(!indented.contains("parent_dir"), "parent_dir должен быть skip");
    }
}
