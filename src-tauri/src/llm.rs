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
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_calls: Option<Vec<SubCall>>,
    // Добавили поле для сохранения имени агента в "мыслях"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
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

// --- НОВЫЙ БЛОК: Чтение GGUF и рендеринг Jinja ---
fn skip_gguf_value(data: &[u8], mut offset: usize, val_type: u32) -> Option<usize> {
    match val_type {
        0 | 1 | 7 => Some(offset + 1), // UINT8, INT8, BOOL
        2 | 3 => Some(offset + 2), // UINT16, INT16
        4 | 5 | 6 => Some(offset + 4), // UINT32, INT32, FLOAT32
        10 | 11 | 12 => Some(offset + 8), // UINT64, INT64, FLOAT64
        8 => { // STRING
            if offset + 8 > data.len() { return None; }
            let len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
            Some(offset + 8 + len)
        },
        9 => { // ARRAY
            if offset + 4 > data.len() { return None; }
            let arr_type = u32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
            offset += 4;
            if offset + 8 > data.len() { return None; }
            let arr_len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
            offset += 8;
            for _ in 0..arr_len {
                offset = skip_gguf_value(data, offset, arr_type)?;
            }
            Some(offset)
        },
        _ => None
    }
}

fn extract_string_from_gguf(path: &str, target_key: &str) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = vec![0; 5 * 1024 * 1024]; // Читаем первые 5 МБ
    let bytes_read = file.read(&mut buffer).ok()?;
    let data = &buffer[..bytes_read];

    if data.len() < 24 || &data[0..4] != b"GGUF" { return None; }
    
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
        } else {
            offset = skip_gguf_value(data, offset, val_type)?;
        }
    }
    None
}

fn render_jinja_template(template_str: &str, messages: &[ChatMessage]) -> Result<String, String> {
    use minijinja::{Environment, context};
    let mut env = Environment::new();
    
    env.add_function("raise_exception", |msg: String| -> Result<String, minijinja::Error> {
        Err(minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, msg))
    });
    
    let safe_template = template_str
        .replace(".strip()", "|trim")
        .replace(".upper()", "|upper")
        .replace(".lower()", "|lower");
        
    env.add_template("chat", &safe_template).map_err(|e| e.to_string())?;
    let tmpl = env.get_template("chat").map_err(|e| e.to_string())?;
    
    let messages_val: Vec<serde_json::Value> = messages.iter().map(|m| {
        serde_json::json!({ "role": m.role, "content": m.content })
    }).collect();

    tmpl.render(context! {
        messages => messages_val,
        add_generation_prompt => true,
        bos_token => "<s>",
        eos_token => "</s>",
    }).map_err(|e| e.to_string())
}
// --------------------------------------------------------

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
            "<turn|>", "/end_of_turn>", "<step>", "<|end_of_text|>", "<｜end of sentence｜>"
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
        let messages = vec![
            ChatMessage { role: "system".to_string(), content: system_prompt.to_string(), sub_calls: None, agent_name: None },
            ChatMessage { role: "user".to_string(), content: user_prompt.to_string(), sub_calls: None, agent_name: None },
        ];
        self.generate_chat(&messages, max_tokens, "Auto", cancel_flag, progress_cb)
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
        
        let mut full_prompt = String::new();
        let mut stop_words: Vec<String> = Vec::new();

        // Попытка достать Jinja шаблон из GGUF
        if pf == PromptFormat::Auto {
            if let Some(template_str) = extract_string_from_gguf(&self.model_path, "tokenizer.chat_template") {
                if let Ok(rendered) = render_jinja_template(&template_str, messages) {
                    full_prompt = rendered;
                }
            }
        }

        // Если шаблон не найден или сломан, используем старый fallback
        if full_prompt.is_empty() {
            if pf == PromptFormat::Auto {
                pf = PromptFormat::detect_from_path(&self.model_path);
            }
            full_prompt = pf.format_messages(messages);
            let words = pf.get_stop_words();
            stop_words = words.into_iter().map(|s| s.to_string()).collect();
        }
        
        let stop_words_refs: Vec<&str> = stop_words.iter().map(|s| s.as_str()).collect();
        self.run_generation(&full_prompt, max_tokens, &stop_words_refs, cancel_flag, progress_cb)
    }
}