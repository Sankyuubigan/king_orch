// Отчёт coding-бенчмарка: JSON-файл + артефакты кода моделей.
use std::fs;
use std::path::Path;

use super::kv_probe::KvProbeResult;

/// Результат одной задачи для одной модели.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskResultRecord {
    pub task_id: String,
    pub suite: String,
    pub category: String,
    pub language: String,
    pub passed: bool,
    pub timed_out: bool,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub prompt_tokens: u32,
    pub generated_tokens: u32,
    pub prompt_tok_per_sec: f64,
    pub gen_tok_per_sec: f64,
    pub ttft_sec: f64,
    pub gen_elapsed_sec: f64,
    pub run_elapsed_ms: u64,
    pub stdout: String,
    pub stderr: String,
    /// Код, сгенерированный моделью (для артефактов).
    #[serde(skip)]
    pub solution_code: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelRunSummary {
    pub model_name: String,
    pub model_path: String,
    pub total: usize,
    pub passed: usize,
    pub pass_rate: f64,
    pub avg_gen_tok_per_sec: f64,
    pub avg_prompt_tok_per_sec: f64,
    pub avg_ttft_sec: f64,
    pub kv_probe: Option<KvProbeResult>,
    pub tasks: Vec<TaskResultRecord>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReportSummary {
    pub timestamp: String,
    pub budget_vram_mb: u64,
    pub quick_per_suite: Option<usize>,
    pub models: Vec<ModelRunSummary>,
    pub report_file: String,
    pub artifacts_dir: String,
}

fn timestamp() -> String {
    let now = chrono::Local::now();
    now.format("%Y-%m-%d_%H-%M-%S").to_string()
}

fn safe_filename(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' { c } else { '_' })
        .collect::<String>()
}

/// Пишет отчёт в tasks_dir/reports/coding_report_<ts>.json и возвращает путь.
pub fn write_report(tasks_dir: &Path, summary: &mut ReportSummary) -> Result<String, String> {
    let reports_dir = tasks_dir.join("reports");
    fs::create_dir_all(&reports_dir).map_err(|e| format!("Ошибка создания {}: {}", reports_dir.display(), e))?;

    summary.timestamp = timestamp();
    let file = reports_dir.join(format!("coding_report_{}.json", summary.timestamp));
    let json = serde_json::to_string_pretty(&summary)
        .map_err(|e| format!("Ошибка сериализации отчёта: {}", e))?;
    fs::write(&file, &json).map_err(|e| format!("Ошибка записи {}: {}", file.display(), e))?;
    summary.report_file = file.to_string_lossy().to_string();
    Ok(summary.report_file.clone())
}

/// Сохраняет код моделей: reports/artifacts/<model>/<task_id>.<ext>.
pub fn write_artifacts(
    tasks_dir: &Path,
    summaries: &[ModelRunSummary],
    extension: &dyn Fn(&str) -> String,
) -> Result<String, String> {
    let artifacts_dir = tasks_dir.join("reports").join("artifacts");
    fs::create_dir_all(&artifacts_dir).map_err(|e| format!("Ошибка создания {}: {}", artifacts_dir.display(), e))?;

    for model in summaries {
        let model_dir = artifacts_dir.join(safe_filename(&model.model_name));
        fs::create_dir_all(&model_dir).map_err(|e| format!("Ошибка создания {}: {}", model_dir.display(), e))?;
        for t in &model.tasks {
            if t.solution_code.trim().is_empty() {
                continue;
            }
            let ext = extension(&t.language);
            let file = model_dir.join(format!("{}.{}", t.task_id, ext));
            if let Err(e) = fs::write(&file, &t.solution_code) {
                return Err(format!("Ошибка записи {}: {}", file.display(), e));
            }
        }
    }
    Ok(artifacts_dir.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_summary() -> ReportSummary {
        ReportSummary {
            timestamp: "".to_string(),
            budget_vram_mb: 14_336,
            quick_per_suite: Some(2),
            models: vec![],
            report_file: "".to_string(),
            artifacts_dir: "".to_string(),
        }
    }

    #[test]
    fn safe_filename_replaces_bad_chars() {
        assert_eq!(safe_filename("Qwen/2.5-Coder-7B-Q4"), "Qwen_2.5-Coder-7B-Q4");
        assert_eq!(safe_filename("нормальное имя"), "нормальное_имя");
    }

    #[test]
    fn timestamp_has_expected_shape() {
        let ts = timestamp();
        assert_eq!(ts.len(), 19);
        assert_eq!(ts.as_bytes()[4], b'-');
        assert_eq!(ts.as_bytes()[7], b'-');
    }

    #[test]
    fn write_report_creates_file() {
        let dir = std::env::temp_dir().join("ko_report_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut s = sample_summary();
        let path = write_report(&dir, &mut s).unwrap();
        assert!(Path::new(&path).exists());
        assert!(s.report_file.ends_with(".json"));
        let _ = fs::remove_dir_all(&dir);
    }
}