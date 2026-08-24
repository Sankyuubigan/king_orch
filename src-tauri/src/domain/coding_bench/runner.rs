// Оркестратор coding-бенчмарка: модели × задачи → отчёт.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::infra::{AppConfig, LlamaEngine, LlmMessage};

use super::evaluator::{append_test_to_solution, assemble_solution, run_command, ExecVerdict};
use super::kv_probe::probe_max_ctx_f16;
use super::report::{write_artifacts, write_report, ModelRunSummary, ReportSummary, TaskResultRecord};
use super::tasks::{load_suite_tasks, CodingTask, TaskFile};

pub struct ModelToRun {
    pub path: String,
    pub name: String,
}

pub struct CodingBenchOptions<'a> {
    pub engine_dir: PathBuf,
    pub bins_dir: PathBuf,
    pub tasks_dir: PathBuf,
    pub models: Vec<ModelToRun>,
    pub suites: Vec<String>,
    pub quick_per_suite: Option<usize>,
    pub vr_budget_mb: u64,
    pub config: &'a AppConfig,
}

/// Запуск бенчмарка. Прогресс — через status_cb(msg, percent).
pub fn run_coding_bench(
    opts: &CodingBenchOptions,
    cancel_flag: Arc<AtomicBool>,
    mut status_cb: impl FnMut(String, u8),
    log_cb: impl Fn(String) + Clone + Send + Sync + 'static,
) -> Result<ReportSummary, String> {
    let tasks = collect_tasks(opts)?;
    if tasks.is_empty() {
        return Err("Не выбрано ни одной задачи".to_string());
    }
    let total = tasks.len();
    let model_count = opts.models.len();
    let mut summaries = Vec::new();

    for (mi, model) in opts.models.iter().enumerate() {
        if cancel_flag.load(Ordering::SeqCst) {
            status_cb("Прервано пользователем".to_string(), 100);
            break;
        }
        log_cb(format!("\n🚀 Модель {}/{}: {}", mi + 1, model_count, model.name));
        status_cb(format!("Модель {}/{}: {}", mi + 1, model_count, model.name), 0);

        // KV-probe (f16) ДО основного движка — в VRAM одновременно только одна модель.
        let kv_probe = match probe_max_ctx_f16(&opts.engine_dir, &model.path, opts.vr_budget_mb, &log_cb) {
            Ok(r) => Some(r),
            Err(e) => {
                log_cb(format!("⚠️ KV-probe пропущен: {}", e));
                None
            }
        };

        let engine = LlamaEngine::new(
            &opts.engine_dir,
            &model.path,
            opts.config.context_size,
            opts.config.kv_quant_keys,
            opts.config.kv_quant_values,
            opts.config.reasoning_budget,
            log_cb.clone(),
            |_chunk: String| {},
        )?;

        let mut records = Vec::with_capacity(total);
        for (ti, task) in tasks.iter().enumerate() {
            if cancel_flag.load(Ordering::SeqCst) {
                break;
            }
            let percent = ((mi * total + ti) * 100 / (model_count * total)) as u8;
            status_cb(
                format!("[{}] {} — {}/{}", model.name, task.id, ti + 1, total),
                percent,
            );
            records.push(run_single_task(&engine, task, opts, model, log_cb.clone(), cancel_flag.clone()));
        }
        drop(engine);

        summaries.push(build_model_summary(model, records, kv_probe));
    }

    let mut report = ReportSummary {
        timestamp: String::new(),
        budget_vram_mb: opts.vr_budget_mb,
        quick_per_suite: opts.quick_per_suite,
        models: summaries,
        report_file: String::new(),
        artifacts_dir: String::new(),
    };
    let report_file = write_report(&opts.tasks_dir, &mut report)?;
    report.artifacts_dir = write_artifacts(&opts.tasks_dir, &report.models, &|lang| match lang {
        "python" => "py".to_string(), "rust" => "rs".to_string(), "js" => "js".to_string(), "ts" => "ts".to_string(),
        "cpp" => "cpp".to_string(), "go" => "go".to_string(), "java" => "java".to_string(), _ => "txt".to_string(),
    })?;
    status_cb(format!("Готово. Отчёт: {}", report_file), 100);
    Ok(report)
}

fn collect_tasks(opts: &CodingBenchOptions) -> Result<Vec<CodingTask>, String> {
    let mut tasks = Vec::new();
    for suite in &opts.suites {
        let mut suite_tasks = load_suite_tasks(&opts.tasks_dir, suite)?;
        if let Some(quick) = opts.quick_per_suite {
            suite_tasks.truncate(quick);
        }
        tasks.extend(suite_tasks);
    }
    Ok(tasks)
}

/// Выполняет одну задачу для модели: генерация → сборка → запуск теста.
/// Никогда не прерывает весь прогон — ошибки фиксируются в записи.
fn run_single_task(
    engine: &LlamaEngine,
    task: &CodingTask,
    opts: &CodingBenchOptions,
    model: &ModelToRun,
    log_cb: impl Fn(String) + Clone,
    cancel_flag: Arc<AtomicBool>,
) -> TaskResultRecord {
    if task.run_with == "skip" {
        return TaskResultRecord {
            task_id: task.id.clone(),
            suite: task.suite.clone(),
            category: task.category.clone(),
            language: task.language.clone(),
            passed: false,
            timed_out: false,
            exit_code: None,
            error: Some("Рантайм не установлен".to_string()),
            prompt_tokens: 0,
            generated_tokens: 0,
            prompt_tok_per_sec: 0.0,
            gen_tok_per_sec: 0.0,
            ttft_sec: 0.0,
            gen_elapsed_sec: 0.0,
            run_elapsed_ms: 0,
            stdout: String::new(),
            stderr: String::new(),
            solution_code: String::new(),
        };
    }

    let mut params = opts.config.model_params.get(&model.path).cloned().unwrap_or_default();
    params.temperature = task.temperature;
    let messages = vec![LlmMessage { role: "user".to_string(), content: task.model_prompt.clone() }];

    let generation = engine.generate_chat(
        &messages,
        task.max_tokens as usize,
        &params,
        &opts.config.prompt_format,
        false,
        cancel_flag.clone(),
        "coding_bench",
        |_progress: f32, _msg: &str| {},
        log_cb.clone(),
    );

    let (code, metrics, gen_err) = match generation {
        Ok(res) => (assemble_solution(task, &res.text), Some(res.metrics), None),
        Err(e) => (String::new(), None, Some(e)),
    };

    let mut record = TaskResultRecord {
        task_id: task.id.clone(),
        suite: task.suite.clone(),
        category: task.category.clone(),
        language: task.language.clone(),
        passed: false,
        timed_out: false,
        exit_code: None,
        error: gen_err,
        prompt_tokens: metrics.as_ref().map(|m| m.prompt_tokens).unwrap_or(0),
        generated_tokens: metrics.as_ref().map(|m| m.generated_tokens).unwrap_or(0),
        prompt_tok_per_sec: metrics.as_ref().map(|m| m.prompt_per_second).unwrap_or(0.0),
        gen_tok_per_sec: metrics.as_ref().map(|m| m.predicted_per_second).unwrap_or(0.0),
        ttft_sec: metrics.as_ref().map(|m| m.ttft_sec).unwrap_or(0.0),
        gen_elapsed_sec: metrics.as_ref().map(|m| m.elapsed_sec).unwrap_or(0.0),
        run_elapsed_ms: 0,
        stdout: String::new(),
        stderr: String::new(),
        solution_code: code.clone(),
    };

    if record.error.is_some() {
        return record;
    }

    match prepare_sandbox(model, task, &code) {
        Ok(sandbox) => match run_command(&task.run_cmd, &sandbox, task.timeout_sec, &opts.bins_dir) {
            Ok(verdict) => apply_verdict(&mut record, verdict),
            Err(e) => record.error = Some(e),
        },
        Err(e) => record.error = Some(e),
    }
    record
}

fn prepare_sandbox(model: &ModelToRun, task: &CodingTask, code: &str) -> Result<PathBuf, String> {
    let sandbox = std::env::temp_dir()
        .join("ko_coding_bench")
        .join(safe_name(&model.name))
        .join(&task.id);
    let _ = std::fs::remove_dir_all(&sandbox);
    std::fs::create_dir_all(&sandbox).map_err(|e| format!("Ошибка создания песочницы: {}", e))?;

    for f in &task.files {
        write_file(&sandbox, f)?;
    }
    let mut content = code.to_string();
    if append_test_to_solution(task) {
        content.push('\n');
        content.push_str(&task.test);
    }
    std::fs::write(sandbox.join(&task.solution_name), content)
        .map_err(|e| format!("Ошибка записи решения: {}", e))?;
    Ok(sandbox)
}

fn write_file(sandbox: &Path, f: &TaskFile) -> Result<(), String> {
    let path = sandbox.join(&f.name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Ошибка создания {}: {}", parent.display(), e))?;
    }
    std::fs::write(&path, &f.content).map_err(|e| format!("Ошибка записи {}: {}", path.display(), e))
}

fn apply_verdict(record: &mut TaskResultRecord, v: ExecVerdict) {
    record.passed = v.passed;
    record.timed_out = v.timed_out;
    record.exit_code = v.exit_code;
    record.stdout = v.stdout;
    record.stderr = v.stderr;
    record.run_elapsed_ms = v.elapsed_ms;
}

fn build_model_summary(
    model: &ModelToRun,
    records: Vec<TaskResultRecord>,
    kv_probe: Option<super::kv_probe::KvProbeResult>,
) -> ModelRunSummary {
    let total = records.len();
    let passed = records.iter().filter(|r| r.passed).count();
    let pass_rate = if total > 0 { passed as f64 * 100.0 / total as f64 } else { 0.0 };
    let avg = |f: fn(&TaskResultRecord) -> f64| -> f64 {
        let vals: Vec<f64> = records.iter().map(f).filter(|v| *v > 0.0).collect();
        if vals.is_empty() { 0.0 } else { vals.iter().sum::<f64>() / vals.len() as f64 }
    };
    ModelRunSummary {
        model_name: model.name.clone(),
        model_path: model.path.clone(),
        total,
        passed,
        pass_rate: (pass_rate * 100.0).round() / 100.0,
        avg_gen_tok_per_sec: (avg(|r| r.gen_tok_per_sec) * 100.0).round() / 100.0,
        avg_prompt_tok_per_sec: (avg(|r| r.prompt_tok_per_sec) * 100.0).round() / 100.0,
        avg_ttft_sec: (avg(|r| r.ttft_sec) * 100.0).round() / 100.0,
        kv_probe,
        tasks: records,
    }
}

fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_name_sanitizes() {
        assert_eq!(safe_name("Qwen/2.5"), "Qwen_2_5");
        assert_eq!(safe_name("ok-name"), "ok-name");
    }

    #[test]
    fn summary_computes_pass_rate() {
        let rec = |passed: bool| TaskResultRecord {
            task_id: "t".into(), suite: "s".into(), category: "codegen".into(),
            language: "python".into(), passed, timed_out: false, exit_code: None,
            error: None, prompt_tokens: 10, generated_tokens: 20,
            prompt_tok_per_sec: 100.0, gen_tok_per_sec: 30.0, ttft_sec: 0.5,
            gen_elapsed_sec: 1.0, run_elapsed_ms: 200, stdout: String::new(),
            stderr: String::new(), solution_code: String::new(),
        };
        let m = ModelToRun { path: "p".into(), name: "m".into() };
        let s = build_model_summary(&m, vec![rec(true), rec(false), rec(true)], None);
        assert_eq!(s.total, 3);
        assert_eq!(s.passed, 2);
        assert!((s.pass_rate - 66.67).abs() < 0.01);
    }

    #[test]
    fn avg_ignores_zeros() {
        let rec = |g: f64| TaskResultRecord {
            task_id: "t".into(), suite: "s".into(), category: "codegen".into(),
            language: "python".into(), passed: true, timed_out: false, exit_code: None,
            error: None, prompt_tokens: 0, generated_tokens: 0,
            prompt_tok_per_sec: 0.0, gen_tok_per_sec: g, ttft_sec: 0.0,
            gen_elapsed_sec: 0.0, run_elapsed_ms: 0, stdout: String::new(),
            stderr: String::new(), solution_code: String::new(),
        };
        let m = ModelToRun { path: "p".into(), name: "m".into() };
        let s = build_model_summary(&m, vec![rec(10.0), rec(0.0), rec(20.0)], None);
        assert!((s.avg_gen_tok_per_sec - 15.0).abs() < 0.01);
    }
}