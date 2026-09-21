//! Поисковые тулы: grep (regex), glob (маски), list_directory (дерево).
//! Все уважают `.gitignore` (через крейт `ignore` — движок ripgrep).
//! Read-only: авто-доступ без плашки.

use std::fs;
use std::path::Path;

use ignore::WalkBuilder;
use serde_json::Value;

use super::{Tool, ToolCtx, ToolError, resolve_path, truncate};

const DEFAULT_MAX_RESULTS: usize = 200;
/// Байтовый бюджет вывода grep (как MAX_TOTAL_BYTES в minimax-code).
const MAX_GREP_TOTAL_BYTES: usize = 256 * 1024;
/// Строка дольше предела режется с пометкой (read_file для полных строк).
const MAX_GREP_LINE_CHARS: usize = 2000;

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

/// Режимы вывода grep (как у minimax-code). Дефолт — `content` (совместимость
/// с существующими агентами); экономный `files_with_matches`/`count` — явно.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GrepMode {
    FilesWithMatches,
    Content,
    Count,
}

impl GrepMode {
    fn parse(s: Option<&str>) -> Option<Self> {
        match s.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            None | Some("content") => Some(GrepMode::Content),
            Some("files_with_matches") => Some(GrepMode::FilesWithMatches),
            Some("count") => Some(GrepMode::Count),
            Some(_) => None,
        }
    }
    fn name(&self) -> &'static str {
        match self {
            GrepMode::FilesWithMatches => "files_with_matches",
            GrepMode::Content => "content",
            GrepMode::Count => "count",
        }
    }
}

/// Обрезать до байтов на границе UTF-8 и дроп оторванный хвост до последнего '\n'.
fn cap_output_bytes(out: &str, cap: usize) -> &str {
    if out.len() <= cap {
        return out;
    }
    let mut end = 0usize;
    for (idx, _) in out.char_indices() {
        if idx > cap {
            break;
        }
        end = idx;
    }
    let slice = &out[..end];
    match slice.rfind('\n') {
        Some(nl) => &slice[..=nl],
        None => slice,
    }
}

impl Tool for Grep {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Поиск по регулярному выражению (regex) в файлах проекта с учётом .gitignore. pattern — регулярное выражение; path — папка или файл (по умолчанию корень проекта); mode — 'content' (строки совпадений, по умолчанию), 'files_with_matches' (только список файлов — дёшево, для локации), 'count' (сколько вхождений на файл); max_results — лимит страницы (по умолчанию 200); offset — пропустить первые N совпадений (для продолжения используй offset из подсказки); case_sensitive — учёт регистра (по умолчанию false). Вывод: файл:строка: текст."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Регулярное выражение (regex) для поиска"},
                "path": {"type": "string", "description": "Папка или файл для поиска (по умолчанию корень проекта)"},
                "mode": {"type": "string", "enum": ["content", "files_with_matches", "count"], "description": "Режим вывода (по умолчанию content)"},
                "max_results": {"type": "integer", "description": "Лимит страницы (по умолчанию 200)"},
                "offset": {"type": "integer", "description": "Сколько первых совпадений пропустить (для продолжения страниц)"},
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
        let mode = GrepMode::parse(args.get("mode").and_then(|v| v.as_str())).ok_or_else(|| {
            ToolError::Usage("параметр 'mode' должен быть files_with_matches | content | count".to_string())
        })?;
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_MAX_RESULTS as u64)
            .clamp(1, 2000) as usize;
        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
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

        let window_end = offset + max_results;
        let mut out = String::new();
        let mut shown = 0usize; // элементов в окне, уже выведено
        let mut total_in_window = 0usize; // найдено всего (считаем и за окном)
        let mut more = false;
        let mut lines_clamped = false;

        // Соберём текстовые файлы для обхода (уважая .gitignore).
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        if base.is_file() {
            if is_text_file(&base) {
                files.push(base.clone());
            }
        } else if base.is_dir() {
            for entry in walker(&base, None) {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }
                if is_text_file(entry.path()) {
                    files.push(entry.path().to_path_buf());
                }
            }
        } else {
            return Err(ToolError::NotFound(format!("путь не найден: {}", base.display())));
        }

        'outer: for path in &files {
            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue, // битый/не читаемый файл пропускаем
            };
            match mode {
                GrepMode::FilesWithMatches => {
                    if !content.lines().any(|l| re.is_match(l)) {
                        continue;
                    }
                    total_in_window += 1;
                    if total_in_window > offset && total_in_window <= window_end {
                        shown += 1;
                        out.push_str(&format!("{}\n", path.display()));
                    }
                    if total_in_window > window_end {
                        more = true;
                        break 'outer;
                    }
                }
                GrepMode::Count => {
                    let occ: usize = content.lines().map(|l| re.find_iter(l).count()).sum();
                    if occ == 0 {
                        continue;
                    }
                    total_in_window += 1;
                    if total_in_window > offset && total_in_window <= window_end {
                        shown += 1;
                        out.push_str(&format!("{}: {}\n", path.display(), occ));
                    }
                    if total_in_window > window_end {
                        more = true;
                        break 'outer;
                    }
                }
                GrepMode::Content => {
                    for (i, line) in content.lines().enumerate() {
                        if !re.is_match(line) {
                            continue;
                        }
                        total_in_window += 1;
                        if total_in_window <= offset {
                            continue;
                        }
                        if total_in_window > window_end {
                            more = true;
                            break 'outer;
                        }
                        shown += 1;
                        out.push_str(&format!("{}:{}: ", path.display(), i + 1));
                        if line.chars().count() > MAX_GREP_LINE_CHARS {
                            lines_clamped = true;
                            let head: String = line.chars().take(MAX_GREP_LINE_CHARS).collect();
                            out.push_str(&head);
                        } else {
                            out.push_str(line);
                        }
                        out.push('\n');
                    }
                }
            }
        }

        if shown == 0 {
            let kw = match mode {
                GrepMode::FilesWithMatches => "файлов",
                GrepMode::Count => "вхождений",
                GrepMode::Content => "совпадений",
            };
            return Ok(format!(
                "🔍 По '{}' (mode: {}) в окне offset=..{} {} не найдено.",
                pattern,
                mode.name(),
                offset + max_results,
                kw
            ));
        }

        // Notice'и: лимит страницы, обрезка длинных строк.
        let mut notices: Vec<String> = Vec::new();
        if more {
            notices.push(format!(
                "достигнут лимит страницы {} — ещё есть; продолжай с offset={}, или уточни pattern",
                max_results,
                offset + max_results
            ));
        }
        if lines_clamped {
            notices.push(format!(
                "некоторые строки обрезаны до {} символов — используй read_file для полных строк",
                MAX_GREP_LINE_CHARS
            ));
        }
        let base_output = out;
        let capped = cap_output_bytes(&base_output, MAX_GREP_TOTAL_BYTES);
        if capped.len() < base_output.len() {
            notices.push(format!("достигнут лимит вывода {}KB", MAX_GREP_TOTAL_BYTES / 1024));
        }
        let mut result = String::from(capped);
        if result.ends_with('\n') {
            result.pop();
        }
        if !notices.is_empty() {
            result.push_str(&format!("\n\n[{}]", notices.join(". ")));
        }
        Ok(result)
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
            write_root: root,
            write_outside: crate::infra::WriteOutside::Prompt,
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
        assert!(r.contains("const foo = 1;"), "дефолт mode=content: {}", r);
        assert!(!r.contains("ignored.txt"), "gitignore должен исключать ignored.txt");
    }

    #[test]
    fn grep_files_with_matches_mode_lists_files_only() {
        let d = tmpdir("grep_files");
        fs::write(d.join("a.ts"), "foo\n").unwrap();
        fs::write(d.join("b.txt"), "foo\n").unwrap();
        fs::write(d.join("c.md"), "no match\n").unwrap();
        let ctx = ctx_for(&d);
        let r = Grep
            .execute(&serde_json::json!({"pattern": "foo", "mode": "files_with_matches"}), &ctx)
            .unwrap();
        assert!(r.contains("a.ts") && r.contains("b.txt"));
        assert!(!r.contains("foo\n"), "не должно быть тел строк в files_with_matches: {}", r);
        assert!(!r.contains("c.md"));
    }

    #[test]
    fn grep_count_mode_reports_occurrences() {
        let d = tmpdir("grep_count");
        fs::write(d.join("a.ts"), "foo foo bar\nfoo\n").unwrap();
        fs::write(d.join("b.txt"), "nope\n").unwrap();
        let ctx = ctx_for(&d);
        let r = Grep
            .execute(&serde_json::json!({"pattern": "foo", "mode": "count"}), &ctx)
            .unwrap();
        assert!(r.contains("a.ts: 3"), "три вхождения в a.ts: {}", r);
        assert!(!r.contains("b.txt"), "без совпадений не выводится: {}", r);
    }

    #[test]
    fn grep_offset_continues_window() {
        let d = tmpdir("grep_offset");
        fs::write(d.join("a.txt"), "hit\nhit\nhit\n").unwrap();
        let ctx = ctx_for(&d);
        let r1 = Grep
            .execute(&serde_json::json!({"pattern": "hit", "max_results": 2}), &ctx)
            .unwrap();
        assert!(r1.contains("a.txt:1: hit"));
        assert!(r1.contains("a.txt:2: hit"));
        assert!(!r1.contains("a.txt:3: hit"));
        assert!(r1.contains("offset=2"), "hint продолжения: {}", r1);
        let r2 = Grep
            .execute(&serde_json::json!({"pattern": "hit", "max_results": 2, "offset": 2}), &ctx)
            .unwrap();
        assert!(r2.contains("a.txt:3: hit"), "offset должен продолжить: {}", r2);
        assert!(!r2.contains("a.txt:1: hit"));
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

#[test]
    fn cap_output_bytes_drops_torn_tail() {
        assert_eq!(cap_output_bytes("abc\ndef", 100), "abc\ndef");
        // Обрезка попадает в середину строки → дроп до последнего '\n'.
        let capped = cap_output_bytes("abcdef\nghijkl", 12);
        assert!(capped.ends_with("abcdef\n"), "оторванный хвост должен быть срезан: {}", capped);
        // Кириллица (небезопасная байтовая граница) не паникует и не рвёт символы.
        let capped = cap_output_bytes("ююююю", 4);
        assert_eq!(capped, "юю", "ровно 4 байта на char-границе: {}", capped);
    }
}