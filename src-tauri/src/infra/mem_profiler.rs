//! Пиковый профайлер памяти (RAM + VRAM) во время генерации LLM.
//!
//! Замеряет в фоновом потоке каждые SAMPLE_INTERVAL мс:
//!   - RSS процесса llama-server (движок — ОТДЕЛЬНЫЙ процесс, PID известен)
//!   - RSS процесса приложения (текущий PID)
//!   - Занятая VRAM GPU (NVML, device-level: под WDDM per-process недоступен)
//! Хранит максимумы. Итог — одна строка лога после генерации (см. `peak_line`).
//! Ошибки измерения не роняют генерацию: поток просто останавливается,
//! а недоступность NVML честно отражается в отчёте (правило: без молчания).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Период семплирования. Каждый сэмпл: sysinfo (2 PID) + NVML ≈ 1-3 мс.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);
/// Страховка от зависшего NVML-вызова: поток сам завершится через 30 минут.
const MAX_SAMPLE_TIME: Duration = Duration::from_secs(30 * 60);

const MB: u64 = 1024 * 1024;

/// Собранные пики за время генерации (все значения в байтах).
#[derive(Debug, Clone, Copy, Default)]
pub struct MemReport {
    /// Пик RSS процесса llama-server (0, если PID не был известен)
    pub rss_server_peak: u64,
    /// Пик RSS процесса приложения
    pub rss_app_peak: u64,
    /// Пик занятой VRAM GPU (device-level, все процессы)
    pub vram_used_peak: u64,
    /// true — хотя бы один сэмпл VRAM прошёл успешно
    pub vram_ok: bool,
    /// Количество успешных сэмплов
    pub samples: u64,
}

/// Текущий RSS процесса приложения (байты). None — измерить не удалось.
pub fn current_process_rss() -> Option<u64> {
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from(std::process::id() as usize);
    sys.refresh_pids_specifics(&[pid], sysinfo::ProcessRefreshKind::new().with_memory());
    sys.process(pid).map(|p| p.memory())
}

/// Фоновый поток-семплер. Остановка — через `stop_and_report()` или Drop (RAII).
pub struct MemSampler {
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<MemReport>>,
}

impl MemSampler {
    /// Запускает семплирование. `server_pid` — PID процесса llama-server
    /// (None, если движок-процесс неизвестен).
    pub fn start(server_pid: Option<u32>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_ref = stop.clone();
        let handle = std::thread::spawn(move || sample_loop(server_pid, stop_ref));
        Self { stop, join: Some(handle) }
    }

    /// Останавливает поток и возвращает собранные пики.
    pub fn stop_and_report(&mut self) -> MemReport {
        self.stop.store(true, Ordering::SeqCst);
        self.join
            .take()
            .and_then(|j| j.join().ok())
            .unwrap_or_default()
    }
}

impl Drop for MemSampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// RAII-гвард замера: гарантированно останавливает семплер на ВСЕХ путях
/// выхода из генерации (ошибки, cancel, успех). Если замер не завершён
/// явно через `finish()` — пишет warn (замер прерван).
pub struct MemGuard<'a, F: Fn(String)> {
    sampler: Option<MemSampler>,
    log_cb: &'a F,
    label: String,
    finished: bool,
}

impl<'a, F: Fn(String)> MemGuard<'a, F> {
    pub fn new(sampler: MemSampler, label: &str, log_cb: &'a F) -> Self {
        Self {
            sampler: Some(sampler),
            log_cb,
            label: label.to_string(),
            finished: false,
        }
    }

    /// Останавливает замер и возвращает пики.
    pub fn finish(mut self) -> MemReport {
        self.finished = true;
        self.sampler
            .take()
            .map(|mut s| s.stop_and_report())
            .unwrap_or_default()
    }
}

impl<'a, F: Fn(String)> Drop for MemGuard<'a, F> {
    fn drop(&mut self) {
        if !self.finished {
            (self.log_cb)(format!(
                "⚠️ Замер памяти прерван ({}) — семплер остановлен",
                self.label
            ));
            self.sampler.take(); // Drop сам остановит поток
        }
    }
}

/// Формирует итоговую строку лога с пиками (одна строка на LLM-вызов).
/// `vram_before` — занятая VRAM до старта движка (байты), `expected_vram_mb` —
/// прогноз `estimate_vram_mb` для сверки, `extra` — токены/скорость/причина.
pub fn peak_line(
    label: &str,
    report: &MemReport,
    vram_before: u64,
    expected_vram_mb: f64,
    extra: &str,
) -> String {
    let mut parts = vec![
        format!("📊 Пик памяти [{}]:", label),
        format!("llama-server RSS={} МБ", report.rss_server_peak / MB),
        format!("приложение RSS={} МБ", report.rss_app_peak / MB),
    ];
    if report.vram_ok {
        parts.push(format!(
            "VRAM пик={} МБ (база {}, дельта {})",
            report.vram_used_peak / MB,
            vram_before / MB,
            report.vram_used_peak.saturating_sub(vram_before) / MB
        ));
    } else {
        parts.push("VRAM: недоступна (NVML)".to_string());
    }
    if !extra.is_empty() {
        parts.push(extra.to_string());
    }
    parts.push(format!("сэмплов: {}", report.samples));
    parts.push(format!("ожидалось ~{:.0} МБ", expected_vram_mb));
    parts.join(", ")
}

fn sample_loop(server_pid: Option<u32>, stop: Arc<AtomicBool>) -> MemReport {
    let mut sys = sysinfo::System::new();
    let app_pid = sysinfo::Pid::from(std::process::id() as usize);
    let server_pid = server_pid.map(|p| sysinfo::Pid::from(p as usize));

    let mut pids: Vec<sysinfo::Pid> = vec![app_pid];
    if let Some(sp) = server_pid {
        pids.push(sp);
    }

    let mut report = MemReport::default();
    let mut nvml_enabled = true;
    let start = Instant::now();

    while !stop.load(Ordering::SeqCst) && start.elapsed() < MAX_SAMPLE_TIME {
        sys.refresh_pids_specifics(&pids, sysinfo::ProcessRefreshKind::new().with_memory());

        if let Some(p) = sys.process(app_pid) {
            report.rss_app_peak = report.rss_app_peak.max(p.memory());
        }
        if let Some(sp) = server_pid {
            if let Some(p) = sys.process(sp) {
                report.rss_server_peak = report.rss_server_peak.max(p.memory());
            }
        }

        if nvml_enabled {
            match nvml_wrapper::Nvml::init() {
                Ok(nvml) => match nvml.device_by_index(0).and_then(|d| d.memory_info()) {
                    Ok(mem) => {
                        report.vram_ok = true;
                        report.vram_used_peak = report.vram_used_peak.max(mem.used);
                    }
                    Err(_) => nvml_enabled = false,
                },
                Err(_) => nvml_enabled = false,
            }
        }

        report.samples += 1;

        let elapsed = start.elapsed();
        if elapsed < SAMPLE_INTERVAL {
            std::thread::sleep(SAMPLE_INTERVAL - elapsed);
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> MemReport {
        MemReport {
            rss_server_peak: 5123 * MB + 100,
            rss_app_peak: 900 * MB,
            vram_used_peak: 12_840 * MB,
            vram_ok: true,
            samples: 42,
        }
    }

    #[test]
    fn peak_line_contains_all_metrics() {
        let line = peak_line("legacy:Агент#2", &report(), 6200 * MB, 12_500.0, "512 токенов за 20.1с (25 tok/s), причина: EOS");
        assert!(line.contains("llama-server RSS=5123 МБ"), "line: {}", line);
        assert!(line.contains("приложение RSS=900 МБ"), "line: {}", line);
        assert!(line.contains("VRAM пик=12840 МБ"), "line: {}", line);
        assert!(line.contains("база 6200"), "line: {}", line);
        assert!(line.contains("дельта 6640"), "line: {}", line);
        assert!(line.contains("сэмплов: 42"), "line: {}", line);
        assert!(line.contains("ожидалось ~12500 МБ"), "line: {}", line);
        assert!(line.contains("512 токенов"), "line: {}", line);
    }

    #[test]
    fn peak_line_marks_vram_unavailable() {
        let mut r = report();
        r.vram_ok = false;
        r.vram_used_peak = 0;
        let line = peak_line("graph:node1", &r, 6200 * MB, 12_500.0, "");
        assert!(line.contains("VRAM: недоступна (NVML)"), "line: {}", line);
        assert!(!line.contains("дельта"), "line: {}", line);
    }

    #[test]
    fn peak_line_saturating_delta_when_vram_dropped() {
        // VRAM упала ниже базы (другие процессы освободили память) — дельта не уходит в минус
        let mut r = report();
        r.vram_used_peak = 5000 * MB;
        let line = peak_line("legacy:X", &r, 6200 * MB, 12_500.0, "");
        assert!(line.contains("дельта 0"), "line: {}", line);
    }

    #[test]
    fn sampler_collects_app_rss_and_stops() {
        let mut s = MemSampler::start(None);
        std::thread::sleep(Duration::from_millis(450));
        let r = s.stop_and_report();
        assert!(r.samples >= 1, "samples: {}", r.samples);
        assert!(r.rss_app_peak > 0, "app rss: {}", r.rss_app_peak);
    }

    #[test]
    fn sampler_stops_without_hanging_on_drop() {
        let s = MemSampler::start(None);
        drop(s); // не должен зависнуть
    }
}
