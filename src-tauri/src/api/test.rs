use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

use crate::domain;
use crate::infra::{self, ChatMessage, SubCall};
use crate::api::AppState;

#[derive(Serialize, Deserialize, Clone)]
pub struct TestCaseDef {
    pub input_data: String,
    pub right_answer_context: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SingleTestResult {
    pub input_data: String,
    pub right_answer_context: String,
    pub responses: Vec<String>, // Ответы от разных моделей/агентов
    pub succeeded: bool,
    pub error: Option<String>,
    pub time_ms: u64,
}

// ─── Лог-файл последнего запуска ───
static LAST_TEST_LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn init_test_log_file() {
    let path = std::path::PathBuf::from("test").join("last_test_log.txt");
    let _ = std::fs::create_dir_all("test");
    if let Ok(file) = std::fs::File::create(&path) {
        if let Ok(mut guard) = LAST_TEST_LOG_FILE.lock() {
            *guard = Some(file);
        }
    }
}

fn append_test_log(msg: &str) {
    if let Ok(mut guard) = LAST_TEST_LOG_FILE.lock() {
        if let Some(ref mut file) = *guard {
            let _ = writeln!(file, "{}", msg);
        }
    }
}

#[tauri::command]
pub async fn run_iterative_test(
    app: AppHandle,
    state: State<'_, AppState>,
    model_path: String,
    agent_id: String,
    test_cases: Vec<TestCaseDef>,
    config: infra::AppConfig,
) -> Result<Vec<SingleTestResult>, String> {
    init_test_log_file();
    append_test_log(&format!(
        "--- Запуск теста для агента '{}' с моделью '{}' ---",
        agent_id, model_path
    ));

    let mut results = Vec::new();
    let agents_dir = infra::find_agents_dir(&app);
    let mcp_servers_dir = infra::find_mcp_servers_dir(&app);
    let agents = domain::load_agents(&agents_dir)?;
    let agent = agents
        .iter()
        .find(|a| a.id == agent_id)
        .ok_or_else(|| format!("Агент '{}' не найден", agent_id))?;

    let log_cb = {
        let app_handle = app.clone();
        move |msg: String| {
            append_test_log(&msg);
            let _ = app_handle.emit("log", format!("[test] {}", msg));
        }
    };
    
    // Исправление ошибки времени жизни: клонируем app для замыкания и используем move
    let app_status = app.clone();
    let status_cb = move |msg: String, progress: u8| {
        let _ = app_status.emit("test_status", msg);
        let _ = app_status.emit("test_progress", progress);
    };
    
    let subcall_cb = |_subcall: &SubCall| { /* Test runner does not handle subcalls directly */ };
    let stream_cb = |_chunk: String| { /* Test runner does not handle streaming directly */ };

    let model_params = config
        .model_params
        .get(&model_path)
        .cloned()
        .unwrap_or_default();
    
    // Инициализация движка LLM
    let engine = infra::llm::LlamaEngine::new(
        &model_path,
        config.context_size,
        config.kv_quant_keys,
        config.kv_quant_values,
        &log_cb,
        stream_cb.clone(),
    )?;

    let format_type = config.prompt_format.clone();
    let cancel_flag = state.cancel_flag.clone();
    let max_gen_tokens = config.max_gen_tokens as usize;

    // Сохраняем длину до вызова into_iter(), чтобы не потерять владение вектором
    let total_cases = test_cases.len();

    for (i, test_case) in test_cases.into_iter().enumerate() {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err("Тест прерван пользователем".to_string());
        }

        status_cb(
            format!(
                "Обработка тест-кейса {}/{}",
                i + 1,
                total_cases
            ),
            (i * 100 / total_cases) as u8,
        );
        append_test_log(&format!(
            "\n--- Тест-кейс {} (Input: {}) ---",
            i + 1,
            test_case.input_data
        ));

        let start_time = Instant::now();
        let mut responses = Vec::new();
        let mut succeeded = false;
        let mut error_msg: Option<String> = None;

        let mut current_chat_messages: Vec<ChatMessage> = Vec::new(); 
        let mut msg_counter = 0; 
        let mut all_sub_calls = Vec::new(); 

        match domain::orchestrator::run_agent_node(
            log_cb.clone(),
            status_cb.clone(),
            subcall_cb.clone(),
            &engine,
            agent,
            &agents,
            test_case.input_data.clone(),
            vec![], // _history не используется напрямую
            &[],    // attachments (не используются в тестах)
            max_gen_tokens,
            &model_params,
            &format_type,
            cancel_flag.clone(),
            0, // depth
            &mut all_sub_calls,
            None, // caller_name
            &mcp_servers_dir,
            &mut current_chat_messages,
            &mut msg_counter,
            String::new(), // injected_reports
            std::sync::Arc::new(std::sync::Mutex::new(domain::StreamMeta::default())),
            false,
        ) {
            Ok(response) => {
                append_test_log(&format!("✅ Ответ LLM: {}", response));
                responses.push(response.clone());
                // Проверка на вхождение "правильного ответа" в сгенерированный
                if response
                    .to_lowercase()
                    .contains(&test_case.right_answer_context.to_lowercase())
                {
                    succeeded = true;
                }
            }
            Err(e) => {
                append_test_log(&format!("❌ Ошибка LLM: {}", e));
                error_msg = Some(e);
            }
        }

        let time_ms = start_time.elapsed().as_millis() as u64;

        results.push(SingleTestResult {
            input_data: test_case.input_data,
            right_answer_context: test_case.right_answer_context,
            responses,
            succeeded,
            error: error_msg,
            time_ms,
        });
    }

    append_test_log("--- Тест завершен ---");
    status_cb("Тесты завершены".to_string(), 100);
    Ok(results)
}

#[tauri::command]
pub fn read_test_file(path: String) -> Result<Vec<TestCaseDef>, String> {
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Ошибка чтения файла: {}", e))?;
    let test_cases: Vec<TestCaseDef> = serde_yaml::from_str(&content)
        .map_err(|e| format!("Ошибка парсинга YAML: {}", e))?;
    Ok(test_cases)
}

#[tauri::command]
pub fn write_test_results(path: String, results: Vec<SingleTestResult>) -> Result<(), String> {
    let content =
        serde_json::to_string_pretty(&results).map_err(|e| format!("Ошибка сериализации: {}", e))?;
    std::fs::write(&path, content).map_err(|e| format!("Ошибка записи файла: {}", e))?;
    Ok(())
}