#![allow(deprecated)]

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
use serde::{Deserialize, Serialize};

use crate::infra::config::ModelParams;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub tool_name: String,
    pub arguments: String,
    pub result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubCall {
    pub agent_name: String,
    pub prompt: String,
    pub response: String,
    pub time_sec: f32,
    pub tool_calls: Vec<ToolCallInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_calls: Option<Vec<SubCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

impl ChatMessage {
    pub fn llm_role(&self) -> &str {
        match (self.msg_type.as_str(), self.author.as_deref()) {
            ("message", Some("user")) => "user",
            ("message", Some("system")) => "system",
            ("message", Some(_)) => "assistant",
            ("message", None) => "user",
            ("thought", _) => "user",
            _ => "user",
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum PromptFormat {
    Auto,
    ChatML,
    Gemma,
    Llama3,
}

impl PromptFormat {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "gemma" => PromptFormat::Gemma,
            "llama3" | "llama-3" => PromptFormat::Llama3,
            "chatml" => PromptFormat::ChatML,
            _ => PromptFormat::Auto,
        }
    }

    pub fn detect_from_path(path: &str) -> Self {
        let lower = path.to_lowercase();
        if lower.contains("gemma") { PromptFormat::Gemma }
        else if lower.contains("llama-3") || lower.contains("llama3") { PromptFormat::Llama3 }
        else { PromptFormat::ChatML }
    }

    pub fn format_messages(&self, messages: &[ChatMessage]) -> String {
        let mut full_prompt = String::new();
        match self {
            PromptFormat::ChatML | PromptFormat::Auto => {
                for msg in messages {
                    full_prompt.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", msg.llm_role(), msg.content));
                }
                full_prompt.push_str("<|im_start|>assistant\n");
            },
            PromptFormat::Gemma => {
                for msg in messages {
                    let role = match msg.llm_role() {
                        "assistant" => "model",
                        r => r,
                    };
                    full_prompt.push_str(&format!("<start_of_turn>{}\n{}<end_of_turn>\n", role, msg.content));
                }
                full_prompt.push_str("<start_of_turn>model\n");
            },
            PromptFormat::Llama3 => {
                for msg in messages {
                    full_prompt.push_str(&format!("<|start_header_id|>{}<|end_header_id|>\n\n{}<|eot_id|>", msg.llm_role(), msg.content));
                }
                full_prompt.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");
            }
        }
        full_prompt
    }
    
    pub fn get_stop_words(&self) -> Vec<&'static str> {
        match self {
            PromptFormat::ChatML | PromptFormat::Auto => vec!["<|im_end|>", "<|im_start|>"],
            PromptFormat::Gemma => vec!["<end_of_turn>", "<start_of_turn>"],
            PromptFormat::Llama3 => vec!["<|eot_id|>", "<|start_header_id|>"],
        }
    }
}

fn read_gguf_header(path: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = vec![0; 5 * 1024 * 1024];
    let bytes_read = file.read(&mut buffer).ok()?;
    let data = &buffer[..bytes_read];
    if data.len() < 24 || &data[0..4] != b"GGUF" { return None; }
    Some(data.to_vec())
}

fn skip_gguf_value(data: &[u8], mut offset: usize, val_type: u32) -> Option<usize> {
    match val_type {
        0 | 1 | 7 => Some(offset + 1), 
        2 | 3 => Some(offset + 2), 
        4 | 5 | 6 => Some(offset + 4), 
        10 | 11 | 12 => Some(offset + 8), 
        8 => { 
            if offset + 8 > data.len() { return None; }
            let len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
            Some(offset + 8 + len)
        },
        9 => { 
            if offset + 4 > data.len() { return None; }
            let arr_type = u32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
            offset += 4;
            if offset + 8 > data.len() { return None; }
            let arr_len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
            offset += 8;
            for _ in 0..arr_len { offset = skip_gguf_value(data, offset, arr_type)?; }
            Some(offset)
        },
        _ => None
    }
}

pub fn extract_string_from_gguf(path: &str, target_key: &str) -> Option<String> {
    let data = read_gguf_header(path)?;
    let kv_count = u64::from_le_bytes(data[16..24].try_into().unwrap());
    let mut offset = 24;
    for _ in 0..kv_count {
        if offset + 8 > data.len() { break; }
        let key_len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
        offset += 8;
        if offset + key_len > data.len() { break; }
        let key = String::from_utf8_lossy(&data[offset..offset+key_len]);
        offset += key_len;
        if offset + 4 > data.len() { break; }
        let val_type = u32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
        offset += 4;
        if key == target_key && val_type == 8 {
            if offset + 8 > data.len() { break; }
            let val_len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
            offset += 8;
            if offset + val_len > data.len() { break; }
            return String::from_utf8(data[offset..offset+val_len].to_vec()).ok();
        } else { offset = skip_gguf_value(&data, offset, val_type)?; }
    }
    None
}

pub fn extract_f32_from_gguf(path: &str, target_key: &str) -> Option<f32> {
    let data = read_gguf_header(path)?;
    let kv_count = u64::from_le_bytes(data[16..24].try_into().unwrap());
    let mut offset = 24;
    for _ in 0..kv_count {
        if offset + 8 > data.len() { break; }
        let key_len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
        offset += 8;
        if offset + key_len > data.len() { break; }
        let key = String::from_utf8_lossy(&data[offset..offset+key_len]);
        offset += key_len;
        if offset + 4 > data.len() { break; }
        let val_type = u32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
        offset += 4;
        if key == target_key && val_type == 6 {
            if offset + 4 > data.len() { break; }
            return Some(f32::from_le_bytes(data[offset..offset+4].try_into().unwrap()));
        } else { offset = skip_gguf_value(&data, offset, val_type)?; }
    }
    None
}

pub fn extract_u32_from_gguf(path: &str, target_key: &str) -> Option<u32> {
    let data = read_gguf_header(path)?;
    let kv_count = u64::from_le_bytes(data[16..24].try_into().unwrap());
    let mut offset = 24;
    for _ in 0..kv_count {
        if offset + 8 > data.len() { break; }
        let key_len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
        offset += 8;
        if offset + key_len > data.len() { break; }
        let key = String::from_utf8_lossy(&data[offset..offset+key_len]);
        offset += key_len;
        if offset + 4 > data.len() { break; }
        let val_type = u32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
        offset += 4;
        if key == target_key && val_type == 4 {
            if offset + 4 > data.len() { break; }
            return Some(u32::from_le_bytes(data[offset..offset+4].try_into().unwrap()));
        } else { offset = skip_gguf_value(&data, offset, val_type)?; }
    }
    None
}

fn render_jinja_template(template_str: &str, messages: &[ChatMessage]) -> Result<String, String> {
    use minijinja::{Environment, context};
    let mut env = Environment::new();
    env.add_function("raise_exception", |msg: String| -> Result<String, minijinja::Error> {
        Err(minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, msg))
    });
    let safe_template = template_str.replace(".strip()", "|trim").replace(".upper()", "|upper").replace(".lower()", "|lower");
    env.add_template("chat", &safe_template).map_err(|e| e.to_string())?;
    let tmpl = env.get_template("chat").map_err(|e| e.to_string())?;
    let messages_val: Vec<serde_json::Value> = messages.iter().map(|m| {
        serde_json::json!({ "role": m.llm_role(), "content": m.content })
    }).collect();
    tmpl.render(context! { messages => messages_val, add_generation_prompt => true, bos_token => "<s>", eos_token => "</s>" }).map_err(|e| e.to_string())
}

pub struct LlamaEngine {
    backend: LlamaBackend,
    model: LlamaModel,
    n_ctx: u32,
    kv_quant: bool,
    model_path: String,
}

impl LlamaEngine {
    pub fn new<L: Fn(String)>(model_path: &str, n_ctx: u32, kv_quant: bool, log_cb: L) -> Result<Self, String> {
        log_cb("⚡ Инициализация llama.cpp...".to_string());
        let backend = LlamaBackend::init().map_err(|e| e.to_string())?;
        let gpu_layers: u32 = 999;
        let mut model_params = LlamaModelParams::default();
        model_params = model_params.with_n_gpu_layers(gpu_layers);
        log_cb(format!("⚙️ GPU слоёв: {} (попытка полного оффлоуда)", gpu_layers));
        let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
            .map_err(|e| format!("Ошибка загрузки модели: {}", e))?;
        log_cb("✅ Модель загружена успешно".to_string());
        Ok(Self { backend, model, n_ctx, kv_quant, model_path: model_path.to_string() })
    }

    fn run_generation<F, L>(
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
        let batch_size = 512; 
        let logical_cores = std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(8);
        let threads = (logical_cores / 2).max(4);

        let mut ctx_params = LlamaContextParams::default();
        ctx_params = ctx_params.with_n_ctx(NonZeroU32::new(self.n_ctx));
        ctx_params = ctx_params.with_n_batch(batch_size as u32);
        ctx_params = ctx_params.with_n_threads(threads);
        ctx_params = ctx_params.with_n_threads_batch(threads);
        ctx_params = ctx_params.with_flash_attention_policy(1);

        if self.kv_quant {
            ctx_params = ctx_params.with_type_k(KvCacheType::Q8_0).with_type_v(KvCacheType::Q8_0);
        }

        let mut ctx = self.model.new_context(&self.backend, ctx_params)
            .map_err(|e| format!("Ошибка создания контекста: {}", e))?;

        let tokens = self.model.str_to_token(full_prompt, AddBos::Always)
            .map_err(|e| format!("Ошибка токенизации: {}", e))?;

        if tokens.is_empty() { return Err("Промпт пуст".to_string()); }
        if tokens.len() as u32 >= self.n_ctx {
            return Err(format!("Текст слишком большой ({} токенов) для контекста ({}).", tokens.len(), self.n_ctx));
        }

        log_cb(format!("📐 Промпт: {} токенов, max_gen={}", tokens.len(), max_tokens));

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

        log_cb(format!("⏱ Промпт обработан за {:.1}с", prompt_start.elapsed().as_secs_f64()));

        let gen_start = Instant::now();

        let mut n_cur = n_past;
        let mut result_text = String::new();
        let mut generated_tokens = 0;

        let mut stop_words = vec![
            "<|im_end|>", "<end_of_turn>", "</s>", "<|eot_id|>", 
            "<turn>", "<|eot|>", "User:", "System:", "<eos>", "Yes",
            "<turn|>", "/end_of_turn>", "<step>", "<|end_of_text|>", "<｜end of sentence｜>",
            "</start_of_turn>"
        ];
        stop_words.extend_from_slice(custom_stop_words);

        while n_cur < self.n_ctx as i32 && generated_tokens < max_tokens {
            if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }

            let candidates_array = ctx.candidates_ith(batch.n_tokens() - 1);
            let candidates = LlamaTokenDataArray::from_iter(candidates_array, false);
            let mut candidates_vec: Vec<(llama_cpp_2::token::LlamaToken, f32)> = candidates.data.iter().map(|d| (d.id(), d.logit())).collect();

            let penalty_last_n = 256.min(past_tokens.len());
            let last_tokens_slice = if past_tokens.len() > penalty_last_n { &past_tokens[past_tokens.len() - penalty_last_n..] } else { &past_tokens };
            let mut penalty_tokens = last_tokens_slice.to_vec();
            penalty_tokens.sort(); penalty_tokens.dedup();

            for (id, logit) in candidates_vec.iter_mut() {
                if penalty_tokens.binary_search(id).is_ok() {
                    *logit -= params.presence_penalty;
                    if *logit <= 0.0 { *logit *= params.repetition_penalty; } 
                    else { *logit /= params.repetition_penalty; }
                }
            }

            let temp = params.temperature.max(0.01);
            for (_, logit) in candidates_vec.iter_mut() { *logit /= temp; }

            let max_logit = candidates_vec.iter().map(|(_, l)| *l).fold(f32::NEG_INFINITY, f32::max);
            let mut sum_exp = 0.0;
            let mut probs: Vec<(llama_cpp_2::token::LlamaToken, f32)> = candidates_vec.into_iter().map(|(id, logit)| {
                let p = (logit - max_logit).exp(); sum_exp += p; (id, p)
            }).collect();
            for (_, p) in probs.iter_mut() { *p /= sum_exp; }
            probs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            let k = (params.top_k as usize).min(probs.len()).max(1);
            probs.truncate(k);
            let max_prob = probs.first().map(|(_, p)| *p).unwrap_or(1.0);
            let min_p_thresh = max_prob * params.min_p;
            probs.retain(|(_, p)| *p >= min_p_thresh);

            let mut cumulative_prob = 0.0;
            let mut top_p_idx = probs.len();
            for (i, (_, p)) in probs.iter().enumerate() {
                cumulative_prob += *p;
                if cumulative_prob >= params.top_p { top_p_idx = i + 1; break; }
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

            if new_token == self.model.token_eos() { break; }
            past_tokens.push(new_token);

            if let Ok(bytes) = self.model.token_to_bytes(new_token, Special::Tokenize) {
                result_text.push_str(&String::from_utf8_lossy(&bytes));
            }

            let mut should_stop = false;
            for word in stop_words.iter() {
                if result_text.contains(word) { result_text = result_text.replace(word, "").trim().to_string(); should_stop = true; break; }
            }
            if should_stop { break; }

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
        log_cb(format!("⚙️ Сгенерировано {} токенов за {:.1}с ({:.0} tok/s)", generated_tokens, gen_elapsed, speed));

        if generated_tokens > 50 {
            let char_count: usize = result_text.chars().count();
            let take = 300.min(char_count);
            let preview: String = result_text.chars().take(take).collect();
            log_cb(format!("📝 Первые {} символов: {}", take, preview.replace('\n', "\\n")));
        }
        Ok(result_text)
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
        let mut pf = PromptFormat::from_str(format_type);
        let mut full_prompt = String::new();
        let mut stop_words: Vec<String> = Vec::new();

        if pf == PromptFormat::Auto {
            if let Some(template_str) = extract_string_from_gguf(&self.model_path, "tokenizer.chat_template") {
                if let Ok(rendered) = render_jinja_template(&template_str, messages) { full_prompt = rendered; }
            }
        }

        if full_prompt.is_empty() {
            if pf == PromptFormat::Auto { pf = PromptFormat::detect_from_path(&self.model_path); }
            full_prompt = pf.format_messages(messages);
            let words = pf.get_stop_words(); stop_words = words.into_iter().map(|s| s.to_string()).collect();
        }
        
        let stop_words_refs: Vec<&str> = stop_words.iter().map(|s| s.as_str()).collect();
        self.run_generation(&full_prompt, max_tokens, model_params, &stop_words_refs, cancel_flag, progress_cb, log_cb)
    }
}