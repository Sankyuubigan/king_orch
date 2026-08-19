//! Файловые тулы: чтение, пакетное чтение, запись, точечная правка.
//! Чтение — любой путь (без подтверждения). Запись — внутри корня авто,
//! вне корня — через `PermissionApprover` (плашка с 3 кнопками).

use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::Value;

use super::{Tool, ToolCtx, ToolError, is_within_root, resolve_path, truncate};

fn arg_str(args: &Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolError::Usage(format!("параметр '{}' (строка) обязателен", key)))
}

fn arg_int(args: &Value, key: &str, default: i64) -> i64 {
    args.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

/// Прочитать файл с номерами строк и фрагментацией (offset/limit).
fn read_file_with_lines(path: &Path, offset: usize, limit: usize) -> Result<String, ToolError> {
    let content = fs::read_to_string(path)
        .map_err(|e| ToolError::NotFound(format!("{}: {}", path.display(), e)))?;
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let start = offset.saturating_sub(1);
    let end = (start + limit).min(total);
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate().take(end).skip(start) {
        out.push_str(&format!("{:>4} | {}\n", i + 1, line));
    }
    if total > end {
        out.push_str(&format!("… [показаны строки {}-{} из {}]\n", start + 1, end, total));
    } else if start > 0 {
        out.push_str(&format!("… [показаны строки {}-{} из {}]\n", start + 1, end, total));
    }
    if out.is_empty() {
        out = format!("[файл пуст или нет строк в диапазоне: {} строк(и)]", total);
    }
    Ok(out)
}

/// Разрешить путь для записи и проверить право через approver.
/// Возвращает путь на диске (можно писать) или Forbidden-ошибку.
fn authorize_write(path: &Path, ctx: &ToolCtx) -> Result<(), ToolError> {
    let abs = resolve_path(ctx.workspace_root, &path.to_string_lossy());
    if is_within_root(ctx.workspace_root, &abs) {
        return Ok(()); // внутри корня — авто (с логами в диспетчере)
    }
    let parent_ok = abs
        .parent()
        .map(|p| is_within_root(ctx.workspace_root, p))
        .unwrap_or(false);
    if parent_ok {
        return Ok(());
    }
    // Вне корня — плашка пользователю.
    ctx.approver
        .check_write(&abs, ctx.session_id, ctx.agent_id, "write")
}

fn write_diff_summary(path: &Path, old: &str, new: &str) -> String {
    if old.is_empty() {
        format!("✅ Файл создан: {} ({} символов)", path.display(), new.chars().count())
    } else if old == new {
        format!("ℹ️ Файл {} уже содержит это содержимое — запись не требуется.", path.display())
    } else {
        let old_lines = old.lines().count();
        let new_lines = new.lines().count();
        format!(
            "✅ Файл обновлён: {} (строк: {} → {})",
            path.display(),
            old_lines,
            new_lines
        )
    }
}

/// `read_file` — чтение одного файла с номерами строк.
pub struct ReadFile;

impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Прочитать текстовый файл с номерами строк. path — путь к файлу (абсолютный или относительно корня проекта). offset — номер строки, с которой начать (1-based, по умолчанию 1). limit — сколько строк прочитать (по умолчанию 200). Чтение доступно по любому пути."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Путь к файлу (абсолютный или относительно корня проекта)"},
                "offset": {"type": "integer", "description": "Номер строки, с которой начать (1-based, по умолчанию 1)"},
                "limit": {"type": "integer", "description": "Сколько строк прочитать (по умолчанию 200)"}
            },
            "required": ["path"]
        })
    }
    fn is_readonly(&self) -> bool {
        true
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let path = resolve_path(ctx.workspace_root, &arg_str(args, "path")?);
        let offset = arg_int(args, "offset", 1).max(1) as usize;
        let limit = arg_int(args, "limit", 200).clamp(1, 2000) as usize;
        Ok(read_file_with_lines(&path, offset, limit)?)
    }
}

/// `read_many_files` — пакетное чтение нескольких файлов.
pub struct ReadManyFiles;

impl Tool for ReadManyFiles {
    fn name(&self) -> &str {
        "read_many_files"
    }
    fn description(&self) -> &str {
        "Прочитать несколько файлов одним вызовом. paths — массив путей (абсолютных или относительно корня проекта). Каждый файл выводится в блоке с заголовком. Экономит вызовы: вместо серии read_file."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "paths": {"type": "array", "items": {"type": "string"}, "description": "Массив путей к файлам"}
            },
            "required": ["paths"]
        })
    }
    fn is_readonly(&self) -> bool {
        true
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let paths = args
            .get("paths")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError::Usage("параметр 'paths' (массив строк) обязателен".to_string()))?;
        let mut out = String::new();
        for p in paths {
            let p_str = p
                .as_str()
                .ok_or_else(|| ToolError::Usage("элемент 'paths' должен быть строкой".to_string()))?;
            let abs = resolve_path(ctx.workspace_root, p_str);
            out.push_str(&format!("===== {} =====\n", abs.display()));
            match read_file_with_lines(&abs, 1, 2000) {
                Ok(content) => out.push_str(&content),
                Err(e) => out.push_str(&format!("⚠️ {}\n", e)),
            }
            out.push('\n');
        }
        Ok(truncate(&out, 16000))
    }
}

/// `write_file` — создать/перезаписать файл (внутри корня — авто).
pub struct WriteFile;

impl Tool for WriteFile {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Создать новый файл или перезаписать существующий. path — путь (абсолютный или относительно корня проекта). content — полное содержимое файла. Запись внутри корня разрешена автоматически; запись вне корня — запросит подтверждение пользователя. Для точечных правок используй edit_file, а не перезапись целиком."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Путь к файлу (абсолютный или относительно корня проекта)"},
                "content": {"type": "string", "description": "Полное содержимое файла"}
            },
            "required": ["path", "content"]
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let path_str = arg_str(args, "path")?;
        let content = arg_str(args, "content")?;
        let abs = resolve_path(ctx.workspace_root, &path_str);
        authorize_write(&abs, ctx)?;
        let old = fs::read_to_string(&abs).unwrap_or_default();
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| ToolError::Io(format!("не удалось создать папку {}: {}", parent.display(), e)))?;
        }
        let mut f = fs::File::create(&abs)
            .map_err(|e| ToolError::Io(format!("не удалось открыть {}: {}", abs.display(), e)))?;
        f.write_all(content.as_bytes())
            .map_err(|e| ToolError::Io(format!("не удалось записать {}: {}", abs.display(), e)))?;
        Ok(write_diff_summary(&abs, &old, &content))
    }
}

/// `edit_file` — точечная правка по точному совпадению фрагмента (exact-match).
pub struct EditFile;

impl Tool for EditFile {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "Точечная замена фрагмента файла по ТОЧНОМУ совпадению. path — путь к файлу; old_string — искомый фрагмент (должен совпадать 1:1, включая отступы); new_string — замена. replace_all — заменить все вхождения (по умолчанию только первое). Если old_string не найден — вернётся ошибка (тишина недопустима). Запись вне корня — запросит подтверждение."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Путь к файлу (абсолютный или относительно корня проекта)"},
                "old_string": {"type": "string", "description": "Искомый фрагмент (точное совпадение)"},
                "new_string": {"type": "string", "description": "Замена"},
                "replace_all": {"type": "boolean", "description": "Заменить все вхождения (по умолчанию false — только первое)"}
            },
            "required": ["path", "old_string", "new_string"]
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let path = resolve_path(ctx.workspace_root, &arg_str(args, "path")?);
        let old = arg_str(args, "old_string")?;
        let new = arg_str(args, "new_string")?;
        let replace_all = args.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
        if old.is_empty() {
            return Err(ToolError::Usage("old_string не может быть пустым — использовать write_file для создания файла".to_string()));
        }
        authorize_write(&path, ctx)?;
        let content = fs::read_to_string(&path)
            .map_err(|e| ToolError::NotFound(format!("{}: {}", path.display(), e)))?;
        if !content.contains(&old) {
            return Err(ToolError::NotFound(format!(
                "фрагмент не найден в {}: {:?}",
                path.display(),
                &old.chars().take(80).collect::<String>()
            )));
        }
        let new_content = if replace_all {
            content.replace(&old, &new)
        } else {
            content.replacen(&old, &new, 1)
        };
        let mut f = fs::File::create(&path)
            .map_err(|e| ToolError::Io(format!("не удалось открыть {}: {}", path.display(), e)))?;
        f.write_all(new_content.as_bytes())
            .map_err(|e| ToolError::Io(format!("не удалось записать {}: {}", path.display(), e)))?;
        Ok(format!(
            "✅ Замена выполнена в {} ({} → {}; {} вхождений)",
            path.display(),
            old.chars().count(),
            new.chars().count(),
            if replace_all {
                content.matches(&old).count()
            } else {
                1
            }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kingorch_tools_{}_{}", name, std::process::id()));
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
    fn read_file_numbers_lines_and_slices() {
        let d = tmpdir("read");
        let f = d.join("a.txt");
        fs::write(&f, "l1\nl2\nl3\nl4\nl5").unwrap();
        let ctx = ctx_for(&d);
        let r = ReadFile
            .execute(&serde_json::json!({"path": "a.txt", "offset": 2, "limit": 2}), &ctx)
            .unwrap();
        assert!(r.contains("2 | l2"));
        assert!(r.contains("3 | l3"));
        assert!(!r.contains("1 | l1"));
        assert!(r.contains("строки 2-3"));
    }

    #[test]
    fn read_file_missing_returns_not_found() {
        let d = tmpdir("read_missing");
        let ctx = ctx_for(&d);
        let err = ReadFile
            .execute(&serde_json::json!({"path": "nope.txt"}), &ctx)
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }

    #[test]
    fn write_file_creates_and_overwrites() {
        let d = tmpdir("write");
        let ctx = ctx_for(&d);
        let r1 = WriteFile
            .execute(&serde_json::json!({"path": "b.ts", "content": "hello"}), &ctx)
            .unwrap();
        assert!(r1.contains("создан"));
        assert_eq!(fs::read_to_string(d.join("b.ts")).unwrap(), "hello");
        let r2 = WriteFile
            .execute(&serde_json::json!({"path": "b.ts", "content": "world"}), &ctx)
            .unwrap();
        assert!(r2.contains("обновлён"));
        assert_eq!(fs::read_to_string(d.join("b.ts")).unwrap(), "world");
    }

    #[test]
    fn edit_file_exact_match_and_error_on_missing() {
        let d = tmpdir("edit");
        fs::write(d.join("c.rs"), "fn a(){}\nfn b(){}\n").unwrap();
        let ctx = ctx_for(&d);
        let r = EditFile
            .execute(
                &serde_json::json!({"path": "c.rs", "old_string": "fn b(){}", "new_string": "fn b2(){}"}),
                &ctx,
            )
            .unwrap();
        assert!(r.contains("Замена выполнена"));
        let content = fs::read_to_string(d.join("c.rs")).unwrap();
        assert!(content.contains("fn b2(){}"));
        assert!(content.contains("fn a(){}"));
        // Не найдено — НЕ молчим.
        let err = EditFile
            .execute(
                &serde_json::json!({"path": "c.rs", "old_string": "zzz", "new_string": "x"}),
                &ctx,
            )
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }

    #[test]
    fn edit_file_replace_all() {
        let d = tmpdir("edit_all");
        fs::write(d.join("d.txt"), "aXaXa").unwrap();
        let ctx = ctx_for(&d);
        EditFile
            .execute(
                &serde_json::json!({"path": "d.txt", "old_string": "X", "new_string": "Y", "replace_all": true}),
                &ctx,
            )
            .unwrap();
        assert_eq!(fs::read_to_string(d.join("d.txt")).unwrap(), "aYaYa");
    }

    #[test]
    fn write_outside_root_requires_approval() {
        let d = tmpdir("write_outside");
        let outside = std::env::temp_dir().join(format!("kingorch_outside_{}", std::process::id()));
        let _ = fs::remove_dir_all(&outside);
        let ctx = ctx_for(&d);
        // Тестовый approver отклоняет всё вне корня → Forbidden.
        let err = WriteFile
            .execute(
                &serde_json::json!({"path": outside.to_string_lossy().to_string(), "content": "x"}),
                &ctx,
            )
            .unwrap_err();
        assert!(matches!(err, ToolError::Forbidden(_)));
    }
}