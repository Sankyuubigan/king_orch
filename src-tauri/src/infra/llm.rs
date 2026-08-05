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

use crate::infra::config::ModelParams;
use crate::infra::detokenizer::compute_stream_diff;

pub use super::llm_types::{ChatMessage, ChatAttachment, SubCall, ToolCallInfo, PromptFormat, push_report, LlmMessage, extract_model_filename, GenerationResult, llm_history};
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
    /// Скорость последней генерации (tok/s)
    last_tok_per_sec: std::cell::Cell<f64>,
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
    pub fn new<L, S>(engine_dir: &Path, model_path: &str, global_ctx_limit: u32, kv_quant_keys: bool, kv_quant_values: bool, log_cb: L, stream_cb: S) -> Result<Self, String>
    where L: Fn(String), S: Fn(String) + Send + Sync + 'static
    {
        Self::new_with_mmproj(engine_dir, model_path, None, global_ctx_limit, kv_quant_keys, kv_quant_values, log_cb, stream_cb)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_mmproj<L, S>(engine_dir: &Path, model_path: &str, mmproj_path: Option<&str>, global_ctx_limit: u32, kv_quant_keys: bool, kv_quant_values: bool, log_cb: L, stream_cb: S) -> Result<Self, String>
    where L: Fn(String), S: Fn(String) + Send + Sync + 'static
    {
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

        // ── Проверка установки движка ──
        let server_exe = engine_dir.join("llama-server.exe");
        if !server_exe.exists() {
            return Err(format!(
                "Движок llama.cpp не установлен (llama-server.exe не найден в {}).\nОткройте Настройки → «Движок запуска нейромоделей» и нажмите «Установить движок».",
                engine_dir.display()
            ));
        }

        // ── Совместимость установленного варианта движка с GPU ──
        // Главный фикс: сборка cuda-12.4 не имеет ядер для RTX 50xx (Blackwell,
        // compute 12.x) — llama-server молча падал в CPU. Теперь это явная ошибка
        // с понятным решением, а не тихий CPU-фолбэк.
        let required_gen = crate::infra::gpu_detector::required_cuda_gen(&gpu_info);
        let installed_meta = crate::infra::llamacpp_installer::installed_meta(engine_dir);
        let installed_variant = installed_meta.as_ref().map(|m| m.variant.clone()).unwrap_or_else(|| "не определён".to_string());
        let installed_family = installed_meta.as_ref()
            .map(|m| crate::infra::llamacpp_installer::EngineFamily::from_variant(&m.variant))
            .unwrap_or(crate::infra::llamacpp_installer::EngineFamily::Cpu);

        let use_cuda = match required_gen {
            None => false,
            Some(gen) => {
                let required_family = match gen {
                    crate::infra::gpu_detector::CudaGen::Cuda13 => crate::infra::llamacpp_installer::EngineFamily::Cuda13,
                    crate::infra::gpu_detector::CudaGen::Cuda12 => crate::infra::llamacpp_installer::EngineFamily::Cuda12,
                };
                if required_family == crate::infra::llamacpp_installer::EngineFamily::Cuda13
                    && installed_family == crate::infra::llamacpp_installer::EngineFamily::Cuda12
                {
                    return Err(format!(
                        "Ваша видеокарта {} — RTX 50xx (Blackwell), а установлен движок без её поддержки (вариант {}).\n\
                         Сборка cuda-12.4 не содержит ядер для Blackwell — модель работала бы только на CPU.\n\
                         Решение: Настройки → «Движок запуска нейромоделей» → «Обновить движок» (будет скачан вариант {} с поддержкой RTX 50xx).",
                        gpu_info.gpu_name, installed_variant, crate::infra::llamacpp_installer::VARIANT_CUDA13
                    ));
                }
                if installed_family == crate::infra::llamacpp_installer::EngineFamily::Cpu {
                    return Err(format!(
                        "Установлен CPU-вариант движка ({}), но на компьютере есть NVIDIA GPU ({}).\n\
                         Решение: Настройки → «Движок запуска нейромоделей» → «Обновить движок» — будет скачан CUDA-вариант.",
                        installed_variant, gpu_info.gpu_name
                    ));
                }
                true
            }
        };
        let gpu_layers: u32 = if use_cuda { 999 } else { 0 };
        let engine_mode = if use_cuda { "gpu".to_string() } else { "cpu".to_string() };
        log_cb(format!(
            "⚙️ Вариант движка: {} (установлен: {}, семейство {}) | GPU-слои: {} ({})",
            required_gen.map(|g| g.label().to_string()).unwrap_or_else(|| "cpu".to_string()),
            installed_variant,
            installed_family.label(),
            gpu_layers,
            if use_cuda { "оффлоуд на GPU" } else { "CUDA недоступна — CPU-режим" }
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
            return Err("Не удалось найти свободный порт для движка llama.cpp".to_string());
        }
        let api_key: String = (0..32).map(|_| {
            const HEX: &[u8] = b"0123456789abcdef";
            HEX[(rng.next() % 16) as usize] as char
        }).collect();

        let server_log = engine_dir.join("llama_server.log");
        let _ = std::fs::remove_file(&server_log);

        let mut cmd = Command::new(&server_exe);
        cmd.current_dir(engine_dir)
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
        if let Some(mmp) = mmproj_path {
            cmd.arg("--mmproj").arg(mmp);
        }
        if kv_quant_keys {
            cmd.arg("--cache-type-k").arg("q8_0");
        }
        if kv_quant_values {
            cmd.arg("--cache-type-v").arg("q8_0");
        }

        log_cb(format!(
            "🚀 Запуск llama-server: порт {}, контекст {} токенов, потоков {}, mmproj={}",
            port,
            global_ctx_limit,
            threads,
            mmproj_path.unwrap_or("нет")
        ));

        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| format!("Ошибка создания HTTP-клиента: {}", e))?;

        #[cfg(target_os = "windows")]
        { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }

        let child = cmd.spawn().map_err(|e| format!("Ошибка запуска llama-server: {}", e))?;

        let mut engine = Self {
            global_ctx_limit,
            model_path: model_path.to_string(),
            mmproj_path: mmproj_path.map(|s| s.to_string()),
            is_multimodal_engine: mmproj_path.is_some(),
            stream_cb: Arc::new(stream_cb),
            client,
            server_log,
            port,
            api_key,
            child: Some(child),
            engine_mode: engine_mode.clone(),
            engine_mode_detail: String::new(),
            last_tok_per_sec: std::cell::Cell::new(0.0),
        };

        // ── Ожидание готовности (модель грузится в память) ──
        let mut child = engine.child.take().expect("child");
        let deadline = Instant::now() + HEALTH_TIMEOUT;
        let mut last_progress_log = Instant::now();
        loop {
            if let Some(code) = child.try_wait().map_err(|e| format!("Ошибка ожидания llama-server: {}", e))? {
                engine.child = Some(child);
                return Err(format!(
                    "Движок llama-server завершился при запуске (код {}). {}",
                    code,
                    read_log_tail(&engine.server_log)
                ));
            }
            if engine.is_healthy() {
                break;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                engine.child = Some(child);
                return Err(format!(
                    "Таймаут запуска движка llama.cpp ({} сек). Модель не загрузилась. {}",
                    HEALTH_TIMEOUT.as_secs(),
                    read_log_tail(&engine.server_log)
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
        engine.child = Some(child);

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
            } else if use_cuda {
                // Намерение было GPU, но VRAM не выросла — llama-server тихо ушёл в CPU.
                // Сообщаем ПРИЧИНУ (из лога сервера), а не выдуманный диагноз.
                engine.engine_mode = "cpu".to_string();
                let diag = diagnose_cuda_fallback(&engine.server_log);
                engine.engine_mode_detail = if !diag.is_empty() {
                    format!("Модель не попала в VRAM: {}", diag)
                } else {
                    format!(
                        "Модель не попала в VRAM: движок (вариант {}) не смог использовать GPU {} (драйвер CUDA {}.{}, compute {}.{}). Подробности — в хвосте llama_server.log.",
                        installed_variant, gpu_info.gpu_name, gpu_info.cuda_major, gpu_info.cuda_minor,
                        gpu_info.compute_major, gpu_info.compute_minor
                    )
                };
                log_cb("❌ ВНИМАНИЕ: VRAM не увеличилась! Модель работает на CPU, а не на GPU!".to_string());
                log_cb(format!("❌ Диагноз: {}", engine.engine_mode_detail));
                log_cb("❌ Решение: Настройки → «Движок запуска нейромоделей» → «Обновить движок».".to_string());
            } else {
                log_cb("ℹ️ GPU-ускорение не используется (CPU-режим) — VRAM не занята. Это ожидаемо.".to_string());
            }
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

    // ─── Генерация ───

    /// Общий цикл генерации: текст и мультимодалка (через multimodal_data).
    pub(crate) fn run_completion<F, L>(
        &self,
        full_prompt: &str,
        multimodal_data: Option<Vec<String>>,
        max_tokens: usize,
        params: &ModelParams,
        stop_words: &[String],
        cancel_flag: Arc<AtomicBool>,
        mut progress_cb: F,
        log_cb: L,
    ) -> Result<GenerationResult, String>
    where F: FnMut(f32, &str), L: Fn(String) {
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
        struct CompletionRequest<'a> {
            prompt: serde_json::Value,
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
        }

        let prompt_value = match &multimodal_data {
            Some(data) => serde_json::json!({
                "prompt_string": full_prompt,
                "multimodal_data": data,
            }),
            None => serde_json::Value::String(full_prompt.to_string()),
        };

        let request = CompletionRequest {
            prompt: prompt_value,
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
        };

        let resp = self.client
            .post(self.url("/completion"))
            .header(AUTHORIZATION, self.auth_header())
            .json(&request)
            .send()
            .map_err(|e| format!("Ошибка отправки запроса генерации: {}.{}", e, read_log_tail(&self.server_log)))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            return Err(format!("llama-server: HTTP {} при генерации: {}.{}", status, truncate_str(&text, 500), read_log_tail(&self.server_log)));
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
        let mut generated_bytes: Vec<u8> = Vec::new();
        let mut generated_tokens: u32 = 0;
        let mut stop_reason = "MAX_TOKENS".to_string();
        let mut last_loop_check_chars = 0usize;
        let mut prompt_done_logged = false;
        let mut predicted_per_second: Option<f64> = None;
        let mut final_timings: Option<Timings> = None;

        loop {
            if cancel_flag.load(Ordering::SeqCst) {
                stop_reason = "CANCELLED".to_string();
                break;
            }

            let line = match lines_rx.recv_timeout(READ_TIMEOUT) {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => break, // сервер закрыл стрим (EOF)
                Ok(Err(e)) => {
                    return Err(format!(
                        "Ошибка чтения потока генерации: {}.{}",
                        e,
                        read_log_tail(&self.server_log)
                    ));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    return Err(format!(
                        "Движок не прислал данные в течение {} сек (завис или слишком долго обрабатывает промпт на CPU).\n{}",
                        READ_TIMEOUT.as_secs(),
                        read_log_tail(&self.server_log)
                    ));
                }
                Err(_) => break, // поток-читатель завершился без данных
            };
            let trimmed = line.trim();
            let Some(data) = trimmed.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            let event: CompletionEvent = match serde_json::from_str(data) {
                Ok(e) => e,
                Err(_) => {
                    if data.contains("\"error\"") {
                        return Err(format!("llama-server: ошибка генерации: {}", truncate_str(data, 500)));
                    }
                    continue;
                }
            };

            if !prompt_done_logged {
                prompt_done_logged = true;
                log_cb(format!(
                    "📐 Промпт принят движком: {} токенов, max_gen={}",
                    event.tokens_evaluated, max_tokens
                ));
            }

            if !event.content.is_empty() {
                generated_bytes.extend_from_slice(event.content.as_bytes());
                let current_text = String::from_utf8_lossy(&generated_bytes).into_owned();
                let diff = compute_stream_diff(&current_text, &result_text).to_string();
                if !diff.is_empty() {
                    (self.stream_cb)(diff);
                }
                result_text = current_text;
                generated_tokens += 1;
            }

            let gen_p = ((generated_tokens as f32 / max_tokens.max(1) as f32) * 50.0).min(50.0);
            progress_cb(50.0 + gen_p, &format!("Генерация: {} токенов...", generated_tokens));

            if let Some(t) = event.timings {
                if t.predicted_per_second > 0.0 {
                    predicted_per_second = Some(t.predicted_per_second);
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

            if event.stop {
                match event.stop_type.as_deref() {
                    Some("eos") => stop_reason = "EOS".to_string(),
                    Some("word") => stop_reason = "STOP_WORD".to_string(),
                    Some("limit") => stop_reason = "MAX_TOKENS".to_string(),
                    _ => {
                        if event.truncated {
                            stop_reason = "MAX_TOKENS".to_string();
                        }
                    }
                }
                break;
            }
        }
        progress_cb(100.0, &format!("Готово ({} токенов)", generated_tokens));

        let gen_elapsed = gen_start.elapsed().as_secs_f64();
        let speed = predicted_per_second
            .or_else(|| final_timings.as_ref().map(|t| t.predicted_per_second))
            .unwrap_or_else(|| if gen_elapsed > 0.0 { generated_tokens as f64 / gen_elapsed } else { 0.0 });
        self.last_tok_per_sec.set(speed);
        log_cb(format!(
            "⚙️ Сгенерировано {} токенов за {:.1}с ({:.0} tok/s). Причина: {}",
            generated_tokens, gen_elapsed, speed, stop_reason
        ));

        if generated_tokens > 50 {
            let char_count: usize = result_text.chars().count();
            let take = 300.min(char_count);
            let preview: String = result_text.chars().take(take).collect();
            log_cb(format!("📝 Первые {} символов: {}", take, preview.replace('\n', "\\n")));
        }

        // ── Фактическое VRAM после инференса (сверка с ожидаемым) ──
        let total_mb = estimate_vram_mb(&self.model_path, self.global_ctx_limit, false, false);
        match nvml_wrapper::Nvml::init() {
            Ok(nvml) => {
                if let Ok(device) = nvml.device_by_index(0) {
                    if let Ok(mem) = device.memory_info() {
                        let used_mb = mem.used / 1024 / 1024;
                        log_cb(format!("📊 Фактическое VRAM после инференса: {} МБ (модель+контекст ~{:.0} МБ)", used_mb, total_mb));
                    }
                }
            }
            Err(_) => {}
        }

        Ok(GenerationResult { text: result_text, stop_reason })
    }

    pub fn is_multimodal(&self) -> bool {
        self.is_multimodal_engine
    }

    pub fn generate_chat<F, L>(
        &self,
        messages: &[LlmMessage],
        max_tokens: usize,
        model_params: &ModelParams,
        format_type: &str,
        cancel_flag: Arc<AtomicBool>,
        progress_cb: F,
        log_cb: L,
    ) -> Result<GenerationResult, String>
    where F: FnMut(f32, &str), L: Fn(String) {
        let (full_prompt, actual_format) = self.build_prompt(messages, format_type, &log_cb);
        log_cb(format!("🔤 Определен формат промпта: {:?}", actual_format));
        let words = actual_format.get_stop_words();
        let stop_words = merged_stop_words(&words);
        self.run_completion(&full_prompt, None, max_tokens, model_params, &stop_words, cancel_flag, progress_cb, log_cb)
    }
}

impl Drop for LlamaEngine {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
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
}

#[derive(Deserialize)]
struct CompletionEvent {
    #[serde(default)]
    content: String,
    #[serde(default)]
    stop: bool,
    #[serde(default)]
    stop_type: Option<String>,
    #[serde(default)]
    truncated: bool,
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
        ("no kernel image", "в сборке движка нет ядер для вашей видеокарты (RTX 50xx нужен вариант cuda-13.x)"),
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
