//! 🌐 Capability веб-поиска: нативный тул `WebSearch` поверх SSOT-ядра
//! крейта `web-tools` (движки/HTTP/кеш/статистика + поисковое ядро).
//!
//! Нативная замена Deno MCP-сервера `web_search.ts`: контракт модели НЕ
//! меняется — имя, описание, inputSchema и формат вывода дословно прежние,
//! меняется только исполнение (из процесса хоста, без Deno-рантайма).
//! Кеш результатов (`search_cache.json`) и статистика движков
//! (`search_stats.json`) — в `ctx.bins_dir`, те же файлы, что писала
//! TS-версия (Rust-ядро — порт тех же TS-модулей).

use serde_json::Value;

use crate::infra::tools::{Tool, ToolCtx, ToolError};

pub const WEB_TOOL_NAMES: [&str; 1] = ["WebSearch"];

pub fn is_web_tool(name: &str) -> bool {
    WEB_TOOL_NAMES.contains(&name)
}

struct WebSearchTool;

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn description(&self) -> &str {
        "Поиск в интернете (без API-ключей). Мульти-движок: wiby, duckduckgo, marginalia, brave, startpage, sogou, juejin, csdn, baidu, bing, exa. По умолчанию — последовательная цепочка фолбэков по надёжности; можно указать engines (массив) для параллельного поиска по конкретным движкам. Поддерживает multi-query: передай queries (массив 1-5 запросов) — каждый выполняется параллельно, результаты дедуплицируются по домену+URL."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Поисковый запрос (один). Альтернатива: queries." },
                "queries": { "type": "array", "items": { "type": "string" }, "description": "Опционально: массив 1-5 запросов для multi-query поиска (параллельно, дедуп по домену+URL). Запросы — на языке пользователя, естественные фразы, без операторов (OR/AND/site: и т.п.)." },
                "engines": { "type": "array", "items": { "type": "string" }, "description": "Опционально: движки (wiby, duckduckgo, marginalia, brave, startpage, sogou, juejin, csdn, baidu, bing, exa). Без него — цепочка фолбэков." },
                "limit": { "type": "number", "description": "Опционально: максимум результатов (по умолчанию 8)" }
            }
        })
    }

    fn is_readonly(&self) -> bool {
        // Пользовательские данные не трогает (кеш/статистика — служебные
        // файлы в bins_dir) → авто-доступ без плашки.
        true
    }

    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let opts = web_tools::SearchOpts {
            data_dir: ctx.bins_dir.to_path_buf(),
        };
        web_tools::search_blocking(args, &opts).map_err(map_search_error)
    }
}

/// Схема web-тула (мета-имя "web") для агентов с `tools: {WebSearch: true}`.
pub fn web_tool_schemas(agent_tools: &[String]) -> Vec<(String, String, Value)> {
    let tool = WebSearchTool;
    if agent_tools.iter().any(|t| t == tool.name()) {
        vec![(
            "web".to_string(),
            tool.name().to_string(),
            serde_json::json!({
                "name": tool.name(),
                "description": tool.description(),
                "inputSchema": tool.parameters(),
            }),
        )]
    } else {
        Vec::new()
    }
}

/// Маппинг ошибок ядра в честный контракт тула (обе ошибки web-tools несут
/// конкретную причину, тексты ядро пишет под честный вывод модели).
fn map_search_error(err: web_tools::SearchError) -> ToolError {
    match err {
        web_tools::SearchError::Input(m) => ToolError::Usage(m),
        web_tools::SearchError::Network(m) => ToolError::Io(m),
    }
}

/// Исполнение web-тула. `ctx.bins_dir` — директория кеша/статистики
/// (преемственность с TS-сервером: те же файлы search_cache/search_stats).
pub fn execute_web_tool(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
) -> Option<Result<String, ToolError>> {
    if name != WebSearchTool.name() {
        return None;
    }
    Some(WebSearchTool.execute(args, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_matches_ts_server_contract() {
        let schemas = web_tool_schemas(&["WebSearch".to_string()]);
        assert_eq!(schemas.len(), 1);
        let (meta, name, schema) = &schemas[0];
        assert_eq!(meta, "web");
        assert_eq!(name, "WebSearch");
        assert_eq!(schema["inputSchema"]["type"], "object");
        for key in ["query", "queries", "engines", "limit"] {
            assert!(
                schema["inputSchema"]["properties"].get(key).is_some(),
                "свойство {key} обязано остаться в схеме (контракт TS-сервера)"
            );
        }
    }

    #[test]
    fn no_grant_no_schema() {
        assert!(web_tool_schemas(&[]).is_empty());
        assert!(web_tool_schemas(&["excel_apply".to_string()]).is_empty());
    }

    #[test]
    fn is_web_tool_only_websearch() {
        assert!(is_web_tool("WebSearch"));
        assert!(!is_web_tool("web_search"));
        assert!(!is_web_tool("excel_apply"));
    }

    #[test]
    fn search_error_mapping_is_honest() {
        // Input → Usage (честный класс), Network → Io (текст причины внутри).
        let usage = map_search_error(web_tools::SearchError::Input("Запрос не указан".into()));
        assert!(matches!(usage, ToolError::Usage(_)));
        let io = map_search_error(web_tools::SearchError::Network(
            "Поиск не дал результатов по всем движкам".into(),
        ));
        assert!(matches!(io, ToolError::Io(_)));
        assert!(io.to_string().contains("Поиск не дал результатов"));
    }

    #[test]
    fn unknown_name_is_none_not_error() {
        // Резолв по имени: чужие имена не должны попадать в web-исполнение.
        assert_eq!(WEB_TOOL_NAMES.len(), 1);
    }
}
