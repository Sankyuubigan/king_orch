//! 🎬 Capability FFmpeg: нативные тула `ffmpeg_probe`/`ffmpeg_run` поверх
//! SSOT-механики плагина `tauri-plugin-ffmpeg` (автоустановка движка +
//! запуск subprocess БЕЗ shell).
//!
//! Архитектурные гейты (§00.2 — структурные, не промптовые):
//! - вход — ТОЛЬКО вложения юзера (allow-read) или артефакт этой же сессии
//!   (сценарий «почини свой же клип»); произвольные пути диска недоступны;
//! - `-i` и `-y` в аргументах модели запрещены: источник строит хост
//!   (ровно один вход — гейтнутый путь), перезапись невозможна (выход
//!   существует → честная ошибка ДО спавна, без `-y` байпаса);
//! - выход всегда `<dir>/<stem><suffix>.<ext>` рядом с исходником —
//!   модель не выбирает путь результата.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::infra::tools::{Tool, ToolCtx, ToolError};
use tauri_plugin_ffmpeg::engine;

pub const FFMPEG_TOOL_NAMES: [&str; 2] = ["ffmpeg_probe", "ffmpeg_run"];

pub fn is_ffmpeg_tool(name: &str) -> bool {
    FFMPEG_TOOL_NAMES.contains(&name)
}

/// Каталог движка: `data_dir` из раннего конфига (нормализованный, хвост
/// `KingOrchData`) → автоматический фолбэк best-drive. Движок живёт в
/// `<хранилище>/ffmpeg/` — как у остальных плагинов (плашка «Хранилище
/// данных программы»).
fn ffmpeg_root() -> PathBuf {
    let configured = crate::infra::load_config_early()
        .data_dir
        .filter(|s| !s.trim().is_empty())
        .map(|raw| crate::infra::data_dir::normalize(&raw));
    configured.unwrap_or_else(crate::infra::data_dir::best_drive_data_dir)
}

/// Каталог установки движка (`<хранилище>/ffmpeg`) — корень для ensure.
fn engine_dir() -> PathBuf {
    ffmpeg_root().join(engine::FFMPEG_DIR_NAME)
}

// ─────────────────────────── Гейт источника ───────────────────────────

/// Нормализация пути для сравнения (Windows: сепаратор и регистр не значимы).
fn normalized_path(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

/// Гейт источника: путь обязан быть абсолютным, без `..`, и входить в
/// allow-read список (вложения юзера текущего хода и истории) ИЛИ быть
/// артефактом этой же сессии из media-реестра (повторный монтаж результата
/// предыдущего тул-вызова в том же ходе).
fn ensure_allowed_source(
    path: &str,
    ctx: &ToolCtx,
    allowed: &[String],
) -> Result<PathBuf, ToolError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ToolError::Usage(
            "path пуст. Возьми абсолютный путь из каталога [ВИДЕО ЗАПРОСА] \
             или из результата предыдущего ffmpeg_run."
                .to_string(),
        ));
    }
    let candidate = Path::new(trimmed);
    if candidate
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ToolError::Usage(
            "path с '..' запрещён: используй точный абсолютный путь".to_string(),
        ));
    }
    if !candidate.is_absolute() {
        return Err(ToolError::Usage(format!(
            "path должен быть абсолютным (получено: '{trimmed}')"
        )));
    }
    let normalized = normalized_path(trimmed);
    let allowed_hit = allowed.iter().any(|a| normalized_path(a) == normalized);
    let session_artifact = ctx
        .image_artifacts
        .is_some_and(|reg| reg.contains_path(trimmed));
    if !allowed_hit && !session_artifact {
        return Err(ToolError::Forbidden(format!(
            "файл '{trimmed}' не входит в allow-read: работай только с видео, \
             прикреплёнными пользователем в чат (пути в [ВИДЕО ЗАПРОСА]), и \
             клипами, созданными этой сессией (результат предыдущего ffmpeg_run)."
        )));
    }
    if !candidate.is_file() {
        return Err(ToolError::NotFound(format!("файл не найден: {trimmed}")));
    }
    Ok(candidate.to_path_buf())
}

// ─────────────────────── Чистые валидаторы (гейты) ───────────────────────

/// Суффикс имени результата: дефолт `_edited`; допустимы [A-Za-z0-9_-]
/// (никаких точек/слэшей — суффикс приклеивается к stem, не к пути).
fn sanitize_suffix(raw: Option<&str>) -> Result<String, ToolError> {
    let suffix = raw.unwrap_or("").trim();
    if suffix.is_empty() {
        return Ok("_edited".to_string());
    }
    if !suffix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ToolError::Usage(format!(
            "suffix '{suffix}' невалиден: разрешены только латиница, цифры, '_' и '-' \
             (суффикс — часть имени файла, а не путь)"
        )));
    }
    Ok(suffix.to_string())
}

/// Расширение результата: дефолт — расширение исходника; допустимы [a-z0-9].
fn sanitize_ext(raw: Option<&str>, source: &Path) -> Result<String, ToolError> {
    let ext = match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(e) => e.to_ascii_lowercase(),
        None => source
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp4")
            .to_ascii_lowercase(),
    };
    if !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ToolError::Usage(format!(
            "output_ext '{ext}' невалиден: разрешены только буквы и цифры без точки \
             (расширение имени файла, а не путь)"
        )));
    }
    Ok(ext)
}

/// Запрещённые токены в аргументах модели: `-i` (второй вход — источник
/// строит хост, ровно один) и `-y` (байпас гейта «выход уже существует»).
fn forbidden_token(args: &[String]) -> Option<&'static str> {
    for token in args {
        let lower = token.trim().to_ascii_lowercase();
        if lower == "-i" {
            return Some("-i");
        }
        if lower == "-y" {
            return Some("-y");
        }
    }
    None
}

/// Разбор списка аргументов из JSON: absent/пусто → Ok(vec![]); массив строк
/// → Ok; что-то другое (строка с пробелами, число) → честный Usage: спавн
/// идёт args-массивом, «строка целиком» в один токен — не команда.
fn parse_args_list(args: &Value, key: &str) -> Result<Vec<String>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        return Err(ToolError::Usage(format!(
                            "{key} должен быть массивом строк (каждый аргумент ffmpeg — \
                             отдельный элемент), получен элемент: {item}"
                        )))
                    }
                }
            }
            Ok(out)
        }
        Some(other) => Err(ToolError::Usage(format!(
            "{key} должен быть массивом строк (например [\"-ss\", \"2\", \"-to\", \"5\"]), \
             получено: {other}"
        ))),
    }
}

/// Выходной путь: рядом с исходником, `<stem><suffix>.<ext>` — суффикс
/// гарантирует имя != исходнику, перезапись структурно исключена.
fn build_output_path(source: &Path, suffix: &str, ext: &str) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "video".to_string());
    source.with_file_name(format!("{stem}{suffix}.{ext}"))
}

// ─────────────────────────── Исполнение тулов ───────────────────────────

/// Единая точка «есть ли движок»: ensure плагина (Mutex + re-check + докачка
/// с логом в «Логи»). Отказ без движка невозможен: каждый спавн после ensure.
fn ensure_engine() -> Result<PathBuf, ToolError> {
    let dir = engine_dir();
    engine::ensure_ffmpeg(&dir, &|line| log::info!("[FFMPEG] {line}"))
        .map_err(|e| ToolError::NotFound(format!("FFmpeg не удалось установить: {e}")))?;
    Ok(dir)
}

fn run_probe(args: &Value, ctx: &ToolCtx, allowed: &[String]) -> Result<String, ToolError> {
    let raw_path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Usage("path обязателен (абсолютный путь видео)".to_string()))?;
    let source = ensure_allowed_source(raw_path, ctx, allowed)?;
    let dir = ensure_engine()?;
    let meta = engine::probe_json(&engine::ffprobe_exe(&dir), &source)
        .map_err(ToolError::Io)?;
    serde_json::to_string(&meta)
        .map_err(|e| ToolError::Io(format!("Сериализация метаданных: {e}")))
}

fn run_edit(args: &Value, ctx: &ToolCtx, allowed: &[String]) -> Result<String, ToolError> {
    let raw_path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Usage("path обязателен (абсолютный путь видео)".to_string()))?;
    let source = ensure_allowed_source(raw_path, ctx, allowed)?;

    let input_args = parse_args_list(args, "input_args")?;
    let output_args = parse_args_list(args, "output_args")?;
    if let Some(token) = forbidden_token(&input_args).or_else(|| forbidden_token(&output_args)) {
        return Err(ToolError::Usage(format!(
            "аргумент '{token}' запрещён: источник передаётся в path (один вход строит \
             хост), результат сохраняется автоматически рядом с исходником"
        )));
    }
    let suffix = sanitize_suffix(args.get("suffix").and_then(Value::as_str))?;
    let ext = sanitize_ext(args.get("output_ext").and_then(Value::as_str), &source)?;
    let output = build_output_path(&source, &suffix, &ext);
    if output.exists() {
        return Err(ToolError::Usage(format!(
            "файл результата '{}' уже существует — перезапись запрещена; выбери другой \
             suffix (например '{}_2')",
            output.display(),
            suffix
        )));
    }

    let dir = ensure_engine()?;
    let mut argv: Vec<String> = vec!["-hide_banner".to_string()];
    argv.extend(input_args);
    argv.push("-i".to_string());
    argv.push(source.to_string_lossy().to_string());
    argv.extend(output_args);
    argv.push(output.to_string_lossy().to_string());

    log::info!("[FFMPEG] запуск: {} {}", engine::ffmpeg_exe(&dir).display(), argv.join(" "));
    let result = engine::run_tool(&engine::ffmpeg_exe(&dir), &argv, engine::DEFAULT_RUN_TIMEOUT)
        .map_err(ToolError::Io)?;
    if !result.success {
        // Честная ошибка аргументов: хвост stderr содержит причину — модель
        // вправе исправить команду (Usage проходит в dispatch без маскировки).
        return Err(ToolError::Usage(format!(
            "ffmpeg завершился с ошибкой (code {:?}): {}",
            result.exit_code, result.stderr_tail
        )));
    }
    let meta = std::fs::metadata(&output)
        .map_err(|e| ToolError::Usage(format!("ffmpeg отчитался об успехе, но файл '{}' не появился: {e}", output.display())))?;
    if meta.len() == 0 {
        return Err(ToolError::Usage(format!(
            "файл '{}' создан пустым (0 байт) — проверь output_args",
            output.display()
        )));
    }

    let registry = ctx.image_artifacts.ok_or_else(|| {
        ToolError::Usage("внутренний media-реестр артефактов недоступен".to_string())
    })?;
    let id = registry
        .register(output.to_string_lossy().to_string(), "ffmpeg_run")
        .map_err(ToolError::Usage)?;
    log::info!(
        "[FFMPEG] готово: {} ({:.1} МБ)",
        output.display(),
        meta.len() as f64 / (1024.0 * 1024.0)
    );
    Ok(format!(
        "[VIDEO_SAVED id={id}] Готово: {} ({:.1} МБ)",
        output
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        meta.len() as f64 / (1024.0 * 1024.0)
    ))
}

// ─────────────────────── Схемы (заглушки Tool) ───────────────────────

struct FfmpegProbeTool;
impl Tool for FfmpegProbeTool {
    fn name(&self) -> &str { "ffmpeg_probe" }
    fn description(&self) -> &str {
        "Снимает метаданные видео через ffprobe: длительность, размер, битрейт, потоки \
         (кодек, разрешение, fps, аудио-каналы). Вызови ПЕРЕД монтажом, чтобы точно \
         планировать команду (длительность, наличие аудио-дорожки)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Абсолютный путь видео из каталога [ВИДЕО ЗАПРОСА]."}
            },
            "required": ["path"]
        })
    }
    fn is_readonly(&self) -> bool { true }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage("ffmpeg_probe должен исполняться через диспетчер".to_string()))
    }
}

struct FfmpegRunTool;
impl Tool for FfmpegRunTool {
    fn name(&self) -> &str { "ffmpeg_run" }
    fn description(&self) -> &str {
        "Монтаж видео через ffmpeg. Источник один — path (вход строится хостом, '-i' \
         запрещён). Команда собирается так: ffmpeg [input_args] -i <path> [output_args] \
         <результат рядом с исходником, имя = исходник + suffix>. Примеры: вырезать \
         фрагмент быстро — input_args=[\"-ss\",\"10\",\"-to\",\"20\"], output_args=[\"-c\",\"copy\"] \
         (рез по ключевым кадрам); точная обрезка — input_args=[\"-ss\",\"10\"], \
         output_args=[\"-t\",\"10\",\"-c:v\",\"libx264\",\"-crf\",\"20\",\"-c:a\",\"aac\"] \
         (покадрово, 10 секунд с 10-й); снять звук — output_args=[\"-an\",\"-c:v\",\"copy\"]. \
         Если файл результата уже существует — выбери другой suffix."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Абсолютный путь исходного видео (из [ВИДЕО ЗАПРОСА] или результата предыдущего ffmpeg_run)."},
                "input_args": {"type": "array", "items": {"type": "string"}, "description": "Аргументы ДО источника (входные опции: -ss, -to, -t…). Каждый аргумент — отдельный элемент массива."},
                "output_args": {"type": "array", "items": {"type": "string"}, "description": "Аргументы ПОСЛЕ источника (кодеки, фильтры -vf/-af, -an/-vn, -t…). '-i' и '-y' запрещены."},
                "suffix": {"type": "string", "description": "Суффикс имени результата (латиница/цифры/_/-, дефолт '_edited'). Пример: suffix='_cut' → clip_cut.mp4."},
                "output_ext": {"type": "string", "description": "Расширение результата без точки (буквы/цифры; дефолт — расширение исходника)."}
            },
            "required": ["path"]
        })
    }
    fn is_readonly(&self) -> bool { false }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage("ffmpeg_run должен исполняться через диспетчер".to_string()))
    }
}

/// Схемы ffmpeg-тулов для промпта агента (мета-имя "ffmpeg").
pub fn ffmpeg_tool_schemas(agent_tools: &[String]) -> Vec<(String, String, Value)> {
    let tools: [&dyn Tool; 2] = [&FfmpegProbeTool, &FfmpegRunTool];
    let mut out = Vec::new();
    for tool in tools {
        if agent_tools.iter().any(|t| t == tool.name()) {
            out.push((
                "ffmpeg".to_string(),
                tool.name().to_string(),
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "inputSchema": tool.parameters(),
                }),
            ));
        }
    }
    out
}

/// Исполнить ffmpeg-тул по имени (диспетчер). `allowed_source_paths` —
/// allow-read список вложений юзера (request_media.current_attachment_paths()).
pub fn execute_ffmpeg_tool(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
    allowed_source_paths: &[String],
) -> Option<Result<String, ToolError>> {
    let outcome = match name {
        "ffmpeg_probe" => run_probe(args, ctx, allowed_source_paths),
        "ffmpeg_run" => run_edit(args, ctx, allowed_source_paths),
        _ => return None,
    };
    Some(outcome)
}

#[cfg(test)]
#[path = "ffmpeg_tests.rs"]
mod ffmpeg_tests;
