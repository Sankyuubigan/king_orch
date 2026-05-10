#![allow(deprecated)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::num::NonZeroU32;

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::context::params::{LlamaContextParams, KvCacheType};
use llama_cpp_2::model::AddBos;
use llama_cpp_2::model::Special;
use llama_cpp_2::token::data_array::LlamaTokenDataArray;
use llama_cpp_2::llama_batch::LlamaBatch;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubCall {
    pub agent_name: String,
    pub prompt: String,
    pub response: String,
    pub time_sec: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "system", "user", "assistant"
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_calls: Option<Vec<SubCall>>,
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
        if lower.contains("gemma") {
            PromptFormat::Gemma
        } else if lower.contains("llama-3") || lower.contains("llama3") {
            PromptFormat::Llama3
        } else {
            PromptFormat::ChatML 
        }
    }

    pub fn format_messages(&self, messages: &[ChatMessage]) -> String {
        let mut full_prompt = String::new();
        match self {
            PromptFormat::ChatML | PromptFormat::Auto => {
                for msg in messages {
                    full_prompt.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", msg.role, msg.content));
                }
                full_prompt.push_str("<|im_start|>assistant\n");
            },
            PromptFormat::Gemma => {
                for msg in messages {
                    let role = if msg.role == "system" { "user" } else { if msg.role == "assistant" { "model" } else { &msg.role } };
                    full_prompt.push_str(&format!("<start_of_turn>{}\n{}<end_of_turn>\n", role, msg.content));
                }
                full_prompt.push_str("<start_of_turn>model\n");
            },
            PromptFormat::Llama3 => {
                for msg in messages {
                    full_prompt.push_str(&format!("<|start_header_id|>{}<|end_header_id|>\n\n{}<|eot_id|>", msg.role, msg.content));
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

pub struct LlamaEngine {
    backend: LlamaBackend,
    model: LlamaModel,
    n_ctx: u32,
    kv_quant: bool,
    model_path: String,
}

impl LlamaEngine {
    pub fn new(model_path: &str, n_ctx: u32, kv_quant: bool) -> Result<Self, String> {
        let backend = LlamaBackend::init().map_err(|e| e.to_string())?;
        
        let mut model_params = LlamaModelParams::default();
        model_params = model_params.with_n_gpu_layers(999);

        let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
            .map_err(|e| format!("Ошибка загрузки модели: {}", e))?;

        Ok(Self {
            backend,
            model,
            n_ctx,
            kv_quant,
            model_path: model_path.to_string(),
        })
    }

    pub fn count_tokens(&self, text: &str) -> Result<usize, String> {
        let tokens = self.model.str_to_token(text, AddBos::Always)
            .map_err(|e| format!("Ошибка токенизации: {}", e))?;
        Ok(tokens.len())
    }

    fn run_generation<F>(
        &self,
        full_prompt: &str,
        max_tokens: usize,
        custom_stop_words: &[&str],
        cancel_flag: Arc<AtomicBool>,
        mut progress_cb: F,
    ) -> Result<String, String>
    where
        F: FnMut(f32, &str),
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
            ctx_params = ctx_params
                .with_type_k(KvCacheType::Q8_0)
                .with_type_v(KvCacheType::Q8_0);
        }

        let mut ctx = self.model.new_context(&self.backend, ctx_params)
            .map_err(|e| format!("Ошибка создания контекста: {}", e))?;

        let tokens = self.model.str_to_token(full_prompt, AddBos::Always)
            .map_err(|e| format!("Ошибка токенизации: {}", e))?;

        if tokens.is_empty() { return Err("Промпт пуст".to_string()); }
        if tokens.len() as u32 >= self.n_ctx {
            return Err(format!("Текст слишком большой ({} токенов) для контекста ({}).", tokens.len(), self.n_ctx));
        }

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

        let mut n_cur = n_past;
        let mut result_text = String::new();
        let mut generated_tokens = 0;

        let mut stop_words = vec![
            "<|im_end|>", "<end_of_turn>", "</s>", "<|eot_id|>", 
            "<turn>", "<|eot|>", "User:", "System:", "<eos>", "<|endoftext|>",
            "<turn|>", "/end_of_turn>", "<step>"
        ];
        stop_words.extend_from_slice(custom_stop_words);

        while n_cur < self.n_ctx as i32 && generated_tokens < max_tokens {
            if cancel_flag.load(Ordering::SeqCst) { return Err("Прервано пользователем".to_string()); }

            let candidates = LlamaTokenDataArray::from_iter(ctx.candidates_ith(batch.n_tokens() - 1), false);
            let new_token = candidates.data.iter()
                .max_by(|a, b| a.logit().partial_cmp(&b.logit()).unwrap_or(std::cmp::Ordering::Equal))
                .map(|d| d.id())
                .unwrap_or_else(|| self.model.token_eos());

            if new_token == self.model.token_eos() { break; }

            if let Ok(bytes) = self.model.token_to_bytes(new_token, Special::Tokenize) {
                result_text.push_str(&String::from_utf8_lossy(&bytes));
            }

            let mut should_stop = false;
            for word in stop_words.iter() {
                if result_text.contains(word) {
                    result_text = result_text.replace(word, "").trim().to_string();
                    should_stop = true;
                    break;
                }
            }
            if should_stop { break; }

            batch.clear();
            batch.add(new_token, n_cur, &[0], true).map_err(|e| e.to_string())?;
            ctx.decode(&mut batch).map_err(|e| e.to_string())?;
            
            n_cur += 1;
            generated_tokens += 1;

            if generated_tokens % 20 == 0 {
                let gen_p = (generated_tokens as f32 / max_tokens as f32) * 50.0;
                progress_cb(50.0 + gen_p, &format!("Генерация: {} токенов...", generated_tokens));
            }
        }
        progress_cb(100.0, &format!("Готово ({} токенов)", generated_tokens));
        Ok(result_text)
    }

    pub fn generate<F>(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_tokens: usize,
        _temperature: f32,
        cancel_flag: Arc<AtomicBool>,
        progress_cb: F,
    ) -> Result<String, String>
    where F: FnMut(f32, &str) {
        let full_prompt = format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
            system_prompt, user_prompt
        );
        self.run_generation(&full_prompt, max_tokens, &["<|im_end|>"], cancel_flag, progress_cb)
    }

    pub fn generate_chat<F>(
        &self,
        messages: &[ChatMessage],
        max_tokens: usize,
        format_type: &str,
        cancel_flag: Arc<AtomicBool>,
        progress_cb: F,
    ) -> Result<String, String>
    where F: FnMut(f32, &str) {
        let mut pf = PromptFormat::from_str(format_type);
        
        if pf == PromptFormat::Auto {
            pf = PromptFormat::detect_from_path(&self.model_path);
        }
        
        let full_prompt = pf.format_messages(messages);
        let stop_words = pf.get_stop_words();
        
        self.run_generation(&full_prompt, max_tokens, &stop_words, cancel_flag, progress_cb)
    }
}