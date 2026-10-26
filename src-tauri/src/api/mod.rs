//! 🚪 API-слой — Tauri команды
//! main.rs регистрирует команды ТОЛЬКО через этот фасад

pub mod config;
pub mod sessions;
pub mod agents;
pub mod chat;
pub mod graph;
pub mod test;
pub mod coding_test;
pub mod file_utils;
pub mod permissions;
pub mod translate;
pub mod updater;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{oneshot, Semaphore};

/// Активный ход чата (один отправленный запрос = один ход).
///
/// Два канала отмены, потому что у хода две фазы ожидания:
/// - `cancel` — кооперативный флаг: его опрашивает цикл агента во время
///   генерации (существующий механизм `run_chat`);
/// - `stop_tx` — мгновенный канал: срабатывает, пока ход ещё СТОИТ В ОЧЕРЕДИ
///   на локальный движок (флаг там никто не опрашивает, а уходить из очереди
///   по «Стоп» нужно сразу, не дождавшись слота и не грузя модель зря).
#[derive(Debug)]
pub struct TurnHandle {
    pub cancel: Arc<AtomicBool>,
    stop_tx: Mutex<Option<oneshot::Sender<()>>>,
}

/// Глобальное состояние приложения
pub struct AppState {
    /// Активные ходы чатов: `session_id` → ход. Инвариант: **один активный ход
    /// на сессию** — повторный запрос той же сессии получает честную ошибку.
    /// Ходы РАЗНЫХ сессий идут параллельно (облачные — без ограничений,
    /// локальные — через `local_engine_gate` по очереди).
    pub active_turns: Mutex<HashMap<String, Arc<TurnHandle>>>,
    /// Гейт локального движка: 1 слот = один `llama-server.exe` в VRAM за раз.
    /// Инвариант «единственный потребитель VRAM» зашит в самом движке (NVML
    /// device 0, снапшот свободной VRAM при старте, `--parallel 1`,
    /// `VRAM_SAFETY_FACTOR` из TDR-инцидента с двумя параллельными инференсами),
    /// поэтому локальные ходы сериализуются здесь, а облачные идут мимо гейта.
    pub local_engine_gate: Arc<Semaphore>,
    /// Счётчики обращений к облачной LLM: `session_id` → число выполненных
    /// HTTP-запросов к роутеру за эту сессию (сумма за весь чат, а не за прогон).
    ///
    /// Ключ — id сессии: у каждой чат-вкладки своя сессия, поэтому вкладки не
    /// мешают друг другу автоматически. Значение — общий `Arc<AtomicU64>`,
    /// который инкрементит сам `CloudEngine` (infra не знает про AppState и
    /// получает счётчик на руки), — двойного учёта быть не может.
    pub cloud_requests: Mutex<HashMap<String, Arc<AtomicU64>>>,
}

/// Результат регистрации хода: ключ реестра (совпадает с `session_id`,
/// либо синтетический для пустого id), handle и канал мгновенной отмены.
#[derive(Debug)]
pub struct TurnRegistration {
    pub key: String,
    pub handle: Arc<TurnHandle>,
    pub stop_rx: oneshot::Receiver<()>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            active_turns: Mutex::new(HashMap::new()),
            local_engine_gate: Arc::new(Semaphore::new(1)),
            cloud_requests: Mutex::new(HashMap::new()),
        }
    }

    /// Зарегистрировать ход сессии. `Err` — если ход этой сессии уже идёт
    /// (второй параллельный ход одному чату запрещён: его итог всё равно
    /// перезаписал бы историю первым). Пустой `session_id` регистрируется под
    /// синтетическим уникальным ключом: такой ход нельзя остановить по имени,
    /// но и соседние ходы он не блокирует.
    pub fn register_turn(&self, session_id: &str) -> Result<TurnRegistration, String> {
        let key = if session_id.trim().is_empty() {
            anon_turn_key()
        } else {
            session_id.to_string()
        };
        let mut map = self.active_turns.lock().unwrap_or_else(|e| e.into_inner());
        if map.contains_key(&key) {
            return Err(
                "В этом чате уже идёт обработка сообщения — дождитесь завершения или нажмите «Стоп»."
                    .to_string(),
            );
        }
        let (tx, rx) = oneshot::channel();
        let handle = Arc::new(TurnHandle {
            cancel: Arc::new(AtomicBool::new(false)),
            stop_tx: Mutex::new(Some(tx)),
        });
        map.insert(key.clone(), handle.clone());
        Ok(TurnRegistration {
            key,
            handle,
            stop_rx: rx,
        })
    }

    /// Снять ход по завершении (вызывается из Drop-гарда — на любом выходе:
    /// успех, ошибка, отмена). Ключ передаётся тот, что вернула регистрация.
    /// Идемпотентно.
    pub fn finish_turn(&self, key: &str) {
        let mut map = self.active_turns.lock().unwrap_or_else(|e| e.into_inner());
        if map.remove(key).is_some() {
            log::debug!("[turns] ход {} завершён и снят с реестра", key);
        }
    }

    /// Отменить ход сессии («Стоп»): поднимает флаг отмены (его увидит цикл
    /// агента) и стреляет мгновенным каналом (выведет ход из очереди на
    /// локальный движок). `true` — ход был активен.
    pub fn cancel_turn(&self, session_id: &str) -> bool {
        let handle = {
            let map = self.active_turns.lock().unwrap_or_else(|e| e.into_inner());
            map.get(session_id).cloned()
        };
        match handle {
            Some(h) => {
                h.cancel.store(true, Ordering::SeqCst);
                if let Some(tx) = h.stop_tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    let _ = tx.send(());
                }
                true
            }
            None => false,
        }
    }

    /// Счётчик сессии (создаётся с нуля при первом обращении). Пустой id не
    /// считается: без сессии счётчик некуда привязать, и сумма «повисла» бы.
    pub fn cloud_counter(&self, session_id: &str) -> Arc<AtomicU64> {
        if session_id.trim().is_empty() {
            return Arc::new(AtomicU64::new(0));
        }
        let mut map = self.cloud_requests.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(session_id.to_string())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)))
            .clone()
    }

    /// Сколько запросов к облачной LLM израсходовано этой сессией.
    pub fn cloud_total(&self, session_id: &str) -> u64 {
        if session_id.trim().is_empty() {
            return 0;
        }
        let map = self.cloud_requests.lock().unwrap_or_else(|e| e.into_inner());
        map.get(session_id)
            .map(|counter| counter.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Поднять счётчик до значения из файла сессии (после перезапуска
    /// приложения). Только вверх: иначе открытый старый чат откатил бы счётчик
    /// и «съел» бы уже оплаченные запросы.
    pub fn prime_cloud_total(&self, session_id: &str, persisted: u64) -> u64 {
        let counter = self.cloud_counter(session_id);
        counter.fetch_max(persisted, Ordering::Relaxed);
        counter.load(Ordering::Relaxed)
    }

    /// Забыть счётчик удалённой сессии — иначе записи копятся в памяти.
    pub fn forget_cloud_counter(&self, session_id: &str) {
        let mut map = self.cloud_requests.lock().unwrap_or_else(|e| e.into_inner());
        if map.remove(session_id).is_some() {
            log::info!("[cloud] Счётчик запросов сессии {} удалён", session_id);
        }
    }
}

/// Синтетический ключ для хода без session_id (уникален в рамках процесса).
fn anon_turn_key() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("__anon_{:x}_{}", nanos, std::process::id())
}

/// RAII-гард: снимает ход с реестра на ЛЮБОМ выходе из команды (успех,
/// ошибка, отмена, паника) — второго шанса снять нет, поэтому только Drop.
pub struct TurnGuard<'a> {
    state: &'a AppState,
    key: String,
}

impl<'a> TurnGuard<'a> {
    pub fn new(state: &'a AppState, key: String) -> Self {
        Self { state, key }
    }
}

impl Drop for TurnGuard<'_> {
    fn drop(&mut self) {
        self.state.finish_turn(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        AppState::new()
    }

    #[test]
    fn cloud_counter_accumulates_per_session() {
        let state = state();
        let a = state.cloud_counter("s1");
        a.fetch_add(3, Ordering::Relaxed);
        a.fetch_add(1, Ordering::Relaxed);
        let b = state.cloud_counter("s2");
        b.fetch_add(7, Ordering::Relaxed);

        assert_eq!(state.cloud_total("s1"), 4);
        assert_eq!(state.cloud_total("s2"), 7);
    }

    #[test]
    fn cloud_counter_is_shared_between_acquisitions() {
        let state = state();
        let first = state.cloud_counter("s1");
        let second = state.cloud_counter("s1");
        first.fetch_add(2, Ordering::Relaxed);
        assert_eq!(second.load(Ordering::Relaxed), 2);
        assert_eq!(state.cloud_total("s1"), 2);
    }

    #[test]
    fn empty_session_id_is_not_counted() {
        let state = state();
        let counter = state.cloud_counter("");
        counter.fetch_add(5, Ordering::Relaxed);
        assert_eq!(state.cloud_total(""), 0);
        assert_eq!(state.cloud_total("missing"), 0);
    }

    #[test]
    fn prime_never_lowers_the_counter() {
        let state = state();
        state.cloud_counter("s1").fetch_add(9, Ordering::Relaxed);
        assert_eq!(state.prime_cloud_total("s1", 4), 9);
        assert_eq!(state.prime_cloud_total("s1", 40), 40);
    }

    #[test]
    fn forget_drops_the_session_counter() {
        let state = state();
        state.cloud_counter("s1").fetch_add(3, Ordering::Relaxed);
        state.forget_cloud_counter("s1");
        assert_eq!(state.cloud_total("s1"), 0);
    }

    // ── Реестр ходов: один активный ход на сессию ──

    #[test]
    fn second_turn_of_same_session_is_rejected() {
        let state = state();
        let reg = state.register_turn("s1").expect("первый ход регистрируется");
        let err = state
            .register_turn("s1")
            .expect_err("второй ход той же сессии обязан быть отвергнут");
        assert!(err.contains("уже идёт"), "текст ошибки объясняет причину: {}", err);
        state.finish_turn(&reg.key);
        assert!(
            state.register_turn("s1").is_ok(),
            "после снятия хода сессия снова может отправлять"
        );
    }

    #[test]
    fn different_sessions_register_in_parallel() {
        let state = state();
        let _a = state.register_turn("s1").expect("s1");
        let _b = state.register_turn("s2").expect("s2: ходы разных сессий параллельны");
    }

    #[test]
    fn empty_session_id_registers_under_unique_keys() {
        let state = state();
        let a = state.register_turn("").expect("анонимный ход 1");
        let b = state.register_turn("").expect("анонимный ход 2: ключи не конфликтуют");
        assert_ne!(a.key, b.key);
    }

    #[test]
    fn turn_guard_releases_registration_on_drop() {
        let state = state();
        {
            let reg = state.register_turn("s1").expect("ход");
            let _guard = TurnGuard::new(&state, reg.key);
            assert!(state.cancel_turn("s1"), "ход активен, отменяется");
        }
        assert!(
            !state.cancel_turn("s1"),
            "после Drop-гарда хода нет — отменять нечего"
        );
        assert!(state.register_turn("s1").is_ok(), "реестр свободен");
    }

    #[test]
    fn cancel_turn_raises_flag_and_fires_stop_channel() {
        let state = state();
        let mut reg = state.register_turn("s1").expect("ход");
        assert!(!reg.handle.cancel.load(Ordering::SeqCst));
        assert!(state.cancel_turn("s1"));
        assert!(reg.handle.cancel.load(Ordering::SeqCst));
        // Мгновенный канал сработал (receiver в руках теста).
        assert!(
            reg.stop_rx.try_recv().is_ok(),
            "stop_rx обязан разрешиться после cancel_turn"
        );
        // Повторный «Стоп» не ломает состояние (канал одноразовый).
        assert!(state.cancel_turn("s1"));
    }

    #[test]
    fn cancel_of_unknown_session_is_reported() {
        let state = state();
        assert!(!state.cancel_turn("missing"));
    }

    // ── Гейт локального движка: один слот ──

    #[tokio::test]
    async fn local_gate_holds_one_permit_and_is_reusable() {
        let state = state();
        let permit = state
            .local_engine_gate
            .clone()
            .try_acquire_owned()
            .expect("первый слот свободен");
        assert!(
            state.local_engine_gate.clone().try_acquire_owned().is_err(),
            "второй слот занят, пока держится первый"
        );
        drop(permit);
        assert!(
            state.local_engine_gate.clone().try_acquire_owned().is_ok(),
            "слот возвращается после Drop"
        );
    }

    #[tokio::test]
    async fn local_gate_waits_in_fifo_order() {
        // Два ждущих берут слот в порядке постановки (FIFO tokio::Semaphore).
        let gate = Arc::new(Semaphore::new(1));
        let first = gate.clone().acquire_owned().await.unwrap();
        let gate2 = gate.clone();
        let waiter1 = tokio::spawn(async move { gate2.acquire_owned().await.unwrap(); });
        let gate3 = gate.clone();
        let waiter2 = tokio::spawn(async move { gate3.acquire_owned().await.unwrap(); });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        drop(first);
        let (w1, w2) = tokio::join!(waiter1, waiter2);
        assert!(w1.is_ok() && w2.is_ok(), "оба ждущих дождались слота");
    }

    #[tokio::test]
    async fn queued_waiter_leaves_queue_when_cancelled() {
        // select! на acquire против канала отмены: дроп future = выход из очереди.
        let gate = Arc::new(Semaphore::new(1));
        let _holder = gate.clone().acquire_owned().await.unwrap();
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let gate2 = gate.clone();
        let queued = tokio::spawn(async move {
            let mut stop_rx = stop_rx;
            let acquire = gate2.acquire_owned();
            tokio::pin!(acquire);
            tokio::select! {
                permit = &mut acquire => Some(permit.expect("гейт не закрывается")),
                _ = &mut stop_rx => None,
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = stop_tx.send(());
        let outcome = queued.await.unwrap();
        assert!(outcome.is_none(), "отмена из очереди возвращает None, не слот");
    }
}