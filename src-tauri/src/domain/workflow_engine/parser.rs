use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// ===========================================================================
// СЕРИАЛИЗАЦИЯ / СОХРАНЕНИЕ WORKFLOW (save_workflow)
// ===========================================================================
// Конвейер сохранения НАМЕРЕННО минимален и надёжен:
//
//     serde_yaml::to_string(&wf)  ->  separate_top_level_fields  ->  validate  ->  write
//
// Нативный вывод serde_yaml УЖЕ является валидным YAML, который корректно
// round-trip'ится (блочный sequence-элемент на отступе своего ключа — легален
// по спеке YAML). Любые строковые «докрутки» отступов (indent_block_sequences,
// protect_block_scalars и т.п.) были УДАЛЕНЫ как источник коррупции: они
// сдвигали вложенные последовательности под неверного родителя и портили
// block scalar (input/task), из-за чего файл становился невалидным
// ("did not find expected '-' indicator").
//
// separate_top_level_fields — единственная пост-обработка: только вставляет
// пустые строки между полями верхнего уровня (безопасно, не влияет на парсинг).
// Финальная валидация (re-parse) оставлена как последняя линия защиты:
// если сгенерированный YAML не парсится — файл НЕ записывается.
// ===========================================================================

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
    // --- Fact Extractor ---
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<FactDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<FactDef>,
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
    #[serde(default)]
    pub phases: Vec<FactDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactDef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriorityCase {
    pub key: String,
    #[serde(rename = "to")]
    pub to: String,
}

/// Защитный десериалайзер: принимает и массив `[{key, to}, ...]` и мапу `{key: to}`
fn deserialize_cases_priority<'de, D>(deserializer: D) -> Result<Option<Vec<PriorityCase>>, D::Error>
where
    D: Deserializer<'de>,
{
    // Пробуем как value, чтобы потом разобрать формат
    let val: serde_json::Value = match serde_json::Value::deserialize(deserializer) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    match val {
        serde_json::Value::Array(arr) => {
            let cases: Vec<PriorityCase> = arr
                .iter()
                .filter_map(|v| {
                    let key = v.get("key")?.as_str()?.to_string();
                    let to = v.get("to")?.as_str()?.to_string();
                    Some(PriorityCase { key, to })
                })
                .collect();
            Ok(Some(cases))
        }
        serde_json::Value::Object(map) => {
            let cases: Vec<PriorityCase> = map
                .iter()
                .map(|(key, val)| PriorityCase {
                    key: key.clone(),
                    to: val.as_str().unwrap_or("").to_string(),
                })
                .collect();
            Ok(Some(cases))
        }
        _ => Ok(None),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    pub switch_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequential_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub true_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub false_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_cases_priority")]
    pub cases_priority: Option<Vec<PriorityCase>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statuses: Option<serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inject_reports: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problems: Option<String>,
    /// Текст, выводимый в чат пользователя как системное сообщение (независим от заметки `input`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
    /// Визуальные координаты для редактора графов (x, y)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_pos: Option<HashMap<String, i32>>,
    /// Нода отключена (не выполняется при активации workflow)
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
}

fn is_false(b: &bool) -> bool { !b }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum NodeType {
    #[default]
    LlmWorker,
    LlmFactExtractor,
    LlmFreeform,
    SystemCondition,
    SubWorkflow,
    Switch,
    LlmSequentialSwitch,
    ConditionCheck,
    Return,
    #[serde(rename = "signal_router")]
    SignalRouter,
    Note,
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
///
/// Функция ПОЛНОСТЬЮ игнорирует содержимое block scalar (`|-`, `|`, `>-`, `>`):
/// serde_yaml всегда выводит его строго глубже ключа (indent ключа + 2), поэтому
/// любая строка на отступе > ключа внутри блока — это литеральное содержимое,
/// которое трогать нельзя. Блок закрывается, когда встречается строка на
/// отступе <= отступа ключа (sibling-ключ или родитель) — она уже не может быть
/// содержимым блока. Такое разделение однозначно и не ломает файл.
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
        assert_eq!(wf.name, "Психотерапевт");
        assert!(wf.visible);
        assert!(wf.nodes.len() > 0, "узлы должны быть");
        assert!(wf.edges.len() > 0, "рёбра должны быть");

        // Факт-экстрактор присутствует и содержит шаблоны.
        let ef = wf.nodes.iter().find(|n| n.id == "extract_facts").unwrap();
        assert_eq!(ef.node_type, NodeType::LlmFactExtractor);
        assert!(ef.input.is_some());
        assert!(ef.input.as_deref().unwrap_or("").contains("{{ user_message }}"));

        // Проверка конфига
        assert!(wf.config.is_some());
        let cfg = wf.config.as_ref().unwrap();
        assert_eq!(cfg.facts_file.as_deref(), Some("facts.yaml"));
        assert!(cfg.facts.is_empty(), "facts больше не вливаются при парсинге");

        // Ключевая проверка бага: реальный файл графа должен проходить
        // надёжный конвейер save_workflow (нативный serde_yaml + безопасные
        // разделители) и обратно парситься, ПРИЧЁМ структура
        // (узлы, рёбра, вложенные поля ui_pos/inject_reports, тексты
        // task/input) должна совпасть побайтово. Этот тест ловит
        // «валидный, но сломанный» YAML (напр. inject_reports,
        // уехавший внутрь task, или ui_pos, потерявший отступ).
        let yaml_str = serde_yaml::to_string(&wf).expect("ser");
        let yaml_final = separate_top_level_fields(&yaml_str);
        let wf2: WorkflowDef = serde_yaml::from_str(&yaml_final)
            .expect("❌ Реальный граф не прошёл конвейер save_workflow!");
        assert_eq!(wf2.nodes.len(), wf.nodes.len(), "число узлов");
        assert_eq!(wf2.edges.len(), wf.edges.len(), "число рёбер");

        // Побайтовая сверка каждого узла.
        for n1 in &wf.nodes {
            let n2 = wf2.nodes.iter().find(|n| n.id == n1.id)
                .unwrap_or_else(|| panic!("узел {} потерян после round-trip", n1.id));
            assert_eq!(n1.node_type, n2.node_type, "type узла {}", n1.id);
            assert_eq!(n1.agent, n2.agent, "agent узла {}", n1.id);
            assert_eq!(n1.task, n2.task, "task узла {}", n1.id);
            assert_eq!(n1.input, n2.input, "input узла {}", n1.id);
            assert_eq!(n1.output_type, n2.output_type, "output_type узла {}", n1.id);
            assert_eq!(n1.inject_reports, n2.inject_reports, "inject_reports узла {}", n1.id);
            // ui_pos должен парситься как карта, а не слететь внутрь task.
            assert_eq!(n1.ui_pos, n2.ui_pos, "ui_pos узла {}", n1.id);
            assert!(n2.ui_pos.as_ref().map_or(false, |m| m.len() == 2),
                "ui_pos узла {} должен содержать ровно x и y", n1.id);
        }
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

        // 3) Надёжный конвейер save_workflow: только безопасные разделители.
        let out = separate_top_level_fields(&serialized);

        // 4) Печатаем оба варианта
        eprintln!("\n========== ОРИГИНАЛЬНЫЙ YAML ==========");
        eprintln!("{}", original_yaml);
        eprintln!("\n========== ПОСЛЕ СЕРИАЛИЗАЦИИ (save_workflow) =========");
        eprintln!("{}", out);
        eprintln!("======================================\n");

        // 5) Проверяем «свежий» парсинг out — не должны потеряться данные
        let wf2: WorkflowDef = serde_yaml::from_str(&out)
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
        }

        // 6) Проверяем, что нет null/[]/~ мусора
        let null_count = out.matches(": null").count();
        let empty_list_count = out.matches(": []").count();
        let tilde_count = out.matches(": ~").count();
        let total_garbage = null_count + empty_list_count + tilde_count;
        eprintln!(
            "🧹 null: {}, []: {}, ~: {} (всего мусора: {})",
            null_count, empty_list_count, tilde_count, total_garbage
        );
        assert_eq!(
            total_garbage, 0,
            "В сохранённом YAML найден мусор (null/[]/~).\n\
             Это значит, что где-то в struct-ах не хватает skip_serializing_if.\n\
             null={}, []= {}, ~={}",
            null_count, empty_list_count, tilde_count
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
        let out = separate_top_level_fields(&serialized);

        eprintln!("\n========== ПОСЛЕ parse_workflow_file + save_workflow =========");
        eprintln!("{}", out);

        // facts_file должен быть, но facts не должен дублироваться
        assert!(
            !out.contains("\nfacts:"),
            "config.facts не должен появляться в сохранённом YAML!\n\
             Внешние facts должны оставаться только в facts.yaml."
        );

        // Проверяем что нет null мусора
        let null_count = out.matches(": null").count();
        assert_eq!(null_count, 0, "В сериализованном YAML есть null поля!");

        // Проверяем что parent_dir не попал в YAML
        assert!(!out.contains("parent_dir"), "parent_dir должен быть skip");
    }

    #[test]
    fn test_note_node_yaml_roundtrip() {
        let yaml = r#"
name: Test Note
nodes:
  - id: note_1
    type: note
    input: "Важная заметка о соматике"
    ui_pos:
      x: 100
      y: 200
edges: []
"#;
        let wf: WorkflowDef = serde_yaml::from_str(yaml)
            .expect("Парсинг YAML с note нодой не удался");
        assert_eq!(wf.nodes.len(), 1);
        let note = &wf.nodes[0];
        assert_eq!(note.id, "note_1");
        assert_eq!(note.node_type, NodeType::Note);
        assert_eq!(note.input.as_deref(), Some("Важная заметка о соматике"));
        assert!(note.ui_pos.is_some());
        assert_eq!(note.ui_pos.as_ref().unwrap().get("x"), Some(&100));
        assert_eq!(note.ui_pos.as_ref().unwrap().get("y"), Some(&200));

        // Сериализация (проверяем через десериализацию обратно)
        let serialized = serde_yaml::to_string(&wf).expect("Сериализация не удалась");
        let out = separate_top_level_fields(&serialized);

        eprintln!("=== Serialized note YAML ===\n{}", out);

        let wf2: WorkflowDef = serde_yaml::from_str(&out)
            .expect("Повторный парсинг note YAML не удался");
        let note2 = &wf2.nodes[0];
        assert_eq!(note2.input.as_deref(), Some("Важная заметка о соматике"));

        // Проверяем отсутствие неиспользуемых полей
        assert!(!out.contains("agent:"), "agent не должно быть в note ноде");
        assert!(!out.contains("task:"), "task не должно быть в note ноде");
    }

    #[test]
    fn test_save_workflow_pipeline_block_scalar_indented_line() {
        // Нативный serde_yaml сам корректно форматирует block scalar с
        // отступом внутри (вызов инструмента emit_signal) — без всяких
        // строковых хаков. Проверяем, что надёжный конвейер save_workflow
        // (to_string -> separate_top_level_fields) даёт валидный YAML
        // и текст input совпадает после round-trip.
        let input = "\
среди мишеней по математике переходов определи корневую мишень.
    Затем, тебе надо сохранить тип деструктора.
    emit_signal(\"destructor_type\", \"[номер]\")";

        let node = NodeDef {
            id: "combining_targets".to_string(),
            node_type: NodeType::LlmFreeform,
            input: Some(input.to_string()),
            ..Default::default()
        };
        let wf = WorkflowDef {
            name: "Test Freeform".to_string(),
            visible: true,
            file_stem: String::new(),
            parent_dir: String::new(),
            config: None,
            nodes: vec![node],
            edges: vec![],
        };

        let serialized = serde_yaml::to_string(&wf).expect("Сериализация не удалась");
        let out = separate_top_level_fields(&serialized);

        eprintln!("=== Save workflow YAML ===\n{}", out);

        // Ключевая проверка: сгенерированный YAML обратно парсится без ошибок.
        let wf2: WorkflowDef = serde_yaml::from_str(&out)
            .expect("❌ Сгенерированный YAML с отступом внутри input невалиден!");
        assert_eq!(wf2.nodes.len(), 1);
        assert_eq!(
            wf2.nodes[0].input.as_deref(),
            Some(input),
            "Текст input должен совпадать после round-trip"
        );
    }

    #[test]
    fn test_save_workflow_pipeline_roundtrip() {
        // Надёжный конвейер save_workflow: to_string -> separate_top_level_fields.
        // Раньше сюда добавлялись строковые трансформеры отступов, которые
        // ломали YAML ("could not find expected ':'"). Проверяем, что
        // нативный вывод сериализуется и обратно парсится без потерь.
        let input = "среди мишеней по математике переходов определи корневую мишень, откуда произрастают все остальные.\n\nЗатем, тебе надо сохранить тип деструктора (убеждения) у корневой мишени. Вызови инструмент emit_signal с результатами анализа:\n\nemit_signal(\"destructor_type\", \"[номер цифру от 1 до 9 включительно]\")";
        let node = NodeDef {
            id: "combining_targets".to_string(),
            node_type: NodeType::LlmFreeform,
            input: Some(input.to_string()),
            ..Default::default()
        };
        let wf = WorkflowDef {
            name: "Test Freeform".to_string(),
            visible: true,
            file_stem: String::new(),
            parent_dir: String::new(),
            config: None,
            nodes: vec![node],
            edges: vec![],
        };

        let yaml_str = serde_yaml::to_string(&wf).expect("ser");
        let yaml_final = separate_top_level_fields(&yaml_str);

        let wf2: WorkflowDef = serde_yaml::from_str(&yaml_final)
            .expect("❌ Пайплайн save_workflow сгенерировал невалидный YAML!");
        assert_eq!(wf2.nodes.len(), 1);
        assert_eq!(
            wf2.nodes[0].input.as_deref(),
            Some(input),
            "Текст input должен совпадать после round-trip через полный пайплайн"
        );
    }

    #[test]
    fn test_save_workflow_pipeline_block_scalar_with_dash() {
        // Block scalar (task/input), чьё содержимое начинается с "- "
        // (маркированный список). Раньше строковый трансформер ошибочно
        // трактовал это как YAML-последовательность и «съедал» строки файла
        // ("did not find expected '-' indicator"). Нативный serde_yaml
        // без хаков обрабатывает это корректно.
        let task = "- определи корневую мишень среди мишеней по математике переходов\n- сохрани тип деструктора (убеждения) у корневой мишени\n- вызови emit_signal(\"destructor_type\", \"[цифра 1-9]\")";

        let node1 = NodeDef {
            id: "analyzer".to_string(),
            node_type: NodeType::LlmWorker,
            agent: Some("some_agent".to_string()),
            task: Some(task.to_string()),
            ..Default::default()
        };
        // Второй узел — чтобы убедиться, что строки ПОСЛЕ блока не «съедаются».
        let node2 = NodeDef {
            id: "emitter".to_string(),
            node_type: NodeType::SignalRouter,
            signal_name: Some("destructor_type".to_string()),
            ..Default::default()
        };
        let wf = WorkflowDef {
            name: "Test Graph".to_string(),
            visible: true,
            file_stem: String::new(),
            parent_dir: String::new(),
            config: None,
            nodes: vec![node1, node2],
            edges: vec![],
        };

        let yaml_str = serde_yaml::to_string(&wf).expect("ser");
        let yaml_final = separate_top_level_fields(&yaml_str);

        let wf2: WorkflowDef = serde_yaml::from_str(&yaml_final)
            .expect("❌ Пайплайн save_workflow сгенерировал невалидный YAML!");
        assert_eq!(wf2.nodes.len(), 2, "Второй узел не должен быть потерян");
        assert_eq!(
            wf2.nodes[0].task.as_deref(),
            Some(task),
            "Текст task должен совпадать после round-trip"
        );
        assert_eq!(wf2.nodes[1].id, "emitter");
    }

    #[test]
    fn debug_real_workflow_pipeline() {
        let path_str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../agents/psychotherapist/transitions/main_conversation_flow.yaml"
        );
        let content = std::fs::read_to_string(path_str).expect("read");
        let wf: WorkflowDef = serde_yaml::from_str(&content).expect("parse original");

        let yaml_str = serde_yaml::to_string(&wf).expect("ser");
        let yaml_final = separate_top_level_fields(&yaml_str);

        match serde_yaml::from_str::<WorkflowDef>(&yaml_final) {
            Ok(_) => eprintln!("REAL WORKFLOW OK"),
            Err(e) => {
                eprintln!("=== REAL WORKFLOW INVALID: {} ===", e);
                eprintln!("{}", yaml_final);
                panic!("invalid");
            }
        }
    }

    #[test]
    fn debug_repro_large_graph() {
        let mut nodes = Vec::new();
        // Узел с task, содержащим маркированный список (буллеты с "- ")
        nodes.push(NodeDef {
            id: "analyzer".to_string(),
            node_type: NodeType::LlmWorker,
            agent: Some("analyst".to_string()),
            task: Some(
                "среди мишеней по математике переходов определи корневую мишень:\n- определи корневую мишень\n- сохрани тип деструктора (убеждения)\n- вызови emit_signal(\"destructor_type\", \"[1-9]\")".to_string(),
            ),
            ..Default::default()
        });
        // Switch с cases_priority
        nodes.push(NodeDef {
            id: "router".to_string(),
            node_type: NodeType::Switch,
            switch_field: Some("phase".to_string()),
            cases_priority: Some(vec![
                PriorityCase { key: "phase_1".to_string(), to: "analyzer".to_string() },
                PriorityCase { key: "phase_2".to_string(), to: "emitter".to_string() },
            ]),
            default: Some("analyzer".to_string()),
            ..Default::default()
        });
        // input с шаблоном {{ }}
        nodes.push(NodeDef {
            id: "extractor".to_string(),
            node_type: NodeType::LlmFactExtractor,
            input: Some("Проанализируй:\n{{ user_message }}\nсигналы: {{ signals }}".to_string()),
            ..Default::default()
        });
        // note с многострочным текстом
        nodes.push(NodeDef {
            id: "note1".to_string(),
            node_type: NodeType::Note,
            task: Some("заметка\n- пункт а\n- пункт б".to_string()),
            ..Default::default()
        });
        // signal_router
        nodes.push(NodeDef {
            id: "emitter".to_string(),
            node_type: NodeType::SignalRouter,
            signal_name: Some("destructor_type".to_string()),
            ..Default::default()
        });
        // sub_workflow
        nodes.push(NodeDef {
            id: "sub1".to_string(),
            node_type: NodeType::SubWorkflow,
            workflow: Some("other_flow".to_string()),
            ..Default::default()
        });
        // condition_check
        nodes.push(NodeDef {
            id: "cond1".to_string(),
            node_type: NodeType::ConditionCheck,
            field: Some("phase".to_string()),
            ..Default::default()
        });
        // return
        nodes.push(NodeDef {
            id: "ret1".to_string(),
            node_type: NodeType::Return,
            ..Default::default()
        });
        // llm_sequential_switch
        nodes.push(NodeDef {
            id: "seq1".to_string(),
            node_type: NodeType::LlmSequentialSwitch,
            ..Default::default()
        });
        // system_condition
        nodes.push(NodeDef {
            id: "sys1".to_string(),
            node_type: NodeType::SystemCondition,
            action: Some("do_thing".to_string()),
            ..Default::default()
        });
        // llm_freeform с task
        nodes.push(NodeDef {
            id: "free1".to_string(),
            node_type: NodeType::LlmFreeform,
            input: Some("свободная форма\n- один\n- два".to_string()),
            ..Default::default()
        });

        let wf = WorkflowDef {
            name: "Repro Graph".to_string(),
            visible: true,
            file_stem: String::new(),
            parent_dir: String::new(),
            config: Some(WorkflowConfig {
                facts_file: Some("facts.yaml".to_string()),
                extractor_prompt: Some("извлеки факты".to_string()),
                ..Default::default()
            }),
            nodes,
            edges: vec![
                EdgeDef { from: "extractor".to_string(), to: "router".to_string(), case: None, condition: None },
                EdgeDef { from: "router".to_string(), to: "analyzer".to_string(), case: Some("phase_1".to_string()), condition: None },
                EdgeDef { from: "router".to_string(), to: "emitter".to_string(), case: Some("phase_2".to_string()), condition: None },
            ],
        };

        let yaml_str = serde_yaml::to_string(&wf).expect("ser");
        let yaml_final = separate_top_level_fields(&yaml_str);

        let wf2: WorkflowDef = serde_yaml::from_str(&yaml_final)
            .expect("❌ РЕПРО: полный граф сгенерировал невалидный YAML!");
        assert_eq!(wf2.nodes.len(), 11, "все узлы должны сохраниться");
    }

    #[test]
    fn debug_stage_dump() {
        let input = "среди мишеней по математике переходов определи корневую мишень, откуда произрастают все остальные.\n\nЗатем, тебе надо сохранить тип деструктора (убеждения) у корневой мишени. Вызови инструмент emit_signal с результатами анализа:\n\nemit_signal(\"destructor_type\", \"[номер цифру от 1 до 9 включительно]\")";
        let node = NodeDef {
            id: "combining_targets".to_string(),
            node_type: NodeType::LlmFreeform,
            input: Some(input.to_string()),
            ..Default::default()
        };
        let wf = WorkflowDef {
            name: "Test Freeform".to_string(),
            visible: true,
            file_stem: String::new(),
            parent_dir: String::new(),
            config: None,
            nodes: vec![node],
            edges: vec![],
        };
        let yaml_str = serde_yaml::to_string(&wf).expect("ser");
        let yaml_final = separate_top_level_fields(&yaml_str);
        let wf2: WorkflowDef = serde_yaml::from_str(&yaml_final)
            .expect("❌ Пайплайн save_workflow сгенерировал невалидный YAML!");
        assert_eq!(wf2.nodes[0].input.as_deref(), Some(input));
    }
}
