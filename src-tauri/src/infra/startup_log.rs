//! Запускной логгер.
//! Пишет текстовый лог-файл `king_orch.log` РЯДОМ С EXE — чтобы юзер мог
//! скинуть его, даже если приложение вообще не открывается или падает
//! в первые секунды. Если папка exe недоступна для записи (Program Files
//! без прав) — fallback в текущую директорию, затем в AppData.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const LOG_FILE_NAME: &str = "king_orch.log";

static LOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Путь лога по умолчанию — рядом с exe
pub fn log_path(exe_dir: &Path) -> PathBuf {
    exe_dir.join(LOG_FILE_NAME)
}

/// Инициализирован ли логгер (путь уже выбран)
pub fn is_initialized() -> bool {
    LOG_PATH.lock().map(|g| g.is_some()).unwrap_or(false)
}

/// Инициализация: выбирает место для лога и пишет заголовок запуска.
/// Возвращает итоговый путь лога.
pub fn init(exe_dir: &Path) -> PathBuf {
    let mut path = log_path(exe_dir);

    let mut ok = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .is_ok();

    if !ok {
        // Fallback 1: текущая рабочая директория
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        path = cwd.join(LOG_FILE_NAME);
        ok = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .is_ok();
    }

    if !ok {
        // Fallback 2: системная temp-директория
        path = std::env::temp_dir().join(LOG_FILE_NAME);
        let _ = OpenOptions::new().create(true).append(true).open(&path);
    }

    if let Ok(mut guard) = LOG_PATH.lock() {
        *guard = Some(path.clone());
    }
    append("INFO", &format!("Лог-файл: {}", path.display()));
    path
}

/// Дописать строку в лог-файл (создаётся при необходимости).
/// Никогда не паникует и не блокирует работу приложения.
pub fn append(level: &str, msg: &str) {
    let path = match LOG_PATH.lock() {
        Ok(guard) => guard.as_deref().map(|p| p.to_path_buf()),
        Err(_) => None,
    };
    let Some(path) = path else { return };

    let line = format!("[{}] [{}] {}\n", timestamp(), level, msg);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }

    // ── Телеметрия: критические ошибки дополнительно уходят в сервис
    // анонимных отчётов (если юзер не отключил эту опцию в настройках).
    // Уровень PANIC здесь НЕ дублируем: паника отправляется через panic-хук
    // плагина телеметрии, а этот файл уже дописывает её в лог выше.
    if matches!(level, "ERROR" | "FATAL" | "CRASH") {
        crate::infra::telemetry::track_error(level, msg);
    }
}

/// Установить panic-hook: любой паникующий поток допишет причину в лог-файл.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "неизвестная паника".to_string());
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        append("PANIC", &format!("{} | {}", msg, loc));
    }));
}

/// Читаемый таймстамп YYYY-MM-DD HH:MM:SS (без внешних крейтов)
pub(crate) fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
