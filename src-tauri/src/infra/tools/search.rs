//! Поисковые тулы: grep (regex), glob (маски), list_directory (дерево).
//! Все уважают `.gitignore` (через крейт `ignore` — движок ripgrep).
//! Read-only: авто-доступ без плашки.

use std::fs;
use std::path::Path;

use ignore::WalkBuilder;
use serde_json::Value;

use super::{Tool, ToolCtx, ToolError, resolve_path, truncate};

const DEFAULT_MAX_RESULTS: usize = 200;

/// Построить обходчик с уважением .gitignore. Пропускаем скрытые/игнор-файлы.
fn walker(path: &Path, max_depth: Option<usize>) -> ignore::Walk {
    let mut b = WalkBuilder::new(path);
    b.standard_filters(true); // .gitignore, .ignore, hidden
    // Применяем .gitignore даже вне git-репозитория (тесты, произвольные папки).
    b.require_git(false);
    if let Some(d) = max_depth {
        b.max_depth(Some(d));
    }
    b.build()
}

fn is_text_file(path: &Path) -> bool {
    // Грубая эвристика: пропускаем бинарники по расширению.
    const BINARY_EXT: [&str; 20] = [
        "png", "jpg", "jpeg", "gif", "webp", "ico", "bmp", "pdf", "zip", "gz",
        "tar", "exe", "dll", "so", "dylib", "bin", "o", "obj", "woff", "ttf",
    ];
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) => !BINARY_EXT.contains(&e.to_lowercase().as_str()),
        None => true,
    }
}

/// `grep` — поиск по регулярному выражению.
pub struct Grep;

impl Tool for Grep {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Поиск по регулярному выражению (regex) в файлах проекта с учётом .gitignore. pattern — регулярное выражение; path — папка или файл для поиска (по умолчанию корень проекта); max_results — лимит совпадений (по умолчанию 200); case_sensitive — учёт регистра (по умолчанию false). Вывод: файл:строка: текст."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Регулярное выражение (regex) для поиска"},
                "path": {"type": "string", "description": "Папка или файл для поиска (по умолчанию корень проекта)"},
                "max_results": {"type": "integer", "description": "Лимит совпадений (по умолчанию 200)"},
                "case_sensitive": {"type": "boolean", "description": "Учитывать регистр (по умолчанию false)"}
            },
            "required": ["pattern"]
        })
    }
    fn is_readonly(&self) -> bool {
        true
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::Usage("параметр 'pattern' (строка) обязателен".to_string()))?;
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_MAX_RESULTS as u64)
            .clamp(1, 2000) as usize;
        let case_sensitive = args.get("case_sensitive").and_then(|v| v.as_bool()).unwrap_or(false);
        let base = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| resolve_path(ctx.workspace_root, p))
            .unwrap_or_else(|| ctx.workspace_root.to_path_buf());

        let re = regex::RegexBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|e| ToolError::Usage(format!("невалидный regex '{}': {}", pattern, e)))?;

        let mut out = String::new();
        let mut count = 0usize;
        let mut matched_files = 0usize;

        if base.is_file() {
            if let Some(lines) = grep_file(&base, &re, &mut count, max_results) {
                if !lines.is_empty() {
                    matched_files += 1;
                    out.push_str(&lines);
                }
            }
        } else if base.is_dir() {
            for entry in walker(&base, None) {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if count >= max_results {
                    break;
                }
                let path = entry.path();
                if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }
                if !is_text_file(path) {
                    continue;
                }
                if let Some(lines) = grep_file(path, &re, &mut count, max_results) {
                    if !lines.is_empty() {
                        matched_files += 1;
                        out.push_str(&lines);
                    }
                }
            }
        } else {
            return Err(ToolError::NotFound(format!("путь не найден: {}", base.display())));
        }

        if out.is_empty() {
            return Ok(format!("🔍 Совпадений по '{}' не найдено.", pattern));
        }
        out.push_str(&format!("\n--- Найдено {} совпадений в {} файлах ---", count, matched_files));
        Ok(truncate(&out, 16000))
    }
}

fn grep_file(path: &Path, re: &regex::Regex, count: &mut usize, max: usize) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let mut out = String::new();
    for (i, line) in content.lines().enumerate() {
        if *count >= max {
            break;
        }
        if re.is_match(line) {
            *count += 1;
            out.push_str(&format!("{}:{}: {}\n", path.display(), i + 1, truncate(line, 300)));
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// `glob` — поиск файлов по маске.
pub struct Glob;

impl Tool for Glob {
    fn name(&self) -> &str {
        "glob"
    }
    fn description(&self) -> &str {
        "Найти файлы по маске (glob) с учётом .gitignore. pattern — маска, например 'src/**/*.ts' или '*.rs'; path — папка для поиска (по умолчанию корень проекта); max_results — лимит (по умолчанию 200). Возвращает список путей."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Маска glob (например 'src/**/*.ts')"},
                "path": {"type": "string", "description": "Папка для поиска (по умолчанию корень проекта)"},
                "max_results": {"type": "integer", "description": "Лимит результатов (по умолчанию 200)"}
            },
            "required": ["pattern"]
        })
    }
    fn is_readonly(&self) -> bool {
        true
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::Usage("параметр 'pattern' (строка) обязателен".to_string()))?;
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_MAX_RESULTS as u64)
            .clamp(1, 2000) as usize;
        let base = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| resolve_path(ctx.workspace_root, p))
            .unwrap_or_else(|| ctx.workspace_root.to_path_buf());

        let matcher = globset::Glob::new(pattern)
            .map_err(|e| ToolError::Usage(format!("невалидная маска '{}': {}", pattern, e)))?
            .compile_matcher();

        let mut out = String::new();
        let mut count = 0usize;
        for entry in walker(&base, None) {
            if count >= max_results {
                break;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let rel = entry.path().strip_prefix(&base).unwrap_or(entry.path());
            if matcher.is_match(rel) {
                out.push_str(&format!("{}\n", entry.path().display()));
                count += 1;
            }
        }
        if out.is_empty() {
            return Ok(format!("🔍 По маске '{}' файлов не найдено.", pattern));
        }
        Ok(truncate(&out, 16000))
    }
}

/// `list_directory` — дерево папки с учётом .gitignore.
pub struct ListDirectory;

impl Tool for ListDirectory {
    fn name(&self) -> &str {
        "list_directory"
    }
    fn description(&self) -> &str {
        "Показать содержимое папки в виде дерева с учётом .gitignore. path — папка (по умолчанию корень проекта); depth — глубина обхода (по умолчанию 2). Чтение доступно по любому пути."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Папка для показа (по умолчанию корень проекта)"},
                "depth": {"type": "integer", "description": "Глубина обхода (по умолчанию 2)"}
            },
            "required": []
        })
    }
    fn is_readonly(&self) -> bool {
        true
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let base = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| resolve_path(ctx.workspace_root, p))
            .unwrap_or_else(|| ctx.workspace_root.to_path_buf());
        let depth = args
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(2)
            .clamp(0, 6) as usize;

        if !base.is_dir() {
            return Err(ToolError::NotFound(format!("папка не найдена: {}", base.display())));
        }

        let mut out = format!("📁 {}\n", base.display());
        for entry in walker(&base, Some(depth)) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let rel = entry.path().strip_prefix(&base).unwrap_or(entry.path());
            let parts: Vec<_> = rel.components().collect();
            let indent = "  ".repeat(parts.len().saturating_sub(1));
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let mark = if is_dir { "📁" } else { "📄" };
            out.push_str(&format!("{}{} {}\n", indent, mark, name));
        }
        Ok(truncate(&out, 16000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kingorch_search_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn ctx_for(root: &Path) -> ToolCtx<'_> {
        ToolCtx {
            workspace_root: root,
            session_id: "test",
            approver: crate::infra::permissions::test_approver(),
            agent_id: "test_agent",
            bins_dir: root,
        }
    }

    #[test]
    fn grep_finds_matches_and_respects_ignore() {
        let d = tmpdir("grep");
        fs::create_dir_all(d.join("src")).unwrap();
        fs::write(d.join("src/a.ts"), "const foo = 1;\nbar").unwrap();
        fs::write(d.join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(d.join("ignored.txt"), "foo inside ignored\n").unwrap();
        let ctx = ctx_for(&d);
        let r = Grep
            .execute(&serde_json::json!({"pattern": "foo"}), &ctx)
            .unwrap();
        assert!(r.contains("a.ts"));
        assert!(r.contains("1 совпадений") || r.contains("1 совпадение"));
        assert!(!r.contains("ignored.txt"), "gitignore должен исключать ignored.txt");
    }

    #[test]
    fn grep_bad_regex_returns_usage() {
        let d = tmpdir("grep_bad");
        let ctx = ctx_for(&d);
        let err = Grep
            .execute(&serde_json::json!({"pattern": "([unclosed"}), &ctx)
            .unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)));
    }

    #[test]
    fn glob_matches_patterns() {
        let d = tmpdir("glob");
        fs::create_dir_all(d.join("src").join("deep")).unwrap();
        fs::write(d.join("src/a.ts"), "").unwrap();
        fs::write(d.join("src/deep/b.ts"), "").unwrap();
        fs::write(d.join("src/c.js"), "").unwrap();
        let ctx = ctx_for(&d);
        let r = Glob
            .execute(&serde_json::json!({"pattern": "**/*.ts"}), &ctx)
            .unwrap();
        assert!(r.contains("a.ts"));
        assert!(r.contains("b.ts"));
        assert!(!r.contains("c.js"));
    }

    #[test]
    fn list_directory_shows_tree() {
        let d = tmpdir("list");
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("top.txt"), "").unwrap();
        fs::write(d.join("sub/nested.txt"), "").unwrap();
        let ctx = ctx_for(&d);
        let r = ListDirectory
            .execute(&serde_json::json!({"depth": 2}), &ctx)
            .unwrap();
        assert!(r.contains("top.txt"));
        assert!(r.contains("sub"));
    }
}