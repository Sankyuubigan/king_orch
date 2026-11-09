//! 📺 Capability YouTube: нативные тула `prepare_video`/`get_chunk` поверх
//! SSOT-механики субтитров плагина `tauri-plugin-ytdlp` (`engine::subtitles`).
//!
//! Нативная замена Deno MCP-сервера `youtube_mcp.ts`: имена тулов, описания,
//! inputSchema и формат вывода дословно прежние, меняется только исполнение
//! (в процессе хоста, без Deno-рантайма). Кеш чанков — процессное состояние
//! модуля субтитров плагина (паритет прежнего in-memory кеша Deno-процесса:
//! живёт между вызовами тулов, умирает с процессом приложения).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::infra::tools::{Tool, ToolCtx, ToolError};
use tauri_plugin_ytdlp::engine::subtitles;

pub const YOUTUBE_TOOL_NAMES: [&str; 2] = ["prepare_video", "get_chunk"];

pub fn is_youtube_tool(name: &str) -> bool {
    YOUTUBE_TOOL_NAMES.contains(&name)
}

/// Каталог движка yt-dlp: override из конфига плагина (`ytdlp_config.json` в
/// app data dir, то же поле, что использует UI «Движки») → иначе рядом с exe
/// (`<exe>/ytdlp`, правило cwd rules.md:53).
fn engine_dir() -> PathBuf {
    let cfg_path = crate::infra::config::app_data_dir_early().join("ytdlp_config.json");
    if let Ok(text) = std::fs::read_to_string(&cfg_path) {
        if let Ok(cfg) = serde_json::from_str::<Value>(&text) {
            if let Some(dir) = cfg.get("ytdlp_dir").and_then(Value::as_str) {
                if !dir.is_empty() {
                    return PathBuf::from(dir);
                }
            }
        }
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    exe_dir.join("ytdlp")
}

/// Cookies из конфига плагина (поле `cookies_file`, задаётся через UI).
/// Прежний TS-скан каталогов `cookies/` не переносится: скан шёл по cwd
/// Deno-процесса (в установленном продукте каталога там не было), явное
/// поле конфига чище и управляемо.
fn cookies_file() -> Option<PathBuf> {
    let cfg_path = crate::infra::config::app_data_dir_early().join("ytdlp_config.json");
    let text = std::fs::read_to_string(&cfg_path).ok()?;
    let cfg = serde_json::from_str::<Value>(&text).ok()?;
    let file = cfg.get("cookies_file").and_then(Value::as_str)?;
    if file.is_empty() {
        return None;
    }
    Some(PathBuf::from(file))
}

/// Маппинг ошибок ядра в честный контракт тула — 1:1 по классам
/// (`SubtitlesError` создан именно под этот маппинг).
fn map_subtitles_error(err: subtitles::SubtitlesError) -> ToolError {
    match err {
        subtitles::SubtitlesError::Usage(m) => ToolError::Usage(m),
        subtitles::SubtitlesError::NotFound(m) => ToolError::NotFound(m),
        subtitles::SubtitlesError::Io(m) => ToolError::Io(m),
    }
}

struct PrepareVideoTool;

impl Tool for PrepareVideoTool {
    fn name(&self) -> &str {
        "prepare_video"
    }

    fn description(&self) -> &str {
        "Скачивает видео, очищает от рекламы и разбивает на части (chunks). Вызови это ПЕРВЫМ."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Ссылка на YouTube видео" },
                "chunk_size": { "type": "number", "description": "Размер части в символах (рекомендуется 12000)" }
            },
            "required": ["url"]
        })
    }

    fn is_readonly(&self) -> bool {
        // Пользовательские данные не трогает (yt-dlp пишет во временный
        // каталог) → авто-доступ без плашки.
        true
    }

    fn execute(&self, args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        // Паритет TS-хендлера: String(args.url || ''), Number(chunk_size) || 12000.
        let url = args.get("url").and_then(Value::as_str).unwrap_or("");
        if url.is_empty() {
            return Err(ToolError::Usage("Ссылка на видео не указана.".into()));
        }
        let chunk_size = args
            .get("chunk_size")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as usize; // 0/absent → DEFAULT_CHUNK_SIZE в ядре
        let prepared = subtitles::prepare_transcript(
            &engine_dir(),
            url,
            chunk_size,
            cookies_file().as_deref(),
        )
        .map_err(map_subtitles_error)?;
        if prepared.cached {
            Ok(format!(
                "Видео уже загружено. Всего частей: {}. Теперь вызывай get_chunk.",
                prepared.chunks_total
            ))
        } else {
            Ok(format!(
                "Успешно. Видео разбито на {} частей. Вызывай get_chunk.",
                prepared.chunks_total
            ))
        }
    }
}

struct GetChunkTool;

impl Tool for GetChunkTool {
    fn name(&self) -> &str {
        "get_chunk"
    }

    fn description(&self) -> &str {
        "Получить конкретную часть текста видео."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "chunk_index": { "type": "number", "description": "Номер части (начиная с 1)" }
            },
            "required": ["url", "chunk_index"]
        })
    }

    fn is_readonly(&self) -> bool {
        true
    }

    fn execute(&self, args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        let url = args.get("url").and_then(Value::as_str).unwrap_or("");
        let index = args
            .get("chunk_index")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as usize; // NaN/absent → 0 → «Неверный индекс» (паритет TS)
        let chunk =
            subtitles::get_transcript_chunk(url, index).map_err(map_subtitles_error)?;
        Ok(format!(
            "--- ЧАСТЬ {} ИЗ {} ---\n{}",
            chunk.index, chunk.chunks_total, chunk.text
        ))
    }
}

/// Схемы youtube-тулов (мета-имя "youtube") для агентов с
/// `tools: {prepare_video: true, get_chunk: true}`.
pub fn youtube_tool_schemas(agent_tools: &[String]) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    let tools: [&dyn Tool; 2] = [&PrepareVideoTool, &GetChunkTool];
    for tool in tools {
        if agent_tools.iter().any(|t| t == tool.name()) {
            out.push((
                "youtube".to_string(),
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

/// Исполнение youtube-тула по имени. `ctx` не используется: каталог движка
/// и cookies резолвятся из конфига плагина / exe-каталога (см. выше).
pub fn execute_youtube_tool(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
) -> Option<Result<String, ToolError>> {
    match name {
        "prepare_video" => Some(PrepareVideoTool.execute(args, ctx)),
        "get_chunk" => Some(GetChunkTool.execute(args, ctx)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_match_ts_server_contract() {
        let schemas = youtube_tool_schemas(&["prepare_video".to_string(), "get_chunk".to_string()]);
        assert_eq!(schemas.len(), 2);
        let (meta1, name1, s1) = &schemas[0];
        assert_eq!(meta1, "youtube");
        assert_eq!(name1, "prepare_video");
        assert_eq!(s1["inputSchema"]["required"], serde_json::json!(["url"]));
        assert!(s1["inputSchema"]["properties"]["url"].is_object());
        assert!(s1["inputSchema"]["properties"]["chunk_size"].is_object());
        let (meta2, name2, s2) = &schemas[1];
        assert_eq!(meta2, "youtube");
        assert_eq!(name2, "get_chunk");
        assert_eq!(
            s2["inputSchema"]["required"],
            serde_json::json!(["url", "chunk_index"])
        );
    }

    #[test]
    fn no_grant_no_schema() {
        assert!(youtube_tool_schemas(&[]).is_empty());
        assert!(youtube_tool_schemas(&["WebSearch".to_string()]).is_empty());
    }

    #[test]
    fn is_youtube_tool_only_two_names() {
        assert!(is_youtube_tool("prepare_video"));
        assert!(is_youtube_tool("get_chunk"));
        assert!(!is_youtube_tool("prepare_transcript"));
        assert!(!is_youtube_tool("youtube_mcp"));
    }

    #[test]
    fn missing_url_is_usage() {
        let ctx = test_ctx();
        let err = execute_youtube_tool("prepare_video", &serde_json::json!({}), &ctx)
            .expect("prepare_video резолвится")
            .unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)));
        assert!(err.to_string().contains("Ссылка на видео не указана"));
    }

    #[test]
    fn get_chunk_without_prepare_is_usage() {
        let ctx = test_ctx();
        let err = execute_youtube_tool(
            "get_chunk",
            &serde_json::json!({ "url": "https://youtu.be/dQw4w9WgXcQ", "chunk_index": 1 }),
            &ctx,
        )
        .expect("get_chunk резолвится")
        .unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)));
        assert!(err.to_string().contains("Сначала вызови prepare_video"));
    }

    #[test]
    fn unknown_name_is_none() {
        let ctx = test_ctx();
        assert!(execute_youtube_tool("excel_apply", &serde_json::json!({}), &ctx).is_none());
    }

    fn test_ctx() -> ToolCtx<'static> {
        // lease-паттерн из excel/mod.rs: static-строки живут дольше ctx.
        ToolCtx {
            workspace_root: Path::new("."),
            write_root: Path::new("."),
            write_outside: crate::infra::WriteOutside::Prompt,
            session_id: "test_session",
            approver: crate::infra::test_approver(),
            agent_id: "youtube_summarizer",
            bins_dir: Path::new("."),
            image_artifacts: None,
        }
    }
}
