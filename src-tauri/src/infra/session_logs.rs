//! Пер-сессионный архив строк лога — `<exe>/session_logs/<session_id>.log`.
//!
//! PRP: `tasks/261119_архив_логов_сессий_PRP.md`; карта сохраняемости логов — AGENTS.md §4.
//!
//! Зачем: runtime-маркеры оркестратора (🧠 `force_no_thinking`, «докачка», «рестарт-класс»)
//! живут только в `king_orch.log`, который обнуляется (truncate) на старте ЗАПУСКА
//! приложения (правило core rules §2.5.1) — разбор старых кейсов терял доказательства
//! (пункт 3 отчёта специалиста: какой путь отключил thinking — недоказуем).
//! Архив ДОПИСЫВАЕТ те же строки в файл сессии и переживает перезапуск.
//!
//! Механика: подписка на строки через `plugin_log_collector::logger::push_emit_hook` —
//! подписчики строк живут списком в плагине (19.11.26: single-slot `set_emit_hook`
//! затирал раннего подписчика — plugin setup ставил UI-хук ПОЗЖЕ хоста и убивал архив).
//! Хук получает каждую готовую запись после нарезки мультистрок (core/logger.rs
//! `write_regular`). Живой `king_orch.log` / `test/last_logs.txt` не трогаем —
//! это дополнительный приёмник, SSOT логгера не меняется.
//!
//! Честные ограничения:
//! - строки до установки логгера (`early_log`, pre-Tauri) идут мимо хука → в архив не
//!   попадают (они и так в `king_orch.log` текущего запуска);
//! - `_startup.log` — приёмник строк до первого хода чата (текущая сессия ещё не известна);
//!   обнуляется на старте запуска, как `king_orch.log`;
//! - архив `<session_id>.log` дописывается (append), не truncate: перезапуск приложения
//!   не убивает архив прошлой сессии;
//! - ретеншен: файлы сессий старше `KEEP_SESSION_FILES` удаляются (ограниченный рост,
//!   core rules §1.2); `_startup.log` в счёте не участвует;
//! - отказ записи не паникует и не блокирует логгер; `log::warn!` ровно один раз
//!   (флаг ставится ДО вызова warn — иначе warn через хук рекурсивен).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::OnceLock;

/// Приёмник строк до открытия чат-сессии (truncate на запуск).
pub const STARTUP_LOG_NAME: &str = "_startup.log";
/// Ретеншен: сколько файлов сессий хранить (PRP §2).
const KEEP_SESSION_FILES: usize = 10;

static ARCHIVE: OnceLock<Mutex<ArchiveState>> = OnceLock::new();

/// Токен для имени файла: только файлобезопасные символы, ограничение длины
/// по границам символов (не байт). SSOT — переехал из api/chat.rs (там
/// использовался тем же образом для имён файлов с session_id).
pub fn sanitize_filename_token(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect();
    cleaned
}

/// Папка архива: рядом с exe (правило cwd: `current_exe().parent()`, НЕ `app.path()`).
fn session_logs_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("session_logs")
}

fn session_file_name(session_id: &str) -> String {
    format!("{}.log", sanitize_filename_token(session_id))
}

fn append_entry(path: &Path, entry: &str) -> std::io::Result<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(entry.as_bytes())?;
    f.write_all(b"\n")
}

/// Удалить старейшие архивы сессий, чтобы файлов (включая активный) осталось
/// не больше `keep`. `_startup.log` и активный файл не удаляются никогда.
/// Возвращает имена удалённых файлов.
fn prune_sessions(dir: &Path, keep: usize, active: Option<&Path>) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let active_path = active.map(|p| p.to_path_buf());
    let mut candidates: Vec<(PathBuf, std::time::SystemTime)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "log").unwrap_or(false))
        .filter(|e| e.file_name().to_string_lossy() != STARTUP_LOG_NAME)
        .filter(|e| active_path.as_deref().map(|a| e.path() != a).unwrap_or(true))
        .filter_map(|e| {
            let t = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), t))
        })
        .collect();
    // Активный файл входит в лимит: candidates — только неактивные.
    let total = candidates.len() + usize::from(active_path.is_some());
    let mut removed = Vec::new();
    if total > keep {
        candidates.sort_by_key(|(_, t)| *t);
        let mut excess = total - keep;
        for (p, _) in candidates {
            if excess == 0 {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                removed.push(
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
                excess -= 1;
            }
        }
    }
    removed
}

struct ArchiveState {
    dir: PathBuf,
    startup: PathBuf,
    current: Option<PathBuf>,
    current_session_id: Option<String>,
    /// Флаг «уже предупредили» о отказе записи: хук не должен спамить warn на
    /// каждую строку при постоянном отказе (папка закрыта антивирусом и т.п.).
    warned: bool,
}

impl ArchiveState {
    /// Создать папку архива и открыть `_startup.log` начисто (truncate на запуск).
    fn new(dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let startup = dir.join(STARTUP_LOG_NAME);
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&startup)?;
        Ok(Self {
            dir,
            startup,
            current: None,
            current_session_id: None,
            warned: false,
        })
    }

    /// Дописать одну запись в активный файл (или в `_startup.log`, если
    /// сессия ещё не известна).
    fn handle_entry(&mut self, entry: &str) -> std::io::Result<()> {
        let path = if let Some(p) = self.current.as_ref() {
            p
        } else {
            &self.startup
        };
        append_entry(path, entry)
    }

    /// Переключить активный файл архива. Переключение на ту же сессию — no-op
    /// (ходы чата звали бы ретеншен на каждой строке промпта).
    fn switch(
        &mut self,
        session_id: Option<&str>,
    ) -> std::io::Result<(Option<PathBuf>, Vec<String>)> {
        let sid = session_id.map(|s| s.to_string());
        if self.current_session_id == sid {
            return Ok((self.current.clone(), Vec::new()));
        }
        self.current_session_id = sid;
        self.current = match session_id {
            Some(id) if !id.is_empty() => {
                let path = self.dir.join(session_file_name(id));
                // Создаём/открываем сразу: первая запись хода гарантированно
                // попадает в файл сессии, а не в _startup.log.
                OpenOptions::new().create(true).append(true).open(&path)?;
                Some(path)
            }
            _ => None,
        };
        let removed = prune_sessions(&self.dir, KEEP_SESSION_FILES, self.current.as_deref());
        Ok((self.current.clone(), removed))
    }
}

/// Инициализация: открыть папку архива и установить хук на строки лога.
/// Зовётся из main.rs сразу после `early_init("king_orch.log")`. Ошибку не
/// роняет: без архива приложение работает как раньше.
pub fn init_session_logs() {
    if ARCHIVE.get().is_some() {
        return;
    }
    match ArchiveState::new(session_logs_dir()) {
        Ok(state) => {
            let _ = ARCHIVE.set(Mutex::new(state));
            plugin_log_collector::logger::push_emit_hook(std::sync::Arc::new(on_log_entry));
        }
        Err(e) => {
            log::warn!("[session_logs] архив логов недоступен, строки не сохраняются: {e}");
        }
    }
}

/// Переключить активный файл архива на сессию. Зовётся при старте хода чата
/// (api/chat.rs). `None` возвращает приёмник в `_startup.log`. Ошибки не
/// паникуют: архив — дополнение, не критический путь.
pub fn set_current_session(session_id: Option<&str>) {
    let Some(mutex) = ARCHIVE.get() else { return };
    let mut outcome: Option<(Option<PathBuf>, Vec<String>)> = None;
    let mut err: Option<std::io::Error> = None;
    if let Ok(mut g) = mutex.lock() {
        match g.switch(session_id) {
            Ok(o) => outcome = Some(o),
            Err(e) => err = Some(e),
        }
    }
    if let Some(e) = err {
        log::warn!("[session_logs] переключение архива не удалось: {e}");
        return;
    }
    if let Some((path, removed)) = outcome {
        match path {
            Some(p) => log::info!("[session_logs] архив сессии → {}", p.display()),
            None => log::info!("[session_logs] архив → {}", STARTUP_LOG_NAME),
        }
        if !removed.is_empty() {
            log::info!(
                "[session_logs] ретеншен: удалено {} старых архивов",
                removed.len()
            );
        }
    }
}

/// Тело хука: одна готовая запись лога. Пишет без паники; при отказе — один
/// `log::warn!` на жизнь процесса (флаг под той же блокировкой, вызов warn —
/// после снятия: warn проходит через хук и без флага был бы бесконечным).
fn on_log_entry(entry: &str) {
    let Some(mutex) = ARCHIVE.get() else { return };
    let failure: Option<std::io::Error> = {
        let mut g = match mutex.lock() {
            Ok(g) => g,
            Err(_) => return, // отравленная блокировка: молча пропускаем
        };
        if g.warned {
            return;
        }
        match g.handle_entry(entry) {
            Ok(()) => None,
            Err(e) => {
                g.warned = true;
                Some(e)
            }
        }
    };
    if let Some(e) = failure {
        log::warn!("[session_logs] запись архива не удалась, строки пропускаются: {e}");
    }
}

impl ArchiveState {
    // Поле-флаг «уже предупредили» живёт в самой структуре (см. warned).
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::time::SystemTime;

    /// Уникальная папка под тест (диаг-прецедент: std::env::temp_dir + префикс).
    fn test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ko_session_logs_{}_{}_{}",
            tag,
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    // ── sanitize_filename_token (переехал из api/chat.rs, SSOT) ──

    #[test]
    fn sanitizes_unsafe_chars_and_keeps_short_ascii() {
        assert_eq!(sanitize_filename_token("basic"), "basic");
        assert_eq!(sanitize_filename_token("9router:gemini"), "9router_gemini");
        assert_eq!(
            sanitize_filename_token("a/b\\c*d?e"),
            "a_b_c_d_e",
            "опасные для путей символы заменяются"
        );
    }

    #[test]
    fn truncates_by_chars_not_bytes() {
        let long = "д".repeat(80); // кириллица: 2 байта на символ
        assert_eq!(sanitize_filename_token(&long).chars().count(), 40);
    }

    // ── append_entry ──

    #[test]
    fn append_entry_writes_and_appends() {
        let dir = test_dir("append");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.log");
        append_entry(&path, "строка 1").unwrap();
        append_entry(&path, "строка 2").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "строка 1\nстрока 2\n");
        cleanup(&dir);
    }

    // ── ArchiveState::new: startup truncate ──

    #[test]
    fn startup_log_truncated_on_new_state() {
        let dir = test_dir("truncate");
        {
            let mut st = ArchiveState::new(dir.clone()).unwrap();
            st.handle_entry("старая запись").unwrap();
        }
        // Новый «запуск» — startup начисто
        let st = ArchiveState::new(dir.clone()).unwrap();
        let content = std::fs::read_to_string(&st.startup).unwrap();
        assert_eq!(content, "", "новый запуск обнуляет _startup.log");
        cleanup(&dir);
    }

    // ── switch: файл сессии, append переживает пересоздание, None → startup ──

    #[test]
    fn switch_routes_entries_and_appends_across_states() {
        let dir = test_dir("switch");
        {
            let mut st = ArchiveState::new(dir.clone()).unwrap();
            st.handle_entry("до сессии").unwrap();
            let (path, removed) = st.switch(Some("sess/1")).unwrap();
            assert_eq!(
                path.unwrap().file_name().unwrap().to_string_lossy(),
                "sess_1.log",
                "session_id санитизируется в имени файла"
            );
            assert!(removed.is_empty());
            st.handle_entry("ход 1").unwrap();
        }
        // Новый ArchiveState с тем же dir: startup обнулён, файл сессии ЖИВ и дописывается
        let mut st = ArchiveState::new(dir.clone()).unwrap();
        let (path, _) = st.switch(Some("sess/1")).unwrap();
        st.handle_entry("ход 2").unwrap();
        let session_content = std::fs::read_to_string(path.unwrap()).unwrap();
        assert_eq!(session_content, "ход 1\nход 2\n", "перезапуск не убивает архив сессии");
        let startup_content = std::fs::read_to_string(&st.startup).unwrap();
        assert_eq!(startup_content, "");
        // None → startup
        st.switch(None).unwrap();
        st.handle_entry("вне сессии").unwrap();
        assert!(std::fs::read_to_string(&st.startup).unwrap().contains("вне сессии"));
        cleanup(&dir);
    }

    #[test]
    fn switch_same_session_is_noop() {
        let dir = test_dir("noop");
        let mut st = ArchiveState::new(dir.clone()).unwrap();
        st.switch(Some("s1")).unwrap();
        let before = st.current.clone();
        let (path, removed) = st.switch(Some("s1")).unwrap();
        assert_eq!(path, before);
        assert!(removed.is_empty());
        cleanup(&dir);
    }

    // ── ретеншен ──

    #[test]
    fn prune_keeps_newest_and_never_touches_active_or_startup() {
        let dir = test_dir("prune");
        std::fs::create_dir_all(&dir).unwrap();
        let startup = dir.join(STARTUP_LOG_NAME);
        File::create(&startup).unwrap();
        // 12 файлов сессий с явно заданным временем изменения (по возрастанию)
        for i in 0..12 {
            let p = dir.join(format!("sess_{:02}.log", i));
            let f = File::create(&p).unwrap();
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i as u64);
            f.set_times(std::fs::FileTimes::new().set_modified(t)).unwrap();
        }
        let active = dir.join("sess_11.log"); // новейший
        let removed = prune_sessions(&dir, 10, Some(&active));
        assert_eq!(removed.len(), 2, "12 файлов − лимит 10 = 2 удаления");
        assert!(removed.contains(&"sess_00.log".to_string()));
        assert!(removed.contains(&"sess_01.log".to_string()));
        for i in 2..12 {
            assert!(dir.join(format!("sess_{:02}.log", i)).exists(), "sess_{:02} жив", i);
        }
        assert!(startup.exists(), "_startup.log не удаляется");
        cleanup(&dir);
    }

    // ── отказ записи без паники ──

    #[test]
    fn new_state_on_file_path_fails_without_panic() {
        let dir = test_dir("badpath");
        std::fs::create_dir_all(&dir).unwrap();
        let file_as_dir = dir.join("это_файл");
        File::create(&file_as_dir).unwrap();
        // dir занят файлом: create_dir_all обязан дать Err, не паникуя
        assert!(ArchiveState::new(file_as_dir).is_err());
        cleanup(&dir);
    }
}
