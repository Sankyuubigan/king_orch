#![allow(deprecated)]

//! LlamaEngine — загрузка модели и генерация текста

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::num::NonZeroU32;
use std::time::Instant;

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::context::params::{LlamaContextParams, KvCacheType};
use llama_cpp_2::model::AddBos;
use llama_cpp_2::model::Special;
use llama_cpp_2::token::data_array::LlamaTokenDataArray;
use llama_cpp_2::llama_batch::LlamaBatch;

#[cfg(feature = "mtmd")]
use llama_cpp_2::mtmd::{MtmdContext, MtmdContextParams};

use crate::infra::config::ModelParams;

pub use super::llm_types::{ChatMessage, ChatAttachment, SubCall, ToolCallInfo, PromptFormat};
pub use super::llm_gguf::{extract_string_from_gguf, extract_f32_from_gguf, extract_u32_from_gguf};

pub struct LlamaEngine {
    pub backend: LlamaBackend,
    pub model: LlamaModel,
    pub global_ctx_limit: u32,
    pub kv_quant_keys: bool,
    pub kv_quant_values: bool,
    pub model_path: String,
    pub model_size_mb: f64,
    pub mmproj_path: Option<String>,
    pub stream_cb: Arc<dyn Fn(String) + Send + Sync>,
    #[cfg(feature = "mtmd")]
    pub mtmd_ctx: Option<MtmdContext>,
}

impl LlamaEngine {
    pub fn new<L, S>(model_path: &str, global_ctx_limit: u32, kv_quant_keys: bool, kv_quant_values: bool, log_cb: L, stream_cb: S) -> Result<Self, String>
    where L: Fn(String), S: Fn(String) + Send + Sync + 'static
    {
        Self::new_with_mmproj(model_path, None, global_ctx_limit, kv_quant_keys, kv_quant_values, log_cb, stream_cb)
    }

    pub fn new_with_mmproj<L, S>(model_path: &str, mmproj_path: Option<&str>, global_ctx_limit: u32, kv_quant_keys: bool, kv_quant_values: bool, log_cb: L, stream_cb: S) -> Result<Self, String>
    where L: Fn(String), S: Fn(String) + Send + Sync + 'static
    {
        log_cb("⚡ Инициализация llama.cpp...".to_string());

        // ── Логирование аппаратного обеспечения ──
        log_cb("🖥️ Аппаратное обеспечение:".to_string());
        let sys = sysinfo::System::new_all();
        if let Some(cpu) = sys.cpus().first() {
            let total_ram_mb = sys.total_memory() / 1024 / 1024;
            log_cb(format!("   CPU: {} | RAM: {} MB", cpu.brand(), total_ram_mb));
        }

        let vram_before = match nvml_wrapper::Nvml::init() {
            Ok(nvml) => {
                match nvml.device_by_index(0) {
                    Ok(device) => {
                        match device.memory_info() {
                            Ok(mem) => {
                                let total_vram_mb = mem.total / 1024 / 1024;
                                let used_vram_mb = mem.used / 1024 / 1024;
                                let free_vram_mb = total_vram_mb - used_vram_mb;
                                log_cb(format!("   GPU: NVIDIA | VRAM: {} MB (Свободно: {} MB)", total_vram_mb, free_vram_mb));
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

        let backend = LlamaBackend::init().map_err(|e| e.to_string())?;
        let gpu_layers: u32 = 999;
        let mut model_params = LlamaModelParams::default();
        model_params = model_params.with_n_gpu_layers(gpu_layers);
        log_cb(format!("⚙️ Запрос GPU слоёв: {} (попытка оффлоуда)", gpu_layers));
        
        let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
            .map_err(|e| format!("Ошибка загрузки модели: {}", e))?;
        log_cb(format!("✅ Модель загружена успешно: {}", model_path));

        // ── Проверка: реально ли модель ушла в VRAM? ──
        if vram_before > 0 {
            let vram_after = match nvml_wrapper::Nvml::init() {
                Ok(nvml) => match nvml.device_by_index(0) {
                    Ok(device) => match device.memory_info() {
                        Ok(mem) => mem.used,
                        Err(_) => 0
                    },
                    Err(_) => 0
                },
                Err(_) => 0
            };
            
            let diff = vram_after as i64 - vram_before as i64;
            if diff > 100_000_000 { // > 100 MB
                log_cb(format!("✅ GPU: Модель загружена в VRAM. Занято {} МБ видеопамяти.", diff / 1024 / 1024));
            } else {
                log_cb("❌ ВНИМАНИЕ: VRAM не увеличилась! Модель работает на CPU, а не на GPU!".to_string());
                log_cb("❅ Причина: llama-cpp-2 не собран с поддержкой CUDA, либо CUDA Toolkit не настроен в системе. Проверьте логи компиляции (cargo build).".to_string());
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

        #[cfg(feature = "mtmd")]
        let mtmd_ctx = if let Some(mmp) = mmproj_path {
            match MtmdContext::init_from_file(mmp, &model, &MtmdContextParams::default()) {
                Ok(ctx) => {
                    log_cb(format!("✅ mmproj загружен: {} (vision={}, audio={})", mmp, ctx.support_vision(), ctx.support_audio()));
                    Some(ctx)
                }
                Err(e) => {
                    log_cb(format!("⚠️ Не удалось загрузить mmproj: {}", e));
                    None
                }
            }
        } else { None };

        Ok(Self {
            backend, model, global_ctx_limit, kv_quant_keys, kv_quant_values,
            model_path: model_path.to_string(),
            model_size_mb,
            mmproj_path: mmproj_path.map(|s| s.to_string()),
            stream_cb: Arc::new(stream_cb),
            #[cfg(feature = "mtmd")]
            mtmd_ctx,
        })
    }

    pub fn build_prompt(&self, messages: &[ChatMessage], format_type: &str, log_cb: &impl Fn(String)) -> (String, PromptFormat) {
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

    pub fn get_tokens_count(&self, messages: &[ChatMessage], format_type: &str) -> Result<usize, String> {
        let (full_prompt, _) = self.build_prompt(messages, format_type, &|_|{});
        let tokens = self.model.str_to_token(&full_prompt, AddBos::Always)
            .map_err(|e| format!("Ошибка токенизации: {}", e))?;
        Ok(tokens.len())
    }

    pub fn run_generation<F, L>(
        &self,
        full_prompt: &str,
        max_tokens: usize,
        params: &ModelParams,
        custom_stop_words: &[&str],
        cancel_flag: Arc<AtomicBool>,
        mut progress_cb: F,
        log_cb: L,
    ) -> Result<String, String>
    where
        F: FnMut(f32, &str),
        L: Fn(String),
    {
        let actual_min_p = params.min_p.max(0.0);
        let actual_rep_pen = params.repetition_penalty.max(1.0);
        let actual_temp = params.temperature.max(0.01);

        log_cb(format!(
            "🎛 Фактические параметры сэмплинга: Temp={:.2}, Top_K={}, Top_P={:.2}, Min_P={:.2}, Rep_Pen={:.2}, Pres_Pen={:.2}",
            actual_temp, params.top_k, params.top_p, actual_min_p, actual_rep_pen, params.presence_penalty
        ));

        let batch_size = 2048;
        let logical_cores = std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(8);
        let threads = (logical_cores / 2).max(4);

        let tokens = self.model.str_to_token(full_prompt, AddBos::Always)
            .map_err(|e| format!("Ошибка токенизации: {}", e))?;

        if tokens.is_empty() { return Err("Промпт пуст".to_string()); }

        let ideal_ctx_size = (tokens.len() as u32 + max_tokens as u32 + 128).min(self.global_ctx_limit);

        let mut ctx_params = LlamaContextParams::default();
        ctx_params = ctx_params.with_n_ctx(NonZeroU32::new(ideal_ctx_size));
        ctx_params = ctx_params.with_n_batch(batch_size as u32);
        ctx_params = ctx_params.with_n_threads(threads);
        ctx_params = ctx_params.with_n_threads_batch(threads);
        
        let type_k = if self.kv_quant_keys { KvCacheType::Q8_0 } else { KvCacheType::F16 };
        let type_v = if self.kv_quant_values { KvCacheType::Q8_0 } else { KvCacheType::F16 };
        ctx_params = ctx_params.with_type_k(type_k).with_type_v(type_v);

        // ── Flash Attention совместим с Q8_0, включаем безусловно ──
        ctx_params = ctx_params.with_flash_attention_policy(1);
        log_cb("⚡ Flash Attention: включен (policy=1).".to_string());

        let mut ctx = self.model.new_context(&self.backend, ctx_params)
            .map_err(|e| format!("Ошибка создания контекста: {}", e))?;

        let total_expected = tokens.len() + max_tokens;
        log_cb(format!("📐 Промпт: {} токенов, max_gen={}, ожидаемый финал: {}/{} (n_ctx)", tokens.len(), max_tokens, total_expected, ideal_ctx_size));

        let layers = extract_u32_from_gguf(&self.model_path, "llama.block_count").unwrap_or(32);
        let heads = extract_u32_from_gguf(&self.model_path, "llama.attention.head_count").unwrap_or(32);
        let heads_kv = extract_u32_from_gguf(&self.model_path, "llama.attention.head_count_kv").unwrap_or(heads);
        let embd = extract_u32_from_gguf(&self.model_path, "llama.embedding_length").unwrap_or(4096);
        let head_dim = embd / heads;

        let b_k = if self.kv_quant_keys { 1.06 } else { 2.0 };
        let b_v = if self.kv_quant_values { 1.06 } else { 2.0 };

        let kv_bytes = (layers as f64 * head_dim as f64 * ideal_ctx_size as f64) * (heads as f64 * b_k + heads_kv as f64 * b_v);
        let kv_mb = kv_bytes / (1024.0 * 1024.0);
        let total_mb = self.model_size_mb + kv_mb;

        log_cb(format!("💾 Ожидаемое потребление VRAM (GPU): Модель ~{:.1} МБ + Кэш ~{:.1} МБ = Итого ~{:.1} МБ", self.model_size_mb, kv_mb, total_mb));

        if tokens.len() as u32 >= ideal_ctx_size {
            let err_msg = format!("Текст слишком большой ({} токенов) для текущего контекста ({}).", tokens.len(), ideal_ctx_size);
            log_cb(format!("❌ ОШИБКА КОНТЕКСТА: {}", err_msg));
            return Err(err_msg);
        }

        let prompt_start = Instant::now();

        let mut past_tokens = tokens.clone();
        let mut batch = LlamaBatch::new(batch_size, 1);
        let last_index = tokens.len() - 1;
        let mut n_past = 0;
        let total_chunks = tokens.chunks(batch_size).count();

        for (chunk_idx, chunk) in tokens.chunks(batch_size).enumerate() {
            if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }
            let local_p = (chunk_idx as f32 / total_chunks as f32) * 50.0;
            progress_cb(local_p, &format!("Чтение в GPU: {}% (Блок {}/{})", (local_p * 2.0) as i32, chunk_idx + 1, total_chunks));

            batch.clear();
            for (i, &token) in chunk.iter().enumerate() {
                let pos = n_past + i as i32;
                batch.add(token, pos, &[0], pos as usize == last_index).map_err(|e| e.to_string())?;
            }
            ctx.decode(&mut batch).map_err(|e| e.to_string())?;
            n_past += chunk.len() as i32;
        }

        let prompt_elapsed = prompt_start.elapsed().as_secs_f64();
        log_cb(format!("⏱ Промпт обработан за {:.1}с (Пакеты по {})", prompt_elapsed, batch_size));

        let prompt_pps = tokens.len() as f64 / prompt_elapsed.max(0.001);
        if prompt_pps < 100.0 {
            log_cb(format!("⚠️ ВНИМАНИЕ: Скорость промпта {:.0} tok/s. Если у вас CUDA сборка, модель возможно работает на CPU вместо GPU!", prompt_pps));
        }

        let gen_start = Instant::now();

        let mut n_cur = n_past;
        let mut result_text = String::new();
        let mut generated_bytes: Vec<u8> = Vec::new();
        let mut generated_tokens = 0;
        let mut gen_tokens: Vec<llama_cpp_2::token::LlamaToken> = Vec::new();

        let mut stop_words: Vec<&str> = vec![
            "<|im_end|>", "<end_of_turn>", "</s>", "<|eot_id|>",
            "<turn>", "<|eot|>", "User:", "System:", "<eos>", "Yes",
            "<turn|>", "/end_of_turn>", "<step>", "<|end_of_text|>", "<｜end of sentence｜>",
            "</start_of_turn>", "<|channel|>"
        ];

        for &w in custom_stop_words {
            if !stop_words.contains(&w) {
                stop_words.push(w);
            }
        }

        let mut _stop_reason = "MAX_TOKENS";

        while n_cur < ideal_ctx_size as i32 && generated_tokens < max_tokens {
            if cancel_flag.load(Ordering::SeqCst) {
                _stop_reason = "CANCELLED";
                break;
            }

            let sample_start = Instant::now();

            let candidates_array = ctx.candidates_ith(batch.n_tokens() - 1);
            let candidates = LlamaTokenDataArray::from_iter(candidates_array, false);
            let mut candidates_vec: Vec<(llama_cpp_2::token::LlamaToken, f32)> = candidates.data.iter().map(|d| (d.id(), d.logit())).collect();

            let penalty_last_n = 256.min(past_tokens.len());
            let last_tokens_slice = if past_tokens.len() > penalty_last_n { &past_tokens[past_tokens.len() - penalty_last_n..] } else { &past_tokens };
            let mut penalty_tokens = last_tokens_slice.to_vec();
            penalty_tokens.sort_unstable();
            penalty_tokens.dedup();

            for (id, logit) in candidates_vec.iter_mut() {
                if penalty_tokens.binary_search(id).is_ok() {
                    *logit -= params.presence_penalty;
                    if *logit <= 0.0 { *logit *= actual_rep_pen; }
                    else { *logit /= actual_rep_pen; }
                }
            }

            for (_, logit) in candidates_vec.iter_mut() { *logit /= actual_temp; }

            // ── ОПТИМИЗАЦИЯ СЭМПЛИНГА ──
            candidates_vec.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            let k = if actual_min_p >= 0.05 { 1000 } else { (params.top_k as usize).min(candidates_vec.len()).max(1) };
            candidates_vec.truncate(k);

            let max_logit = candidates_vec.first().map(|(_, l)| *l).unwrap_or(0.0);
            let mut sum_exp = 0.0;
            let mut probs: Vec<(llama_cpp_2::token::LlamaToken, f32)> = candidates_vec.into_iter().map(|(id, logit)| {
                let p = (logit - max_logit).exp(); sum_exp += p; (id, p)
            }).collect();
            
            for (_, p) in probs.iter_mut() { *p /= sum_exp; }

            let max_prob = probs.first().map(|(_, p)| *p).unwrap_or(1.0);

            let min_p_thresh = max_prob * actual_min_p;
            probs.retain(|(_, p)| *p >= min_p_thresh);

            let top_p_thresh = if actual_min_p >= 0.05 { 1.0 } else { params.top_p };
            let mut cumulative_prob = 0.0;
            let mut top_p_idx = probs.len();
            for (i, (_, p)) in probs.iter().enumerate() {
                cumulative_prob += *p;
                if cumulative_prob >= top_p_thresh { top_p_idx = i + 1; break; }
            }
            probs.truncate(top_p_idx);

            let sum_prob: f32 = probs.iter().map(|(_, p)| *p).sum();
            for (_, p) in probs.iter_mut() { *p /= sum_prob; }

            static SEED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1337);
            let mut seed = SEED.load(Ordering::SeqCst);
            if seed == 1337 { seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos().max(1); }
            seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5;
            SEED.store(seed, Ordering::SeqCst);
            let r = (seed as f32) / (u32::MAX as f32);

            let mut cumulative = 0.0;
            let mut new_token = probs.last().map(|(id, _)| *id).unwrap_or_else(|| self.model.token_eos());
            for (id, p) in probs.iter() { cumulative += *p; if r <= cumulative { new_token = *id; break; } }

            if generated_tokens == 0 || generated_tokens % 50 == 0 {
                log_cb(format!("⏱ Сэмплер отработал за {:.2} мс", sample_start.elapsed().as_millis()));
            }

            if new_token == self.model.token_eos() { _stop_reason = "EOS"; break; }
            past_tokens.push(new_token);
            gen_tokens.push(new_token);

            let mut loop_detected = false;
            let g_len = gen_tokens.len();
            for l in 1..=32 {
                let required_repeats = match l { 1 => 15, 2 => 6, _ => 4 };
                if g_len >= l * required_repeats {
                    let mut is_loop = true;
                    let suffix = &gen_tokens[g_len - l..g_len];
                    for i in 1..required_repeats {
                        let start = g_len - l * (i + 1);
                        let end = g_len - l * i;
                        if &gen_tokens[start..end] != suffix { is_loop = false; break; }
                    }
                    if is_loop { loop_detected = true; break; }
                }
            }

            if loop_detected {
                log_cb("🛑 Сработала аппаратная защита N-Gram: обнаружено зацикливание фразы. Жесткое прерывание.".to_string());
                _stop_reason = "LOOP_DETECTED";
                break;
            }

            if let Ok(bytes) = self.model.token_to_bytes(new_token, Special::Tokenize) {
                generated_bytes.extend_from_slice(&bytes);
                let current_text = String::from_utf8_lossy(&generated_bytes).into_owned();
                let diff = current_text[result_text.len()..].to_string();
                if !diff.is_empty() { (self.stream_cb)(diff); }
                result_text = current_text;
            }

            let mut should_stop = false;
            let mut matched_word = String::new();
            for word in stop_words.iter() {
                if result_text.contains(word) {
                    matched_word = word.to_string();
                    result_text = result_text.replace(word, "").trim().to_string();
                    should_stop = true;
                    break;
                }
            }
            if should_stop { log_cb(format!("🛑 Стоп-слово '{}' на токене {}", matched_word, generated_tokens)); _stop_reason = "STOP_WORD"; break; }

            batch.clear();
            batch.add(new_token, n_cur, &[0], true).map_err(|e| e.to_string())?;
            ctx.decode(&mut batch).map_err(|e| e.to_string())?;

            n_cur += 1; generated_tokens += 1;
            if generated_tokens % 20 == 0 {
                let gen_p = (generated_tokens as f32 / max_tokens as f32) * 50.0;
                progress_cb(50.0 + gen_p, &format!("Генерация: {} токенов...", generated_tokens));
            }
        }
        progress_cb(100.0, &format!("Готово ({} токенов)", generated_tokens));

        let gen_elapsed = gen_start.elapsed().as_secs_f64();
        let speed = if gen_elapsed > 0.0 { generated_tokens as f64 / gen_elapsed } else { 0.0 };
        log_cb(format!("⚙️ Сгенерировано {} токенов за {:.1}с ({:.0} tok/s). Причина: {}", generated_tokens, gen_elapsed, speed, _stop_reason));

        if generated_tokens > 50 {
            let char_count: usize = result_text.chars().count();
            let take = 300.min(char_count);
            let preview: String = result_text.chars().take(take).collect();
            log_cb(format!("📝 Первые {} символов: {}", take, preview.replace('\n', "\\n")));
        }
        Ok(result_text)
    }

    pub fn is_multimodal(&self) -> bool {
        #[cfg(feature = "mtmd")]
        { self.mtmd_ctx.is_some() }
        #[cfg(not(feature = "mtmd"))]
        { false }
    }

    pub fn generate_chat<F, L>(
        &self,
        messages: &[ChatMessage],
        max_tokens: usize,
        model_params: &ModelParams,
        format_type: &str,
        cancel_flag: Arc<AtomicBool>,
        progress_cb: F,
        log_cb: L,
    ) -> Result<String, String>
    where F: FnMut(f32, &str), L: Fn(String) {
        let (full_prompt, actual_format) = self.build_prompt(messages, format_type, &log_cb);
        log_cb(format!("🔤 Определен формат промпта: {:?}", actual_format));
        let words = actual_format.get_stop_words();
        self.run_generation(&full_prompt, max_tokens, model_params, &words, cancel_flag, progress_cb, log_cb)
    }
}