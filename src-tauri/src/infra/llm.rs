#![allow(deprecated)]

//! LlamaEngine — управление движком llama.cpp как ОТДЕЛЬНЫМ ПРОЦЕССОМ (llama-server).
//!
//! Новая архитектура (как в Jan): приложение не линкует llama.cpp (нет PE-импортов,
//! нет DLL рядом с exe). При каждом запросе движок запускается как subprocess,
//! общение — по HTTP (localhost):
//!   - GET  /health   — проверка готовности
//!   - POST /completion (stream) — генерация (текст и мультимодалка через multimodal_data)
//!   - POST /tokenize — точный подсчёт токенов
//! Процесс гарантированно убивается при Drop (drop(self.child) в конце запроса).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::header::AUTHORIZATION;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::infra::config::ModelParams;
use crate::infra::detokenizer::compute_stream_diff;

pub use super::llm_types::{ChatMessage, ChatAttachment, SubCall, ToolCallInfo, PromptFormat, push_report, LlmMessage, extract_model_filename, GenerationResult, LlmMetrics, llm_history, GrammarSpec, build_base_grammar, build_json_only_grammar, build_json_object_grammar_with_keys};
pub use super::llm_gguf::{extract_string_from_gguf, extract_f32_from_gguf, extract_u32_from_gguf};

/// Таймаут ожидания готовности движка (загрузка модели в память)
const HEALTH_TIMEOUT: Duration = Duration::from_secs(120);
/// Таймаут чтения потока генерации: если за это время от сервера не пришло
/// НИ одного байта — движок завис (первый токен не пришёл / оборвался стрим).
/// Работает как таймаут «первого токена», но НЕ обрезает длинную генерацию:
/// при живом стриме данные идут чаще.
const READ_TIMEOUT: Duration = Duration::from_secs(120);
/// Диапазон портов для локального сервера
const PORT_MIN: u16 = 17800;
const PORT_RANGE: u16 = 1500;

/// Базовые стоп-слова, одинаковые для всех моделей
const DEFAULT_STOP_WORDS: &[&str] = &[
    "<|im_end|>", "<end_of_turn>", "</s>", "<|eot_id|>",
    "<turn>", "<|eot|>", "User:", "System:", "<eos>",
    "<turn|>", "/end_of_turn>", "<step>", "<|end_of_text|>", "<｜end of sentence｜>",
    "</start_of_turn>", "<|channel|>",
];

/// Прогноз потребления VRAM для заданного размера контекста:
/// размер файла модели + KV-кэш (зависит от архитектуры и n_ctx).
pub fn estimate_vram_mb(model_path: &str, ctx_size: u32, kv_quant_keys: bool, kv_quant_values: bool) -> f64 {
    let model_size_mb = std::fs::metadata(model_path).map(|m| m.len() as f64 / (1024.0 * 1024.0)).unwrap_or(0.0);

    let layers = extract_u32_from_gguf(model_path, "llama.block_count").unwrap_or(32);
    let heads = extract_u32_from_gguf(model_path, "llama.attention.head_count").unwrap_or(32);
    let heads_kv = extract_u32_from_gguf(model_path, "llama.attention.head_count_kv").unwrap_or(heads);
    let embd = extract_u32_from_gguf(model_path, "llama.embedding_length").unwrap_or(4096);
    let head_dim = embd / heads.max(1);

    let b_k = if kv_quant_keys { 1.06 } else { 2.0 };
    let b_v = if kv_quant_values { 1.06 } else { 2.0 };

    let kv_bytes = (layers as f64 * head_dim as f64 * ctx_size as f64) * (heads as f64 * b_k + heads_kv as f64 * b_v);
    let kv_mb = kv_bytes / (1024.0 * 1024.0);

    model_size_mb + kv_mb
}

pub struct LlamaEngine {
    pub global_ctx_limit: u32,
    pub model_path: String,
    pub mmproj_path: Option<String>,
    /// true = движок запущен с mmproj (может принимать изображения)
    pub is_multimodal_engine: bool,
    stream_cb: Arc<dyn Fn(String) + Send + Sync>,
    client: Client,
    server_log: PathBuf,
    port: u16,
    api_key: String,
    child: Option<Child>,
    /// Режим работы: "gpu" (модель реально загружена в VRAM) или "cpu"
    engine_mode: String,
    /// Причина CPU-режима (пустая строка, если режим GPU)
    engine_mode_detail: String,
    /// Занятая VRAM (байты, device-level) ДО старта движка — база для дельты пиков
    vram_before: u64,
    /// Скорость последней генерации (tok/s)
    last_tok_per_sec: std::cell::Cell<f64>,
    /// Грамматика для СЛЕДУЮЩЕГО вызова generate_chat (consume-and-clear).
    /// Per-node/per-agent грамматика задаётся через set_grammar() перед вызовом;
    /// если не задана — generate_chat подставляет базовую (текст|JSON).
    pending_grammar: std::sync::Mutex<Option<GrammarSpec>>,
}

// ─── Простой ГПСЧ для порта/ключа (без внешних зависимостей) ───
struct Prng(u64);
impl Prng {
    fn new() -> Self {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let pid = std::process::id() as u64;
        let mut seed = t ^ (pid << 32) ^ (pid.rotate_left(17));
        if seed == 0 { seed = 0x9E3779B97F4A7C15; }
        Prng(seed)
    }
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

impl LlamaEngine {
    pub fn new<L, S>(engine_dir: &Path, model_path: &str, global_ctx_limit: u32, kv_quant_keys: bool, kv_quant_values: bool, reasoning_budget: u32, log_cb: L, stream_cb: S) -> Result<Self, String>
    where L: Fn(String) + Send + Sync + 'static, S: Fn(String) + Send + Sync + 'static
    {
        Self::new_with_mmproj(engine_dir, model_path, None, global_ctx_limit, kv_quant_keys, kv_quant_values, reasoning_budget, log_cb, stream_cb)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_mmproj<L, S>(engine_dir: &Path, model_path: &str, mmproj_path: Option<&str>, global_ctx_limit: u32, kv_quant_keys: bool, kv_quant_values: bool, reasoning_budget: u32, log_cb: L, stream_cb: S) -> Result<Self, String>
    where L: Fn(String) + Send + Sync + 'static, S: Fn(String) + Send + Sync + 'static
    {
        let log_cb: std::sync::Arc<dyn Fn(String) + Send + Sync> = std::sync::Arc::new(log_cb);
        log_cb("⚡ Запуск движка llama.cpp (llama-server)...".to_string());

        // ── Логирование аппаратного обеспечения ──
        log_cb("🖥️ Аппаратное обеспечение:".to_string());
        let sys = sysinfo::System::new_all();
        if let Some(cpu) = sys.cpus().first() {
            let total_ram_mb = sys.total_memory() / 1024 / 1024;
            log_cb(format!("   CPU: {} | RAM: {} MB", cpu.brand(), total_ram_mb));
        }

        let gpu_info = crate::infra::gpu_detector::detect_gpu();
        log_cb(format!("   GPU: {}", crate::infra::gpu_detector::describe_gpu(&gpu_info)));

        let vram_before = match nvml_wrapper::Nvml::init() {
            Ok(nvml) => {
                match nvml.device_by_index(0) {
                    Ok(device) => {
                        match device.memory_info() {
                            Ok(mem) => {
                                let total_vram_mb = mem.total / 1024 / 1024;
                                let used_vram_mb = mem.used / 1024 / 1024;
                                let free_vram_mb = total_vram_mb - used_vram_mb;
                                let name = device.name().unwrap_or_else(|_| "NVIDIA GPU".to_string());
                                log_cb(format!("   GPU: {} | VRAM: {} MB (Свободно: {} MB)", name, total_vram_mb, free_vram_mb));
                                mem.used
                            },
                            Err(e) => { log_cb(format!("   GPU: NVML memory_info error: {}", e)); 0 }
                        }
                    },
                    Err(e) => { log_cb(format!("   GPU: NVML device error: {}", e)); 0 }
                }
            },
            Err(e) => {
                log_cb(format!("   GPU: NVIDIA драйверы не найдены (NVML ошибка: {}).", e));
                0
            }
        };

        let model_size_mb = std::fs::metadata(model_path).map(|m| m.len() as f64 / (1024.0 * 1024.0)).unwrap_or(0.0);
        log_cb(format!("💽 Файл модели: ~{:.1} МБ.", model_size_mb));

        // ── Единая точка выхода с ошибкой: пишем в лог-файл + телеметрию ──
        let fail = |msg: String| -> Result<Self, String> {
            crate::infra::startup_log::append("ERROR", &format!("LlamaEngine::new: {}", msg));
            Err(msg)
        };

        // ── Проверка целостности GGUF-файла модели ──
        // Битые/криво сконвертированные файлы (напр. block_count объявлен
        // больше, чем реально есть тензоров blk.N) роняют llama-server с
        // непонятным хвостом лога. Проверяем заранее и даём понятную ошибку.
        if let Err(msg) = crate::infra::llm_gguf::validate_gguf(model_path) {
            return fail(format!("Файл модели повреждён.\n{}", msg));
        }

        // ── Проверка установки движка ──
        // Бекенд выбирается юзером в настройках (engine_variant в app_config.json,
        // "auto" → подбор по GPU). Каждый вариант живёт в backends/<variant>/.
        let cfg_early = crate::infra::config::load_config_early();
        let pref = cfg_early.engine_variant.as_deref();
        let selected_variant = crate::infra::llamacpp_installer::resolve_variant(pref);
        let installed_family = crate::infra::llamacpp_installer::EngineFamily::from_variant(&selected_variant);

        let mut server_exe = crate::infra::llamacpp_installer::variant_dir(engine_dir, &selected_variant).join("llama-server.exe");
        if !server_exe.exists() {
            // Legacy-фолбэк: бинарь в корне папки движка (старый формат до миграции)
            server_exe = engine_dir.join("llama-server.exe");
        }
        if !server_exe.exists() {
            return fail(format!(
                "Движок llama.cpp не установлен (llama-server.exe не найден для варианта «{}» в {}).\nОткройте Настройки → «Движок запуска нейромоделей» и установите движок.",
                crate::infra::llamacpp_installer::variant_label(&selected_variant),
                engine_dir.display()
            ));
        }

        // ── Предлётная проверка CUDA-рантайма ──
        // В релизах llama.cpp b10275+ cublas64_*.dll вынесены из архива движка
        // в отдельный архив cudart-llama-bin. Без них ggml-cuda.dll не грузится
        // и движок ТИХО уходит в CPU — проверяем заранее и говорим явно.
        let cuda_runtime_dll = match installed_family {
            crate::infra::llamacpp_installer::EngineFamily::Cuda13 => Some("cublas64_13.dll"),
            crate::infra::llamacpp_installer::EngineFamily::Cuda12 => Some("cublas64_12.dll"),
            _ => None,
        };
        if let Some(dll_name) = cuda_runtime_dll {
            let server_dir = std::path::Path::new(&server_exe)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| engine_dir.to_path_buf());
            let nearby = server_dir.join(dll_name);
            let in_system = std::path::Path::new(r"C:\Windows\System32").join(dll_name);
            if !nearby.exists() && !in_system.exists() {
                return fail(format!(
                    "CUDA-библиотека {} не найдена рядом с llama-server.exe (в {}) и в System32.\n\
                     Без неё GPU-режим не работает — движок тихо уходит в CPU.\n\
                     Решение: Настройки → «Движок запуска нейромоделей» → переустановите движок (установщик докачает CUDA-рантайм автоматически).",
                    dll_name, server_dir.display()
                ));
            }
        }

        // ── Совместимость выбранного бекенда с GPU ──
        // Раньше несовместимость (cuda-12.4 на RTX 50xx) была жёсткой ошибкой.
        // Теперь выбор бекенда — осознанное решение юзера: предупреждаем, но
        // запускаем (llama-server сам упадёт в CPU, а VRAM-проверка ниже это
        // поймает и объяснит причину).
        let required_gen = crate::infra::gpu_detector::required_cuda_gen(&gpu_info);
        let required_label = required_gen.map(|g| g.label().to_string()).unwrap_or_else(|| "cpu".to_string());
        if let Some(gen) = required_gen {
            let required_family = match gen {
                crate::infra::gpu_detector::CudaGen::Cuda13 => crate::infra::llamacpp_installer::EngineFamily::Cuda13,
                crate::infra::gpu_detector::CudaGen::Cuda12 => crate::infra::llamacpp_installer::EngineFamily::Cuda12,
            };
            if required_family == crate::infra::llamacpp_installer::EngineFamily::Cuda13
                && installed_family == crate::infra::llamacpp_installer::EngineFamily::Cuda12
                && gpu_info.compute_major >= 12
            {
                // Жёсткая несовместимость только для Blackwell: сборка cuda-12.x
                // не содержит ядер sm_120. Для 40xx cuda-13.x — лишь предпочтение
                // свежего драйвера, cuda-12.x при этом работает нормально.
                log_cb(format!(
                    "⚠️ Ваша видеокарта {} (Blackwell, compute {}.{}) — бекенд {} не содержит ядер Blackwell. Модель будет работать только на CPU.\n\
                     Решение: Настройки → «Движок запуска нейромоделей» → выберите «{}».",
                    gpu_info.gpu_name,
                    gpu_info.compute_major,
                    gpu_info.compute_minor,
                    crate::infra::llamacpp_installer::variant_label(&selected_variant),
                    crate::infra::llamacpp_installer::variant_label(crate::infra::llamacpp_installer::VARIANT_CUDA13)
                ));
            }
            if installed_family == crate::infra::llamacpp_installer::EngineFamily::Cpu {
                log_cb(format!(
                    "ℹ️ Выбран CPU-бекенд, хотя на компьютере есть NVIDIA GPU ({}). GPU-ускорение не будет использоваться. Выбрать CUDA можно в Настройках → «Движок запуска нейромоделей».",
                    gpu_info.gpu_name
                ));
            }
        }

        let use_gpu = installed_family.is_gpu();
        let gpu_layers: u32 = if use_gpu { 999 } else { 0 };
        let engine_mode = if use_gpu { "gpu".to_string() } else { "cpu".to_string() };
        log_cb(format!(
            "⚙️ Бекенд: {} ({}; авто-подбор для этой машины: {}) | GPU-слои: {} ({})",
            crate::infra::llamacpp_installer::variant_label(&selected_variant),
            selected_variant,
            required_label,
            gpu_layers,
            if use_gpu { "оффлоуд на GPU" } else { "CPU-режим" }
        ));

        let logical_cores = std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(8);
        let threads = (logical_cores / 2).max(4);

        // ── Случайные порт и ключ ──
        let mut rng = Prng::new();
        let mut port = 0u16;
        for _ in 0..5 {
            let candidate = PORT_MIN + (rng.next() % PORT_RANGE as u64) as u16;
            if std::net::TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
                port = candidate;
                break;
            }
        }
        if port == 0 {
            return fail("Не удалось найти свободный порт для движка llama.cpp".to_string());
        }
        let api_key: String = (0..32).map(|_| {
            const HEX: &[u8] = b"0123456789abcdef";
            HEX[(rng.next() % 16) as usize] as char
        }).collect();

        let server_log = engine_dir.join("llama_server.log");
        let _ = std::fs::remove_file(&server_log);

        let use_reasoning_default = reasoning_budget > 0;
        // mmproj передаётся в llama-server ТОЛЬКО когда он реально нужен
        // (запрос с изображениями) и файл проектора валиден. Если проектор
        // битый/несовместимый — движок не должен ронять весь чат: делаем
        // фолбэк в текстовый режим без mmproj (см. цикл запуска ниже).
        let mmproj_orig: Option<String> = mmproj_path.map(|s| s.to_string());
        let mut attempt_mmproj = mmproj_orig.is_some();
        let mut attempt_reasoning = use_reasoning_default;
        let auth_value = format!("Bearer {}", api_key);
        let build_cmd = {
            let server_exe = server_exe.clone();
            let server_log = server_log.clone();
            let api_key = api_key.clone();
            move |use_reasoning: bool, use_mmproj: bool| {
                let mut c = Command::new(&server_exe);
                c.current_dir(engine_dir)
                    .arg("-m").arg(model_path)
                    .arg("--host").arg("127.0.0.1")
                    .arg("--port").arg(port.to_string())
                    .arg("--api-key").arg(&api_key)
                    .arg("--ctx-size").arg(global_ctx_limit.to_string())
                .arg("-t").arg(threads.to_string())
                .arg("-ngl").arg(gpu_layers.to_string())
                .arg("--flash-attn").arg("on")
                .arg("--no-webui")
                .arg("--log-file").arg(&server_log);
                if use_mmproj {
                    if let Some(mmp) = mmproj_path {
                        c.arg("--mmproj").arg(mmp);
                    }
                }
            if kv_quant_keys {
                c.arg("--cache-type-k").arg("q8_0");
            }
            if kv_quant_values {
                c.arg("--cache-type-v").arg("q8_0");
            }
            if use_reasoning {
                // Думатель выносится в отдельное поле reasoning_content, ограничен
                // бюджетом и НЕ сохраняется в истории слота. Если старая версия
                // движка не знает этих флагов — fallback-перезапуск без них (ниже).
                c.arg("--reasoning-format").arg("deepseek");
                c.arg("--reasoning-budget").arg(reasoning_budget.to_string());
                c.arg("--no-reasoning-preserve");
            }
            #[cfg(target_os = "windows")]
            { use std::os::windows::process::CommandExt; c.creation_flags(0x08000000); }
            // CUDA/ggml-ошибки идут в stderr (не в --log-file) — захватываем их
            c.stderr(std::process::Stdio::piped());
            c
            }
        };

        log_cb(format!(
            "🚀 Запуск llama-server: порт {}, контекст {} токенов, потоков {}, mmproj={}, думатель={}",
            port,
            global_ctx_limit,
            threads,
            if attempt_mmproj { mmproj_orig.as_deref().unwrap_or("да") } else { "нет" },
            if use_reasoning_default { format!("до {} токенов (deepseek-формат)", reasoning_budget) } else { "выключен".to_string() }
        ));

        let client = match Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => return fail(format!("Ошибка создания HTTP-клиента: {}", e)),
        };

        // Поток чтения stderr llama-server: ggml_cuda_init, CUDA-ошибки, варнинги
        let spawn_stderr_reader = {
            let log_cb = log_cb.clone();
            move |stderr: std::process::ChildStderr| {
                let log_cb = log_cb.clone();
                std::thread::spawn(move || {
                    use std::io::BufRead;
                    let reader = std::io::BufReader::new(stderr);
                    for line in reader.lines() {
                        let line = match line {
                            Ok(l) => l,
                            Err(_) => break,
                        };
                        let line = line.trim();
                        if !line.is_empty() {
                            log_cb(format!("[llama-server] {}", line));
                        }
                    }
                });
            }
        };

        // ── Цикл запуска с фолбэками ──
        // 1) reasoning-флаги: старая сборка движка не знает --reasoning-* →
        //    рестарт без них (уже было).
        // 2) mmproj: проектор повреждён/несовместим → рестарт БЕЗ mmproj
        //    (текстовый режим), вместо падения всего чата.
        // 3) VL-модель требует проектор, а мы его не передали → рестарт С ним.
        let mut engine_child: Option<std::process::Child> = None;
        let mut tried_no_reasoning = false;
        let mut tried_no_mmproj = false;
        let mut tried_with_mmproj = false;

        'launch: loop {
            let mut c = build_cmd(attempt_reasoning, attempt_mmproj);
            let mut child = match c.spawn() {
                Ok(c) => c,
                Err(e) => return fail(format!("Ошибка запуска llama-server: {}", e)),
            };
            // ── Гарантия зачистки при выходе из приложения ──
            // Регистрируем PID (для докиля в обработчике выхода main.rs) и назначаем
            // процесс в Windows Job Object с KILL_ON_JOB_CLOSE — тогда ОС убьёт
            // сервер даже при насильственном закрытии приложения.
            crate::infra::process_util::register_engine_pid(child.id());
            #[cfg(windows)]
            crate::infra::process_util::assign_child_to_kill_job(&child);
            if let Some(stderr) = child.stderr.take() {
                spawn_stderr_reader(stderr);
            }

            let mut deadline = Instant::now() + HEALTH_TIMEOUT;
            let mut last_progress_log = Instant::now();
            let mut exit_code: Option<i32> = None;
            let mut healthy = false;
            loop {
                match child.try_wait() {
                    Ok(state) => {
                        if let Some(status) = state {
                            exit_code = Some(status.code().unwrap_or(-1));
                            break;
                        }
                    }
                    Err(e) => return fail(format!("Ошибка ожидания llama-server: {}", e)),
                }
                if engine_health_check(&client, port, &auth_value) {
                    healthy = true;
                    break;
                }
                if Instant::now() > deadline {
                    let _ = child.kill();
                    return fail(format!(
                        "Таймаут запуска движка llama.cpp ({} сек). Модель не загрузилась. {}",
                        HEALTH_TIMEOUT.as_secs(),
                        read_log_tail(&server_log)
                    ));
                }
                if last_progress_log.elapsed() >= Duration::from_secs(5) {
                    last_progress_log = Instant::now();
                    log_cb(format!(
                        "⏳ Загрузка модели в память... ({} сек)",
                        deadline.duration_since(Instant::now()).as_secs().min(HEALTH_TIMEOUT.as_secs())
                    ));
                }
                std::thread::sleep(Duration::from_millis(500));
            }

            if healthy {
                engine_child = Some(child);
                break 'launch;
            }

            // ── Сервер завершился с ошибкой при старте ──
            let code = exit_code.unwrap_or(-1);
            let log = read_log_tail(&server_log);

            // Фолбэк 1: движок не понимает флаги reasoning (старая сборка)
            if attempt_reasoning && !tried_no_reasoning && !is_mmproj_load_error(&log) {
                log_cb(format!(
                    "⚠️ llama-server завершился при запуске (код {}) — перезапуск БЕЗ reasoning-флагов (движок их не поддерживает). Думатель будет отключён.",
                    code
                ));
                attempt_reasoning = false;
                tried_no_reasoning = true;
                crate::infra::process_util::unregister_engine_pid(child.id());
                continue 'launch;
            }

            // Фолбэк 2: битый/несовместимый проектор mmproj → текстовый режим
            if attempt_mmproj && is_mmproj_load_error(&log) && !tried_no_mmproj {
                log_cb(format!(
                    "⚠️ Проектор mmproj '{}' не загрузился (вероятно повреждён или несовместим с этой сборкой llama.cpp): {}. Запуск в текстовом режиме без vision.",
                    mmproj_orig.as_deref().unwrap_or(""),
                    mmproj_error_detail(&log)
                ));
                attempt_mmproj = false;
                tried_no_mmproj = true;
                crate::infra::process_util::unregister_engine_pid(child.id());
                continue 'launch;
            }

            // Фолбэк 3: VL-модель требует проектор, а мы его не передали
            if !attempt_mmproj && mmproj_orig.is_some() && !tried_with_mmproj && is_mmproj_required_error(&log) {
                log_cb("ℹ️ Модель мультимодальная и требует проектор — подключаем mmproj.".to_string());
                attempt_mmproj = true;
                tried_with_mmproj = true;
                crate::infra::process_util::unregister_engine_pid(child.id());
                continue 'launch;
            }

            // ── Финальная ошибка ──
            let msg = if is_mmproj_load_error(&log) || is_mmproj_required_error(&log) {
                let path = mmproj_orig.as_deref().unwrap_or("<не указан>");
                format!(
                    "Ошибка загрузки мультимодального проектора (mmproj) для модели.\nФайл: {}\nПричина: {}\n\nВероятно файл повреждён или собран для несовместимой версии llama.cpp. Скачайте корректный mmproj, соответствующий вашей сборке движка (например cuda-13.x).",
                    path,
                    mmproj_error_detail(&log)
                )
            } else {
                format!(
                    "Движок llama-server завершился при запуске (код {}). {}",
                    code,
                    log
                )
            };
            return fail(msg);
        }

        let child = engine_child.expect("engine child after successful launch");

        let mut engine = Self {
            global_ctx_limit,
            model_path: model_path.to_string(),
            mmproj_path: if attempt_mmproj { mmproj_orig.clone() } else { None },
            is_multimodal_engine: attempt_mmproj,
            stream_cb: Arc::new(stream_cb),
            client,
            server_log,
            port,
            api_key,
            child: Some(child),
            engine_mode: engine_mode.clone(),
            engine_mode_detail: String::new(),
            vram_before,
            last_tok_per_sec: std::cell::Cell::new(0.0),
            pending_grammar: std::sync::Mutex::new(None),
        };

        log_cb(format!(
            "✅ Движок llama-server запущен: {} (режим {}), порт {}",
            engine.model_path, engine.engine_mode, engine.port
        ));

        // ── Проверка: реально ли модель ушла в VRAM? ──
        if vram_before > 0 {
            let vram_after = nvml_wrapper::Nvml::init().ok().and_then(|nvml| {
                nvml.device_by_index(0)
                    .ok()
                    .and_then(|device| device.memory_info().ok().map(|mem| mem.used))
            }).unwrap_or(0);

            let diff = vram_after as i64 - vram_before as i64;
            if diff > 100_000_000 { // > 100 MB
                engine.engine_mode = "gpu".to_string();
                log_cb(format!("✅ GPU: Модель загружена в VRAM. Занято {} МБ видеопамяти.", diff / 1024 / 1024));
            } else if use_gpu {
                // Намерение было GPU, но VRAM не выросла — llama-server тихо ушёл в CPU.
                // Сообщаем ПРИЧИНУ (из лога сервера), а не выдуманный диагноз.
                engine.engine_mode = "cpu".to_string();
                let diag = diagnose_cuda_fallback(&engine.server_log);
                engine.engine_mode_detail = if !diag.is_empty() {
                    format!("Модель не попала в VRAM: {}", diag)
                } else {
                    format!(
                        "Модель не попала в VRAM: бекенд ({}) не смог использовать GPU {} (драйвер CUDA {}.{}, compute {}.{}). Подробности — в хвосте llama_server.log.",
                        selected_variant, gpu_info.gpu_name, gpu_info.cuda_major, gpu_info.cuda_minor,
                        gpu_info.compute_major, gpu_info.compute_minor
                    )
                };
                log_cb("❌ ВНИМАНИЕ: VRAM не увеличилась! Модель работает на CPU, а не на GPU!".to_string());
                log_cb(format!("❌ Диагноз: {}", engine.engine_mode_detail));
                log_cb("❌ Решение: Настройки → «Движок запуска нейромоделей» → выберите подходящий бекенд.".to_string());
            } else {
                log_cb("ℹ️ GPU-ускорение не используется (CPU-режим) — VRAM не занята. Это ожидаемо.".to_string());
            }
        }

        // ── Предупреждение о нехватке памяти (не блокирует запуск) ──
        // Оценка (модель + KV-кэш) сравнивается со свободной RAM. На CPU-режиме
        // модель живёт в RAM (вместе с KV), на GPU — файл всё равно мапится.
        let need_mb = estimate_vram_mb(model_path, global_ctx_limit, kv_quant_keys, kv_quant_values) + 512.0;
        let free_ram_mb = sys.free_memory() as f64 / (1024.0 * 1024.0);
        if need_mb > free_ram_mb {
            let warn = format!(
                "⚠️ ВАЖНО: модели с контекстом нужно ~{} МБ памяти, свободно всего {} МБ. \
                 Загрузка может упасть или работа будет очень медленной. Уменьшите размер контекста в настройках.",
                need_mb as i64, free_ram_mb as i64
            );
            log_cb(warn.clone());
            crate::infra::startup_log::append("WARN", &warn);
        }

        let mut gguf_params = Vec::new();
        if let Some(v) = extract_f32_from_gguf(model_path, "tokenizer.ggml.temp") { gguf_params.push(format!("Temp={:.2}", v)); }
        if let Some(v) = extract_u32_from_gguf(model_path, "tokenizer.ggml.top_k") { gguf_params.push(format!("Top_K={}", v)); }
        if let Some(v) = extract_f32_from_gguf(model_path, "tokenizer.ggml.top_p") { gguf_params.push(format!("Top_P={:.2}", v)); }
        if let Some(v) = extract_f32_from_gguf(model_path, "tokenizer.ggml.min_p") { gguf_params.push(format!("Min_P={:.2}", v)); }
        if let Some(v) = extract_f32_from_gguf(model_path, "tokenizer.ggml.repetition_penalty") { gguf_params.push(format!("Rep_Pen={:.2}", v)); }

        if !gguf_params.is_empty() {
            log_cb(format!("📦 Вшитые параметры GGUF: {}", gguf_params.join(", ")));
        } else {
            log_cb("📦 Вшитые параметры GGUF: отсутствуют".to_string());
        }

        if let Some(mmp) = &engine.mmproj_path {
            log_cb(format!("✅ mmproj передан движку: {} (изображения поддерживаются)", mmp));
        }

        Ok(engine)
    }

    /// Режим движка: "gpu" (модель в VRAM) или "cpu" (fallback)
    pub fn engine_mode(&self) -> &str {
        &self.engine_mode
    }

    /// Причина CPU-режима (пусто, если GPU)
    pub fn engine_mode_detail(&self) -> &str {
        &self.engine_mode_detail
    }

    /// Скорость последней генерации (tok/s), 0 до первой генерации
    pub fn tok_per_sec(&self) -> f64 {
        self.last_tok_per_sec.get()
    }

    pub fn build_prompt(&self, messages: &[LlmMessage], format_type: &str, log_cb: &impl Fn(String)) -> (String, PromptFormat) {
        let pf = PromptFormat::from_str(format_type);
        let actual_format = if pf == PromptFormat::Auto {
            PromptFormat::detect_from_gguf(&self.model_path)
        } else {
            pf.clone()
        };

        let mut full_prompt = String::new();
        if pf == PromptFormat::Auto {
            if let Some(template) = extract_string_from_gguf(&self.model_path, "tokenizer.chat_template") {
                if let Some(rendered) = PromptFormat::format_messages_jinja(&template, messages) {
                    full_prompt = rendered;
                    log_cb("✨ Использован Jinja шаблон из GGUF".to_string());
                } else {
                    log_cb("⚠️ Не удалось применить Jinja шаблон, используется фолбэк ручной склейки.".to_string());
                }
            }
        }

        if full_prompt.is_empty() {
             full_prompt = actual_format.format_messages(messages);
        }

        (full_prompt, actual_format)
    }

    pub fn get_tokens_count(&self, messages: &[LlmMessage], format_type: &str) -> Result<usize, String> {
        let (full_prompt, _) = self.build_prompt(messages, format_type, &|_|{});
        let tokens = self.tokenize(&full_prompt)?;
        Ok(tokens.len())
    }

    // ─── HTTP-примитивы ───

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    fn is_healthy(&self) -> bool {
        let resp = self.client
            .get(self.url("/health"))
            .header(AUTHORIZATION, self.auth_header())
            .timeout(Duration::from_secs(3))
            .send();
        match resp {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }

    fn post_json<T: Serialize, R: DeserializeOwned>(&self, path: &str, body: &T) -> Result<R, String> {
        let resp = self.client
            .post(self.url(path))
            .header(AUTHORIZATION, self.auth_header())
            .json(body)
            .send()
            .map_err(|e| format!("Ошибка HTTP {}: {}", path, e))?;
        let status = resp.status();
        let text = resp.text().map_err(|e| format!("Ошибка чтения ответа {}: {}", path, e))?;
        if !status.is_success() {
            return Err(format!("llama-server: HTTP {} на {}: {}", status, path, truncate_str(&text, 300)));
        }
        serde_json::from_str(&text).map_err(|e| format!("Ошибка парсинга ответа {}: {}", path, e))
    }

    pub(crate) fn tokenize(&self, text: &str) -> Result<Vec<i64>, String> {
        #[derive(Serialize)]
        struct TokenizeReq<'a> {
            content: &'a str,
            add_special: bool,
            parse_special: bool,
        }
        #[derive(Deserialize)]
        struct TokenizeResp {
            tokens: Vec<i64>,
        }
        let resp: TokenizeResp = self.post_json("/tokenize", &TokenizeReq {
            content: text,
            add_special: true,
            parse_special: true,
        })?;
        Ok(resp.tokens)
    }

    // ─── Диагностика ошибок генерации ───

    /// Полный текст ошибки генерации: причина + пики памяти + хвост лога
    /// llama-server. Пики берутся из `MemReport` (см. err path).
    fn err_details(&self, report: &crate::infra::mem_profiler::MemReport, reason: &str) -> String {
        let mem = if report.samples > 0 {
            format!(
                " | пик памяти: llama-server RSS={} МБ, приложение RSS={} МБ, VRAM={} МБ",
                report.rss_server_peak / (1024 * 1024),
                report.rss_app_peak / (1024 * 1024),
                report.vram_used_peak / (1024 * 1024),
            )
        } else {
            String::new()
        };
        format!("{}{}.{}", reason, mem, read_log_tail(&self.server_log))
    }

    /// Ошибка генерации гарантированно уходит в лог-файл и телеметрию.
    fn report_generation_error(&self, ctx_label: &str, report: &crate::infra::mem_profiler::MemReport, message: &str) {
        crate::infra::startup_log::append("ERROR", &format!("LLM генерация [{}]: {}", ctx_label, message));
        crate::infra::telemetry::track_event(
            "llm_error",
            json!({
                "ctx": ctx_label,
                "model": extract_model_filename(&self.model_path),
                "mode": self.engine_mode,
                "samples": report.samples,
                "error": message,
            }),
        );
    }

    // ─── Генерация ───

    /// Общий цикл генерации через OpenAI-совместимый `/v1/chat/completions`.
    /// Промпт рендерит САМ движок (llama.cpp: jinja-интерпретатор + GGUF
    /// chat_template) — клиент передаёт только `messages[]`. Вложения
    /// (`attachments`) превращаются в image-части последнего user-сообщения.
    /// `ctx_label` — контекст вызова для лога пиков памяти ("legacy:агент#N" / "graph:узел").
    pub(crate) fn run_chat_completions<F, L>(
        &self,
        messages: &[LlmMessage],
        attachments: Option<&[ChatAttachment]>,
        max_tokens: usize,
        params: &ModelParams,
        stop_words: &[String],
        grammar: Option<GrammarSpec>,
        disable_reasoning: bool,
        cancel_flag: Arc<AtomicBool>,
        ctx_label: &str,
        mut progress_cb: F,
        log_cb: L,
    ) -> Result<GenerationResult, String>
    where F: FnMut(f32, &str), L: Fn(String) {
        if let Some(g) = &grammar {
            if let Some(gbnf) = &g.gbnf {
                log_cb(format!("🎯 Грамматика GBNF: {} символов, корень: {}", gbnf.len(), gbnf.lines().next().unwrap_or("?")));
            } else if g.json_schema.is_some() {
                log_cb("🎯 Грамматика: json_schema (конвертируется движком)".to_string());
            }
        }
        let actual_min_p = params.min_p.max(0.0);
        let actual_rep_pen = params.repetition_penalty.max(1.0);
        let actual_temp = params.temperature.max(0.01);

        log_cb(format!(
            "🎛 Фактические параметры сэмплинга: Temp={:.2}, Top_K={}, Top_P={:.2}, Min_P={:.2}, Rep_Pen={:.2}, Pres_Pen={:.2}, DRY={:.2}/{:.2}/{}, XTC={:.2}/{:.2}",
            actual_temp, params.top_k, params.top_p, actual_min_p, actual_rep_pen, params.presence_penalty,
            params.dry_multiplier, params.dry_base, params.dry_penalty_last_n,
            params.xtc_probability, params.xtc_threshold
        ));

        #[derive(Serialize)]
        struct ChatCompletionsRequest<'a> {
            messages: Vec<serde_json::Value>,
            n_predict: usize,
            stream: bool,
            temperature: f32,
            top_k: u32,
            top_p: f32,
            min_p: f32,
            repeat_penalty: f32,
            presence_penalty: f32,
            frequency_penalty: f32,
            dry_multiplier: f32,
            dry_base: f32,
            dry_allowed_length: i32,
            dry_penalty_last_n: i32,
            xtc_probability: f32,
            xtc_threshold: f32,
            stop: &'a [String],
            cache_prompt: bool,
            seed: i32,
            #[serde(skip_serializing_if = "Option::is_none")]
            grammar: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            json_schema: Option<&'a serde_json::Value>,
            #[serde(skip_serializing_if = "Option::is_none")]
            chat_template_kwargs: Option<serde_json::Value>,
        }

        // messages[] — строки; вложения — image-части последнего user-сообщения
        // (OAI-совместимый формат content parts; рендер на стороне движка).
        let mut messages_val: Vec<serde_json::Value> = Vec::with_capacity(messages.len());
        let last_user_idx = messages.iter().rposition(|m| m.role == "user");
        for (i, m) in messages.iter().enumerate() {
            let attach_here = attachments.is_some_and(|a| !a.is_empty()) && last_user_idx == Some(i);
            if !attach_here {
                messages_val.push(json!({"role": m.role, "content": m.content}));
                continue;
            }
            let mut parts = vec![json!({"type": "text", "text": m.content})];
            for a in attachments.unwrap() {
                parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:{};base64,{}", a.mime_type, a.data_base64) }
                }));
            }
            messages_val.push(json!({"role": m.role, "content": parts}));
        }
        // Вложения есть, но последний user отсутствует — добавляем отдельное сообщение.
        if let Some(atts) = attachments {
            if !atts.is_empty() && last_user_idx.is_none() {
                let mut parts = Vec::with_capacity(atts.len());
                for a in atts {
                    parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:{};base64,{}", a.mime_type, a.data_base64) }
                    }));
                }
                messages_val.push(json!({"role": "user", "content": parts}));
            }
        }

        let request = ChatCompletionsRequest {
            messages: messages_val,
            n_predict: max_tokens,
            stream: true,
            temperature: actual_temp,
            top_k: params.top_k,
            top_p: params.top_p,
            min_p: actual_min_p,
            repeat_penalty: actual_rep_pen,
            presence_penalty: params.presence_penalty,
            frequency_penalty: 0.0,
            dry_multiplier: params.dry_multiplier,
            dry_base: params.dry_base,
            dry_allowed_length: params.dry_allowed_length,
            dry_penalty_last_n: params.dry_penalty_last_n,
            xtc_probability: params.xtc_probability,
            xtc_threshold: params.xtc_threshold,
            stop: stop_words,
            cache_prompt: true,
            seed: -1,
            grammar: grammar.as_ref().and_then(|g| g.gbnf.as_deref()),
            json_schema: grammar.as_ref().and_then(|g| g.json_schema.as_ref()),
            chat_template_kwargs: if disable_reasoning {
                Some(serde_json::json!({"enable_thinking": false}))
            } else {
                None
            },
        };

        // ── Телеметрия: старт генерации ──
        crate::infra::telemetry::track_event(
            "llm_started",
            json!({
                "ctx": ctx_label,
                "model": extract_model_filename(&self.model_path),
                "mode": self.engine_mode,
                "ctx_limit": self.global_ctx_limit,
                "max_tokens": max_tokens,
            }),
        );

        // ── Замер пиков памяти (RAM + VRAM) на время генерации ──
        // Гвард останавливает семплер на любом пути выхода (успех/ошибка/cancel).
        let server_pid = self.child.as_ref().map(|c| c.id());
        let sampler = crate::infra::MemSampler::start(server_pid);
        let mem_guard = crate::infra::MemGuard::new(sampler, ctx_label, &log_cb);

        let resp = self.client
            .post(self.url("/v1/chat/completions"))
            .header(AUTHORIZATION, self.auth_header())
            .json(&request)
            .send()
            .map_err(|e| format!(
                "Ошибка отправки запроса генерации: {}.{}",
                chain_err(&e, 3),
                read_log_tail(&self.server_log)
            ))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            let report = mem_guard.finish();
            let message = self.err_details(
                &report,
                &format!("llama-server: HTTP {} при генерации: {}", status, truncate_str(&text, 500)),
            );
            self.report_generation_error(ctx_label, &report, &message);
            return Err(message);
        }

        let gen_start = Instant::now();
        // Чтение стрима — в отдельном потоке: блокирующий read_line нельзя
        // прервать по таймауту. Основной цикл ждёт строки с recv_timeout —
        // так работает таймаут «первого токена» (и детект зависшего движка),
        // при этом длинная живая генерация не обрезается.
        let (lines_tx, lines_rx) = std::sync::mpsc::channel::<Result<Option<String>, String>>();
        let mut reader = BufReader::new(resp);
        std::thread::spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => { let _ = lines_tx.send(Ok(None)); break; }
                    Ok(_) => {
                        if lines_tx.send(Ok(Some(line.clone()))).is_err() {
                            break; // основной цикл вышел — стрим больше не нужен
                        }
                    }
                    Err(e) => { let _ = lines_tx.send(Err(e.to_string())); break; }
                }
            }
        });
        let mut result_text = String::new();
        let mut reasoning_text = String::new();
        let mut generated_bytes: Vec<u8> = Vec::new();
        let mut generated_tokens: u32 = 0;
        let mut reasoning_tokens: u32 = 0;
        let mut stop_reason = "MAX_TOKENS".to_string();
        let mut last_loop_check_chars = 0usize;
        let mut prompt_done_logged = false;
        let mut predicted_per_second: Option<f64> = None;
        let mut prompt_per_second: Option<f64> = None;
        let mut prompt_tokens: u32 = 0;
        let mut final_timings: Option<Timings> = None;
        let mut first_token_at: Option<Instant> = None;

        loop {
            if cancel_flag.load(Ordering::SeqCst) {
                stop_reason = "CANCELLED".to_string();
                break;
            }

            let line = match lines_rx.recv_timeout(READ_TIMEOUT) {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => break, // сервер закрыл стрим (EOF)
                Ok(Err(e)) => {
                    let report = mem_guard.finish();
                    let message = self.err_details(&report, &format!("Ошибка чтения потока генерации: {}", e));
                    self.report_generation_error(ctx_label, &report, &message);
                    return Err(message);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    let report = mem_guard.finish();
                    let message = self.err_details(
                        &report,
                        &format!(
                            "Движок не прислал данные в течение {} сек (завис или слишком долго обрабатывает промпт на CPU)",
                            READ_TIMEOUT.as_secs()
                        ),
                    );
                    self.report_generation_error(ctx_label, &report, &message);
                    return Err(message);
                }
                Err(_) => break, // поток-читатель завершился без данных
            };
            let trimmed = line.trim();
            let Some(data) = trimmed.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            let event: ChatCompletionEvent = match serde_json::from_str(data) {
                Ok(e) => e,
                Err(_) => {
                    if data.contains("\"error\"") {
                        let report = mem_guard.finish();
                        let message = self.err_details(
                            &report,
                            &format!("llama-server: ошибка генерации: {}", truncate_str(data, 500)),
                        );
                        self.report_generation_error(ctx_label, &report, &message);
                        return Err(message);
                    }
                    continue;
                }
            };

            if !prompt_done_logged {
                prompt_done_logged = true;
                let pt = event.tokens_evaluated
                    .max(event.timings.as_ref().map(|t| t.prompt_n).unwrap_or(0));
                prompt_tokens = pt;
                log_cb(format!(
                    "📐 Промпт принят движком: {} токенов, max_gen={}",
                    pt, max_tokens
                ));
            }

            if let Some(delta) = event.choices.first().and_then(|c| c.delta.content.as_deref()) {
                if !delta.is_empty() {
                    if first_token_at.is_none() {
                        first_token_at = Some(Instant::now());
                    }
                    generated_bytes.extend_from_slice(delta.as_bytes());
                    let current_text = String::from_utf8_lossy(&generated_bytes).into_owned();
                    let diff = compute_stream_diff(&current_text, &result_text).to_string();
                    if !diff.is_empty() {
                        (self.stream_cb)(diff);
                    }
                    result_text = current_text;
                    generated_tokens += 1;
                }
            }
            if let Some(r) = event.choices.first().and_then(|c| c.delta.reasoning_content.as_deref()) {
                if !r.is_empty() {
                    reasoning_text.push_str(r);
                    reasoning_tokens += 1;
                }
            }

            let total_tokens = generated_tokens + reasoning_tokens;
            let gen_p = ((total_tokens as f32 / max_tokens.max(1) as f32) * 50.0).min(50.0);
            progress_cb(50.0 + gen_p, &format!("Генерация: {} токенов...", total_tokens));

            if let Some(t) = event.timings {
                if t.predicted_per_second > 0.0 {
                    predicted_per_second = Some(t.predicted_per_second);
                }
                if t.prompt_per_second > 0.0 {
                    prompt_per_second = Some(t.prompt_per_second);
                }
                if t.prompt_n > 0 {
                    prompt_tokens = t.prompt_n;
                }
                if t.predicted_n > 0 {
                    final_timings = Some(t);
                }
            }

            // Лёгкая защита от зацикливания (проверяем хвост текста каждые ~300 символов)
            let chars_now = result_text.chars().count();
            if chars_now - last_loop_check_chars >= 300 {
                last_loop_check_chars = chars_now;
                if detect_repetition(&result_text, &log_cb) {
                    stop_reason = "LOOP_DETECTED".to_string();
                    break;
                }
            }

            if let Some(fr) = event.choices.first().and_then(|c| c.finish_reason.as_deref()) {
                match fr {
                    "stop" => stop_reason = "EOS".to_string(),
                    "length" => stop_reason = "MAX_TOKENS".to_string(),
                    "tool_calls" => stop_reason = "TOOL_CALLS".to_string(),
                    _ => stop_reason = fr.to_string(),
                }
                break;
            }
        }
        progress_cb(100.0, &format!("Готово ({} токенов)", generated_tokens + reasoning_tokens));

        let gen_elapsed = gen_start.elapsed().as_secs_f64();
        let total_tokens = generated_tokens + reasoning_tokens;
        let speed = predicted_per_second
            .or_else(|| final_timings.as_ref().map(|t| t.predicted_per_second))
            .unwrap_or_else(|| if gen_elapsed > 0.0 { total_tokens as f64 / gen_elapsed } else { 0.0 });
        let prompt_speed = prompt_per_second
            .or_else(|| final_timings.as_ref().map(|t| t.prompt_per_second))
            .unwrap_or(0.0);
        let ttft_sec = first_token_at
            .map(|t| t.duration_since(gen_start).as_secs_f64())
            .unwrap_or(gen_elapsed);
        self.last_tok_per_sec.set(speed);
        if reasoning_tokens > 0 {
            log_cb(format!(
                "⚙️ Сгенерировано {} токенов ответа + {} токенов думателя за {:.1}с ({:.0} tok/s). Причина: {}",
                generated_tokens, reasoning_tokens, gen_elapsed, speed, stop_reason
            ));
        } else {
            log_cb(format!(
                "⚙️ Сгенерировано {} токенов за {:.1}с ({:.0} tok/s). Причина: {}",
                generated_tokens, gen_elapsed, speed, stop_reason
            ));
        }

        if reasoning_tokens > 0 {
            let take = 300.min(reasoning_text.chars().count());
            let preview: String = reasoning_text.chars().take(take).collect();
            log_cb(format!("🧠 Думатель ({} токенов). Первые {} символов: {}", reasoning_tokens, take, preview.replace('\n', "\\n")));
        }

        if generated_tokens > 50 {
            let char_count: usize = result_text.chars().count();
            let take = 300.min(char_count);
            let preview: String = result_text.chars().take(take).collect();
            log_cb(format!("📝 Первые {} символов: {}", take, preview.replace('\n', "\\n")));
        }

        // ── Пиковые показатели памяти за время генерации (сверка с прогнозом) ──
        let report = mem_guard.finish();
        let total_mb = estimate_vram_mb(&self.model_path, self.global_ctx_limit, false, false);
        let extra = format!(
            "{} токенов (+{} думателя) за {:.1}с ({:.0} tok/s), причина: {}",
            generated_tokens, reasoning_tokens, gen_elapsed, speed, stop_reason
        );
        log_cb(crate::infra::peak_line(ctx_label, &report, self.vram_before, total_mb, &extra));

        // ── Телеметрия: итоги генерации ──
        crate::infra::telemetry::track_event(
            "llm_finished",
            json!({
                "ctx": ctx_label,
                "model": extract_model_filename(&self.model_path),
                "mode": self.engine_mode,
                "tokens": generated_tokens,
                "reasoning_tokens": reasoning_tokens,
                "elapsed_s": (gen_elapsed * 10.0).round() / 10.0,
                "tok_per_sec": (speed * 10.0).round() / 10.0,
                "stop_reason": stop_reason,
                "rss_server_mb": report.rss_server_peak / (1024 * 1024),
                "rss_app_mb": report.rss_app_peak / (1024 * 1024),
                "vram_mb": report.vram_used_peak / (1024 * 1024),
                "vram_ok": report.vram_ok,
            }),
        );

        Ok(GenerationResult {
            text: result_text,
            stop_reason,
            reasoning: reasoning_text,
            metrics: LlmMetrics {
                prompt_tokens,
                generated_tokens,
                reasoning_tokens,
                prompt_per_second: prompt_speed,
                predicted_per_second: speed,
                ttft_sec,
                elapsed_sec: gen_elapsed,
            },
        })
    }

    pub fn is_multimodal(&self) -> bool {
        self.is_multimodal_engine
    }

    /// Задаёт грамматику для СЛЕДУЮЩЕГО вызова generate_chat/generate_chat_multimodal.
    /// Грамматика «потребляется» одним вызовом (consume-and-clear), поэтому
    /// повторные итерации цикла агента (докачки, компакты, результаты tools)
    /// автоматически получают базовую грамматику, а не per-agent.
    pub fn set_grammar(&self, spec: Option<GrammarSpec>) {
        *self.pending_grammar.lock().unwrap() = spec;
    }

    /// Забирает заданную грамматику для текущего вызова (consume-and-clear).
    pub(crate) fn take_pending_grammar(&self) -> Option<GrammarSpec> {
        self.pending_grammar.lock().unwrap().take()
    }

    pub fn generate_chat<F, L>(
        &self,
        messages: &[LlmMessage],
        max_tokens: usize,
        model_params: &ModelParams,
        _format_type: &str,
        disable_reasoning: bool,
        cancel_flag: Arc<AtomicBool>,
        ctx_label: &str,
        progress_cb: F,
        log_cb: L,
    ) -> Result<GenerationResult, String>
    where F: FnMut(f32, &str), L: Fn(String) {
        // Рендер промпта выполняет движок (llama.cpp jinja по GGUF chat_template) —
        // единый канонический путь для ВСЕХ моделей. Формат из GGUF нужен только
        // для стоп-слов и базовой грамматики.
        let actual_format = PromptFormat::detect_from_gguf(&self.model_path);
        log_cb(format!(
            "🎯 Рендер промпта: chat template из GGUF (движок llama.cpp, jinja), формат {:?} — стоп-слова и грамматика",
            actual_format
        ));
        let words = actual_format.get_stop_words();
        let stop_words = merged_stop_words(&words);
        let pending = self.take_pending_grammar();
        let grammar = pending.or_else(|| build_base_grammar(&actual_format).map(|gbnf| GrammarSpec { gbnf: Some(gbnf), json_schema: None }));
        self.run_chat_completions(messages, None, max_tokens, model_params, &stop_words, grammar, disable_reasoning, cancel_flag, ctx_label, progress_cb, log_cb)
    }
}

impl Drop for LlamaEngine {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let pid = child.id();
            crate::infra::process_util::unregister_engine_pid(pid);
            crate::infra::startup_log::append(
                "INFO",
                &format!("🔻 Остановка llama-server (pid {})", pid),
            );
            // kill_process_tree убивает всё дерево (на Windows `child.kill()`
            // оставил бы потомков «висящими» в памяти).
            crate::infra::process_util::kill_process_tree(&mut child);
            let _ = child.wait();
        }
    }
}

#[derive(Deserialize, Default)]
struct Timings {
    #[serde(default)]
    predicted_per_second: f64,
    #[serde(default, alias = "predicted_n")]
    predicted_n: u32,
    #[serde(default)]
    prompt_n: u32,
    #[serde(default)]
    prompt_per_second: f64,
}

#[derive(Deserialize, Default)]
struct ChatDelta {
    #[serde(default)]
    content: Option<String>,
    /// Думатель модели: llama.cpp с --reasoning-format deepseek отдаёт его
    /// отдельным полем reasoning_content (Ollama использует то же имя).
    #[serde(default, alias = "thinking")]
    reasoning_content: Option<String>,
}

#[derive(Deserialize)]
struct ChatChoice {
    #[serde(default)]
    delta: ChatDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatCompletionEvent {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    tokens_evaluated: u32,
    #[serde(default)]
    timings: Option<Timings>,
}

/// Объединение стоп-слов: статический список + стоп-слова формата промпта
pub(crate) fn merged_stop_words(format_words: &[&'static str]) -> Vec<String> {
    let mut stops: Vec<String> = DEFAULT_STOP_WORDS.iter().map(|s| s.to_string()).collect();
    for w in format_words {
        if !stops.iter().any(|s| s == w) {
            stops.push(w.to_string());
        }
    }
    stops
}

/// Лёгкая защита от зацикливания: ищем повторяющиеся подстроки в хвосте текста
/// (аппаратный аналог N-gram loop-detection; сэмплинг и DRY теперь нативно в сервере).
fn detect_repetition(text: &str, log_cb: &impl Fn(String)) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n < 32 {
        return false;
    }
    let window = &chars[n.saturating_sub(1024)..];
    let w = window.len();
    let max_len = 256.min(w / 4);
    for l in 8..=max_len {
        if w < l * 4 {
            continue;
        }
        let block = &window[w - l..];
        let mut is_loop = true;
        for i in 1..4 {
            let start = w - l * (i + 1);
            let end = w - l * i;
            if &window[start..end] != block {
                is_loop = false;
                break;
            }
        }
        if is_loop {
            log_cb(format!(
                "🛑 Аппаратная защита: обнаружено зацикливание фразы (повтор подстроки из {} символов). Жёсткое прерывание.",
                l
            ));
            return true;
        }
    }
    false
}

fn truncate_str(s: &str, max: usize) -> String {
    let end = s.char_indices()
        .take_while(|(i, _)| *i < max)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(max.min(s.len()));
    format!("{}", &s[..end])
}

/// Полная цепочка источников ошибки (reqwest → hyper → io/tls), до 3 уровней.
/// Одиночный to_string() у reqwest скрывает реальную причину (порт закрыт
/// и т.п.), цепочка source() показывает её.
fn chain_err(e: &dyn std::error::Error, max: usize) -> String {
    let mut msg = String::new();
    let mut cur: Option<&dyn std::error::Error> = Some(e);
    for _ in 0..=max {
        let Some(c) = cur else { break };
        if !msg.is_empty() {
            msg.push_str(" → ");
        }
        msg.push_str(&c.to_string());
        cur = c.source();
    }
    msg
}

/// Хвост лога движка для диагностики ошибок запуска
fn read_log_tail(path: &Path) -> String {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let tail = content.chars().rev().take(800).collect::<String>().chars().rev().collect::<String>();
            format!("\n--- хвост лога llama-server ---\n{}", tail)
        }
        Err(_) => String::new(),
    }
}

/// Проверка: упал ли llama-server из-за ОШИБКИ ЗАГРУЗКИ проектора mmproj
/// (битый/несовместимый файл, нечитаемый тензор и т.п.).
fn is_mmproj_load_error(log: &str) -> bool {
    let l = log.to_lowercase();
    l.contains("clip_init")
        || l.contains("mtmd_init")
        || l.contains("failed to load multimodal")
        || (l.contains("failed to seek for tensor"))
        || (l.contains("failed to load model") && l.contains("mmproj"))
}

/// Проверка: требует ли модель проектор, которого не передали
/// (VL-модель запущена без --mmproj).
fn is_mmproj_required_error(log: &str) -> bool {
    let l = log.to_lowercase();
    (l.contains("multimodal") && (l.contains("no multimodal projector") || l.contains("no mmproj") || l.contains("no projector") || l.contains("requires a multimodal")))
        || (l.contains("model is multimodal") && l.contains("projector"))
}

/// Извлекает из лога конкретную строку с ошибкой проектора — для понятного сообщения.
fn mmproj_error_detail(log: &str) -> String {
    for line in log.lines().rev() {
        let l = line.to_lowercase();
        if (l.contains("mmproj") || l.contains("clip") || l.contains("mtmd") || l.contains("tensor") || l.contains("multimodal"))
            && (l.contains("error") || l.contains("failed") || l.contains("exiting"))
        {
            return line.trim().to_string();
        }
    }
    for line in log.lines().rev() {
        let l = line.to_lowercase();
        if l.contains("error") || l.contains("failed") {
            return line.trim().to_string();
        }
    }
    "неизвестная ошибка загрузки проектора".to_string()
}

/// Проверка готовности llama-server по HTTP /health (без создания структуры engine).
fn engine_health_check(client: &Client, port: u16, auth: &str) -> bool {
    client
        .get(format!("http://127.0.0.1:{}/health", port))
        .header(AUTHORIZATION, auth)
        .timeout(Duration::from_secs(3))
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Поиск в логе llama-server реальной причины CPU-фолбэка (почему CUDA не сработала).
/// Возвращает человекочитаемый диагноз или пустую строку.
fn diagnose_cuda_fallback(log_path: &Path) -> String {
    let content = match std::fs::read_to_string(log_path) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let lower = content.to_lowercase();
    // (паттерн в логе, человекочитаемое объяснение)
    const PATTERNS: &[(&str, &str)] = &[
        ("no kernel image", "в сборке движка нет ядер для вашей видеокарты (для RTX 50xx / Blackwell нужен вариант cuda-13.x)"),
        ("driver version is insufficient", "драйвер NVIDIA слишком старый для этой CUDA-сборки (для cuda-13.x нужен драйвер 580+ / CUDA 13, для cuda-12.4 — 527.41+)"),
        ("compute capability", "в сборке движка нет ядер для вашего GPU (compute capability)"),
        ("failed to initialize cuda", "CUDA не инициализировалась — неполадка драйвера"),
        ("cuda error", "ошибка CUDA"),
        ("ggml_cuda", "ошибка CUDA-бэкенда"),
        ("cuda", "CUDA-проблемы в логе движка"),
    ];
    for (needle, reason) in PATTERNS {
        if !lower.contains(needle) {
            continue;
        }
        for line in content.lines().rev().take(60) {
            if line.to_lowercase().contains(needle) {
                return format!("{} (из лога: {})", reason, line.trim());
            }
        }
        return reason.to_string();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuda_fallback_diagnosis_finds_no_kernel_image() {
        let dir = std::env::temp_dir().join(format!("ko_diag_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("llama_server.log");
        std::fs::write(&log, concat!(
            "load_backend: loaded CUDA backend\n",
            "CUDA error: no kernel image is available for execution on the device (CUDA_ERROR_NO_KERNEL_IMAGE)\n",
            "llama_model_load: error loading model: failed to load model\n"
        )).unwrap();
        let diag = diagnose_cuda_fallback(&log);
        assert!(diag.contains("нет ядер"), "diag: {}", diag);
        assert!(diag.contains("no kernel image"), "diag: {}", diag);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cuda_fallback_diagnosis_empty_on_clean_log() {
        let dir = std::env::temp_dir().join(format!("ko_diag2_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("llama_server.log");
        std::fs::write(&log, "llama server listening on port 17800").unwrap();
        assert_eq!(diagnose_cuda_fallback(&log), "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
