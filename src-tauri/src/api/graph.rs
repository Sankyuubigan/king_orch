use serde::Serialize;
use std::fs;
use std::path::Path;

use crate::domain::workflow_engine::parser::{
    indent_yaml, separate_top_level_fields, EdgeDef, NodeDef, WorkflowConfig,
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

/// Загружает один YAML-файл workflow по полному пути
#[tauri::command]
pub fn read_workflow_file(path: String) -> Result<GraphWorkflowDef, String> {
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Ошибка чтения файла {}: {}", path, e))?;
    let file_stem = Path::new(&path)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut wf: crate::domain::workflow_engine::parser::WorkflowDef =
        serde_yaml::from_str(&content)
            .map_err(|e| format!("Ошибка парсинга YAML: {}", e))?;
    wf.file_stem = file_stem.clone();

    Ok(GraphWorkflowDef {
        team: String::new(),
        name: wf.name,
        file_stem,
        visible: wf.visible,
        config: wf.config,
        nodes: wf.nodes,
        edges: wf.edges,
    })
}

/// Сохраняет workflow в указанный файл (полный путь)
#[tauri::command]
pub fn save_workflow(
    app: tauri::AppHandle,
    path: String,
    workflow: crate::domain::workflow_engine::parser::WorkflowDef,
) -> Result<(), String> {
    let _ = &app;

    let yaml_str = serde_yaml::to_string(&workflow)
        .map_err(|e| format!("Ошибка сериализации YAML: {}", e))?;
    let yaml_indented = indent_yaml(&yaml_str);
    let yaml_separated = separate_top_level_fields(&yaml_indented);
    fs::write(&path, &yaml_separated)
        .map_err(|e| format!("Ошибка записи файла {}: {}", path, e))?;
    Ok(())
}
