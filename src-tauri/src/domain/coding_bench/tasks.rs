// Задачи coding-бенчмарка: загрузка из tasks_for_test_llm/<suite>/tasks.jsonl.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Дополнительный файл, который кладётся в песочницу задачи
/// (например, benchmark/refactor_tools.py для проверки рефакторинга).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskFile {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodingTask {
    pub id: String,
    pub suite: String,
    pub language: String,
    /// codegen | bugfix | refactor
    pub category: String,
    /// python | deno | node | rust | skip
    pub run_with: String,
    /// Текст, который отправляется модели как user-сообщение.
    pub model_prompt: String,
    /// Имя файла, куда пишется код модели.
    pub solution_name: String,
    /// Заглушка функции (сигнатура + докстринг) для lenient-сборки.
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub entry_point: Option<String>,
    /// Регэксп для определения «модель вернула сигнатуру целиком».
    #[serde(default)]
    pub signature_re: Option<String>,
    /// Тест (конкатенируется к решению для codegen/bugfix).
    #[serde(default)]
    pub test: String,
    /// Команда запуска (токены python/deno/node/rustc заменяются на пути).
    /// None у задач без рантайма (`run_with: "skip"`): явный `null` в tasks.jsonl —
    /// валидное состояние, а не грязные данные (инцидент 05.11.2026: humanevalpack
    /// cpp/go/java писали `null` и роняли парсер структуры со `String`).
    #[serde(default)]
    pub run_cmd: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub temperature: f32,
    #[serde(default = "default_timeout_sec")]
    pub timeout_sec: u64,
    #[serde(default)]
    pub files: Vec<TaskFile>,
}

fn default_max_tokens() -> u32 { 512 }
fn default_timeout_sec() -> u64 { 60 }

/// Сводка по набору (для UI): язык, категории, число задач.
#[derive(Debug, Clone, Serialize)]
pub struct SuiteInfo {
    pub id: String,
    pub language: String,
    pub total: usize,
    pub runnable: usize,
    pub categories: HashMap<String, usize>,
    /// Текст ошибки загрузки набора (битый tasks.jsonl и т.п.). Изоляция ошибок:
    /// один битый набор не роняет список — UI показывает его с пометкой.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Перечисляет наборы в tasks_dir (по подпапкам с tasks.jsonl).
pub fn list_suites(tasks_dir: &Path) -> Result<Vec<SuiteInfo>, String> {
    let mut suites = Vec::new();
    let entries = fs::read_dir(tasks_dir).map_err(|e| format!("Нет папки тестов {}: {}", tasks_dir.display(), e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || path.file_name().map(|n| n == "reports").unwrap_or(false) {
            continue;
        }
        let task_file = path.join("tasks.jsonl");
        if !task_file.exists() {
            continue;
        }
        let suite_id = entry.file_name().to_string_lossy().to_string();
        let tasks = match load_suite_tasks(tasks_dir, &suite_id) {
            Ok(t) => t,
            Err(e) => {
                // Изоляция ошибок (§00.2): раньше `?` пробрасывал ошибку одного
                // битого набора на всю вкладку («Ошибка загрузки» вместо списка).
                log::warn!("Набор '{}' недоступен: {}", suite_id, e);
                suites.push(SuiteInfo {
                    id: suite_id,
                    language: "unknown".to_string(),
                    total: 0,
                    runnable: 0,
                    categories: HashMap::new(),
                    error: Some(e),
                });
                continue;
            }
        };
        let mut categories: HashMap<String, usize> = HashMap::new();
        let mut language = "unknown".to_string();
        let mut runnable = 0;
        for t in &tasks {
            *categories.entry(t.category.clone()).or_insert(0) += 1;
            language = t.language.clone();
            if t.run_with != "skip" {
                runnable += 1;
            }
        }
        suites.push(SuiteInfo {
            id: entry.file_name().to_string_lossy().to_string(),
            language,
            total: tasks.len(),
            runnable,
            categories,
            error: None,
        });
    }
    suites.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(suites)
}

/// Загружает все задачи набора из tasks.jsonl.
pub fn load_suite_tasks(tasks_dir: &Path, suite: &str) -> Result<Vec<CodingTask>, String> {
    let path = tasks_dir.join(suite).join("tasks.jsonl");
    let content = fs::read_to_string(&path).map_err(|e| format!("Ошибка чтения {}: {}", path.display(), e))?;
    let mut tasks = Vec::new();
    for (i, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let task: CodingTask = serde_json::from_str(line)
            .map_err(|e| format!("Ошибка парсинга {}:{}: {}", path.display(), i + 1, e))?;
        tasks.push(task);
    }
    if tasks.is_empty() {
        return Err(format!("Набор '{}' пуст ({}). Запустите scripts/fetch_coding_tasks.cjs", suite, path.display()));
    }
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task() -> CodingTask {
        serde_json::from_str(r#"{
            "id": "t_0",
            "suite": "s",
            "language": "python",
            "category": "codegen",
            "run_with": "python",
            "model_prompt": "def f():",
            "solution_name": "main.py",
            "prefix": "def f():",
            "entry_point": "f",
            "signature_re": "def\\s+f\\b",
            "test": "assert f() == 1",
            "run_cmd": "python main.py",
            "max_tokens": 512,
            "temperature": 0,
            "timeout_sec": 60,
            "files": []
        }"#).unwrap()
    }

    #[test]
    fn parses_suite_task() {
        let t = sample_task();
        assert_eq!(t.category, "codegen");
        assert_eq!(t.run_with, "python");
        assert_eq!(t.entry_point.as_deref(), Some("f"));
        assert!(t.prefix.is_some());
        assert_eq!(t.run_cmd.as_deref(), Some("python main.py"));
    }

    /// Регрессия инцидента 05.11.2026: humanevalpack cpp/go/java пишут
    /// `"run_cmd":null` — явный null не должен ронять парсер.
    #[test]
    fn parses_task_with_null_run_cmd() {
        let t: CodingTask = serde_json::from_str(r#"{
            "id": "t_0",
            "suite": "s",
            "language": "cpp",
            "category": "bugfix",
            "run_with": "skip",
            "model_prompt": "",
            "solution_name": "main.py",
            "test": "",
            "run_cmd": null
        }"#).unwrap();
        assert_eq!(t.run_cmd, None);
        assert_eq!(t.run_with, "skip");
    }

    /// Поле может и отсутствовать: serde default даёт None без ошибки.
    #[test]
    fn parses_task_without_run_cmd_field() {
        let t: CodingTask = serde_json::from_str(r#"{
            "id": "t_0",
            "suite": "s",
            "language": "python",
            "category": "codegen",
            "run_with": "python",
            "model_prompt": "p",
            "solution_name": "main.py",
            "test": ""
        }"#).unwrap();
        assert_eq!(t.run_cmd, None);
    }

    #[test]
    fn suite_info_builds_from_tasks() {
        let mut categories: HashMap<String, usize> = HashMap::new();
        categories.insert("codegen".to_string(), 2);
        let info = SuiteInfo {
            id: "s".to_string(),
            language: "python".to_string(),
            total: 2,
            runnable: 2,
            categories,
            error: None,
        };
        assert_eq!(info.language, "python");
        assert_eq!(info.runnable, 2);
    }

    /// Изоляция ошибок: битый tasks.jsonl одного набора не роняет список —
    /// набор возвращается с error, соседний грузится целиком.
    #[test]
    fn list_suites_isolates_broken_suite() {
        let root = std::env::temp_dir().join(format!("ko_suites_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let good = root.join("good_suite");
        let bad = root.join("bad_suite");
        fs::create_dir_all(&good).unwrap();
        fs::create_dir_all(&bad).unwrap();
        fs::write(
            good.join("tasks.jsonl"),
            r#"{"id":"g_0","suite":"good_suite","language":"python","category":"codegen","run_with":"python","model_prompt":"p","solution_name":"main.py","test":"","run_cmd":"python main.py"}"#,
        )
        .unwrap();
        fs::write(bad.join("tasks.jsonl"), "{not json}\n").unwrap();

        let suites = list_suites(&root).unwrap();
        let good_info = suites.iter().find(|s| s.id == "good_suite").expect("good_suite в списке");
        let bad_info = suites.iter().find(|s| s.id == "bad_suite").expect("bad_suite в списке");
        assert!(good_info.error.is_none());
        assert_eq!(good_info.total, 1);
        assert_eq!(good_info.runnable, 1);
        assert!(bad_info.error.is_some());
        assert_eq!(bad_info.total, 0);
        assert_eq!(bad_info.runnable, 0);

        let _ = fs::remove_dir_all(&root);
    }
}
