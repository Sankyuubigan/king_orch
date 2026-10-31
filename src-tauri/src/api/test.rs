use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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
    let bins_dir =
        infra::bin_downloader::get_bins_dir(&crate::infra::storage::app_data_root());
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

    // ── Гейт локального движка ──
    // Бенч не может делить VRAM с ходом чата: ждём слот (FIFO вместе с чатами).
    let _engine_permit = state
        .local_engine_gate
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| format!("Гейт локального движка: {}", e))?;

    let model_params = config
        .model_params
        .get(&model_path)
        .cloned()
        .unwrap_or_default();

    // Инициализация движка LLM (папка движка llama.cpp — рядом с exe)
    let engine_dir = crate::infra::get_engine_dir(&app);
    let engine = infra::LlmEngine::local(infra::llm::LlamaEngine::new(
        &engine_dir,
        &model_path,
        config.context_size,
        config.kv_quant_keys,
        config.kv_quant_values,
        config.reasoning_budget,
        log_cb.clone(),
        stream_cb.clone(),
    )?);

    let format_type = config.prompt_format.clone();
    // Свой флаг отмены: глобального флага больше нет (ходы чатов отменяются
    // по session_id), жизнь бенча от чатов независима.
    let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
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
        let mut pending_signal = None;

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
            vec![],
            std::sync::Arc::new(domain::RequestMedia::empty()),
            std::sync::Arc::new(crate::infra::ImageArtifactRegistry::new()),
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
            None,          // agent_frame
            std::sync::Arc::new(std::sync::Mutex::new(domain::StreamMeta::default())),
            false,
            None,
            format!("test_case_{}", i),
            agents_dir.parent().unwrap_or(&agents_dir).to_path_buf(),
            agents_dir.parent().unwrap_or(&agents_dir).to_path_buf(), // write_root
            crate::infra::WriteOutside::Prompt,
            &mut pending_signal,
        ) {
            Ok(response) => {
                append_test_log(&format!("✅ Ответ LLM: {}", response));
                responses.push(response.clone());
                if let Some(signal) = pending_signal.take() {
                    current_chat_messages.push(signal);
                }
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
    state: State<'_, AppState>,
    test_id: String,
    model_path: String,
) -> Result<domain::pipeline_test::PipelineTestResult, String> {
    init_test_log_file();
    append_test_log(&format!(
        "--- Запуск pipeline test '{}' с моделью '{}' ---",
        test_id, model_path
    ));

    // Гейт локального движка: pipeline-тест тоже потребитель VRAM.
    let _engine_permit = state
        .local_engine_gate
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| format!("Гейт локального движка: {}", e))?;

    let agents_dir = infra::find_agents_dir(&app);
    let project_root = agents_dir.parent().unwrap_or(&agents_dir).to_path_buf();

    let mut test_def = domain::pipeline_test::load_pipeline_tests(&project_root)?
        .into_iter()
        .find(|t| t.id == test_id)
        .ok_or_else(|| format!("Тест '{}' не найден в fixtures", test_id))?;
    test_def.validation.model_path = model_path.clone();

    let engine_dir = crate::infra::get_engine_dir(&app);
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

    // Снимок PID llama-server'ов ДО запуска движка: из дельты находим PID НАШЕГО
    // llama-server — рабочая релизная копия King Orch может держать собственный,
    // и её память в сравнение моделей попадать не должна.
    let before_pids = llama_server_pids();
    let vram_before = crate::domain::coding_bench::kv_probe::vram_used_mb();

    let engine = infra::LlmEngine::local(infra::LlamaEngine::new(
        &engine_dir,
        &model_path,
        config.context_size,
        config.kv_quant_keys,
        config.kv_quant_values,
        config.reasoning_budget,
        log_cb.clone(),
        |_| {},
    )?);
    // СООТ-сэмплер памяти движка (mem_profiler): RSS llama-server + VRAM GPU.
    let mut mem_sampler = crate::infra::MemSampler::start(new_llama_server_pid(&before_pids));

    let mut result = domain::pipeline_test::run_pipeline_test(
        &test_def,
        &engine,
        &agents_dir,
        &project_root,
        log_cb,
        status_cb,
    );

    let (ram_peak, vram_peak, vram_delta) = mem_fields(mem_sampler.stop_and_report(), vram_before);
    if let Ok(r) = result.as_mut() {
        r.ram_peak_mb = ram_peak;
        r.vram_peak_mb = vram_peak;
        r.vram_delta_mb = vram_delta;
        append_memory_section(r.report_path.as_deref(), ram_peak, vram_peak, vram_delta);
    }

    append_test_log(&format!(
        "--- Pipeline test '{}' завершён: {} ---",
        test_id,
        if result.as_ref().map(|r| r.overall_passed).unwrap_or(false) { "PASS" } else { "FAIL" }
    ));

    let _ = app.emit("pipeline_done", result.as_ref().ok().map(|r| r.overall_passed));
    result
}

// ─── Замер памяти llama-server во время теста ───
// СООТ: сэмплирование — существующий `MemSampler` движка (mem_profiler, 200 мс,
// device-level VRAM: под WDDM per-process недоступен); чтение текущей VRAM —
// существующий `kv_probe::vram_used_mb`. Здесь только нахождение PID нашего
// llama-server (снимок процессов до/после создания движка) и сборка полей отчёта.

/// Снимок PID всех запущенных llama-server'ов — вызывается ДО создания движка.
pub fn llama_server_pids() -> HashSet<u32> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    sys.processes()
        .values()
        .filter(|p| p.name().to_string().to_lowercase().contains("llama-server"))
        .map(|p| p.pid().as_u32())
        .collect()
}

/// PID нашего llama-server: появился после снапшота. Гейт VRAM гарантирует,
/// что другие локальные потребители (чат, бенчи) в этот момент не стартуют.
fn new_llama_server_pid(before: &HashSet<u32>) -> Option<u32> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    sys.processes()
        .values()
        .filter(|p| {
            p.name().to_string().to_lowercase().contains("llama-server")
                && !before.contains(&p.pid().as_u32())
        })
        .map(|p| p.pid().as_u32())
        .max()
}

/// Пики из MemReport → поля результата: (ОЗУ llama-server, VRAM пик, VRAM Δ от базы), МБ.
///
/// VRAM device-level — вся занятая VRAM GPU, включая чужие процессы; сравнивать
/// модели между собой корректнее по Δ (пик − база до запуска движка).
fn mem_fields(
    report: tauri_plugin_llama_engine::engine::mem_profiler::MemReport,
    vram_before_mb: Option<f64>,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    const MB: f64 = 1024.0 * 1024.0;
    let ram = (report.rss_server_peak > 0).then(|| report.rss_server_peak as f64 / MB);
    let vram = (report.vram_ok && report.vram_used_peak > 0)
        .then(|| report.vram_used_peak as f64 / MB);
    let delta = vram
        .zip(vram_before_mb)
        .map(|(peak, base)| (peak - base).max(0.0));
    (ram, vram, delta)
}

fn fmt_mb(mb: Option<f64>) -> String {
    match mb {
        Some(v) => format!("{:.0} МБ", v),
        None => "—".to_string(),
    }
}

/// Ячейка VRAM для сводной таблицы: Δ (сравнимая величина) + абсолютный пик.
fn fmt_vram_cell(delta: Option<f64>, peak: Option<f64>) -> String {
    match (delta, peak) {
        (Some(d), Some(p)) => format!("Δ{:.0} (из {:.0}) МБ", d, p),
        (None, Some(p)) => format!("{:.0} МБ (без базы)", p),
        _ => "—".to_string(),
    }
}

/// Суммарное время батча для отчётов: None — фронт не передал (старая версия).
fn fmt_total_duration(total_ms: Option<u64>) -> String {
    match total_ms {
        Some(ms) => format!("{:.1}s", ms as f64 / 1000.0),
        None => "—".to_string(),
    }
}

/// Цепочка переходов одним плоским текстом (для маркдауна, в ограждении):
/// по строке на сообщение, новые строки внутри превью сплющены в пробел —
/// иначе маркдаун ответов моделей ломает структуру списка («поплывёт»).
fn chain_flat_text(messages: &[domain::pipeline_test::AgentMessage]) -> String {
    messages
        .iter()
        .filter(|m| !m.author.is_empty())
        .map(|m| {
            format!(
                "[{}] {}: {}\n",
                m.msg_type,
                m.author,
                m.content_preview.replace(['\r', '\n'], " ")
            )
        })
        .collect()
}

/// Автор итогового ответа: последнее message-сообщение от агента (не сигнал,
/// не user). Если цепочка пуста — "неизвестно".
fn final_answer_author(messages: &[domain::pipeline_test::AgentMessage]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m.msg_type == "message" && !m.author.is_empty() && m.author != "user")
        .or_else(|| messages.iter().rev().find(|m| !m.author.is_empty()))
        .map(|m| m.author.clone())
        .unwrap_or_else(|| "неизвестно".to_string())
}

// ─── Просмотрщик сохранённых отчётов ───

/// Метка отчёта для выпадающего списка на странице теста.
#[derive(Serialize)]
pub struct TestReportMeta {
    pub file_name: String,
    pub path: String,
    /// "custom" (кастомный промпт) | "pipeline" (батч фикстуры).
    pub kind: String,
    /// Агент/пайплайн (custom) или id фикстуры (pipeline).
    pub label: String,
    pub created_ms: u64,
}

/// Список сохранённых сводных отчётов (JSON): custom_runs + batch-отчёты фикстур.
#[tauri::command]
pub fn list_test_reports(app: AppHandle) -> Vec<TestReportMeta> {
    let agents_dir = infra::find_agents_dir(&app);
    let project_root = agents_dir.parent().unwrap_or(&agents_dir).to_path_buf();
    let test_cases = project_root.join("test_cases");
    let mut reports: Vec<TestReportMeta> = Vec::new();

    let scan = |dir: &std::path::Path, pattern: &str, kind: &str, reports: &mut Vec<TestReportMeta>| {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.starts_with(pattern) || !name.ends_with(".json") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&p) else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) else { continue };
            let label = v
                .get("agent_id")
                .or_else(|| v.get("test_id"))
                .and_then(|x| x.as_str())
                .unwrap_or("?")
                .to_string();
            let created_ms = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            reports.push(TestReportMeta {
                file_name: name.to_string(),
                path: p.display().to_string(),
                kind: kind.to_string(),
                label,
                created_ms,
            });
        }
    };

    scan(&test_cases.join("custom_runs"), "custom_report_", "custom", &mut reports);
    if let Ok(rd) = std::fs::read_dir(test_cases.join("fixtures")) {
        for fixture in rd.flatten() {
            scan(&fixture.path(), "batch_report_", "pipeline", &mut reports);
        }
    }
    reports.sort_by(|a, b| b.created_ms.cmp(&a.created_ms));
    reports
}

/// Прочитать сохранённый JSON-отчёт для просмотрщика.
///
/// Гейт источников: путь обязан лежать внутри `<project>/test_cases` (без `..`),
/// иначе произвольное чтение диска через команду.
#[tauri::command]
pub fn read_test_report(app: AppHandle, path: String) -> Result<serde_json::Value, String> {
    let agents_dir = infra::find_agents_dir(&app);
    let project_root = agents_dir.parent().unwrap_or(&agents_dir).to_path_buf();
    let base = project_root
        .join("test_cases")
        .canonicalize()
        .map_err(|e| format!("test_cases недоступна: {}", e))?;
    let p = std::path::PathBuf::from(&path);
    if !p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("Недопустимый путь отчёта".to_string());
    }
    let canonical = p
        .canonicalize()
        .map_err(|e| format!("Файл отчёта недоступен: {}", e))?;
    if !canonical.starts_with(&base) || !canonical.is_file() {
        return Err("Отчёт должен лежать внутри test_cases".to_string());
    }
    let content = std::fs::read_to_string(&canonical)
        .map_err(|e| format!("Ошибка чтения {}: {}", canonical.display(), e))?;
    serde_json::from_str(&content).map_err(|e| format!("Отчёт не JSON: {}", e))
}

/// Дописать в индивидуальный отчёт секцию памяти (отчёт пишется доменом ДО того,
/// как хост узнает пики, поэтому секция добавляется хостом пост-фактум).
fn append_memory_section(
    report_path: Option<&str>,
    ram: Option<f64>,
    vram: Option<f64>,
    vram_delta: Option<f64>,
) {
    let Some(path) = report_path else { return };
    if ram.is_none() && vram.is_none() {
        return;
    }
    let vram_line = match (vram, vram_delta) {
        (Some(p), Some(d)) => format!("{:.0} МБ (Δ{:.0} от базы до запуска)", p, d),
        (Some(p), None) => format!("{:.0} МБ (device-level GPU)", p),
        (None, _) => "замер не удался (NVML недоступен)".to_string(),
    };
    let section = format!(
        "\n## Память (пик за прогон)\n\n- VRAM: {}\n- ОЗУ llama-server: {}\n",
        vram_line,
        fmt_mb(ram),
    );
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path) {
        let _ = f.write_all(section.as_bytes());
    }
}

/// Одна строка батч-прогона: либо результат харнесса, либо ошибка запуска модели.
///
/// Ошибка запуска (движок не загрузился, модель не найдена) не должна ронять весь
/// батч — фронтенд продолжает со следующей моделью, а несостоявшийся прогон попадает
/// в сводный отчёт отдельной строкой, чтобы «тихо пропавшая модель» была видна.
/// Serialize — батч-отчёт пишется и в JSON (точный формат для просмотрщика отчётов).
#[derive(Serialize, Deserialize)]
pub struct PipelineBatchEntry {
    pub model: String,
    pub error: Option<String>,
    pub result: Option<domain::pipeline_test::PipelineTestResult>,
}

fn short_model_name(model: &str) -> &str {
    model.rsplit(|c| c == '/' || c == '\\').next().unwrap_or(model)
}

/// Сводный отчёт батч-прогона pipeline-теста по нескольким моделям.
///
/// Пишется в папку фикстуры (`test_cases/fixtures/<test_id>/batch_report_<ts>.md`),
/// рядом с одиночными `report_*.md`, которые харнесс сохраняет за каждый прогон:
/// сводная таблица + ошибки запуска + детали уровней по каждой модели.
///
/// Точный артефакт — `.json` рядом (его читает просмотрщик отчётов на странице
/// теста): маркдаун — человекочитаемый экспорт, в нём превью сообщений, а
/// содержимое моделей обёрнуто в ограждения, чтобы их маркдаун не ломал структуру.
/// `total_duration_ms` — суммарное время батча (стена), меряет фронтенд.
#[tauri::command]
pub fn write_pipeline_batch_report(
    app: AppHandle,
    test_id: String,
    entries: Vec<PipelineBatchEntry>,
    total_duration_ms: Option<u64>,
) -> Result<String, String> {
    let agents_dir = infra::find_agents_dir(&app);
    let project_root = agents_dir.parent().unwrap_or(&agents_dir).to_path_buf();
    let report_dir = project_root.join("test_cases").join("fixtures").join(&test_id);
    std::fs::create_dir_all(&report_dir)
        .map_err(|e| format!("Ошибка создания {}: {}", report_dir.display(), e))?;

    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let report_path = report_dir.join(format!("batch_report_{}.md", timestamp));
    let json_path = report_dir.join(format!("batch_report_{}.json", timestamp));

    let total_line = fmt_total_duration(total_duration_ms);
    let mut md = format!(
        "# Сводный отчёт батч-прогона: {}\n\nДата: {}\nМоделей в прогоне: {}\nОбщее время батча: {}\n\n",
        test_id,
        timestamp,
        entries.len(),
        total_line,
    );

    md.push_str("| # | Модель | Итог | Время | VRAM | ОЗУ | Индивидуальный отчёт |\n");
    md.push_str("|---|--------|------|-------|------|-----|----------------------|\n");
    for (i, entry) in entries.iter().enumerate() {
        let (verdict, duration, vram, ram, report) = match &entry.result {
            Some(r) => (
                (if r.overall_passed { "✅ ПРОЙДЕН" } else { "❌ НЕ ПРОЙДЕН" }).to_string(),
                format!("{:.1}s", r.duration_ms as f64 / 1000.0),
                fmt_vram_cell(r.vram_delta_mb, r.vram_peak_mb),
                fmt_mb(r.ram_peak_mb),
                r.report_path.clone().unwrap_or_default(),
            ),
            None => (
                "💥 ОШИБКА ЗАПУСКА".to_string(),
                "—".to_string(),
                "—".to_string(),
                "—".to_string(),
                String::new(),
            ),
        };
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            i + 1,
            short_model_name(&entry.model),
            verdict,
            duration,
            vram,
            ram,
            report,
        ));
    }

    let failures: Vec<&PipelineBatchEntry> = entries.iter().filter(|e| e.error.is_some()).collect();
    if !failures.is_empty() {
        md.push_str("\n## Ошибки запуска\n\n");
        for f in failures {
            md.push_str(&format!(
                "### {}\n\n```\n{}\n```\n\n",
                short_model_name(&f.model),
                f.error.as_deref().unwrap_or(""),
            ));
        }
    }

    md.push_str("\n## Детали по моделям\n");
    for entry in &entries {
        let Some(r) = &entry.result else { continue };
        md.push_str(&format!(
            "\n### {} — {}\n\nДлительность: {:.1}s · VRAM: {} · ОЗУ: {}\n",
            r.model_name,
            (if r.overall_passed { "✅ ПРОЙДЕН" } else { "❌ НЕ ПРОЙДЕН" }),
            r.duration_ms as f64 / 1000.0,
            fmt_vram_cell(r.vram_delta_mb, r.vram_peak_mb),
            fmt_mb(r.ram_peak_mb),
        ));
        for (name, level) in [
            ("Уровень 1: Структура", &r.level1_structure),
            ("Уровень 2: Файл", &r.level2_file),
            ("Уровень 3: Functional", &r.level3_functional),
        ] {
            md.push_str(&format!("\n**{}**: {}\n", name, if level.passed { "✅" } else { "❌" }));
            for d in &level.details {
                md.push_str(&format!("- {}\n", d));
            }
        }
        md.push_str("\n**Цепочка переходов:**\n\n```\n");
        md.push_str(&chain_flat_text(&r.messages));
        md.push_str("```\n");
    }

    std::fs::write(&report_path, &md)
        .map_err(|e| format!("Ошибка записи {}: {}", report_path.display(), e))?;

    // JSON — точный артефакт для просмотрщика отчётов (полная структура без
    // маркдаун-экранирования): type/created/total_duration_ms/entries.
    let json = serde_json::json!({
        "type": "pipeline_batch_report",
        "created": timestamp,
        "test_id": test_id,
        "total_duration_ms": total_duration_ms,
        "markdown_path": report_path.display().to_string(),
        "entries": entries,
    });
    std::fs::write(
        &json_path,
        serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Ошибка записи {}: {}", json_path.display(), e))?;

    Ok(json_path.display().to_string())
}

// ─── Кастомный промпт (третий режим теста) ───

/// Результат одного прогона кастомного промпта на одной модели.
/// Deserialize нужен, т.к. результат входит в CustomPromptEntry, который
/// фронтенд передаёт обратно в write_custom_prompt_report.
#[derive(Serialize, Deserialize, Clone)]
pub struct CustomPromptResult {
    pub model_name: String,
    pub agent_id: String,
    pub response: String,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub ram_peak_mb: Option<f64>,
    pub vram_peak_mb: Option<f64>,
    pub vram_delta_mb: Option<f64>,
    pub messages: Vec<domain::pipeline_test::AgentMessage>,
}

/// Serialize — entries целиком уходят в JSON-отчёт (просмотрщик отчётов).
#[derive(Serialize, Deserialize)]
pub struct CustomPromptEntry {
    pub model: String,
    pub error: Option<String>,
    pub result: Option<CustomPromptResult>,
}

/// Прогнать произвольный промпт через выбранного агента/пайплайн на одной модели.
///
/// Механика — та же, что у YAML-теста (`run_iterative_test`), но с одним кейсом:
/// конфиг хост грузит сам, ошибки агента не роняют команду (попадают в `error`),
/// чтобы батч по моделям продолжался, а память/время всё равно записались.
#[tauri::command]
pub async fn run_custom_prompt_test(
    app: AppHandle,
    state: State<'_, AppState>,
    prompt: String,
    agent_id: String,
    model_path: String,
) -> Result<CustomPromptResult, String> {
    init_test_log_file();
    append_test_log(&format!(
        "--- Кастомный промпт: агент '{}' модель '{}' ---",
        agent_id, model_path
    ));

    // Гейт локального движка: кастомный тест тоже потребитель VRAM.
    let _engine_permit = state
        .local_engine_gate
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| format!("Гейт локального движка: {}", e))?;

    let agents_dir = infra::find_agents_dir(&app);
    let project_root = agents_dir.parent().unwrap_or(&agents_dir).to_path_buf();
    let mcp_servers_dir = infra::find_mcp_servers_dir(&app);
    let bins_dir =
        infra::bin_downloader::get_bins_dir(&crate::infra::storage::app_data_root());
    let agents = domain::load_agents(&agents_dir)?;
    // .md-агент ИЛИ workflow-граф: у графа AgentEntry.id = file_stem yaml-файла
    // (agent_manager::load_entry_points), в списке AgentProfile его нет.
    let agent = agents.iter().find(|a| a.id == agent_id);

    let config = infra::load_config(&app);
    let model_params = config.model_params.get(&model_path).cloned().unwrap_or_default();
    let format_type = config.prompt_format.clone();
    let max_gen_tokens = config.max_gen_tokens as usize;
    let engine_dir = crate::infra::get_engine_dir(&app);

    let log_cb = {
        let app_handle = app.clone();
        move |msg: String| {
            append_test_log(&msg);
            let _ = app_handle.emit("log", format!("[custom_test] {}", msg));
        }
    };
    let status_cb = {
        let app_handle = app.clone();
        move |msg: String, progress: u8| {
            let _ = app_handle.emit("custom_test_status", &msg);
            let _ = app_handle.emit("custom_test_progress", progress as f64);
        }
    };
    let subcall_cb = |_subcall: &SubCall| {};
    let stream_cb = |_chunk: String| {};
    let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Снимок PID до создания движка — как в pipeline-тесте (находим наш llama-server).
    let before_pids = llama_server_pids();
    let vram_before = crate::domain::coding_bench::kv_probe::vram_used_mb();

    let engine = infra::LlmEngine::local(infra::LlamaEngine::new(
        &engine_dir,
        &model_path,
        config.context_size,
        config.kv_quant_keys,
        config.kv_quant_values,
        config.reasoning_budget,
        log_cb.clone(),
        stream_cb,
    )?);
    let mut mem_sampler = crate::infra::MemSampler::start(new_llama_server_pid(&before_pids));

    let start_time = Instant::now();
    let mut msg_counter: u32 = 0;
    let mut all_sub_calls: Vec<SubCall> = Vec::new();
    let mcp_pool: crate::infra::mcp_client::McpPool = std::sync::Arc::new(std::sync::Mutex::new(
        std::collections::HashMap::<String, crate::infra::mcp_client::SharedMcpClient>::new(),
    ));

    // Ветка 1: .md-агент — run_agent_node (механика YAML-теста).
    // Ветка 2: workflow-граф — run_workflow, тот же исполнитель, что у
    // pipeline-харнесса (СООТ). Зона записи — как в харнессе: .agents_workspace,
    // запись вне её запрещена (тест не должен мусорить в корне репозитория).
    let (outcome, current_chat_messages) = if let Some(agent) = agent {
        let mut current_chat_messages: Vec<ChatMessage> = Vec::new();
        let mut pending_signal = None;
        let outcome = domain::orchestrator::run_agent_node(
            log_cb.clone(),
            status_cb.clone(),
            subcall_cb,
            &engine,
            agent,
            &agents,
            prompt,
            vec![],
            std::sync::Arc::new(domain::RequestMedia::empty()),
            std::sync::Arc::new(crate::infra::ImageArtifactRegistry::new()),
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
            None,          // agent_frame
            std::sync::Arc::new(std::sync::Mutex::new(domain::StreamMeta::default())),
            false,
            None,
            "custom_prompt".to_string(),
            project_root.clone(),
            project_root.clone(), // write_root
            crate::infra::WriteOutside::Prompt,
            &mut pending_signal,
        );
        if outcome.is_ok() {
            if let Some(signal) = pending_signal.take() {
                current_chat_messages.push(signal);
            }
        }
        (outcome, current_chat_messages)
    } else {
        let workflows_owned = domain::workflow_engine::parser::load_workflows(&agents_dir)?;
        let workflow = workflows_owned
            .iter()
            .find(|w| w.file_stem == agent_id || w.name == agent_id)
            .ok_or_else(|| format!("Агент или пайплайн '{}' не найден", agent_id))?;
        let grammars_dir = domain::orchestrator::resolve_grammars_dir(&agents_dir, Some(workflow));
        let sampling_presets = crate::infra::load_sampling_presets(&project_root);
        let mut runner = domain::workflow_engine::WorkflowRunner {
            engine: &engine,
            agents: &agents,
            workflows: &workflows_owned,
            log_cb: log_cb.clone(),
            status_cb: status_cb.clone(),
            subcall_cb: |_s: &SubCall| {},
            max_gen_tokens,
            model_params: &model_params,
            format_type: "Auto",
            cancel_flag: cancel_flag.clone(),
            mcp_servers_dir: &mcp_servers_dir,
            bins_dir: &bins_dir,
            grammars_dir: &grammars_dir,
            agents_dir: &agents_dir,
            all_sub_calls: &mut all_sub_calls,
            msg_counter: &mut msg_counter,
            stream_meta: std::sync::Arc::new(std::sync::Mutex::new(domain::StreamMeta::default())),
            sampling_presets: &sampling_presets,
            prompt_log: None,
            session_id: "custom_prompt_test".to_string(),
            workspace_root: project_root.clone(),
            write_root: project_root.join(".agents_workspace"),
            request_media: std::sync::Arc::new(domain::RequestMedia::empty()),
            image_artifacts: std::sync::Arc::new(crate::infra::ImageArtifactRegistry::new()),
            write_outside: crate::infra::WriteOutside::Deny,
        };
        let mut ctx = domain::workflow_engine::context::WorkflowContext::new(prompt, vec![], vec![]);
        match domain::workflow_engine::run_workflow(workflow, &mut ctx, &mut runner) {
            Ok(response) => (Ok(response), ctx.messages),
            Err((e, msgs)) => (Err(e), msgs),
        }
    };

    let duration_ms = start_time.elapsed().as_millis() as u64;

    match &outcome {
        Ok(response) => append_test_log(&format!("✅ Ответ LLM: {}", response)),
        Err(e) => append_test_log(&format!("❌ Ошибка LLM: {}", e)),
    }

    let (ram_peak, vram_peak, vram_delta) = mem_fields(mem_sampler.stop_and_report(), vram_before);

    let messages: Vec<domain::pipeline_test::AgentMessage> = current_chat_messages
        .iter()
        .map(|m| domain::pipeline_test::AgentMessage {
            author: m.author.clone().unwrap_or_default(),
            content_preview: m.content.chars().take(300).collect(),
            msg_type: m.msg_type.clone(),
        })
        .collect();

    append_test_log("--- Кастомный промпт завершён ---");

    Ok(match outcome {
        Ok(response) => CustomPromptResult {
            model_name: model_path,
            agent_id,
            response,
            error: None,
            duration_ms,
            ram_peak_mb: ram_peak,
            vram_peak_mb: vram_peak,
            vram_delta_mb: vram_delta,
            messages,
        },
        Err(e) => CustomPromptResult {
            model_name: model_path,
            agent_id,
            response: String::new(),
            error: Some(e),
            duration_ms,
            ram_peak_mb: ram_peak,
            vram_peak_mb: vram_peak,
            vram_delta_mb: vram_delta,
            messages,
        },
    })
}

/// Сводный отчёт батч-прогона кастомного промпта.
///
/// Пишется пара: `custom_report_<ts>.json` (точный артефакт для просмотрщика
/// отчётов) + `custom_report_<ts>.md` (человекочитаемый экспорт). Содержимое
/// моделей в маркдауне обёрнуто в ограждения, чтобы их маркдаун не ломал структуру.
/// `total_duration_ms` — суммарное время батча (стена), меряет фронтенд.
#[tauri::command]
pub fn write_custom_prompt_report(
    app: AppHandle,
    agent_id: String,
    prompt: String,
    entries: Vec<CustomPromptEntry>,
    total_duration_ms: Option<u64>,
) -> Result<String, String> {
    let agents_dir = infra::find_agents_dir(&app);
    let project_root = agents_dir.parent().unwrap_or(&agents_dir).to_path_buf();
    let report_dir = project_root.join("test_cases").join("custom_runs");
    std::fs::create_dir_all(&report_dir)
        .map_err(|e| format!("Ошибка создания {}: {}", report_dir.display(), e))?;

    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let report_path = report_dir.join(format!("custom_report_{}.md", timestamp));
    let json_path = report_dir.join(format!("custom_report_{}.json", timestamp));

    let mut md = format!(
        "# Кастомный промпт — сводный отчёт\n\nДата: {}\nАгент/пайплайн: `{}`\nМоделей в прогоне: {}\nОбщее время батча: {}\n\n## Промпт\n\n```\n{}\n```\n\n",
        timestamp,
        agent_id,
        entries.len(),
        fmt_total_duration(total_duration_ms),
        prompt,
    );

    md.push_str("| # | Модель | Итог | Время | VRAM | ОЗУ |\n");
    md.push_str("|---|--------|------|-------|------|-----|\n");
    for (i, entry) in entries.iter().enumerate() {
        let (verdict, duration, vram, ram) = match &entry.result {
            Some(r) if r.error.is_none() => (
                "✅ ОТВЕТ".to_string(),
                format!("{:.1}s", r.duration_ms as f64 / 1000.0),
                fmt_vram_cell(r.vram_delta_mb, r.vram_peak_mb),
                fmt_mb(r.ram_peak_mb),
            ),
            Some(r) => (
                "💥 ОШИБКА".to_string(),
                format!("{:.1}s", r.duration_ms as f64 / 1000.0),
                fmt_vram_cell(r.vram_delta_mb, r.vram_peak_mb),
                fmt_mb(r.ram_peak_mb),
            ),
            None => (
                "💥 ОШИБКА ЗАПУСКА".to_string(),
                "—".to_string(),
                "—".to_string(),
                "—".to_string(),
            ),
        };
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            i + 1,
            short_model_name(&entry.model),
            verdict,
            duration,
            vram,
            ram,
        ));
    }

    md.push_str("\n## Ответы по моделям\n");
    for entry in &entries {
        md.push_str(&format!("\n### {}\n\n", short_model_name(&entry.model)));
        if let Some(err) = &entry.error {
            md.push_str(&format!("**Ошибка запуска:**\n\n```\n{}\n```\n", err));
        }
        if let Some(r) = &entry.result {
            if let Some(err) = &r.error {
                md.push_str(&format!("**Ошибка выполнения:**\n\n```\n{}\n```\n", err));
            } else {
                md.push_str(&format!(
                    "Время: {:.1}s · VRAM: {} · ОЗУ: {}\n\n**Итоговый ответ** (агент: {}):\n\n````\n{}\n````\n",
                    r.duration_ms as f64 / 1000.0,
                    fmt_vram_cell(r.vram_delta_mb, r.vram_peak_mb),
                    fmt_mb(r.ram_peak_mb),
                    final_answer_author(&r.messages),
                    r.response,
                ));
            }
            md.push_str("\n**Цепочка переходов:**\n\n```\n");
            md.push_str(&chain_flat_text(&r.messages));
            md.push_str("```\n");
        }
    }

    std::fs::write(&report_path, &md)
        .map_err(|e| format!("Ошибка записи {}: {}", report_path.display(), e))?;

    // JSON — точный артефакт для просмотрщика отчётов (полная структура).
    let json = serde_json::json!({
        "type": "custom_prompt_report",
        "created": timestamp,
        "agent_id": agent_id,
        "prompt": prompt,
        "total_duration_ms": total_duration_ms,
        "markdown_path": report_path.display().to_string(),
        "entries": entries,
    });
    std::fs::write(
        &json_path,
        serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Ошибка записи {}: {}", json_path.display(), e))?;

    Ok(json_path.display().to_string())
}

/// Остановка текущего тест-прогона (pipeline / кастомный промпт).
///
/// В момент вызова единственный зарегистрированный локальный движок — движок
/// текущего теста: гейт `local_engine_gate` FIFO не пускает чат и другие тесты,
/// пока тест держит слот. Убийство llama-server ломает активный HTTP-запрос —
/// workflow завершается ошибкой, и прогон модели заканчивается за секунды,
/// а не по полному таймауту. Реестр PID живёт в памяти нашего процесса,
/// поэтому llama-server установленной релизной копии не затрагивается.
#[tauri::command]
pub fn cancel_test_run() -> Result<(), String> {
    tauri_plugin_llama_engine::engine::process_util::kill_active_engines();
    Ok(())
}