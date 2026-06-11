use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

use crate::domain::workflow_engine::parser::{
    indent_yaml, load_workflows, separate_top_level_fields, EdgeDef, NodeDef, WorkflowConfig,
};

/// Ответ для визуализатора графов
#[derive(Debug, Serialize)]
pub struct GraphResponse {
    pub teams: Vec<String>,
    pub workflows: Vec<GraphWorkflowDef>,
    pub agents: Vec<GraphAgent>,
}

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

/// Агент с указанием команды
#[derive(Debug, Serialize)]
pub struct GraphAgent {
    pub team: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub is_hidden: bool,
    pub mode: String,
}

#[tauri::command]
pub fn get_workflow_graphs(app: tauri::AppHandle) -> Result<GraphResponse, String> {
    let agents_dir = find_agents_dir(&app);
    let mut workflow_defs = Vec::new();
    let mut agent_defs = Vec::new();
    let mut teams = Vec::new();

    // Сканируем поддиректории agents/ (каждая — команда агентов)
    if let Ok(entries) = fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let team_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            teams.push(team_name.clone());

            // Загружаем workflow этой команды
            if let Ok(wfs) = load_workflows(&path) {
                for wf in wfs {
                    workflow_defs.push(GraphWorkflowDef {
                        team: team_name.clone(),
                        name: wf.name.clone(),
                        file_stem: wf.file_stem.clone(),
                        visible: wf.visible,
                        config: wf.config.clone(),
                        nodes: wf.nodes.clone(),
                        edges: wf.edges.clone(),
                    });
                }
            }

            // Загружаем агентов этой команды
            if let Ok(agents) = crate::domain::load_agents(&path) {
                for a in agents {
                    agent_defs.push(GraphAgent {
                        team: team_name.clone(),
                        id: a.id,
                        name: a.name,
                        description: a.description,
                        system_prompt: a.system_prompt,
                        is_hidden: a.is_hidden,
                        mode: a.mode,
                    });
                }
            }
        }
    }

    Ok(GraphResponse {
        teams,
        workflows: workflow_defs,
        agents: agent_defs,
    })
}

fn find_agents_dir(app: &tauri::AppHandle) -> PathBuf {
    let exe_dir = app
        .path()
        .executable_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let resource_dir = app
        .path()
        .resource_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    for dir in [
        exe_dir.join("agents"),
        resource_dir.join("agents"),
        PathBuf::from("agents"),
        exe_dir.join("..").join("..").join("agents"),
    ] {
        if dir.exists() {
            return dir;
        }
    }
    let default = exe_dir.join("agents");
    let _ = fs::create_dir_all(&default);
    default
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
