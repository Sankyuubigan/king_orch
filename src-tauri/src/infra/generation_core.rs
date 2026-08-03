//! Общий генерационный цикл для текстовой и мультимодальной генерации.
//!
//! Оба пути (llm.rs::run_generation и llm_multimodal.rs::generate_chat_multimodal)
//! содержат идентичный ~180-строчный цикл сэмплинга. Этот модуль содержит его
//! ОДИН раз. Различия (индекс кандидатов, управление батчем) передаются через замыкания.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use llama_cpp_2::model::{LlamaModel, Special};
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::token::data_array::LlamaTokenDataArray;
use llama_cpp_2::llama_batch::LlamaBatch;

use crate::infra::config::ModelParams;
use crate::infra::sampler::{self, DryParams, XtcParams};
use crate::infra::detokenizer::compute_stream_diff;
use super::llm_types::GenerationResult;

/// Общий генерационный цикл.
///
/// `get_candidates` — возвращает кандидатов из контекста.
///   - llm.rs: `ctx.candidates_ith(batch.n_tokens() - 1)`
///   - multimodal: `ctx.candidates_ith(0)`
///
/// `prepare_batch` — подготавливает батч и декодирует.
///   - llm.rs: `batch.clear(); batch.add(...); ctx.decode(&mut batch)`
///   - multimodal: `LlamaBatch::new(); batch.add(...); ctx.decode(&mut batch)`
pub(crate) fn run_generation_core<F, G, H>(
    ctx: &mut LlamaContext,
    model: &LlamaModel,
    max_tokens: usize,
    params: &ModelParams,
    ideal_ctx_size: u32,
    n_past: i32,
    stop_words: &[&str],
    cancel_flag: Arc<AtomicBool>,
    mut progress_cb: F,
    log_cb: &dyn Fn(String),
    stream_cb: &dyn Fn(String),
    mut get_candidates: G,
    mut prepare_batch: H,
) -> Result<GenerationResult, String>
where
    F: FnMut(f32, &str),
    G: FnMut(&LlamaBatch) -> LlamaTokenDataArray,
    H: FnMut(&mut LlamaBatch, llama_cpp_2::token::LlamaToken, i32) -> Result<(), String>,
{
    let actual_min_p = params.min_p.max(0.0);
    let actual_rep_pen = params.repetition_penalty.max(1.0);
    let actual_temp = params.temperature.max(0.01);

    let dry_params = DryParams {
        multiplier: params.dry_multiplier,
        base: params.dry_base,
        allowed_length: params.dry_allowed_length,
        penalty_last_n: params.dry_penalty_last_n,
    };
    let xtc_params = XtcParams {
        probability: params.xtc_probability,
        threshold: params.xtc_threshold,
        min_keep: 1,
    };

    let gen_start = Instant::now();
    let mut n_cur = n_past;
    let mut result_text = String::new();
    let mut generated_bytes: Vec<u8> = Vec::new();
    let mut generated_tokens = 0;
    let mut past_tokens: Vec<llama_cpp_2::token::LlamaToken> = Vec::new();
    let mut gen_tokens: Vec<llama_cpp_2::token::LlamaToken> = Vec::new();
    let mut dry_last_tokens: Vec<llama_cpp_2::token::LlamaToken> = Vec::new();
    let mut _stop_reason = "MAX_TOKENS";

    let mut batch = LlamaBatch::new(2048, 1);

    while n_cur < ideal_ctx_size as i32 && generated_tokens < max_tokens {
        if cancel_flag.load(Ordering::SeqCst) {
            _stop_reason = "CANCELLED";
            break;
        }

        let candidates = get_candidates(&batch);
        let mut candidates_vec: Vec<(llama_cpp_2::token::LlamaToken, f32)> =
            candidates.data.iter().map(|d| (d.id(), d.logit())).collect();

        // ── Presence/Repetition penalty ──
        let penalty_last_n = 256.min(past_tokens.len());
        let last_tokens_slice = if past_tokens.len() > penalty_last_n {
            &past_tokens[past_tokens.len() - penalty_last_n..]
        } else {
            &past_tokens
        };
        let mut penalty_tokens = last_tokens_slice.to_vec();
        penalty_tokens.sort_unstable();
        penalty_tokens.dedup();

        for (id, logit) in candidates_vec.iter_mut() {
            if penalty_tokens.binary_search(id).is_ok() {
                *logit -= params.presence_penalty;
                if *logit <= 0.0 {
                    *logit *= actual_rep_pen;
                } else {
                    *logit /= actual_rep_pen;
                }
            }
        }

        // ── DRY ──
        if dry_params.multiplier > 0.0 && dry_params.penalty_last_n != 0 {
            sampler::apply_dry(&mut candidates_vec, &dry_last_tokens, &dry_params, sampler::default_seq_breakers());
        }

        // ── Temperature ──
        for (_, logit) in candidates_vec.iter_mut() {
            *logit /= actual_temp;
        }

        // ── Top-K ──
        candidates_vec.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let k = if actual_min_p >= 0.05 {
            1000
        } else {
            (params.top_k as usize).min(candidates_vec.len()).max(1)
        };
        candidates_vec.truncate(k);

        // ── Softmax ──
        let max_logit = candidates_vec.first().map(|(_, l)| *l).unwrap_or(0.0);
        let mut sum_exp = 0.0;
        let mut probs: Vec<(llama_cpp_2::token::LlamaToken, f32)> = candidates_vec
            .into_iter()
            .map(|(id, logit)| {
                let p = (logit - max_logit).exp();
                sum_exp += p;
                (id, p)
            })
            .collect();
        for (_, p) in probs.iter_mut() {
            *p /= sum_exp;
        }

        // ── Min-P ──
        let max_prob = probs.first().map(|(_, p)| *p).unwrap_or(1.0);
        let min_p_thresh = max_prob * actual_min_p;
        probs.retain(|(_, p)| *p >= min_p_thresh);

        // ── Top-P ──
        let top_p_thresh = if actual_min_p >= 0.05 { 1.0 } else { params.top_p };
        let mut cumulative_prob = 0.0;
        let mut top_p_idx = probs.len();
        for (i, (_, p)) in probs.iter().enumerate() {
            cumulative_prob += *p;
            if cumulative_prob >= top_p_thresh {
                top_p_idx = i + 1;
                break;
            }
        }
        probs.truncate(top_p_idx);

        // ── Normalize ──
        let sum_prob: f32 = probs.iter().map(|(_, p)| *p).sum();
        for (_, p) in probs.iter_mut() {
            *p /= sum_prob;
        }

        // ── RNG + XTC ──
        static SEED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1337);
        let mut seed = SEED.load(Ordering::SeqCst);
        if seed == 1337 {
            seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
                .max(1);
        }
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        SEED.store(seed, Ordering::SeqCst);
        let r = (seed as f32) / (u32::MAX as f32);

        if xtc_params.probability > 0.0 && xtc_params.threshold <= 0.5 {
            sampler::apply_xtc(&mut probs, &xtc_params, r);
        }

        // ── Sample ──
        let mut cumulative = 0.0;
        let mut new_token = probs
            .last()
            .map(|(id, _)| *id)
            .unwrap_or_else(|| model.token_eos());
        for (id, p) in probs.iter() {
            cumulative += *p;
            if r <= cumulative {
                new_token = *id;
                break;
            }
        }

        if new_token == model.token_eos() {
            _stop_reason = "EOS";
            break;
        }
        past_tokens.push(new_token);
        gen_tokens.push(new_token);
        dry_last_tokens.push(new_token);
        if dry_last_tokens.len() > 256 {
            dry_last_tokens.remove(0);
        }

        // ── N-Gram loop detection ──
        let mut loop_detected = false;
        let g_len = gen_tokens.len();
        for l in 1..=32 {
            let required_repeats = match l {
                1 => 15,
                2 => 6,
                _ => 4,
            };
            if g_len >= l * required_repeats {
                let mut is_loop = true;
                let suffix = &gen_tokens[g_len - l..g_len];
                for i in 1..required_repeats {
                    let start = g_len - l * (i + 1);
                    let end = g_len - l * i;
                    if &gen_tokens[start..end] != suffix {
                        is_loop = false;
                        break;
                    }
                }
                if is_loop {
                    loop_detected = true;
                    break;
                }
            }
        }

        if loop_detected {
            log_cb("🛑 Сработала аппаратная защита N-Gram: обнаружено зацикливание фразы. Жесткое прерывание.".to_string());
            _stop_reason = "LOOP_DETECTED";
            break;
        }

        // ── Streaming detokenization (безопасный) ──
        if let Ok(bytes) = model.token_to_bytes(new_token, Special::Tokenize) {
            generated_bytes.extend_from_slice(&bytes);
            let current_text = String::from_utf8_lossy(&generated_bytes).into_owned();
            let diff = compute_stream_diff(&current_text, &result_text).to_string();
            if !diff.is_empty() {
                stream_cb(diff);
            }
            result_text = current_text;
        }

        // ── Stop words ──
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
        if should_stop {
            log_cb(format!("🛑 Стоп-слово '{}' на токене {}", matched_word, generated_tokens));
            _stop_reason = "STOP_WORD";
            break;
        }

        // ── Batch + Decode ──
        prepare_batch(&mut batch, new_token, n_cur)?;

        n_cur += 1;
        generated_tokens += 1;
        if generated_tokens % 20 == 0 {
            let gen_p = (generated_tokens as f32 / max_tokens as f32) * 50.0;
            progress_cb(50.0 + gen_p, &format!("Генерация: {} токенов...", generated_tokens));
        }
    }

    progress_cb(100.0, &format!("Готово ({} токенов)", generated_tokens));

    let gen_elapsed = gen_start.elapsed().as_secs_f64();
    let speed = if gen_elapsed > 0.0 {
        generated_tokens as f64 / gen_elapsed
    } else {
        0.0
    };
    log_cb(format!(
        "⚙️ Сгенерировано {} токенов за {:.1}с ({:.0} tok/s). Причина: {}",
        generated_tokens, gen_elapsed, speed, _stop_reason
    ));

    Ok(GenerationResult {
        text: result_text,
        stop_reason: _stop_reason.to_string(),
    })
}
