use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

use crate::domain::workflow_engine::parser::{
    EdgeDef, load_workflows, NodeDef, WorkflowConfig,
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
    #[serde(default)]
    pub visible_agents: Vec<String>,
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
                        visible_agents: wf.visible_agents.clone(),
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
