use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};

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
    let bins_dir = infra::bin_downloader::get_bins_dir(
        &app.path().app_data_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
    );
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
    
    // Инициализация движка LLM (папка движка llama.cpp — рядом с exe)
    let engine_dir = crate::api::llamacpp::get_engine_dir(&app);
    let engine = infra::llm::LlamaEngine::new(
        &engine_dir,
        &model_path,
        config.context_size,
        config.kv_quant_keys,
        config.kv_quant_values,
        config.reasoning_budget,
        log_cb.clone(),
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

        let mcp_pool: crate::infra::mcp_client::McpPool = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::HashMap::<String, crate::infra::mcp_client::SharedMcpClient>::new(),
        ));

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
            &bins_dir,
            &domain::orchestrator::resolve_grammars_dir(&agents_dir, None),
            mcp_pool,
            &mut current_chat_messages,
            &mut msg_counter,
            String::new(), // injected_reports
            std::sync::Arc::new(std::sync::Mutex::new(domain::StreamMeta::default())),
            false,
            None,
            format!("test_case_{}", i),
            agents_dir.parent().unwrap_or(&agents_dir).to_path_buf(),
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

// ─── Pipeline Tests ───

#[tauri::command]
pub fn get_pipeline_test_list(app: AppHandle) -> Result<Vec<domain::pipeline_test::PipelineTestInfo>, String> {
    let agents_dir = infra::find_agents_dir(&app);
    let project_root = agents_dir.parent().unwrap_or(&agents_dir).to_path_buf();
    domain::pipeline_test::get_pipeline_test_infos(&project_root)
}

#[tauri::command]
pub async fn run_pipeline_test_cmd(
    app: AppHandle,
    _state: State<'_, AppState>,
    test_id: String,
    model_path: String,
) -> Result<domain::pipeline_test::PipelineTestResult, String> {
    init_test_log_file();
    append_test_log(&format!(
        "--- Запуск pipeline test '{}' с моделью '{}' ---",
        test_id, model_path
    ));

    let agents_dir = infra::find_agents_dir(&app);
    let project_root = agents_dir.parent().unwrap_or(&agents_dir).to_path_buf();
    let fixtures_dir = domain::pipeline_test::find_fixtures_dir(&project_root);
    let test_dir = fixtures_dir.join(&test_id);

    let mut test_def = domain::pipeline_test::load_single_test(&test_dir)?;
    test_def.validation.model_path = model_path.clone();

    let engine_dir = crate::api::llamacpp::get_engine_dir(&app);
    let config = infra::load_config(&app);

    let log_cb = {
        let app_handle = app.clone();
        move |msg: String| {
            append_test_log(&msg);
            let _ = app_handle.emit("log", format!("[pipeline_test] {}", msg));
        }
    };

    let status_cb = {
        let app_handle = app.clone();
        move |msg: String, progress: u8| {
            let _ = app_handle.emit("pipeline_status", &msg);
            let _ = app_handle.emit("pipeline_progress", progress as f64);
        }
    };

    let engine = infra::LlamaEngine::new(
        &engine_dir,
        &model_path,
        config.context_size,
        config.kv_quant_keys,
        config.kv_quant_values,
        config.reasoning_budget,
        log_cb.clone(),
        |_| {},
    )?;

    let result = domain::pipeline_test::run_pipeline_test(
        &test_def,
        &engine,
        &agents_dir,
        &project_root,
        log_cb,
        status_cb,
    );

    append_test_log(&format!(
        "--- Pipeline test '{}' завершён: {} ---",
        test_id,
        if result.as_ref().map(|r| r.overall_passed).unwrap_or(false) { "PASS" } else { "FAIL" }
    ));

    let _ = app.emit("pipeline_done", result.as_ref().ok().map(|r| r.overall_passed));
    result
}