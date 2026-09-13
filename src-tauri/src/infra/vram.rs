//! 🎛 Блокирующий диалог pre-flight VRAM: модель не влезает в видеопамять.
//!
//! При запуске LlamaEngine оценивает потребность (модель + KV-кэш + буферы)
//! и сравнивает со свободной VRAM. Если не влезает — спрашиваем юзера:
//! «Отмена» (не запускать) или «Запустить с выгрузкой части слоёв в ОЗУ»
//! (урезать -ngl). Механика та же, что у PermissionApprover: mpsc-канал в
//! pending-словаре + событие на in-process шине + Tauri-команда
//! `respond_vram_choice` из фронтенда.
//!
//! Без UI (тесты, ранние этапы старта до инициализации фронта, таймаут) —
//! авто-решение `ProceedWithRam` (fallback как раньше): pre-flight столь же
//! снижает -ngl, но НЕ блокирует запуск модели.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::sync::mpsc;
use std::time::Duration;

use crate::infra::event_bus::{AgentEvent, global_bus};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Решение пользователя по диалогу pre-flight VRAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VramChoice {
    /// Юзер отменил запуск модели (не помещается в видеопамять).
    Cancel,
    /// Запустить с выгрузкой части слоёв в ОЗУ (урезанный -ngl).
    ProceedWithRam,
}

impl VramChoice {
    pub fn from_str(s: &str) -> Option<VramChoice> {
        match s {
            "cancel" => Some(VramChoice::Cancel),
            "proceed" => Some(VramChoice::ProceedWithRam),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            VramChoice::Cancel => "cancel",
            VramChoice::ProceedWithRam => "proceed",
        }
    }
}

struct Inner {
    /// request_id -> отправитель решения (канал, на котором ждёт pre-flight).
    pending: Mutex<std::collections::HashMap<String, mpsc::Sender<VramChoice>>>,
    /// Флаг «фронтенд готов»: true — можно блокировать и ждать решение юзера.
    ui_ready: AtomicBool,
}

/// Единый глобальный решатель по VRAM. Один на приложение.
pub struct VramApprover {
    inner: Inner,
}

impl VramApprover {
    /// Спросить юзера, запускать ли модель с выгрузкой части слоёв в ОЗУ.
    /// Вызывается синхронно из агентского цикла (spawn_blocking). Возвращает:
    /// - `Cancel` — юзер отменил (или UI недоступен и разрешён авто-режим);
    /// - `ProceedWithRam` — разрешил запуск с урезанным -ngl (или таймаут).
    /// `note` — человеко-читаемое описание (вставляется в текст диалога и лог).
    pub fn confirm_vram_reduction(&self, note: &str) -> VramChoice {
        // Фронтенд ещё не готов (тесты/ранний старт) — не блокируем, как раньше.
        if !self.inner.ui_ready.load(Ordering::Relaxed) {
            return VramChoice::ProceedWithRam;
        }

        let (tx, rx) = mpsc::channel::<VramChoice>();
        let request_id = format!("vram_{}", uuid_short());
        self.inner.pending.lock().unwrap().insert(request_id.clone(), tx);

        global_bus().publish(AgentEvent::VramConfirmRequest {
            request_id: request_id.clone(),
            note: note.to_string(),
        });

        match rx.recv_timeout(REQUEST_TIMEOUT) {
            Ok(choice) => choice,
            // Таймаут — стартуем с выгрузкой в ОЗУ (fallback, не блокируем модель).
            Err(_) => {
                self.inner.pending.lock().unwrap().remove(&request_id);
                VramChoice::ProceedWithRam
            }
        }
    }

    /// Разрешить ожидание из фронтенда (команда `respond_vram_choice`).
    pub fn resolve(&self, request_id: &str, choice: VramChoice) -> bool {
        let sender = self.inner.pending.lock().unwrap().remove(request_id);
        match sender {
            Some(tx) => tx.send(choice).is_ok(),
            None => false,
        }
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

static GLOBAL: OnceLock<Box<VramApprover>> = OnceLock::new();

/// Глобальный решатель по VRAM. По умолчанию `ui_ready=false`: блокирующий
/// диалог активен ТОЛЬКО после `setup()` приложения; тесты/CLI идут в
/// авто-режиме (ProceedWithRam без блокировки).
pub fn global_vram_approver() -> &'static VramApprover {
    GLOBAL
        .get_or_init(|| Box::new(VramApprover {
            inner: Inner {
                pending: Mutex::new(std::collections::HashMap::new()),
                ui_ready: AtomicBool::new(false),
            },
        }))
}

/// Пометить, что фронтенд готов принимать диалоги (вызывается в setup()).
pub fn set_ui_ready(ready: bool) {
    global_vram_approver().inner.ui_ready.store(ready, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_parses_from_frontend() {
        assert_eq!(VramChoice::from_str("cancel"), Some(VramChoice::Cancel));
        assert_eq!(VramChoice::from_str("proceed"), Some(VramChoice::ProceedWithRam));
        assert_eq!(VramChoice::from_str("delete_all"), None);
        assert_eq!(VramChoice::Cancel.as_str(), "cancel");
        assert_eq!(VramChoice::ProceedWithRam.as_str(), "proceed");
    }

    #[test]
    fn no_ui_auto_proceeds() {
        let a = VramApprover {
            inner: Inner {
                pending: Mutex::new(std::collections::HashMap::new()),
                ui_ready: AtomicBool::new(false),
            },
        };
        assert_eq!(
            a.confirm_vram_reduction("note"),
            VramChoice::ProceedWithRam
        );
    }

    #[test]
    fn resolve_wakes_the_channel() {
        let a = std::sync::Arc::new(VramApprover {
            inner: Inner {
                pending: Mutex::new(std::collections::HashMap::new()),
                ui_ready: AtomicBool::new(true),
            },
        });
        let a2 = a.clone();
        let worker = std::thread::spawn(move || {
            a2.confirm_vram_reduction("модель не влезает")
        });
        std::thread::sleep(Duration::from_millis(100));
        let req_id = a.inner.pending.lock().unwrap().keys().next().unwrap().clone();
        assert!(a.resolve(&req_id, VramChoice::ProceedWithRam));
        assert_eq!(worker.join().unwrap(), VramChoice::ProceedWithRam);
    }

    #[test]
    fn resolve_unknown_returns_false() {
        let a = VramApprover {
            inner: Inner {
                pending: Mutex::new(std::collections::HashMap::new()),
                ui_ready: AtomicBool::new(true),
            },
        };
        assert!(!a.resolve("vram_nonexistent", VramChoice::Cancel));
    }
}