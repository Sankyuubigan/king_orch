//! 🧰 Инструменты кодинга и файлов (Rust-ядро, SSOT).
//!
//! Единый источник правды: все тулы регистрируются здесь один раз (трейт [`Tool`]).
//! Read-only / read+write наборы — это ФИЛЬТРЫ над единым реестром (по флагу
//! `is_readonly`), а НЕ отдельные реализации. Дублирование кода исключено.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::infra::permissions::PermissionApprover;

pub mod fs;
pub mod lsp;
pub mod search;
pub mod shell;

/// Мета-имя в `agent.tools` — набор только для чтения (read-only).
pub const TOOLSET_READ: &str = "code_read";
/// Мета-имя в `agent.tools` — полный набор (чтение + запись).
pub const TOOLSET_WRITE: &str = "code_write";

/// Контекст исполнения тула: корень проекта + сессия + approver разрешений.
/// Живёт в `RunContext` цикла агента и пробрасывается в каждый вызов.
pub struct ToolCtx<'a> {
    pub workspace_root: &'a Path,
    pub session_id: &'a str,
    pub approver: &'a PermissionApprover,
    pub agent_id: &'a str,
    pub bins_dir: &'a Path,
}

/// Ошибка тула. Все ветки завершаются честным сообщением (правило 2.2: тишина = ложь).
#[derive(Debug)]
pub enum ToolError {
    /// Неверные/недостающие аргументы вызова.
    Usage(String),
    /// Файл/директория не найдены.
    NotFound(String),
    /// Запрещено (вне корня / отклонено пользователем / запрещённая операция).
    Forbidden(String),
    /// Ошибка ввода/вывода.
    Io(String),
    /// Превышен таймаут.
    Timeout(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::Usage(m) => write!(f, "Неверные аргументы: {}", m),
            ToolError::NotFound(m) => write!(f, "Не найдено: {}", m),
            ToolError::Forbidden(m) => write!(f, "Запрещено: {}", m),
            ToolError::Io(m) => write!(f, "Ошибка ввода/вывода: {}", m),
            ToolError::Timeout(m) => write!(f, "Таймаут: {}", m),
        }
    }
}

impl std::error::Error for ToolError {}

/// Контракт инструмента. Реализации — в `fs.rs`, `search.rs`, `shell.rs`.
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON-Schema параметров (формат `inputSchema` в промпте).
    fn parameters(&self) -> Value;
    /// `true` — тул ничего не меняет на диске (авто-доступ без плашки).
    fn is_readonly(&self) -> bool;
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError>;
}

/// SSOT-реестр: ВСЕ тулы кодинга. Единая точка, где живут реализации.
/// `code_read`/`code_write` — фильтры по `is_readonly()` над этим списком.
pub fn all_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(fs::ReadFile),
        Box::new(fs::ReadManyFiles),
        Box::new(fs::WriteFile),
        Box::new(fs::EditFile),
        Box::new(search::Grep),
        Box::new(search::Glob),
        Box::new(search::ListDirectory),
        Box::new(shell::Bash),
        Box::new(shell::RunTests),
        Box::new(lsp::LspGetDefinition),
        Box::new(lsp::LspGetReferences),
        Box::new(lsp::LspGetDiagnostics),
    ]
}

/// Схемы тулов для промпта агента: список `(meta, name, schema)`.
/// `meta` — метка происхождения («tools» для built-in кодовых тулов, чтобы
/// диспетчер отличал их от MCP-серверов).
pub fn tool_schemas(include_write: bool, explicit: &[String]) -> Vec<(String, String, Value)> {
    let mut result = Vec::new();
    for tool in all_tools() {
        let name = tool.name().to_string();
        let allowed = if explicit.contains(&name) {
            true
        } else if include_write {
            true
        } else {
            tool.is_readonly()
        };
        if allowed {
            result.push((
                "tools".to_string(),
                name,
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "inputSchema": tool.parameters(),
                }),
            ));
        }
    }
    result
}

/// Исполнить built-in кодовый тул по имени. Возвращает текст результата.
pub fn execute_tool(name: &str, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
    for tool in all_tools() {
        if tool.name() == name {
            return tool.execute(args, ctx);
        }
    }
    Err(ToolError::Usage(format!("Неизвестный тул '{}'", name)))
}

/// Является ли имя мета-набором или именем конкретного кодового тула.
pub fn is_code_tool_reference(name: &str) -> bool {
    name == TOOLSET_READ || name == TOOLSET_WRITE || all_tools().iter().any(|t| t.name() == name)
}

/// Capability-проверка для диспетчера (defense-in-depth): выдан ли тул агенту.
/// `all_tools` — набор схем, показанных агенту в промпте; код-тулы помечены
/// мета-имёнем "tools" (в отличие от MCP-тулов с мета-именем сервера).
pub fn is_tool_granted(all_tools: &[(String, String, Value)], tool_name: &str) -> bool {
    all_tools.iter().any(|(meta, name, _)| meta == "tools" && name == tool_name)
}

/// Разрешить путь (абсолютный или относительный корня) в абсолютный.
pub fn resolve_path(root: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    }
}

/// Проверка: путь находится ВНУТРИ корня (или равен ему). Для read-тулов.
pub fn is_within_root(root: &Path, path: &Path) -> bool {
    let root_abs = normalize_canonical(root);
    let path_abs = normalize_canonical(path);
    path_abs.starts_with(&root_abs)
}

/// canonicalize с очисткой Windows-префикса `\\?\` (иначе `starts_with` ломается).
fn normalize_canonical(p: &Path) -> PathBuf {
    let c = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let s = c.to_string_lossy().to_string();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    PathBuf::from(s)
}

/// Обрезать длинный текст до лимита с пометкой (для читаемых выводов тулов).
pub fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        let head: String = text.chars().take(limit).collect();
        format!("{}…\n[вывод обрезан до {} символов]", head, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolCtx<'static> {
        ToolCtx {
            workspace_root: std::path::Path::new("."),
            session_id: "test",
            approver: crate::infra::permissions::test_approver(),
            agent_id: "test_agent",
            bins_dir: std::path::Path::new("."),
        }
    }

    #[test]
    fn registry_has_no_duplicate_names() {
        let names: Vec<String> = all_tools().iter().map(|t| t.name().to_string()).collect();
        let unique: std::collections::HashSet<&String> = names.iter().collect();
        assert_eq!(names.len(), unique.len(), "дубли имён тулов в реестре");
    }

    #[test]
    fn write_filter_includes_all_readonly() {
        let read_only = all_tools().iter().filter(|t| t.is_readonly()).count();
        let read_schemas = tool_schemas(false, &[]);
        assert_eq!(read_schemas.len(), read_only, "code_read = все readonly-тулы");
        // LSP-тулы — read-only (авто-доступ без плашки).
        let names: Vec<String> = read_schemas.iter().map(|(_, n, _)| n.clone()).collect();
        assert!(names.contains(&"lsp_get_definition".to_string()));
        assert!(names.contains(&"lsp_get_references".to_string()));
        assert!(names.contains(&"lsp_get_diagnostics".to_string()));
    }

    #[test]
    fn write_set_is_superset_of_read() {
        let read: std::collections::HashSet<String> =
            tool_schemas(false, &[]).into_iter().map(|(_, n, _)| n).collect();
        let write: std::collections::HashSet<String> =
            tool_schemas(true, &[]).into_iter().map(|(_, n, _)| n).collect();
        assert!(read.is_subset(&write), "code_write ⊇ code_read");
    }

    #[test]
    fn explicit_names_are_added_to_readonly_base() {
        // explicit добавляет мутаторы к базовому read-only набору.
        let names: std::collections::HashSet<String> =
            tool_schemas(false, &["bash".to_string()]).into_iter().map(|(_, n, _)| n).collect();
        assert!(names.contains("bash"), "explicit должен добавить bash");
        assert!(names.contains("read_file"), "readonly-база остаётся");
        assert!(!names.contains("write_file"), "write_file не должен попадать без include_write");
    }

    #[test]
    fn resolve_path_handles_absolute_and_relative() {
        let root = Path::new("C:/proj");
        assert_eq!(resolve_path(root, "a/b.ts"), PathBuf::from("C:/proj/a/b.ts"));
        assert_eq!(resolve_path(root, "D:/other/c.ts"), PathBuf::from("D:/other/c.ts"));
    }

    #[test]
    fn truncate_marks_cut() {
        let t = truncate("abcdef", 4);
        assert!(t.contains("[вывод обрезан"));
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn unknown_tool_returns_usage_error() {
        let err = execute_tool("nope", &serde_json::json!({}), &ctx()).unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)));
    }

    // ── Capability-проверка диспетчера (defense-in-depth) ──
    // Диспетчер собирает all_tools так же, как run_agent_node: agent_code_tool_schemas
    // (мета-имя "tools") + MCP-тулы (мета-имя сервера) + builtin + todo.

    #[test]
    fn grant_check_code_read_agent_cannot_call_mutators() {
        // Имитация all_tools для code_read-агента: только read-only + явные.
        let schemas = tool_schemas(false, &[]);
        // Передаём как есть (диспетчер принимает готовый список).
        let mut all_tools: Vec<(String, String, serde_json::Value)> = schemas.clone();
        // Добавляем "чужой" MCP-тул с тем же именем, что код-мутатор — не должен
        // дать доступ (мета-имя сервера, а не "tools").
        all_tools.push(("fs_write".to_string(), "write_file".to_string(), serde_json::json!({})));

        assert!(is_tool_granted(&all_tools, "read_file"), "read_file выдан code_read");
        assert!(!is_tool_granted(&all_tools, "write_file"), "write_file НЕ выдан code_read (даже если MCP-тул с тем же именем)");
        assert!(!is_tool_granted(&all_tools, "bash"), "bash НЕ выдан code_read");
        assert!(!is_tool_granted(&all_tools, "edit_file"), "edit_file НЕ выдан code_read");
        // LSP-тулы read-only — выдан.
        assert!(is_tool_granted(&all_tools, "lsp_get_definition"));
    }

    #[test]
    fn grant_check_code_write_agent_can_call_mutators() {
        let all_tools = tool_schemas(true, &[]);
        assert!(is_tool_granted(&all_tools, "read_file"));
        assert!(is_tool_granted(&all_tools, "write_file"), "write_file выдан code_write");
        assert!(is_tool_granted(&all_tools, "bash"), "bash выдан code_write");
        assert!(is_tool_granted(&all_tools, "edit_file"));
    }

    #[test]
    fn grant_check_explicit_bash_grants_bash_not_write() {
        let all_tools = tool_schemas(false, &["bash".to_string()]);
        assert!(is_tool_granted(&all_tools, "bash"), "явный bash выдан");
        assert!(!is_tool_granted(&all_tools, "write_file"), "write_file НЕ выдан при явном bash");
    }

    #[test]
    fn grant_check_unknown_tool_never_granted() {
        let all_tools = tool_schemas(true, &[]);
        assert!(!is_tool_granted(&all_tools, "totally_unknown"));
    }
}