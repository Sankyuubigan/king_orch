//! Менеджер sidecar LSP-серверов: один процесс на workspace root, кэшируется
//! на время жизни приложения. Запуск ленивый — при первом вызове тула.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::{LspClient, find_lsp_server};

/// Глобальный менеджер: `workspace_root -> LspClient`.
pub struct LspManager {
    clients: Mutex<HashMap<PathBuf, LspClient>>,
}

fn global_manager() -> &'static LspManager {
    use std::sync::OnceLock;
    static M: OnceLock<LspManager> = OnceLock::new();
    M.get_or_init(|| LspManager { clients: Mutex::new(HashMap::new()) })
}

/// Выполнить операцию с LSP-клиентом для root. Lock удерживается на время
/// всей операции (безопасный доступ), сервер при необходимости стартует лениво.
fn with_client<R>(
    workspace_root: &Path,
    bins_dir: &Path,
    log_cb: impl Fn(String) + Send + Sync + 'static,
    op: impl FnOnce(&mut LspClient) -> Result<R, String>,
) -> Result<R, String> {
    let server = find_lsp_server(bins_dir)
        .ok_or_else(|| {
            format!(
                "LSP-сервер '{}' не установлен. Ожидался в '{}' или в PATH. \
                 Скачайте rust-analyzer и положите его в папку bins (см. bin_downloader).",
                super::LSP_SERVER_NAME,
                bins_dir.display()
            )
        })?;

    let manager = global_manager();
    let mut clients = lock_clients(&manager)?;

    if let Some(client) = clients.get_mut(workspace_root) {
        if let Ok(Some(status)) = client.try_wait() {
            clients.remove(workspace_root);
            return Err(format!(
                "LSP-сервер '{}' завершился с кодом {} — перезапустите обращение (сервер будет поднят заново).",
                super::LSP_SERVER_NAME,
                status.code().unwrap_or(-1)
            ));
        }
    }

    if !clients.contains_key(workspace_root) {
        let client = LspClient::spawn(&server, workspace_root, log_cb)?;
        clients.insert(workspace_root.to_path_buf(), client);
    }

    let client = clients
        .get_mut(workspace_root)
        .ok_or_else(|| "Не удалось получить LSP-клиент".to_string())?;
    op(client)
}

fn lock_clients(manager: &'static LspManager) -> Result<MutexGuard<'static, HashMap<PathBuf, LspClient>>, String> {
    manager.clients.lock().map_err(|_| "LspManager poisoned".to_string())
}

/// Ленивая проверка установленности сервера (для честного сообщения в тулах).
pub fn is_lsp_server_available(bins_dir: &Path) -> bool {
    find_lsp_server(bins_dir).is_some()
}

// ─── Тул-обёртки (блокирующие, с таймаутом внутри LspClient) ───

/// `textDocument/definition` — вернуть путь и позицию определения символа.
pub fn get_definition(
    workspace_root: &Path,
    bins_dir: &Path,
    path: &Path,
    line: u64,
    character: u64,
    log_cb: impl Fn(String) + Send + Sync + 'static,
) -> Result<String, String> {
    with_client(workspace_root, bins_dir, log_cb, |client| {
        let result = client.definition(path, line, character)?;
        Ok(super::format_locations(&result))
    })
}

/// `textDocument/references` — все ссылки на символ.
pub fn get_references(
    workspace_root: &Path,
    bins_dir: &Path,
    path: &Path,
    line: u64,
    character: u64,
    include_declaration: bool,
    log_cb: impl Fn(String) + Send + Sync + 'static,
) -> Result<String, String> {
    with_client(workspace_root, bins_dir, log_cb, |client| {
        let result = client.references(path, line, character, include_declaration)?;
        Ok(super::format_locations(&result))
    })
}

/// `textDocument/diagnostic` — диагностика файла (ошибки/предупреждения).
pub fn get_diagnostics(
    workspace_root: &Path,
    bins_dir: &Path,
    path: &Path,
    log_cb: impl Fn(String) + Send + Sync + 'static,
) -> Result<String, String> {
    with_client(workspace_root, bins_dir, log_cb, |client| {
        let result = client.diagnostics(path)?;
        Ok(super::format_diagnostics(&result))
    })
}
