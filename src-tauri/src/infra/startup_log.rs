//! Запускной логгер.
//! Пишет текстовый лог-файл `king_orch.log` РЯДОМ С EXE — чтобы юзер мог
//! скинуть его, даже если приложение вообще не открывается или падает
//! в первые секунды. Если папка exe недоступна для записи (Program Files
//! без прав) — fallback в текущую директорию, затем в AppData.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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

/// Читаемый таймстамп YYYY-MM-DD HH:MM:SS в ЛОКАЛЬНОМ времени.
/// GUI (src/controllers/chat.ts) пишет время через toLocaleTimeString() —
/// единый источник правды: лог должен совпадать с экраном.
pub(crate) fn timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_format_and_local_time() {
        let ts = timestamp();
        assert!(ts.len() >= 19, "формат YYYY-MM-DD HH:MM:SS, получено: {}", ts);
        assert!(ts.chars().nth(4) == Some('-') && ts.chars().nth(7) == Some('-'), "дата YYYY-MM-DD");
        assert!(ts.chars().nth(10) == Some(' '), "пробел между датой и временем");
        assert!(ts.chars().nth(13) == Some(':') && ts.chars().nth(16) == Some(':'), "время HH:MM:SS");
        let secs = chrono::Local::now();
        let utc = chrono::Utc::now();
        let offset = utc.signed_duration_since(secs.with_timezone(&chrono::Utc)).num_seconds();
        assert_eq!(offset, 0, "timestamp() берёт локальное время (Utc::now() == Local::now() переведённое)");
        assert_eq!(format!("{}", secs.format("%Y-%m-%d %H:%M:%S")), ts, "совпадает с chrono-форматом");
    }
}
