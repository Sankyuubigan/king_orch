use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::infra::{LlamaEngine, ModelParams, SubCall};
use crate::domain::agent_manager::load_agents;
use crate::domain::workflow_engine::{run_workflow, WorkflowRunner};
use crate::domain::workflow_engine::context::WorkflowContext;
use crate::domain::workflow_engine::parser::load_workflows;
use crate::domain::StreamMeta;

// ─── Типы ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRules {
    pub workflow_name: String,
    pub model_path: String,
    #[serde(default = "default_timeout")]
    pub timeout_sec: u64,
    pub source_file: String,
    pub target_path: String,
    pub levels: ValidationLevels,
}

fn default_timeout() -> u64 { 600 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationLevels {
    pub structure: StructureLevel,
    pub file_change: FileChangeLevel,
    pub functional: FunctionalLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructureLevel {
    pub required_agents: Vec<String>,
    pub forbidden_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChangeLevel {
    pub must_contain: Vec<String>,
    pub must_not_contain: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionalLevel {
    pub run_cmd: String,
    pub expected_stdout_contains: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineTestDef {
    pub id: String,
    pub dir: PathBuf,
    pub task_prompt: String,
    pub validation: ValidationRules,
    pub source_file_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelResult {
    pub passed: bool,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineTestResult {
    pub test_id: String,
    pub model_name: String,
    pub workflow_name: String,
    pub duration_ms: u64,
    pub level1_structure: LevelResult,
    pub level2_file: LevelResult,
    pub level3_functional: LevelResult,
    pub overall_passed: bool,
    pub messages: Vec<AgentMessage>,
    pub report_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub author: String,
    pub content_preview: String,
    pub msg_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineTestInfo {
    pub id: String,
    pub workflow_name: String,
    pub model_path: String,
    pub source_file: String,
    pub timeout_sec: u64,
}

// ─── Загрузка fixture-папок ───

pub fn find_fixtures_dir(project_root: &Path) -> PathBuf {
    project_root.join("test_cases").join("fixtures")
}

pub fn load_pipeline_tests(project_root: &Path) -> Result<Vec<PipelineTestDef>, String> {
    let fixtures_dir = find_fixtures_dir(project_root);
    if !fixtures_dir.exists() {
        return Err(format!("Папка fixtures не найдена: {}", fixtures_dir.display()));
    }

    let mut tests = Vec::new();
    let entries = std::fs::read_dir(&fixtures_dir)
        .map_err(|e| format!("Ошибка чтения {}: {}", fixtures_dir.display(), e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match load_single_test(&path) {
            Ok(test) => tests.push(test),
            Err(e) => {
                eprintln!("⚠️ Пропуск fixture {}: {}", path.display(), e);
            }
        }
    }

    tests.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(tests)
}

pub fn load_single_test(dir: &Path) -> Result<PipelineTestDef, String> {
    let id = dir.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Читаем task.md
    let task_path = dir.join("task.md");
    if !task_path.exists() {
        return Err(format!("task.md не найден в {}", dir.display()));
    }
    let task_prompt = std::fs::read_to_string(&task_path)
        .map_err(|e| format!("Ошибка чтения task.md: {}", e))?;

    // Читаем validation.json
    let validation_path = dir.join("validation.json");
    if !validation_path.exists() {
        return Err(format!("validation.json не найден в {}", dir.display()));
    }
    let validation_str = std::fs::read_to_string(&validation_path)
        .map_err(|e| format!("Ошибка чтения validation.json: {}", e))?;
    let validation: ValidationRules = serde_json::from_str(&validation_str)
        .map_err(|e| format!("Ошибка парсинга validation.json: {}", e))?;

    // Читаем исходный файл
    let source_path = dir.join(&validation.source_file);
    if !source_path.exists() {
        return Err(format!("Исходный файл {} не найден", validation.source_file));
    }
    let source_file_content = std::fs::read_to_string(&source_path)
        .map_err(|e| format!("Ошибка чтения {}: {}", validation.source_file, e))?;

    Ok(PipelineTestDef {
        id,
        dir: dir.to_path_buf(),
        task_prompt,
        validation,
        source_file_content,
    })
}

pub fn get_pipeline_test_infos(project_root: &Path) -> Result<Vec<PipelineTestInfo>, String> {
    let tests = load_pipeline_tests(project_root)?;
    Ok(tests.into_iter().map(|t| PipelineTestInfo {
        id: t.id,
        workflow_name: t.validation.workflow_name,
        model_path: t.validation.model_path,
        source_file: t.validation.source_file,
        timeout_sec: t.validation.timeout_sec,
    }).collect())
}

// ─── Запуск pipeline test ───

pub fn run_pipeline_test(
    test: &PipelineTestDef,
    engine: &LlamaEngine,
    agents_dir: &Path,
    project_root: &Path,
    log_cb: impl Fn(String) + Clone + Send + Sync + 'static,
    status_cb: impl Fn(String, u8) + Clone + Send + Sync + 'static,
) -> Result<PipelineTestResult, String> {
    let start = Instant::now();
    let model_name = Path::new(&test.validation.model_path)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    status_cb(format!("Загрузка агентов и workflow..."), 0);

    let agents = load_agents(agents_dir)
        .map_err(|e| format!("Ошибка загрузки агентов: {}", e))?;
    let workflows = load_workflows(agents_dir)
        .map_err(|e| format!("Ошибка загрузки workflow: {}", e))?;

    let workflow = workflows.iter()
        .find(|w| w.name == test.validation.workflow_name)
        .ok_or_else(|| format!("Workflow '{}' не найден", test.validation.workflow_name))?;

    // Копируем исходный файл в .agents_workspace
    let target_dir = project_root.join(
        Path::new(&test.validation.target_path).parent().unwrap_or(Path::new("."))
    );
    std::fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Ошибка создания {}: {}", target_dir.display(), e))?;
    let target_file = project_root.join(&test.validation.target_path);
    std::fs::write(&target_file, &test.source_file_content)
        .map_err(|e| format!("Ошибка записи {}: {}", target_file.display(), e))?;

    status_cb(format!("Запуск workflow '{}'...", test.validation.workflow_name), 10);

    // Настраиваем движок
    let grammars_dir = crate::domain::orchestrator::grammar::resolve_grammars_dir(agents_dir, Some(workflow));
    let mcp_servers_dir = project_root.join("src-tauri").join("mcp_servers");
    let bins_dir = project_root.join("src-tauri").join("bin");
    let sampling_presets = crate::infra::load_sampling_presets(project_root);
    let model_params = ModelParams::default();

    let mut all_sub_calls: Vec<SubCall> = Vec::new();
    let mut msg_counter: u32 = 0;

    let mut runner = WorkflowRunner {
        engine,
        agents: &agents,
        workflows: &workflows,
        log_cb: log_cb.clone(),
        status_cb: status_cb.clone(),
        subcall_cb: |_: &SubCall| {},
        max_gen_tokens: 2048,
        model_params: &model_params,
        format_type: "Auto",
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        mcp_servers_dir: &mcp_servers_dir,
        bins_dir: &bins_dir,
        grammars_dir: &grammars_dir,
        all_sub_calls: &mut all_sub_calls,
        msg_counter: &mut msg_counter,
        stream_meta: std::sync::Arc::new(std::sync::Mutex::new(StreamMeta::default())),
        sampling_presets: &sampling_presets,
        prompt_log: None,
        session_id: format!("pipeline_test_{}", test.id),
        workspace_root: project_root.to_path_buf(),
    };

    let mut ctx = WorkflowContext::new(test.task_prompt.clone(), vec![], vec![]);

    let workflow_result = run_workflow(workflow, &mut ctx, &mut runner);
    let duration_ms = start.elapsed().as_millis() as u64;

    status_cb("Валидация результатов...".to_string(), 80);

    // Собираем сообщения агентов
    let messages: Vec<AgentMessage> = ctx.messages.iter().map(|m| {
        AgentMessage {
            author: m.author.clone().unwrap_or_default(),
            content_preview: m.content.chars().take(300).collect(),
            msg_type: m.msg_type.clone(),
        }
    }).collect();

    // Уровень 1: Структура
    let level1 = validate_structure(&workflow_result, &ctx, &test.validation.levels.structure);

    // Уровень 2: Файл
    let level2 = validate_file_change(project_root, &test.validation);

    // Уровень 3: Functional
    let level3 = if level2.passed {
        validate_functional(project_root, &test.validation.levels.functional)
    } else {
        LevelResult {
            passed: false,
            details: vec!["Пропущен: Уровень 2 не пройден".to_string()],
        }
    };

    let overall_passed = level1.passed && level2.passed && level3.passed;

    // Генерируем отчёт
    let report_path = generate_report(
        test, &model_name, duration_ms,
        &level1, &level2, &level3,
        overall_passed, &messages, project_root,
    );

    status_cb(if overall_passed { "Тест пройден".to_string() } else { "Тест НЕ пройден".to_string() }, 100);

    Ok(PipelineTestResult {
        test_id: test.id.clone(),
        model_name,
        workflow_name: test.validation.workflow_name.clone(),
        duration_ms,
        level1_structure: level1,
        level2_file: level2,
        level3_functional: level3,
        overall_passed,
        messages,
        report_path,
    })
}

// ─── Валидация ───

fn validate_structure(
    workflow_result: &Result<String, String>,
    ctx: &WorkflowContext,
    rules: &StructureLevel,
) -> LevelResult {
    let mut details = Vec::new();
    let mut passed = true;

    // Проверяем что workflow завершился OK
    match workflow_result {
        Ok(_) => details.push("✅ Workflow завершился OK".to_string()),
        Err(e) => {
            details.push(format!("❌ Workflow упал: {}", e));
            passed = false;
        }
    }

    // Проверяем что есть сообщения
    if ctx.messages.is_empty() {
        details.push("❌ Нет сообщений в контексте".to_string());
        passed = false;
    } else {
        details.push(format!("✅ Сообщений в контексте: {}", ctx.messages.len()));
    }

    // Проверяем достигнутых агентов
    let reached: std::collections::HashSet<&str> = ctx.messages.iter()
        .filter_map(|m| m.author.as_deref())
        .collect();

    for required in &rules.required_agents {
        if reached.contains(required.as_str()) {
            details.push(format!("✅ Агент '{}' вызван", required));
        } else {
            details.push(format!("❌ Агент '{}' НЕ вызван", required));
            passed = false;
        }
    }

    // Проверяем запрещённые пути
    let full_path = ctx.messages.iter()
        .filter_map(|m| m.author.as_deref())
        .collect::<Vec<_>>()
        .join(" → ");

    for forbidden in &rules.forbidden_paths {
        if full_path.contains(forbidden) {
            details.push(format!("❌ Пайплайн прошёл через запрещённый узел '{}'", forbidden));
            passed = false;
        }
    }

    LevelResult { passed, details }
}

fn validate_file_change(project_root: &Path, validation: &ValidationRules) -> LevelResult {
    let mut details = Vec::new();
    let mut passed = true;

    let target_file = project_root.join(&validation.target_path);
    if !target_file.exists() {
        return LevelResult {
            passed: false,
            details: vec![format!("❌ Файл {} не существует", validation.target_path)],
        };
    }

    let content = match std::fs::read_to_string(&target_file) {
        Ok(c) => c,
        Err(e) => {
            return LevelResult {
                passed: false,
                details: vec![format!("❌ Ошибка чтения {}: {}", validation.target_path, e)],
            };
        }
    };

    // Проверяем must_contain
    for pattern in &validation.levels.file_change.must_contain {
        if content.contains(pattern) {
            details.push(format!("✅ Содержит: {}", pattern));
        } else {
            details.push(format!("❌ Не содержит: {}", pattern));
            passed = false;
        }
    }

    // Проверяем must_not_contain
    for pattern in &validation.levels.file_change.must_not_contain {
        if !content.contains(pattern) {
            details.push(format!("✅ Не содержит (баг исправлен): {}", pattern));
        } else {
            details.push(format!("❌ Всё ещё содержит (баг не исправлен): {}", pattern));
            passed = false;
        }
    }

    LevelResult { passed, details }
}

fn validate_functional(project_root: &Path, rules: &FunctionalLevel) -> LevelResult {
    let mut details = Vec::new();

    let output = std::process::Command::new("cmd")
        .args(["/C", &rules.run_cmd])
        .current_dir(project_root)
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);

            if !out.status.success() {
                details.push(format!("❌ Код возврата: {:?}", out.status.code()));
                details.push(format!("   stdout: {}", stdout.chars().take(200).collect::<String>()));
                details.push(format!("   stderr: {}", stderr.chars().take(200).collect::<String>()));
                return LevelResult { passed: false, details };
            }

            if stdout.contains(&rules.expected_stdout_contains) {
                details.push(format!("✅ stdout содержит '{}'", rules.expected_stdout_contains));
                LevelResult { passed: true, details }
            } else {
                details.push(format!("❌ stdout НЕ содержит '{}'", rules.expected_stdout_contains));
                details.push(format!("   stdout: {}", stdout.chars().take(200).collect::<String>()));
                LevelResult { passed: false, details }
            }
        }
        Err(e) => {
            details.push(format!("❌ Ошибка запуска: {}", e));
            LevelResult { passed: false, details }
        }
    }
}

// ─── Генерация отчёта ───

fn generate_report(
    test: &PipelineTestDef,
    model_name: &str,
    duration_ms: u64,
    level1: &LevelResult,
    level2: &LevelResult,
    level3: &LevelResult,
    overall_passed: bool,
    messages: &[AgentMessage],
    project_root: &Path,
) -> Option<String> {
    let timestamp = chrono_timestamp();
    let report_dir = project_root.join("test_cases").join("fixtures").join(&test.id);
    let report_path = report_dir.join(format!("report_{}.md", timestamp));

    let mut report = format!(
        "# Отчёт: {}\n\nДата: {}\nМодель: {}\nПайплайн: {}\nДлительность: {}s\nРезультат: {}\n\n",
        test.id, timestamp, model_name, test.validation.workflow_name,
        duration_ms / 1000,
        if overall_passed { "✅ ПРОЙДЕН" } else { "❌ НЕ ПРОЙДЕН" },
    );

    report.push_str("## Уровень 1: Структура\n");
    for d in &level1.details {
        report.push_str(&format!("{}\n", d));
    }
    report.push_str("\n## Уровень 2: Файл\n");
    for d in &level2.details {
        report.push_str(&format!("{}\n", d));
    }
    report.push_str("\n## Уровень 3: Functional\n");
    for d in &level3.details {
        report.push_str(&format!("{}\n", d));
    }

    report.push_str("\n## Сообщения агентов\n");
    for m in messages {
        if !m.author.is_empty() {
            report.push_str(&format!("\n### [{}] {}\n{}\n", m.msg_type, m.author, m.content_preview));
        }
    }

    let _ = std::fs::write(&report_path, &report);
    Some(report_path.display().to_string())
}

fn chrono_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Простая конвертация без chrono
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;
    // Грубо: дней с 1970-01-01
    let y = 1970 + (days / 365);
    format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", y, 1, 1, h, m, s)
}

// ─── CLI-обёртка ───

pub fn run_pipeline_test_cli(
    test_id: &str,
    model_path: &str,
    project_root: &Path,
) -> Result<PipelineTestResult, String> {
    let agents_dir = project_root.join("agents");
    let fixtures_dir = find_fixtures_dir(project_root);
    let test_dir = fixtures_dir.join(test_id);

    if !test_dir.exists() {
        return Err(format!("Fixture '{}' не найден: {}", test_id, test_dir.display()));
    }

    let test_def = load_single_test(&test_dir)?;

    // Перезаписываем model_path из CLI-аргумента
    let mut test_def = test_def;
    test_def.validation.model_path = model_path.to_string();

    let engine_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .map(|d| crate::infra::llamacpp_installer::default_dir(&d))
        .unwrap_or_else(PathBuf::new);
    let engine_dir = if engine_dir.join("backends").exists() {
        engine_dir
    } else {
        std::env::var("APPDATA")
            .ok()
            .map(|a| Path::new(&a).join("com.kingorch.app").join("app_config.json"))
            .and_then(|cfg| std::fs::read_to_string(cfg).ok())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|v| v.get("llamacpp_dir").and_then(|d| d.as_str()).map(PathBuf::from))
            .filter(|p| p.join("backends").exists())
            .unwrap_or(engine_dir)
    };

    let engine = LlamaEngine::new(
        &engine_dir, model_path, 24576, false, false, 0,
        &|msg| { eprintln!("[LOG] {}", msg); },
        |_| {},
    ).map_err(|e| format!("Ошибка запуска движка: {}", e))?;

    run_pipeline_test(
        &test_def, &engine, &agents_dir, project_root,
        |msg| { eprintln!("[LOG] {}", msg); },
        |msg, pct| { eprintln!("[STATUS {}%] {}", pct, msg); },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_coding_team_fixtures() {
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let tests = load_pipeline_tests(project_root).expect("fixtures не загрузились");
        assert!(!tests.is_empty(), "нет fixture-папок");

        let t = tests.iter().find(|t| t.id == "coding_team_bugfix1")
            .expect("coding_team_bugfix1 не найден");
        assert_eq!(t.validation.workflow_name, "Кодер");
        assert!(t.source_file_content.contains("range(len(data) - 1)"));
        assert!(!t.task_prompt.is_empty());
    }

    #[test]
    #[ignore]
    fn test_coding_team_bugfix_e2e() {
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let model_path = std::env::var("TEST_MODEL_PATH")
            .unwrap_or_else(|_| "D:\\nn\\models\\llm\\uncen\\qwen3.8-9b\\Qwen3.8-9B-heretic-uncensored.i1-IQ4_NL.gguf".to_string());

        let result = run_pipeline_test_cli(
            "coding_team_bugfix1",
            &model_path,
            project_root,
        );

        // Диагностика
        match &result {
            Ok(r) => {
                eprintln!("=== RESULT ===");
                eprintln!("overall_passed: {}", r.overall_passed);
                eprintln!("level1: {} - {:?}", r.level1_structure.passed, r.level1_structure.details);
                eprintln!("level2: {} - {:?}", r.level2_file.passed, r.level2_file.details);
                eprintln!("level3: {} - {:?}", r.level3_functional.passed, r.level3_functional.details);
                eprintln!("report: {:?}", r.report_path);
                for m in &r.messages {
                    let preview: String = m.content_preview.chars().take(100).collect();
                    eprintln!("[{}] {}: {}", m.msg_type, m.author, preview);
                }
            }
            Err(e) => {
                eprintln!("=== ERROR: {} ===", e);
            }
        }

        let result = result.expect("pipeline test упал");
        assert!(result.overall_passed, "pipeline test НЕ пройден: {:?}", result.level1_structure.details);
    }

    #[test]
    #[ignore]
    fn test_psychotherapist_back_pain_e2e() {
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let model_path = std::env::var("TEST_MODEL_PATH")
            .unwrap_or_else(|_| "D:\\nn\\models\\llm\\uncen\\qwen3.8-9b\\Qwen3.8-9B-heretic-uncensored.i1-IQ4_NL.gguf".to_string());

        let result = run_pipeline_test_cli(
            "psychotherapist_back_pain",
            &model_path,
            project_root,
        );

        match &result {
            Ok(r) => {
                eprintln!("=== PSYCHOTHERAPIST BACK PAIN RESULT ===");
                eprintln!("overall_passed: {}", r.overall_passed);
                eprintln!("level1: {} - {:?}", r.level1_structure.passed, r.level1_structure.details);
                eprintln!("report: {:?}", r.report_path);
                for m in &r.messages {
                    let preview: String = m.content_preview.chars().take(150).collect();
                    eprintln!("[{}] {}: {}", m.msg_type, m.author, preview);
                }
                // Проверяем что data_collector — последний активный агент (терминальный узел)
                let last_agent = r.messages.iter()
                    .filter(|m| m.msg_type == "message" && !m.author.is_empty())
                    .last();
                if let Some(msg) = last_agent {
                    eprintln!("Last agent message: {} - {}", msg.author, msg.content_preview.chars().take(100).collect::<String>());
                    assert_eq!(msg.author, "data_collector",
                        "Последний ответ должен быть от data_collector (терминальный узел)");
                } else {
                    panic!("Нет сообщений от агентов в контексте");
                }
            }
            Err(e) => {
                eprintln!("=== ERROR: {} ===", e);
            }
        }

        let result = result.expect("pipeline test упал");
        assert!(result.overall_passed, "pipeline test НЕ пройден: {:?}", result.level1_structure.details);
    }
}
