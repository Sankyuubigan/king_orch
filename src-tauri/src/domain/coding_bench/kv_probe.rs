// KV-probe: бинарный поиск максимального контекста при KV-кэше f16 (без квантизации).
// Запускает llama-server на каждом шаге и меряет дельту занятой VRAM через NVML.
use std::path::Path;
use std::time::Duration;

use crate::infra::llm::LlamaEngine;

/// Результат пробы: максимальный контекст и занятая VRAM при нём (МБ).
#[derive(Debug, Clone, serde::Serialize)]
pub struct KvProbeResult {
    pub max_ctx: u32,
    pub vram_mb: f64,
}

const MIN_CTX: u32 = 256;
const MAX_CTX: u32 = 131_072;
const MAX_ITERATIONS: u32 = 12;

/// Текущая занятая VRAM GPU (NVML, device-level), МБ.
fn vram_used_mb() -> Option<f64> {
    let nvml = nvml_wrapper::Nvml::init().ok()?;
    let device = nvml.device_by_index(0).ok()?;
    let mem = device.memory_info().ok()?;
    Some(mem.used as f64 / (1024.0 * 1024.0))
}

/// Бинарный поиск максимального ctx, при котором модель помещается в бюджет VRAM
/// с KV-кэшем f16. ~8-12 перезапусков сервера (~10 сек каждый для 8B-модели).
pub fn probe_max_ctx_f16(
    engine_dir: &Path,
    model_path: &str,
    budget_mb: u64,
    log_cb: &dyn Fn(String),
) -> Result<KvProbeResult, String> {
    if budget_mb == 0 {
        return Err("Бюджет VRAM не задан".to_string());
    }

    let mut lo = MIN_CTX;
    let mut hi = MAX_CTX;
    let mut best_ctx = 0u32;
    let mut best_vram = 0.0f64;
    let mut iterations = 0;

    while lo <= hi && iterations < MAX_ITERATIONS {
        iterations += 1;
        let mid = lo + (hi - lo) / 2;
        // Даём VRAM осесть после предыдущего сервера.
        std::thread::sleep(Duration::from_millis(300));
        let baseline = vram_used_mb().unwrap_or(0.0);
        log_cb(format!("🔎 KV-probe: ctx={} (f16), базовая VRAM={:.0} МБ…", mid, baseline));

        let engine = LlamaEngine::new(
            engine_dir,
            model_path,
            mid,
            false, // -ctk f16
            false, // -ctv f16
            0,
            |msg: String| { let _ = msg; },
            |_chunk: String| {},
        );

        match engine {
            Ok(engine) => {
                let used = vram_used_mb().unwrap_or(0.0);
                let delta = (used - baseline).max(0.0);
                log_cb(format!(
                    "   ctx={}: VRAM после загрузки {:.0} МБ (дельта {:.0} МБ), бюджет {} МБ",
                    mid, used, delta, budget_mb
                ));
                drop(engine); // останавливает llama-server
                if delta <= budget_mb as f64 {
                    best_ctx = mid;
                    best_vram = delta;
                    lo = mid + 1;
                } else {
                    hi = mid - 1;
                }
            }
            Err(e) => {
                log_cb(format!("   ctx={}: движок не запустился ({}) — считаем слишком большим", mid, truncate_err(&e)));
                hi = mid - 1;
            }
        }
    }

    if best_ctx == 0 {
        return Err(format!("Модель не помещается даже в {} токенов контекста при бюджете {} МБ", MIN_CTX, budget_mb));
    }
    log_cb(format!("✅ KV-probe (f16): max_ctx={}, VRAM={:.0} МБ", best_ctx, best_vram));
    Ok(KvProbeResult { max_ctx: best_ctx, vram_mb: best_vram })
}

fn truncate_err(e: &str) -> String {
    let s = e.replace('\n', " ");
    if s.chars().count() > 120 {
        s.chars().take(120).collect::<String>() + "…"
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_long_error() {
        let long = "x".repeat(500);
        assert_eq!(truncate_err(&long).chars().count(), 121);
        assert!(truncate_err(&long).ends_with('…'));
        assert_eq!(truncate_err("короткая"), "короткая");
    }

    #[test]
    fn probe_rejects_zero_budget() {
        let dir = std::path::PathBuf::from(".");
        let err = probe_max_ctx_f16(&dir, "model.gguf", 0, &|_| {});
        assert!(err.is_err());
    }
}