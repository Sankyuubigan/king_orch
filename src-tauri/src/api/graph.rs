use serde::Serialize;
use std::fs;
use std::path::Path;

use crate::domain::{
    analyze_workflow_fidelity, separate_top_level_fields, EdgeDef, FactsFile, GraphDiagnostic,
    NodeDef, WorkflowConfig, WorkflowDef,
};

/// Workflow со включённым file_stem + team
#[derive(Debug, Clone, Serialize)]
pub struct GraphWorkflowDef {
    pub team: String,
    pub name: String,
    pub file_stem: String,
    pub visible: bool,
    pub config: Option<WorkflowConfig>,
    pub nodes: Vec<NodeDef>,
    pub edges: Vec<EdgeDef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphWorkflowReadResult {
    pub workflow: GraphWorkflowDef,
    pub diagnostics: Vec<GraphDiagnostic>,
}

/// Загружает один YAML-файл workflow по полному пути
#[tauri::command]
pub fn read_workflow_file(path: String) -> Result<GraphWorkflowReadResult, String> {
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Ошибка чтения файла {}: {}", path, e))?;
    let file_stem = Path::new(&path)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let source: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|e| format!("Ошибка парсинга YAML: {}", e))?;
    let mut wf: WorkflowDef = serde_yaml::from_value(source.clone())
        .map_err(|e| format!("Ошибка парсинга YAML: {}", e))?;
    let diagnostics = analyze_workflow_fidelity(&source, &wf)?;
    wf.file_stem = file_stem.clone();

    // Если facts пуст, но указан facts_file — загружаем факты для отображения во фронтенде
    if let Some(ref mut config) = wf.config {
        if config.facts.is_empty() {
            if let Some(ref facts_file) = config.facts_file {
                let workflow_dir = Path::new(&path).parent().unwrap_or(Path::new("."));
                let ext_path = workflow_dir.join(facts_file);
                if let Ok(content) = fs::read_to_string(&ext_path) {
                    if let Ok(ext) = serde_yaml::from_str::<FactsFile>(&content) {
                        config.facts = ext.facts;
                    }
                }
            }
        }
    }

    Ok(GraphWorkflowReadResult {
        workflow: GraphWorkflowDef {
            team: String::new(),
            name: wf.name,
            file_stem,
            visible: wf.visible,
            config: wf.config,
            nodes: wf.nodes,
            edges: wf.edges,
        },
        diagnostics,
    })
}

/// Сохраняет workflow в указанный файл (полный путь)
#[tauri::command]
pub fn save_workflow(
    app: tauri::AppHandle,
    path: String,
    mut workflow: WorkflowDef,
) -> Result<(), String> {
    let _ = &app;

    // Факты не должны дублироваться в workflow YAML — они живут в отдельном facts.yaml
    if let Some(ref config) = workflow.config {
        if config.facts_file.is_some() {
            if let Some(ref mut cfg) = workflow.config {
                cfg.facts = Vec::new();
            }
        }
    }

    let yaml_str = serde_yaml::to_string(&workflow)
        .map_err(|e| format!("Ошибка сериализации YAML: {}", e))?;
    // Нативный вывод serde_yaml уже валиден и корректно round-trip'ится
    // (блочный sequence-айтем на отступе ключа — легален по спеке YAML).
    // НКАКИХ строковых трансформеров отступов: они были источником
    // коррупции (сдвигали вложенные последовательности под неверного
    // родителя). Только безопасная косметика — пустые строки между
    // полями верхнего уровня (не влияет на парсинг).
    let yaml_final = separate_top_level_fields(&yaml_str);

    // Валидация: сгенерированный YAML должен обратно парситься.
    // Если нет — файл НЕ записываем (иначе данные потеряются).
    serde_yaml::from_str::<WorkflowDef>(&yaml_final)
        .map_err(|e| format!("❌ Сгенерированный YAML невалиден, файл НЕ сохранён: {}", e))?;

    fs::write(&path, &yaml_final)
        .map_err(|e| format!("Ошибка записи файла {}: {}", path, e))?;
    Ok(())
}
