// Оценка решения: сборка файла, запуск в песочнице с таймаутом, вердикт.
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::tasks::CodingTask;

const MAX_OUTPUT: usize = 6000;

/// Вердикт исполнения теста в песочнице.
#[derive(Debug, Clone, Default)]
pub struct ExecVerdict {
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub compile_ok: bool,
    pub stdout: String,
    pub stderr: String,
    pub elapsed_ms: u64,
}

/// Снимает markdown-обёртки ```lang ... ``` (если модель обернула код).
pub fn strip_markdown_fences(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.starts_with("```") {
        let mut lines = trimmed.lines();
        let _ = lines.next(); // открывающий ```lang
        let body: Vec<&str> = lines.take_while(|l| !l.trim_start().starts_with("```")).collect();
        return body.join("\n");
    }
    content.to_string()
}

/// Проверяет, что модель вернула полную сигнатуру (а не только тело).
fn has_signature(completion: &str, signature_re: Option<&str>) -> bool {
    signature_re
        .filter(|re| !re.is_empty())
        .map(|re| regex::Regex::new(re).map(|r| r.is_match(completion)).unwrap_or(false))
        .unwrap_or(true)
}

/// Собирает содержимое файла решения из ответа модели.
/// Для codegen/bugfix модель может вернуть только тело — тогда подставляем
/// сигнатуру из `prefix` (lenient-подход, как в официальных harness'ах).
pub fn assemble_solution(task: &CodingTask, completion: &str) -> String {
    let completion = strip_markdown_fences(completion);
    let completion = completion.trim_end();
    match task.category.as_str() {
        "refactor" => completion.to_string(),
        _ => {
            if task.prefix.is_some() && !has_signature(completion, task.signature_re.as_deref()) {
                format!("{}\n{}", task.prefix.as_deref().unwrap_or_default(), completion)
            } else {
                completion.to_string()
            }
        }
    }
}

/// Нужно ли конкатенировать тест к файлу решения (для refactor — нет:
/// тест лежит отдельным файлом и запускается через unittest).
pub fn append_test_to_solution(task: &CodingTask) -> bool {
    task.category != "refactor" && !task.test.trim().is_empty()
}

/// Разрешение имени рантайма в полный путь.
fn resolve_program(prog: &str, bins_dir: &Path) -> Result<String, String> {
    match prog {
        "python" => find_python_exe().ok_or_else(|| "python не найден в PATH".to_string()),
        "node" => find_on_path("node").ok_or_else(|| "node не найден в PATH".to_string()),
        "rustc" => find_on_path("rustc").ok_or_else(|| "rustc не найден в PATH".to_string()),
        "deno" => crate::infra::bin_downloader::ensure_runtime_bin("deno", bins_dir, |_| {})
            .map(|p| p.to_string_lossy().to_string()),
        other => Err(format!("Неизвестный рантайм '{}'", other)),
    }
}

fn find_on_path(name: &str) -> Option<String> {
    let exe = if cfg!(windows) { format!("{}.exe", name) } else { name.to_string() };
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let cand = dir.join(&exe);
        if cand.is_file() {
            return Some(cand.to_string_lossy().to_string());
        }
    }
    None
}

fn find_python_exe() -> Option<String> {
    if let Some(p) = find_on_path("python") {
        return Some(p);
    }
    // Известные пути установки Python на Windows.
    let home = std::env::var("LOCALAPPDATA").ok()?;
    let candidates = [
        format!(r"{}\Programs\Python\Python312\python.exe", home),
        format!(r"{}\Programs\Python\Python311\python.exe", home),
        r"C:\Program Files\Python312\python.exe".to_string(),
        r"D:\Programs\Python\Python312\python.exe".to_string(),
    ];
    candidates.into_iter().find(|p| Path::new(p).is_file())
}

/// Запуск команды в песочнице с таймаутом. `run_cmd` содержит токены
/// python/deno/node/rustc — они заменяются на полные пути.
pub fn run_command(cmd: &str, cwd: &Path, timeout_sec: u64, bins_dir: &Path) -> Result<ExecVerdict, String> {
    let mut parts = cmd.split_whitespace();
    let prog = parts.next().ok_or_else(|| "Пустая run_cmd".to_string())?;
    let resolved = resolve_program(prog, bins_dir)?;
    let rest: Vec<&str> = parts.collect();
    let full_cmd = format!("\"{}\" {}", resolved, rest.join(" "));

    let start = Instant::now();
    let mut child = if cfg!(windows) {
        Command::new("cmd")
    } else {
        Command::new("sh")
    };
    child
        .arg(if cfg!(windows) { "/C" } else { "-c" })
        .arg(&full_cmd)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = child
        .spawn()
        .map_err(|e| format!("Ошибка запуска {}: {}", full_cmd, e))?;

    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");
    let out_h = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_h = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let timeout = Duration::from_secs(timeout_sec.max(1));
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if start.elapsed() > timeout {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(format!("Ошибка wait: {}", e));
            }
        }
    };

    let out = out_h.join().unwrap_or_default();
    let err = err_h.join().unwrap_or_default();
    let elapsed_ms = start.elapsed().as_millis() as u64;

    let trunc = |b: Vec<u8>| -> String {
        let s = String::from_utf8_lossy(&b);
        let s = s.trim();
        if s.chars().count() > MAX_OUTPUT {
            let take: String = s.chars().take(MAX_OUTPUT).collect();
            format!("{}…\n[обрезано]", take)
        } else {
            s.to_string()
        }
    };

    Ok(ExecVerdict {
        exit_code: status.map(|s| s.code().unwrap_or(-1)),
        timed_out,
        // Процесс запустился и завершился (таймаут — единственный сбой запуска).
        compile_ok: !timed_out,
        stdout: trunc(out),
        stderr: trunc(err),
        elapsed_ms,
        passed: status.map(|s| s.success()).unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(category: &str, prefix: Option<&str>, re: Option<&str>) -> CodingTask {
        CodingTask {
            id: "t".to_string(),
            suite: "s".to_string(),
            language: "python".to_string(),
            category: category.to_string(),
            run_with: "python".to_string(),
            model_prompt: "x".to_string(),
            solution_name: "main.py".to_string(),
            prefix: prefix.map(|p| p.to_string()),
            entry_point: Some("f".to_string()),
            signature_re: re.map(|r| r.to_string()),
            test: "assert f() == 1".to_string(),
            run_cmd: "python main.py".to_string(),
            max_tokens: 512,
            temperature: 0.0,
            timeout_sec: 60,
            files: vec![],
        }
    }

    #[test]
    fn strips_markdown_fences() {
        assert_eq!(strip_markdown_fences("```python\nx=1\n```"), "x=1");
        assert_eq!(strip_markdown_fences("x=1"), "x=1");
    }

    #[test]
    fn codegen_prepends_prefix_when_body_only() {
        let t = task("codegen", Some("def f():\n"), Some("def\\s+f\\b"));
        assert_eq!(assemble_solution(&t, "    return 1"), "def f():\n\n    return 1");
    }

    #[test]
    fn codegen_keeps_full_signature() {
        let t = task("codegen", Some("def f():\n"), Some("def\\s+f\\b"));
        assert_eq!(assemble_solution(&t, "def f():\n    return 1"), "def f():\n    return 1");
    }

    #[test]
    fn refactor_takes_whole_file() {
        let t = task("refactor", None, None);
        let sol = assemble_solution(&t, "```python\nimport os\n```");
        assert_eq!(sol, "import os");
    }

    #[test]
    fn append_test_only_for_codegen_bugfix() {
        assert!(append_test_to_solution(&task("codegen", None, None)));
        assert!(append_test_to_solution(&task("bugfix", None, None)));
        assert!(!append_test_to_solution(&task("refactor", None, None)));
    }
}
