//! Линт-гейт хоста: проверка изменённого файла через LSP-сервер.
//!
//! ## Зачем
//!
//! Ядро кодо-тулов умеет откатить правку, которая ломает код, — но только если
//! кто-то умеет ответить «есть ли в файле новая синтаксическая ошибка». Ответчик
//! здесь — LSP-сервер (`rust-analyzer`), поднятый менеджером.
//!
//! ## Честность состояний
//!
//! - сервер ответил → возвращаем список проблем;
//! - сервер недоступен (не установлен, не запустился) → возвращаем `Ok(None)`:
//!   это «проверка НЕ выполнена», и ядро сообщит об этом агенту в ответе тула.
//!   Молча выдать пустой список значило бы соврать, что код проверен;
//! - сервер ответил ошибкой → `Err` с причиной.
//!
//! ## Что проверяется
//!
//! Только Rust-файлы (`.rs`): единственный сервер, который у нас есть, —
//! `rust-analyzer`. Для других расширений проверки нет, и это не отказ: список
//! пуст, а ядро считает гейт выполненным (проверять нечего).

use std::path::Path;

use ko_code_tools::{LintGate, LintProblem};

use super::manager;

/// Гейт проверки качества кода поверх LSP-сервера.
pub struct LspLintGate<'a> {
    /// Корень проекта (область работы LSP-сервера).
    pub workspace_root: &'a Path,
    /// Каталог, где ищется исполняемый файл сервера.
    pub bins_dir: &'a Path,
}

impl LspLintGate<'_> {
    /// Проблемы файла по данным LSP-сервера.
    ///
    /// `Ok(None)` — сервер недоступен: проверка не выполнена (не «ошибок нет»).
    fn problems_of(&self, path: &Path) -> Result<Option<Vec<LintProblem>>, String> {
        if path.extension().and_then(|e| e.to_str()).unwrap_or("") != "rs" {
            // Проверять нечего: не-Rust файлы этим сервером не проверяются.
            return Ok(Some(Vec::new()));
        }
        let items = match manager::diagnostic_items(
            self.workspace_root,
            self.bins_dir,
            path,
            |message| log::warn!("lint-gate: {message}"),
        ) {
            Ok(items) => items,
            Err(reason) => {
                log::warn!(
                    "lint-gate: {} — проверка качества не выполнена: {}",
                    path.display(),
                    reason
                );
                return Ok(None);
            }
        };
        Ok(Some(
            items
                .into_iter()
                .map(|item| LintProblem {
                    line: item["range"]["start"]["line"].as_u64().unwrap_or(0) as u32 + 1,
                    message: format!(
                        "[{}] {}",
                        severity_name(item["severity"].as_u64().unwrap_or(3)),
                        item["message"].as_str().unwrap_or("без описания")
                    ),
                })
                .collect(),
        ))
    }
}

/// Имя уровня диагностики (для сообщения агенту).
fn severity_name(severity: u64) -> &'static str {
    match severity {
        1 => "ERROR",
        2 => "WARN",
        3 => "INFO",
        4 => "HINT",
        _ => "?",
    }
}

impl LintGate for LspLintGate<'_> {
    fn snapshot(&self, path: &Path) -> Result<Option<Vec<LintProblem>>, String> {
        self.problems_of(path)
    }

    fn check(
        &self,
        path: &Path,
        baseline: &[LintProblem],
    ) -> Result<Option<Vec<LintProblem>>, String> {
        let Some(current) = self.problems_of(path)? else {
            return Ok(None);
        };
        let fresh: Vec<LintProblem> = current
            .into_iter()
            .filter(|problem| !baseline.contains(problem))
            .collect();
        Ok(Some(fresh))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Проверка без сервера: `.rs` без установленного rust-analyzer → «не выполнено».
    #[test]
    fn non_rust_file_has_nothing_to_check() {
        let gate = LspLintGate {
            workspace_root: Path::new("."),
            bins_dir: Path::new("."),
        };
        let problems = gate
            .problems_of(Path::new("a.txt"))
            .expect("не-Rust файл не требует сервера");
        assert_eq!(problems, Some(Vec::new()));
    }

    /// Отсутствие сервера — это «проверка не выполнена», а не «ошибок нет».
    #[test]
    fn missing_server_means_not_checked() {
        let gate = LspLintGate {
            workspace_root: Path::new("."),
            bins_dir: Path::new("definitely-not-a-dir-with-lsp"),
        };
        let result = gate.problems_of(Path::new("a.rs"));
        assert!(
            matches!(result, Ok(None)),
            "без сервера проверка не выполнена: {result:?}"
        );
    }
}
