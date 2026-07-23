#![allow(deprecated)]

//! Мультимодальная генерация (изображения/аудио через mmproj)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::num::NonZeroU32;
use std::time::Instant;

use llama_cpp_2::context::params::{LlamaContextParams, KvCacheType};
use llama_cpp_2::model::Special;
use llama_cpp_2::token::data_array::LlamaTokenDataArray;
use llama_cpp_2::llama_batch::LlamaBatch;

#[cfg(feature = "mtmd")]
use llama_cpp_2::mtmd::{MtmdBitmap, mtmd_default_marker, MtmdInputText};

use crate::infra::config::ModelParams;
use super::llm::LlamaEngine;
use super::llm_types::{ChatAttachment, LlmMessage};
use super::llm_gguf::extract_u32_from_gguf;

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(input)
        .map_err(|e| format!("base64 decode error: {}", e))
}

impl LlamaEngine {
    #[cfg(feature = "mtmd")]
    pub fn generate_chat_multimodal<F, L>(
        &self,
        messages: &[LlmMessage],
        attachments: &[ChatAttachment],
        max_tokens: usize,
        model_params: &ModelParams,
        format_type: &str,
        cancel_flag: Arc<AtomicBool>,
        mut progress_cb: F,
        log_cb: L,
    ) -> Result<String, String>
    where F: FnMut(f32, &str), L: Fn(String) {
        let mtmd_ctx = self.mtmd_ctx.as_ref().ok_or("mmproj не загружен")?;

        let (mut full_prompt, actual_format) = self.build_prompt(messages, format_type, &log_cb);
        log_cb(format!("🔤 Определен формат промпта (мультимодальный): {:?}", actual_format));

        let marker = mtmd_default_marker();
        for _ in attachments.iter() {
            full_prompt.push_str(marker);
        }

        log_cb(format!("📐 Мультимодальный промпт: {} символов, {} вложений", full_prompt.len(), attachments.len()));

        let mut bitmaps = Vec::new();
        for att in attachments {
            let data = base64_decode(&att.data_base64)
                .map_err(|_| format!("Ошибка декодирования base64: {}", att.file_name))?;
            let bitmap = MtmdBitmap::from_buffer(mtmd_ctx, &data, false)
                .map_err(|e| format!("Ошибка загрузки {}: {:?}", att.file_name, e))?;
            bitmaps.push(bitmap);
        }

        let bitmap_refs: Vec<&MtmdBitmap> = bitmaps.iter().collect();

        let input_text = MtmdInputText {
            text: full_prompt.clone(),
            add_special: true,
            parse_special: true,
        };

        let chunks = mtmd_ctx.tokenize(input_text, &bitmap_refs)
            .map_err(|e| format!("Ошибка токенизации: {:?}", e))?;

        let batch_size = 2048;
        let logical_cores = std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(8);
        let threads = (logical_cores / 2).max(4);

        let ideal_ctx_size = (full_prompt.len() as u32 / 3 + max_tokens as u32 + 128).min(self.global_ctx_limit);

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

        let n_past = chunks.eval_chunks(mtmd_ctx, &ctx, 0, 0, batch_size, true)
            .map_err(|e| format!("Ошибка eval_chunks: {:?}", e))?;

        progress_cb(50.0, "Генерация ответа...");

        let actual_min_p = model_params.min_p.max(0.0);
        let actual_rep_pen = model_params.repetition_penalty.max(1.0);
        let actual_temp = model_params.temperature.max(0.01);

        log_cb(format!(
            "🎛 Фактические параметры сэмплинга: Temp={:.2}, Top_K={}, Top_P={:.2}, Min_P={:.2}, Rep_Pen={:.2}, Pres_Pen={:.2}",
            actual_temp, model_params.top_k, model_params.top_p, actual_min_p, actual_rep_pen, model_params.presence_penalty
        ));

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

        let gen_start = Instant::now();
        let mut n_cur = n_past as i32;
        let mut generated_tokens = 0;
        let mut result_text = String::new();
        let mut generated_bytes: Vec<u8> = Vec::new();
        let mut past_tokens: Vec<llama_cpp_2::token::LlamaToken> = Vec::new();
        let mut gen_tokens: Vec<llama_cpp_2::token::LlamaToken> = Vec::new();
        let mut _stop_reason = "MAX_TOKENS";

        let mut stop_words: Vec<&str> = vec![
            "<|im_end|>", "<end_of_turn>", "</s>", "<|eot_id|>",
            "<turn>", "<|eot|>", "User:", "System:", "<eos>", "Yes",
            "<turn|>", "/end_of_turn>", "<step>", "<|end_of_text|>", "<｜end of sentence｜>",
            "</start_of_turn>"
        ];

        let words = actual_format.get_stop_words();
        for &w in &words {
            if !stop_words.contains(&w) {
                stop_words.push(w);
            }
        }

        while n_cur < ideal_ctx_size as i32 && generated_tokens < max_tokens as usize {
            if cancel_flag.load(Ordering::SeqCst) {
                _stop_reason = "CANCELLED";
                break;
            }

            let candidates_array = ctx.candidates_ith(0);
            let candidates = LlamaTokenDataArray::from_iter(candidates_array, false);
            let mut candidates_vec: Vec<(llama_cpp_2::token::LlamaToken, f32)> = candidates.data.iter().map(|d| (d.id(), d.logit())).collect();

            let penalty_last_n = 256.min(past_tokens.len());
            let last_tokens_slice = if past_tokens.len() > penalty_last_n { &past_tokens[past_tokens.len() - penalty_last_n..] } else { &past_tokens };
            let mut penalty_tokens = last_tokens_slice.to_vec();
            penalty_tokens.sort_unstable();
            penalty_tokens.dedup();

            for (id, logit) in candidates_vec.iter_mut() {
                if penalty_tokens.binary_search(id).is_ok() {
                    *logit -= model_params.presence_penalty;
                    if *logit <= 0.0 { *logit *= actual_rep_pen; }
                    else { *logit /= actual_rep_pen; }
                }
            }

            for (_, logit) in candidates_vec.iter_mut() { *logit /= actual_temp; }

            // ── ОПТИМИЗАЦИЯ СЭМПЛИНГА ──
            candidates_vec.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            let k = if actual_min_p >= 0.05 { 1000 } else { (model_params.top_k as usize).min(candidates_vec.len()).max(1) };
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

            let top_p_thresh = if actual_min_p >= 0.05 { 1.0 } else { model_params.top_p };
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

            let mut batch = LlamaBatch::new(batch_size as usize, 1);
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

        Ok(result_text)
    }

    #[cfg(not(feature = "mtmd"))]
    pub fn generate_chat_multimodal<F, L>(
        &self,
        _messages: &[LlmMessage],
        _attachments: &[ChatAttachment],
        _max_tokens: usize,
        _model_params: &ModelParams,
        _format_type: &str,
        _cancel_flag: Arc<AtomicBool>,
        mut _progress_cb: F,
        _log_cb: L,
    ) -> Result<String, String>
    where F: FnMut(f32, &str), L: Fn(String) {
        Err("Мультимодальный режим не поддерживается в этой сборке (отсутствует фича mtmd)".to_string())
    }
}