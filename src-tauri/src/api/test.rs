use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use std::fs;

use crate::domain;
use crate::infra::{self, ChatMessage, LlamaEngine, ModelParams};

#[derive(Debug, Deserialize, Serialize)]
pub struct TestCaseDef {
    pub input_data: String,
    pub right_answer_context: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SingleTestResult {
    pub input_data: String,
    pub right_answer_context: String,
    pub responses: HashMap<String, String>,
}

#[tauri::command]
pub fn read_test_file(path: String) -> Result<Vec<TestCaseDef>, String> {
    let content = fs::read_to_string(&path).map_err(|e| format!("Ошибка чтения файла: {}", e))?;
    let parsed: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| format!("Ошибка парсинга YAML: {}", e))?;
    let cases = parsed.get("cases").and_then(|v| v.as_sequence()).ok_or("YAML не содержит поле 'cases'")?;
    let mut result = Vec::new();
    for c in cases {
        let input_data = c.get("inputData").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let right_answer_context = c.get("rightAnswerContext").and_then(|v| v.as_str()).unwrap_or("").to_string();
        result.push(TestCaseDef { input_data, right_answer_context });
    }
    Ok(result)
}

#[tauri::command]
pub fn write_test_results(results: Vec<SingleTestResult>, path: String) -> Result<(), String> {
    let yaml = serde_yaml::to_string(&results).map_err(|e| format!("Ошибка сериализации: {}", e))?;
    fs::write(&path, &yaml).map_err(|e| format!("Ошибка записи файла: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn run_iterative_test(
    app: AppHandle,
    test_cases: Vec<TestCaseDef>,
    agent_ids: Vec<String>,
    model_paths: Vec<String>,
) -> Result<Vec<SingleTestResult>, String> {
    let agents_dir = infra::find_agents_dir(&app);
    let all_agents = domain::load_agents(&agents_dir).map_err(|e| format!("Ошибка загрузки агентов: {}", e))?;
    let cancel_flag = Arc::new(AtomicBool::new(false));

    let mut results: Vec<SingleTestResult> = Vec::new();

    for tc in &test_cases {
        let mut result = SingleTestResult {
            input_data: tc.input_data.clone(),
            right_answer_context: tc.right_answer_context.clone(),
            responses: HashMap::new(),
        };

        for model_path in &model_paths {
            let engine = LlamaEngine::new(model_path, 8192, false, &|msg| {
                let _ = app.emit("log", format!("[test] {}", msg));
            })?;

            for agent_id in &agent_ids {
                if let Some(agent) = all_agents.iter().find(|a| a.id == *agent_id) {
                    let system_prompt = domain::build_system_prompt(
                        agent,
                        &[],
                        "main",
                        false,
                        &[],
                    );

                    let llm_messages = vec![
                        ChatMessage {
                            id: None,
                            msg_type: "message".to_string(),
                            content: system_prompt,
                            namespace: None,
                            sub_calls: None,
                            author: Some("system".to_string()),
                        },
                        ChatMessage {
                            id: None,
                            msg_type: "message".to_string(),
                            content: tc.input_data.clone(),
                            namespace: None,
                            sub_calls: None,
                            author: Some("user".to_string()),
                        },
                    ];

                    let key = format!("{}_{}", model_path.split('/').last().unwrap_or(model_path).replace(".gguf", ""), agent_id);

                    match engine.generate_chat(
                        &llm_messages,
                        4096,
                        &ModelParams::default(),
                        "Auto",
                        cancel_flag.clone(),
                        |_, _| {},
                        |_| {},
                    ) {
                        Ok(response) => {
                            result.responses.insert(key, response);
                        }
                        Err(e) => {
                            result.responses.insert(key, format!("ERROR: {}", e));
                        }
                    }
                }
            }
        }

        results.push(result);
    }

    let results_dir = get_results_dir(&app);
    let timestamp = chrono_now();
    let filename = format!("test_results_{}.yaml", timestamp);
    let filepath = results_dir.join(&filename);
    if let Ok(yaml) = serde_yaml::to_string(&results) {
        let _ = fs::write(&filepath, &yaml);
        let _ = app.emit("log", format!("[test] Результаты сохранены: {}", filepath.display()));
    }

    Ok(results)
}

fn get_results_dir(app: &AppHandle) -> PathBuf {
    let base = app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."));
    let dir = base.join("test_results");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn chrono_now() -> String {
    // Simple timestamp without chrono dependency
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;
    format!("{:04}-{:02}-{:02}_{:02}{:02}{:02}", 1970 + (days / 365) as u32, 1 + ((days % 365) / 30) as u32, 1 + (days % 30) as u32, hours, minutes, seconds)
}
