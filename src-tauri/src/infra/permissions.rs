//! 🔐 Система разрешений записи для инструментов кодинга.
//!
//! Правила (согласовано с пользователем):
//! - Чтение — любые пути, без подтверждения.
//! - Запись внутри корня проекта — авто (с логированием в диспетчере).
//! - Запись вне корня — плашка в UI: **Запретить / Разрешить 1 раз /
//!   Разрешить в этом чате** (для сессии).
//!
//! `check_write` вызывается синхронно из цикла агента (spawn_blocking) и
//! блокируется на канале до ответа пользователя; `respond_permission`
//! (Tauri-команда) разрешает ожидание из async-рантайма.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::infra::event_bus::{AgentEvent, global_bus};
use crate::infra::tools::ToolError;

/// Решение пользователя по плашке.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantDecision {
    /// Запретить эту запись.
    Deny,
    /// Разрешить один раз (не запоминать на будущее).
    AllowOnce,
    /// Разрешить в этой сессии чата (запоминаем путь до конца сессии).
    AllowSession,
}

impl GrantDecision {
    pub fn from_str(s: &str) -> Option<GrantDecision> {
        match s {
            "deny" => Some(GrantDecision::Deny),
            "allow_once" => Some(GrantDecision::AllowOnce),
            "allow_session" => Some(GrantDecision::AllowSession),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            GrantDecision::Deny => "deny",
            GrantDecision::AllowOnce => "allow_once",
            GrantDecision::AllowSession => "allow_session",
        }
    }
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

struct Inner {
    /// session_id -> пути, разрешённые на всю сессию («в этом чате»).
    session_grants: Mutex<HashMap<String, HashSet<PathBuf>>>,
    /// request_id -> отправитель решения (канал, на котором ждёт агентский цикл).
    pending: Mutex<HashMap<String, mpsc::Sender<GrantDecision>>>,
    /// Тестовый режим: любые запросы вне корня отклоняются автоматически (Deny).
    /// Сам корень в тестах проверяют тулы (ctx.workspace_root), сюда доходят
    /// только пути, уже признанные «вне корня».
    test_auto_deny: bool,
}

/// Единый глобальный approver. Один на приложение (чат один за раз).
pub struct PermissionApprover {
    inner: Inner,
}

impl PermissionApprover {
    fn new(test_auto_deny: bool) -> Self {
        PermissionApprover {
            inner: Inner {
                session_grants: Mutex::new(HashMap::new()),
                pending: Mutex::new(HashMap::new()),
                test_auto_deny,
            },
        }
    }

    /// Проверить право на запись по пути. Вызывается ТОЛЬКО для путей вне корня
    /// (корень проверяют сами тулы через `ctx.workspace_root`). Ok — писать можно;
    /// Err — запрещено.
    pub fn check_write(
        &self,
        path: &Path,
        session_id: &str,
        agent_id: &str,
        tool: &str,
    ) -> Result<(), ToolError> {
        // Разрешённый на сессию путь — авто.
        {
            let grants = self.inner.session_grants.lock().unwrap();
            if let Some(set) = grants.get(session_id) {
                if set.iter().any(|grant| path.starts_with(grant)) {
                    return Ok(());
                }
            }
        }
        if self.inner.test_auto_deny {
            return Err(ToolError::Forbidden(format!(
                "запись вне корня проекта запрещена (тест): {}",
                path.display()
            )));
        }

        // Плашка пользователю: регистрируем ожидание и ждём решения.
        let (tx, rx) = mpsc::channel::<GrantDecision>();
        let request_id = format!("perm_{}", uuid_short());
        self.inner.pending.lock().unwrap().insert(request_id.clone(), tx);

        global_bus().publish(AgentEvent::PermissionRequest {
            request_id: request_id.clone(),
            agent: agent_id.to_string(),
            tool: tool.to_string(),
            path: path.display().to_string(),
        });

        // Блокируем агентский цикл до ответа пользователя (или таймаута).
        match rx.recv_timeout(REQUEST_TIMEOUT) {
            Ok(GrantDecision::AllowOnce) => Ok(()),
            Ok(GrantDecision::AllowSession) => {
                self.inner
                    .session_grants
                    .lock()
                    .unwrap()
                    .entry(session_id.to_string())
                    .or_default()
                    .insert(path.to_path_buf());
                Ok(())
            }
            Ok(GrantDecision::Deny) => Err(ToolError::Forbidden(format!(
                "пользователь запретил запись по пути: {}",
                path.display()
            ))),
            Err(_) => {
                self.inner.pending.lock().unwrap().remove(&request_id);
                Err(ToolError::Forbidden(format!(
                    "истекло время ожидания подтверждения записи по пути: {}",
                    path.display()
                )))
            }
        }
    }

    /// Разрешить ожидание из фронтенда (команда `respond_permission`).
    pub fn resolve(&self, request_id: &str, decision: GrantDecision) -> bool {
        let sender = self.inner.pending.lock().unwrap().remove(request_id);
        match sender {
            Some(tx) => tx.send(decision).is_ok(),
            None => false,
        }
    }

    /// Сбросить все session-гранты (при новой сессии чата).
    pub fn reset_session(&self, session_id: &str) {
        self.inner.session_grants.lock().unwrap().remove(session_id);
    }
}

fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos)
}

static GLOBAL: OnceLock<Arc<PermissionApprover>> = OnceLock::new();
static TEST: OnceLock<Arc<PermissionApprover>> = OnceLock::new();

/// Глобальный approver приложения (используется циклом агента и командой).
pub fn global_approver() -> Arc<PermissionApprover> {
    GLOBAL
        .get_or_init(|| Arc::new(PermissionApprover::new(false)))
        .clone()
}

/// Тестовый approver: решения вне корня принимаются автоматически (Deny).
pub fn test_approver() -> &'static PermissionApprover {
    TEST.get_or_init(|| Arc::new(PermissionApprover::new(true)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn grant_decision_roundtrip() {
        assert_eq!(GrantDecision::from_str("deny"), Some(GrantDecision::Deny));
        assert_eq!(GrantDecision::from_str("allow_once"), Some(GrantDecision::AllowOnce));
        assert_eq!(GrantDecision::from_str("allow_session"), Some(GrantDecision::AllowSession));
        assert_eq!(GrantDecision::from_str("x"), None);
        assert_eq!(GrantDecision::Deny.as_str(), "deny");
    }

    #[test]
    fn test_mode_denies_all_unless_granted() {
        let a = PermissionApprover::new(true);
        let p = std::env::temp_dir().join("kingorch_perm_deny.txt");
        let _ = fs::remove_file(&p);
        let err = a.check_write(&p, "s1", "agent", "write_file").unwrap_err();
        assert!(matches!(err, ToolError::Forbidden(_)));
    }

    #[test]
    fn resolve_allows_session_grant() {
        let outside = std::env::temp_dir().join(format!("kingorch_perm_grant_{}.txt", std::process::id()));
        let _ = fs::remove_file(&outside);
        let a = PermissionApprover::new(false);

        // Публикуем в шину (нет подписчиков — безопасно) и ждём решение в потоке.
        let a2 = Arc::new(a);
        let a3 = a2.clone();
        let outside_inner = outside.clone();
        let worker = std::thread::spawn(move || {
            a3.check_write(&outside_inner, "sess1", "agent", "write_file")
        });
        std::thread::sleep(Duration::from_millis(100));
        // Достаём последний pending-request и разрешаем на сессию.
        let req_id = a2.inner.pending.lock().unwrap().keys().next().unwrap().clone();
        assert!(a2.resolve(&req_id, GrantDecision::AllowSession));
        let res = worker.join().unwrap();
        assert!(res.is_ok(), "AllowSession должен вернуть Ok: {:?}", res);
        // Повторный запрос того же пути — уже авто (в сессии).
        assert!(a2.check_write(&outside, "sess1", "agent", "write_file").is_ok());
    }
}