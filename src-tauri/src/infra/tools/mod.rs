//! 🧰 Инструменты кодинга и файлов (SSOT в крейте `ko-code-tools`).
//!
//! Реализации ВСЕХ кодовых тулов (файлы/поиск/shell) живут в крейте
//! `ko-code-tools` (репозиторий my-tauri-plugins) — тем же ядром пользуется
//! CLI `agent-tools`. Хост держит только:
//!  - публичные [`Tool`] / [`ToolError`] / [`ToolCtx`] (контракт King Orch);
//!  - политику записи хоста (`PermissionApprover` / `WriteOutside` / `authorize_write`);
//!  - [`CoreAuthorizer`] — мост хостовой политики к ядровому хуку `WriteAuthorizer`;
//!  - тонкие адаптеры (макрос [`adapter_tool!`]): `Tool`-обёртки над `CodeTool` ядра.
//! Перенос 1:1; паритет имён/описаний/схем проверен гейтом каталогов.

use std::path::Path;

use serde_json::Value;

use crate::infra::permissions::PermissionApprover;

pub mod lsp;
pub mod media;

/// Хелперы ядра: ними пользуются lsp.rs/media.rs через `super::` (имена прежние).
pub use ko_code_tools::{is_within_root, resolve_path};

/// Ошибка тула — единый тип с ядром (варианты и Display-тексты 1:1, конверсий нет).
pub use ko_code_tools::ToolError;
/// Трейт ядра нужен в scope: макрос вызывает его методы на конкретных типах.
use ko_code_tools::CodeTool;

/// Мета-имя в `agent.tools` — набор только для чтения (read-only).
pub const TOOLSET_READ: &str = "code_read";
/// Мета-имя в `agent.tools` — полный набор (чтение + запись).
pub const TOOLSET_WRITE: &str = "code_write";

/// Политика записи ВНЕ `write_root` (авто-зоны записи пайплайна).
///
/// Разделяет два режима пайплайнов:
/// - `Prompt` — «Кодер»: запись в рабочую директорию чата авто, вне — плашка юзеру.
/// - `Deny` — «Аналитик кода»: запись только в `.agents_workspace`, вне — жёсткий
///   запрет без плашки (инвариант: аналитический пайплайн не меняет код проекта).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOutside {
    /// Спросить пользователя (плашка Запретить / Разрешить 1 раз / Разрешить в чате).
    Prompt,
    /// Жёстко запретить без плашки.
    Deny,
}

impl WriteOutside {
    /// Разбор значения из `WorkflowConfig.write_outside` (дефолт — `Prompt`).
    pub fn parse(s: &str) -> WriteOutside {
        match s.trim().to_lowercase().as_str() {
            "deny" => WriteOutside::Deny,
            _ => WriteOutside::Prompt,
        }
    }
}

/// Контекст исполнения тула: корень чтения + авто-зона записи + политика вне неё.
/// Живёт в `RunContext` цикла агента и пробрасывается в каждый вызов.
pub struct ToolCtx<'a> {
    /// База для резолва относительных путей и чтения (рабочая директория чата).
    pub workspace_root: &'a Path,
    /// Авто-зона записи пайплайна (обычно совпадает с `workspace_root`, для
    /// аналитического пайплайна — `<workspace_root>/.agents_workspace`).
    pub write_root: &'a Path,
    /// Поведение при записи вне `write_root`.
    pub write_outside: WriteOutside,
    pub session_id: &'a str,
    pub approver: &'a PermissionApprover,
    pub agent_id: &'a str,
    pub bins_dir: &'a Path,
    pub image_artifacts: Option<&'a media::ImageArtifactRegistry>,
}

/// Контракт инструмента. Реализации: адаптеры ядра в этом модуле + lsp.rs/media.rs.
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON-Schema параметров (формат `inputSchema` в промпте).
    fn parameters(&self) -> Value;
    /// `true` — тул ничего не меняет на диске (авто-доступ без плашки).
    fn is_readonly(&self) -> bool;
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError>;
}

/// Централизованная проверка права записи по абсолютному пути.
///
/// Внутри `ctx.write_root` — авто (логируется диспетчером); вне — поведение
/// задаёт `ctx.write_outside`: `Deny` (жёсткий запрет, аналитический пайплайн)
/// или `Prompt` (плашка юзеру, пайплайн-исполнитель).
pub fn authorize_write(abs: &Path, ctx: &ToolCtx, tool: &str) -> Result<(), ToolError> {
    if is_within_root(ctx.write_root, abs) {
        return Ok(());
    }
    // Новый файл: сам путь может ещё не существовать — проверяем родителя.
    if let Some(parent) = abs.parent() {
        if is_within_root(ctx.write_root, parent) {
            return Ok(());
        }
    }
    match ctx.write_outside {
        WriteOutside::Deny => Err(ToolError::Forbidden(format!(
            "запись вне рабочей области '{}' запрещена политикой пайплайна (путь: {})",
            ctx.write_root.display(),
            abs.display()
        ))),
        WriteOutside::Prompt => {
            ctx.approver
                .check_write(abs, ctx.session_id, ctx.agent_id, tool)
        }
    }
}

/// Мост: хостовая политика записи → хук ядра `WriteAuthorizer`.
/// `ToolError` хоста и ядра — один и тот же тип, конверсий не требуется.
struct CoreAuthorizer<'a> {
    ctx: &'a ToolCtx<'a>,
}

impl ko_code_tools::WriteAuthorizer for CoreAuthorizer<'_> {
    fn authorize(&self, abs_path: &Path, tool: &str) -> Result<(), ToolError> {
        authorize_write(abs_path, self.ctx, tool)
    }
}

/// Тонкий адаптер: хостовый `Tool` поверх ядерного `CodeTool`.
/// name/description/parameters/is_readonly берутся из ядра (паритет без дублей);
/// execute поднимает ядерный ToolCtx с хостовой политикой записи через CoreAuthorizer.
macro_rules! adapter_tool {
    ($name:ident, $core_ty:path) => {
        pub struct $name;

        impl Tool for $name {
            fn name(&self) -> &str {
                $core_ty.name()
            }
            fn description(&self) -> &str {
                $core_ty.description()
            }
            fn parameters(&self) -> Value {
                $core_ty.parameters()
            }
            fn is_readonly(&self) -> bool {
                $core_ty.is_readonly()
            }
            fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
                let ko_auth = CoreAuthorizer { ctx };
                // Линт-гейт хоста: правка, ломающая код, откатывается.
                //
                // Гейт подключается ТОЛЬКО если сервер проверки установлен: без
                // него проверять нечем, и «проверка не выполнена» — это не отказ
                // правки, а отсутствие инструмента. Причина уходит в лог
                // (правило 3.4: честно и без шума в ответе агенту).
                //
                // Гейт объявлен ДО выбора: его ссылка живёт дольше контекста,
                // поэтому в ветке `if` он не создаётся.
                let gate = crate::infra::lsp::lint_gate::LspLintGate {
                    workspace_root: ctx.workspace_root,
                    bins_dir: ctx.bins_dir,
                };
                let server_available =
                    crate::infra::lsp::manager::is_lsp_server_available(ctx.bins_dir);
                if !server_available {
                    log::warn!(
                        "lint-gate: сервер проверки ('{}') не установлен в '{}' — проверка правок отключена",
                        crate::infra::lsp::LSP_SERVER_NAME,
                        ctx.bins_dir.display()
                    );
                }
                let ko_ctx = ko_code_tools::ToolCtx::new(ctx.workspace_root, &ko_auth);
                // Гейт ленивый — сервер поднимается только если проверка
                // реально понадобилась (то есть при записи в файл).
                let ko_ctx = if server_available {
                    ko_ctx.with_lint_gate(&gate)
                } else {
                    ko_ctx
                };
                $core_ty.execute(args, &ko_ctx)
            }
        }
    };
}

// Файловые тулы (SSOT: ko-code-tools::fs).
adapter_tool!(ReadFile, ko_code_tools::fs::ReadFile);
adapter_tool!(ReadManyFiles, ko_code_tools::fs::ReadManyFiles);
adapter_tool!(WriteFile, ko_code_tools::fs::WriteFile);
adapter_tool!(EditFile, ko_code_tools::fs::EditFile);
// Поисковые тулы (SSOT: ko-code-tools::search).
adapter_tool!(Grep, ko_code_tools::search::Grep);
adapter_tool!(Glob, ko_code_tools::search::Glob);
adapter_tool!(ListDirectory, ko_code_tools::search::ListDirectory);
// Shell-тулы (SSOT: ko-code-tools::shell).
adapter_tool!(Bash, ko_code_tools::shell::Bash);
adapter_tool!(RunTests, ko_code_tools::shell::RunTests);

/// SSOT-реестр: ВСЕ тулы кодинга. Ядро (fs/search/shell) + LSP хоста.
/// `code_read`/`code_write` — фильтры по `is_readonly()` над этим списком.
pub fn all_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ReadFile),
        Box::new(ReadManyFiles),
        Box::new(WriteFile),
        Box::new(EditFile),
        Box::new(Grep),
        Box::new(Glob),
        Box::new(ListDirectory),
        Box::new(Bash),
        Box::new(RunTests),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolCtx<'static> {
        ToolCtx {
            workspace_root: std::path::Path::new("."),
            write_root: std::path::Path::new("."),
            write_outside: WriteOutside::Prompt,
            session_id: "test",
            approver: crate::infra::permissions::test_approver(),
            agent_id: "test_agent",
            bins_dir: std::path::Path::new("."),
            image_artifacts: None,
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
    fn unknown_tool_returns_usage_error() {
        let err = execute_tool("nope", &serde_json::json!({}), &ctx()).unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)));
    }

    // ── Гейт паритета адаптеров: адаптер — зеркало ядра (имя/описание/схема/флаг) ──

    /// Пары (хост-адаптер, ядерный CodeTool) — реестр один-в-один с ядерным.
    fn adapter_core_pairs() -> Vec<(Box<dyn Tool>, Box<dyn ko_code_tools::CodeTool>)> {
        vec![
            (Box::new(ReadFile), Box::new(ko_code_tools::fs::ReadFile)),
            (Box::new(ReadManyFiles), Box::new(ko_code_tools::fs::ReadManyFiles)),
            (Box::new(WriteFile), Box::new(ko_code_tools::fs::WriteFile)),
            (Box::new(EditFile), Box::new(ko_code_tools::fs::EditFile)),
            (Box::new(Grep), Box::new(ko_code_tools::search::Grep)),
            (Box::new(Glob), Box::new(ko_code_tools::search::Glob)),
            (Box::new(ListDirectory), Box::new(ko_code_tools::search::ListDirectory)),
            (Box::new(Bash), Box::new(ko_code_tools::shell::Bash)),
            (Box::new(RunTests), Box::new(ko_code_tools::shell::RunTests)),
        ]
    }

    #[test]
    fn adapters_mirror_core_contract() {
        for (tool, core) in adapter_core_pairs() {
            assert_eq!(tool.name(), core.name(), "имя адаптера != ядра: {}", tool.name());
            assert_eq!(
                tool.description(),
                core.description(),
                "описание адаптера != ядра: {}",
                tool.name()
            );
            assert_eq!(
                tool.parameters(),
                core.parameters(),
                "схема адаптера != ядра: {}",
                tool.name()
            );
            assert_eq!(tool.is_readonly(), core.is_readonly(), "флаг readonly: {}", tool.name());
        }
    }

    #[test]
    fn host_registry_matches_core_registry_order() {
        // 9 кодовых тулов в реестре хоста — в том же порядке, что и в ядерном реестре.
        let core_names: Vec<String> =
            ko_code_tools::all_code_tools().iter().map(|t| t.name().to_string()).collect();
        let host_names: Vec<String> =
            all_tools().iter().map(|t| t.name().to_string()).collect();
        assert_eq!(
            &host_names[..core_names.len()],
            &core_names[..],
            "порядок кодовых тулов хоста обязан совпадать с ядерным"
        );
    }
}
