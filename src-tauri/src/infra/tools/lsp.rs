//! LSP-тулы: `lsp_get_definition`, `lsp_get_references`, `lsp_get_diagnostics`.
//! Все — read-only (авто-доступ без плашки). Требуют установленного
//! rust-analyzer (bins/ или PATH); иначе — честная ошибка «не установлен».

use std::path::Path;

use serde_json::Value;

use super::{Tool, ToolCtx, ToolError, is_within_root, resolve_path};
use crate::infra::lsp::manager;

fn resolve_file(ctx: &ToolCtx, path: &str) -> Result<std::path::PathBuf, ToolError> {
    let p = resolve_path(ctx.workspace_root, path);
    if !p.is_file() {
        return Err(ToolError::NotFound(format!("файл '{}' не найден", p.display())));
    }
    // LSP читает файлы — только в корне (безопасность: не даём серверу чужие пути).
    if !is_within_root(ctx.workspace_root, &p) {
        return Err(ToolError::Forbidden(format!("файл '{}' вне корня проекта", p.display())));
    }
    Ok(p)
}

fn position(args: &Value) -> Result<(u64, u64), ToolError> {
    let line = args
        .get("line")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ToolError::Usage("параметр 'line' (целое, 0-based) обязателен".to_string()))?;
    let character = args
        .get("character")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ToolError::Usage("параметр 'character' (целое, 0-based) обязателен".to_string()))?;
    Ok((line, character))
}

/// Схема общих параметров позиции (line/character).
fn position_params() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "Путь к файлу (абсолютный или относительно корня проекта)"},
            "line": {"type": "integer", "description": "Номер строки, 0-based"},
            "character": {"type": "integer", "description": "Позиция в строке, 0-based"}
        },
        "required": ["path", "line", "character"]
    })
}

/// `lsp_get_definition` — куда ведёт символ в позиции (путь:строка).
pub struct LspGetDefinition;

impl Tool for LspGetDefinition {
    fn name(&self) -> &str {
        "lsp_get_definition"
    }
    fn description(&self) -> &str {
        "Найти определение символа (функции, переменной, типа) по позиции в файле через LSP-сервер (rust-analyzer). path — файл; line/character — позиция (0-based). Возвращает список локаций (файл:строка:колонка). Требует установленного rust-analyzer. Read-only."
    }
    fn parameters(&self) -> Value {
        position_params()
    }
    fn is_readonly(&self) -> bool {
        true
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::Usage("параметр 'path' (строка) обязателен".to_string()))?;
        let file = resolve_file(ctx, path_str)?;
        let (line, character) = position(args)?;
        let log_cb = |s: String| { /* тишина — ложь: логируем в stdout при необходимости */ let _ = s; };
        match manager::get_definition(ctx.workspace_root, ctx.bins_dir, &file, line, character, log_cb) {
            Ok(res) => Ok(res),
            Err(e) => Err(ToolError::Forbidden(e)),
        }
    }
}

/// `lsp_get_references` — все использования символа.
pub struct LspGetReferences;

impl Tool for LspGetReferences {
    fn name(&self) -> &str {
        "lsp_get_references"
    }
    fn description(&self) -> &str {
        "Найти все ссылки/использования символа по позиции в файле через LSP-сервер (rust-analyzer). path — файл; line/character — позиция (0-based); include_declaration — включать ли объявление (по умолчанию true). Возвращает список локаций. Требует установленного rust-analyzer. Read-only."
    }
    fn parameters(&self) -> Value {
        let mut p = position_params();
        p["properties"]["include_declaration"] = serde_json::json!({"type": "boolean", "description": "Включать объявление (по умолчанию true)"});
        p
    }
    fn is_readonly(&self) -> bool {
        true
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::Usage("параметр 'path' (строка) обязателен".to_string()))?;
        let file = resolve_file(ctx, path_str)?;
        let (line, character) = position(args)?;
        let include_declaration = args.get("include_declaration").and_then(|v| v.as_bool()).unwrap_or(true);
        let log_cb = |s: String| { let _ = s; };
        match manager::get_references(ctx.workspace_root, ctx.bins_dir, &file, line, character, include_declaration, log_cb) {
            Ok(res) => Ok(res),
            Err(e) => Err(ToolError::Forbidden(e)),
        }
    }
}

/// `lsp_get_diagnostics` — ошибки/предупреждения компилятора в файле.
pub struct LspGetDiagnostics;

impl Tool for LspGetDiagnostics {
    fn name(&self) -> &str {
        "lsp_get_diagnostics"
    }
    fn description(&self) -> &str {
        "Получить диагностику (ошибки, предупреждения) для файла через LSP-сервер (rust-analyzer). path — файл. Возвращает список [SEVERITY] строка:колонка сообщение. Требует установленного rust-analyzer. Read-only."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Путь к файлу (абсолютный или относительно корня проекта)"}
            },
            "required": ["path"]
        })
    }
    fn is_readonly(&self) -> bool {
        true
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::Usage("параметр 'path' (строка) обязателен".to_string()))?;
        let file = resolve_file(ctx, path_str)?;
        let log_cb = |s: String| { let _ = s; };
        match manager::get_diagnostics(ctx.workspace_root, ctx.bins_dir, &file, log_cb) {
            Ok(res) => Ok(res),
            Err(e) => Err(ToolError::Forbidden(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ko_lsp_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
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
    fn definition_missing_required_args_is_usage() {
        let d = tmpdir("def");
        let ctx = ctx_for(&d);
        let err = LspGetDefinition.execute(&serde_json::json!({}), &ctx).unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn definition_missing_file_is_not_found() {
        let d = tmpdir("def2");
        let ctx = ctx_for(&d);
        let err = LspGetDefinition
            .execute(&serde_json::json!({"path": "nope.rs", "line": 0, "character": 0}), &ctx)
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn definition_outside_root_is_forbidden() {
        let d = tmpdir("def3");
        let ctx = ctx_for(&d);
        let outside = d.parent().unwrap().join("outside_lsp.rs");
        let mut f = std::fs::File::create(&outside).unwrap();
        f.write_all(b"fn main(){}").unwrap();
        let err = LspGetDefinition
            .execute(&serde_json::json!({"path": outside.to_string_lossy(), "line": 0, "character": 0}), &ctx)
            .unwrap_err();
        assert!(matches!(err, ToolError::Forbidden(_)), "вне корня — запрещено: {}", err);
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn server_missing_gives_honest_forbidden() {
        let d = tmpdir("def4");
        let ctx = ctx_for(&d);
        let file = d.join("a.rs");
        std::fs::write(&file, "fn main(){}").unwrap();
        let res = LspGetDefinition
            .execute(&serde_json::json!({"path": "a.rs", "line": 0, "character": 0}), &ctx);
        // Без rust-analyzer в bins — честная ошибка (либо «не установлен», либо
        // сервер из PATH упал и сообщил об этом), а НЕ молчание.
        match res {
            Err(ToolError::Forbidden(msg)) => assert!(!msg.trim().is_empty(), "честная ошибка не должна быть пустой: {}", msg),
            Err(e) => panic!("ожидали Forbidden (честная ошибка), получили: {}", e),
            Ok(s) => {
                // Если сервер реально работает — результат валиден (не пустой).
                assert!(!s.is_empty());
            }
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
