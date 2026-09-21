//! Файловые тулы: чтение, пакетное чтение, запись, точечная правка.
//! Чтение — любой путь (без подтверждения). Запись — внутри корня авто,
//! вне корня — через `PermissionApprover` (плашка с 3 кнопками).

use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::Value;

use super::{Tool, ToolCtx, ToolError, authorize_write, resolve_path, truncate};

fn arg_str(args: &Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolError::Usage(format!("параметр '{}' (строка) обязателен", key)))
}

fn arg_int(args: &Value, key: &str, default: i64) -> i64 {
    args.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

/// Одна строка длиннее этого предела режется с пометкой (и модель знает,
/// что нельзя копировать её в edit_file целиком — иначе exact-match упадёт).
const MAX_READ_LINE_CHARS: usize = 2000;

/// Резервные имена устройств Windows. Проверка по stem: `NUL.txt` тоже устройство.
const DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CLOCK$",
    "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Бинарные расширения: модель читает только текст (как binary-guard у minimax-code).
const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tiff", "tif", "avif",
    "mp4", "avi", "mkv", "mov", "webm", "flv", "wmv", "m4v", "mpg", "mpeg",
    "mp3", "wav", "flac", "ogg", "oga", "aac", "m4a", "wma", "opus",
    "zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz", "zst", "lz4",
    "exe", "dll", "so", "dylib", "bin", "obj", "o", "class", "jar", "war",
    "wasm", "pyc", "pyo", "pyd", "pdb", "deb", "rpm", "msi",
    "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "pdf",
    "ttf", "otf", "woff", "woff2", "eot",
    "db", "sqlite", "sqlite3", "mdb", "dbf", "accdb",
    "gguf", "ggml", "safetensors", "ckpt", "onnx", "pb", "tflite", "mlmodel", "pt", "pth",
    "iso", "img", "dmg", "lib", "a", "rlib",
];

fn is_windows_device_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let stem = name.split('.').next().unwrap_or(name);
    let upper = stem.to_ascii_uppercase();
    DEVICE_NAMES.contains(&upper.as_str())
}

fn is_binary_extension(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    BINARY_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

/// Сэмпл выглядит бинарным: встречен NUL-байт, либо доля управляющих
/// символов больше 30% (как 4КБ-сэмпл у read-guards minimax-code).
fn detect_binary_bytes(sample: &[u8]) -> bool {
    if sample.contains(&0) {
        return true;
    }
    if sample.is_empty() {
        return false;
    }
    let control = sample
        .iter()
        .filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r'))
        .count();
    control * 100 / sample.len() > 30
}

/// Гарды до чтения: устройство, бинарное расширение, не-регулярный файл.
fn guard_read_path(path: &Path) -> Result<(), ToolError> {
    if is_windows_device_path(path) {
        return Err(ToolError::Forbidden(format!(
            "{}: зарезервированное устройство Windows. Модель читает только обычные файлы — используй bash, если нужен реальный доступ",
            path.display()
        )));
    }
    if is_binary_extension(path) {
        return Err(ToolError::Forbidden(format!(
            "{}: бинарный файл. Модель читает только текст — используй bash для работы с этим файлом",
            path.display()
        )));
    }
    match fs::metadata(path) {
        Ok(md) => {
            if !md.is_file() {
                return Err(ToolError::Forbidden(format!(
                    "{}: не обычный файл (каталог/устройство/pipe)",
                    path.display()
                )));
            }
        }
        Err(e) => return Err(ToolError::NotFound(format!("{}: {}", path.display(), e))),
    }
    Ok(())
}

/// Прочитать файл с номерами строк и фрагментацией (offset/limit).
/// Формат строки — `     N→ текст` (5-поз. номер + стрелка), как у minimax-code.
fn read_file_with_lines(path: &Path, offset: usize, limit: usize) -> Result<String, ToolError> {
    guard_read_path(path)?;
    let bytes = fs::read(path)
        .map_err(|e| ToolError::NotFound(format!("{}: {}", path.display(), e)))?;
    if detect_binary_bytes(&bytes[..bytes.len().min(4096)]) {
        return Err(ToolError::Forbidden(format!(
            "{}: содержимое выглядит бинарным. Модель читает только текст — используй bash для работы с этим файлом",
            path.display()
        )));
    }
    let content = String::from_utf8(bytes).map_err(|_| {
        ToolError::Forbidden(format!(
            "{}: файл не в UTF-8 (бинарная кодировка). Модель читает только текст — используй bash",
            path.display()
        ))
    })?;
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let start = offset.saturating_sub(1);
    let end = (start + limit).min(total);
    let mut out = String::new();
    let mut truncated: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate().take(end).skip(start) {
        let lineno = i + 1;
        if line.chars().count() > MAX_READ_LINE_CHARS {
            truncated.push(lineno);
            let head: String = line.chars().take(MAX_READ_LINE_CHARS).collect();
            out.push_str(&format!("{:>5}→ {}\n", lineno, head));
        } else {
            out.push_str(&format!("{:>5}→ {}\n", lineno, line));
        }
    }
    if total > end {
        out.push_str(&format!(
            "… [показаны строки {}-{} из {}. Продолжай с offset={}]\n",
            start + 1,
            end,
            total,
            end + 1
        ));
    } else if start > 0 {
        out.push_str(&format!("… [показаны строки {}-{} из {}]\n", start + 1, end, total));
    }
    if !truncated.is_empty() {
        let nums: Vec<String> = truncated.iter().map(|n| n.to_string()).collect();
        out.push_str(&format!(
            "[Строки [{}] обрезаны до {} символов — не копируй их в edit_file целиком.]\n",
            nums.join(", "),
            MAX_READ_LINE_CHARS
        ));
    }
    if out.is_empty() {
        out = format!("[файл пуст или нет строк в диапазоне: {} строк(и)]", total);
    }
    Ok(out)
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
        "Прочитать текстовый файл с номерами строк (формат '     N→ текст'). path — путь к файлу (абсолютный или относительно корня проекта). offset — номер строки, с которой начать (1-based, по умолчанию 1); limit — сколько строк прочитать (по умолчанию 200). Строки длиннее 2000 символов обрезаются с пометкой внизу. Если в конце вывода есть подсказка 'Продолжай с offset=N' — читай дальше диапазонами. Чтение доступно по любому пути. Бинарные файлы и устройства (NUL и т.п.) не читаются — вернётся 'Запрещено'; для них используй bash."
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
        "Прочитать несколько файлов одним вызовом. paths — массив путей (абсолютных или относительно корня проекта). Каждый файл выводится в блоке с заголовком. Экономит вызовы: вместо серии read_file. Бинарные файлы и устройства не читаются ('Запрещено') — для них используй bash."
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
        authorize_write(&abs, ctx, "write_file")?;
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

/// Одна замена в мульти-правке `edits[]`.
struct EditSpec {
    old: String,
    new: String,
}

/// Разобрать строку блоков `edits` из аргументов тула.
fn parse_edits_array(args: &Value) -> Result<Vec<EditSpec>, ToolError> {
    let arr = args
        .get("edits")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ToolError::Usage("параметр 'edits' (массив {oldText,newText}) обязателен".to_string()))?;
    if arr.is_empty() {
        return Err(ToolError::Usage("edits не может быть пустым".to_string()));
    }
    let mut specs = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let old = item
            .get("oldText")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::Usage(format!("edits[{}].oldText (строка) обязателен", i)))?
            .to_string();
        let new = item
            .get("newText")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if old.is_empty() {
            return Err(ToolError::Usage(format!(
                "edits[{}].oldText не может быть пустым",
                i
            )));
        }
        specs.push(EditSpec { old, new });
    }
    Ok(specs)
}

/// Снять с тела строки префикс номера: `<пробелы><цифры><пробелы>(→|\\|)<пробелы>`.
/// Возвращает `None`, если префикс не подходит (строка НЕ выглядит как выдача read_file).
fn match_line_number_prefix(body: &str) -> Option<&str> {
    let mut rest = body;
    // Ведущие пробелы.
    if rest.chars().next().is_some_and(|c| c.is_whitespace()) {
        let idx = rest
            .char_indices()
            .find(|(_, c)| !c.is_whitespace())
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        rest = &rest[idx..];
    }
    // Цифры номера.
    let digits_end = rest
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    if digits_end == 0 {
        return None;
    }
    rest = &rest[digits_end..];
    // Пробелы после номера.
    if rest.chars().next().is_some_and(|c| c.is_whitespace()) {
        let idx = rest
            .char_indices()
            .find(|(_, c)| !c.is_whitespace())
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        rest = &rest[idx..];
    }
    // Маркер: '→' (minimax) или '|' (наш прежний формат).
    let marker = rest.chars().next()?;
    if marker != '→' && marker != '|' {
        return None;
    }
    rest = &rest[marker.len_utf8()..];
    // Хвостовые пробелы.
    if rest.chars().next().is_some_and(|c| c.is_whitespace()) {
        let idx = rest
            .char_indices()
            .find(|(_, c)| !c.is_whitespace())
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        rest = &rest[idx..];
    }
    Some(rest)
}

/// Снять префиксы номеров со ВСЕГО блока. Возвращает `None`, если хотя бы одна
/// non-empty строка не-префиксная — тогда блок НЕ трогаем (контент может
/// легитимно выглядеть как пронумерованный).
fn strip_line_number_prefixes_from_block(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let has_nl = line.ends_with('\n');
        let body = line.strip_suffix('\n').unwrap_or(line);
        if body.trim().is_empty() {
            out.push_str(line);
            continue;
        }
        let stripped = match_line_number_prefix(body)?;
        out.push_str(stripped);
        if has_nl {
            out.push('\n');
        }
    }
    Some(out)
}

/// Атомарная мульти-правка: применения идут по порядку по ОСТАТКУ исходного
/// контента. Любой oldText не найден → ошибка до записи (ничего не пишем).
fn apply_edits_sequential(original: &str, edits: &[EditSpec]) -> Result<(String, String), String> {
    let mut result = String::with_capacity(original.len());
    let mut rest = original;
    for (i, e) in edits.iter().enumerate() {
        match rest.find(&e.old) {
            Some(pos) => {
                result.push_str(&rest[..pos]);
                result.push_str(&e.new);
                rest = &rest[pos + e.old.len()..];
            }
            None => {
                let preview: String = e.old.chars().take(80).collect();
                return Err(format!("edits[{}] oldText не найден в файле: {:?}", i, preview));
            }
        }
    }
    result.push_str(rest);
    Ok((result, format!("Заменено {} фрагмента(ов)", edits.len())))
}

impl Tool for EditFile {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "Точечная замена фрагментов файла по ТОЧНОМУ совпадению. Два режима: 1) edits — массив {oldText,newText}: несколько правок за один вызов (атомарно, применяются к оригиналу, вырезы НЕ должны перекрываться); 2) legacy old_string/new_string (одна правка) + replace_all. oldText должен совпадать 1:1, включая отступы; если вставил его из вывода read_file — номера строк/стрелки снимаются автоматически при повторе. Ошибка возвращается как результат (тишина недопустима). Запись вне корня — запросит подтверждение."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Путь к файлу (абсолютный или относительно корня проекта)"},
                "edits": {"type": "array", "items": {"type": "object", "properties": {
                    "oldText": {"type": "string", "description": "Искомый фрагмент (точное совпадение)"},
                    "newText": {"type": "string", "description": "Замена"}
                }, "required": ["oldText", "newText"]}, "description": "Массив правок (рекомендуется)"},
                "old_string": {"type": "string", "description": "Искомый фрагмент (точное совпадение, legacy)"},
                "new_string": {"type": "string", "description": "Замена (legacy)"},
                "replace_all": {"type": "boolean", "description": "Заменить все вхождения (legacy, по умолчанию false — только первое)"}
            },
            "required": ["path"]
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let path = resolve_path(ctx.workspace_root, &arg_str(args, "path")?);
        let has_edits = args.get("edits").is_some();
        let has_legacy = args.get("old_string").is_some() || args.get("new_string").is_some();
        if has_edits && has_legacy {
            return Err(ToolError::Usage(
                "укажи либо edits[], либо old_string/new_string — не оба".to_string(),
            ));
        }
        if !has_edits && !has_legacy {
            return Err(ToolError::Usage(
                "нужен либо edits[] (массив {oldText,newText}), либо old_string + new_string".to_string(),
            ));
        }

        authorize_write(&path, ctx, "edit_file")?;
        let content = fs::read_to_string(&path)
            .map_err(|e| ToolError::NotFound(format!("{}: {}", path.display(), e)))?;

        if has_legacy {
            let old = arg_str(args, "old_string")?;
            if old.is_empty() {
                return Err(ToolError::Usage(
                    "old_string не может быть пустым — использовать write_file для создания файла".to_string(),
                ));
            }
            let new = arg_str(args, "new_string")?;
            let replace_all = args.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
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
            return Ok(format!(
                "✅ Замена выполнена в {} ({} → {}; {} вхождений)",
                path.display(),
                old.chars().count(),
                new.chars().count(),
                if replace_all { content.matches(&old).count() } else { 1 }
            ));
        }

        // Режим edits[]: первая попытка + один fail-only ретрай с префикс-стрипом.
        let edits = parse_edits_array(args)?;
        let apply = |specs: &[EditSpec]| -> Result<String, ToolError> {
            let (new_content, summary) =
                apply_edits_sequential(&content, specs).map_err(ToolError::NotFound)?;
            let mut f = fs::File::create(&path)
                .map_err(|e| ToolError::Io(format!("не удалось открыть {}: {}", path.display(), e)))?;
            f.write_all(new_content.as_bytes())
                .map_err(|e| ToolError::Io(format!("не удалось записать {}: {}", path.display(), e)))?;
            Ok(summary)
        };

        match apply(&edits) {
            Ok(summary) => Ok(format!("✅ {} в {}", summary, path.display())),
            // Ретрай — только если упало на не-нахождении (NotFound о тексте),
            // а не на реальной IO-ошибке.
            Err(ToolError::NotFound(first_err)) => {
                // Fail-only ретрай: только если КАЖДЫЙ блок нормализуется (префикс-формы).
                let mut normalized = Vec::with_capacity(edits.len());
                let mut changed = false;
                for e in &edits {
                    let old = strip_line_number_prefixes_from_block(&e.old)
                        .unwrap_or_else(|| e.old.clone());
                    let new = strip_line_number_prefixes_from_block(&e.new)
                        .unwrap_or_else(|| e.new.clone());
                    if old != e.old || new != e.new {
                        changed = true;
                    }
                    normalized.push(EditSpec { old, new });
                }
                if !changed {
                    return Err(ToolError::NotFound(first_err));
                }
                match apply(&normalized) {
                    Ok(summary) => Ok(format!(
                        "✅ {} в {} (префиксы номеров строк сняты)",
                        summary,
                        path.display()
                    )),
                    Err(_) => {
                        // Повторный провал → оригинальная ошибка (модель видит реальный текст).
                        Err(ToolError::NotFound(first_err))
                    }
                }
            }
            Err(e) => Err(e), // IO/другие ошибки — без бессмысленного ретрая.
        }
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
            write_root: root,
            write_outside: crate::infra::WriteOutside::Prompt,
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
        assert!(r.contains("    2→ l2"), "формат '     N→': {}", r);
        assert!(r.contains("    3→ l3"));
        assert!(!r.contains("    1→ l1"));
        assert!(r.contains("строки 2-3"), "произошла замена формата: {}", r);
        assert!(r.contains("Продолжай с offset=4"), "hint продолжения: {}", r);
    }

    #[test]
    fn read_file_clamps_long_lines_and_announces() {
        let d = tmpdir("read_long");
        let long_line = "x".repeat(5000);
        fs::write(d.join("long.txt"), format!("{}\n{}", long_line, "short")).unwrap();
        let ctx = ctx_for(&d);
        let r = ReadFile
            .execute(&serde_json::json!({"path": "long.txt"}), &ctx)
            .unwrap();
        assert!(r.contains("Строки [1] обрезаны"), "должна быть пометка обрезки: {}", r);
        assert!(!r.contains("x".repeat(2001).as_str()), "строка должна быть обрезана");
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
    fn edit_file_multi_edits_atomic() {
        let d = tmpdir("edit_multi");
        fs::write(d.join("m.rs"), "fn a(){}\nfn b(){}\nfn c(){}\n").unwrap();
        let ctx = ctx_for(&d);
        let r = EditFile
            .execute(
                &serde_json::json!({"path": "m.rs", "edits": [
                    {"oldText": "fn a(){}", "newText": "fn a2(){}"},
                    {"oldText": "fn c(){}", "newText": "fn c2(){}"}
                ]}),
                &ctx,
            )
            .unwrap();
        assert!(r.contains("Заменено 2"));
        let content = fs::read_to_string(d.join("m.rs")).unwrap();
        assert!(content.contains("fn a2(){}"));
        assert!(content.contains("fn b(){}"));
        assert!(content.contains("fn c2(){}"));
        assert!(!content.contains("fn a(){}"));
    }

    #[test]
    fn edit_file_multi_edits_do_not_partially_write_on_failure() {
        let d = tmpdir("edit_multi_fail");
        fs::write(d.join("n.rs"), "fn a(){}\nfn b(){}\n").unwrap();
        let ctx = ctx_for(&d);
        let err = EditFile
            .execute(
                &serde_json::json!({"path": "n.rs", "edits": [
                    {"oldText": "fn a(){}", "newText": "fn a2(){}"},
                    {"oldText": "ZZZ_NOPE", "newText": "x"}
                ]}),
                &ctx,
            )
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
        // Атомарность: первая правка НЕ применена, файл не изменён.
        let content = fs::read_to_string(d.join("n.rs")).unwrap();
        assert!(content.contains("fn a(){}"), "частичная запись при провале!");
        assert!(!content.contains("fn a2(){}"));
    }

    #[test]
    fn edit_file_strips_line_number_prefixes_on_retry() {
        let d = tmpdir("edit_retry");
        fs::write(d.join("r.rs"), "fn foo() {\n    return 1;\n}\n").unwrap();
        let ctx = ctx_for(&d);
        // oldText скопирован из вывода read_file (формат '     N→').
        let r = EditFile
            .execute(
                &serde_json::json!({"path": "r.rs", "edits": [
                    {"oldText": "    2→     return 1;", "newText": "    2→     return 2;"}
                ]}),
                &ctx,
            )
            .unwrap();
        assert!(r.contains("сняты"), "должен сработать ретрай со strip: {}", r);
        let content = fs::read_to_string(d.join("r.rs")).unwrap();
        assert!(content.contains("return 2;"), "замена должна примениться: {}", content);
        assert!(!content.contains("2→"), "новый текст с префиксом — ненормализован");
    }

    #[test]
    fn edit_file_legacy_prefix_style_also_stripped() {
        let d = tmpdir("edit_retry_legacy");
        fs::write(d.join("s.rs"), "alpha\nbeta\n").unwrap();
        let ctx = ctx_for(&d);
        let r = EditFile
            .execute(
                &serde_json::json!({"path": "s.rs", "edits": [
                    {"oldText": "   2 | beta", "newText": "   2 | gamma"}
                ]}),
                &ctx,
            )
            .unwrap();
        assert!(r.contains("сняты"));
        assert!(fs::read_to_string(d.join("s.rs")).unwrap().contains("gamma"));
    }

    #[test]
    fn edit_file_does_not_strip_mixed_content() {
        let d = tmpdir("edit_nostrip");
        fs::write(d.join("t.txt"), "hello world\n").unwrap();
        let ctx = ctx_for(&d);
        // Одна строка префиксная, другая нет → весь блок НЕ нормализуется.
        let err = EditFile
            .execute(
                &serde_json::json!({"path": "t.txt", "edits": [
                    {"oldText": "hello world\nextra non-prefixed line", "newText": "X"}
                ]}),
                &ctx,
            )
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
        assert_eq!(fs::read_to_string(d.join("t.txt")).unwrap(), "hello world\n");
    }

    #[test]
    fn edit_file_requires_edits_or_legacy() {
        let d = tmpdir("edit_require");
        fs::write(d.join("u.txt"), "x").unwrap();
        let ctx = ctx_for(&d);
        let err = EditFile
            .execute(&serde_json::json!({"path": "u.txt"}), &ctx)
            .unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)));
        let err2 = EditFile
            .execute(
                &serde_json::json!({"path": "u.txt", "old_string": "x", "new_string": "y", "edits": []}),
                &ctx,
            )
            .unwrap_err();
        assert!(matches!(err2, ToolError::Usage(_)));
    }

    #[test]
    fn edit_file_empty_oldText_rejected_in_edits() {
        let d = tmpdir("edit_empty");
        fs::write(d.join("v.txt"), "x").unwrap();
        let ctx = ctx_for(&d);
        let err = EditFile
            .execute(
                &serde_json::json!({"path": "v.txt", "edits": [{"oldText": "", "newText": "y"}]}),
                &ctx,
            )
            .unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)));
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

    #[test]
    fn read_guard_rejects_windows_device_names() {
        for name in ["NUL", "NUL.txt", "CON", "PRN", "AUX", "COM1", "LPT3", "CLOCK$"] {
            let err = guard_read_path(Path::new(name)).unwrap_err();
            assert!(matches!(err, ToolError::Forbidden(_)), "{} должен быть device", name);
        }
        // Обычный файл — проходит.
        let d = tmpdir("device");
        let f = d.join("data.txt");
        fs::write(&f, "hi").unwrap();
        assert!(guard_read_path(&f).is_ok());
    }

    #[test]
    fn read_guard_rejects_binary_extension() {
        let d = tmpdir("bin_ext");
        let f = d.join("image.png");
        fs::write(&f, "not really png").unwrap();
        let err = guard_read_path(&f).unwrap_err();
        assert!(matches!(err, ToolError::Forbidden(_)));
    }

    #[test]
    fn read_file_rejects_binary_sniff() {
        let d = tmpdir("bin_sniff");
        // NUL-байт без бинарного расширения.
        fs::write(d.join("payload"), b"abc\x00def").unwrap();
        let ctx = ctx_for(&d);
        let err = ReadFile
            .execute(&serde_json::json!({"path": "payload"}), &ctx)
            .unwrap_err();
        assert!(matches!(err, ToolError::Forbidden(_)));
        // Высокая доля управляющих символов.
        fs::write(d.join("ctrl.dat"), [0x01u8; 40]).unwrap();
        let err2 = ReadFile
            .execute(&serde_json::json!({"path": "ctrl.dat"}), &ctx)
            .unwrap_err();
        assert!(matches!(err2, ToolError::Forbidden(_)));
    }

    #[test]
    fn read_accepts_text_with_whitespace_controls() {
        let d = tmpdir("text_ok");
        fs::write(d.join("ok.ts"), "fn a() {\n\treturn 1;\n}\r\n").unwrap();
        let ctx = ctx_for(&d);
        let r = ReadFile
            .execute(&serde_json::json!({"path": "ok.ts"}), &ctx)
            .unwrap();
        assert!(r.contains("fn a() {"), "обычный текст читается: {}", r);
    }
}