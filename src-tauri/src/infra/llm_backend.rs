use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri_plugin_cloud_routers::router::client::{self, ChatMessage, ChatRequest};
use tauri_plugin_llama_engine::engine::llm_types::GenerationResult;
use tauri_plugin_llama_engine::engine::{
    ChatAttachment, GrammarSpec, LlmMessage, LlamaEngine, ModelParams, ToolCall, ToolDefinition,
};

#[derive(Clone, Debug)]
pub struct CloudEndpoint {
    pub base_url: String,
    pub api_key: Option<String>,
}

fn attachment_data_url(attachment: &ChatAttachment) -> Result<String, String> {
    if !attachment.data_base64.is_empty() {
        return Ok(format!(
            "data:{};base64,{}",
            attachment.mime_type, attachment.data_base64
        ));
    }
    if attachment.is_dir.unwrap_or(false) {
        return Err(format!("Папку нельзя передать модели как файл: {}", attachment.file_name));
    }
    let path = attachment
        .file_path
        .as_deref()
        .ok_or_else(|| format!("У вложения '{}' нет ни пути, ни данных", attachment.file_name))?;
    let bytes = fs::read(path).map_err(|error| {
        log::error!("Не удалось прочитать вложение {:?}: {}", path, error);
        format!("Не удалось прочитать вложение '{}': {}", attachment.file_name, error)
    })?;
    if bytes.is_empty() {
        return Err(format!("Вложение '{}' пустое", attachment.file_name));
    }
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{};base64,{}", attachment.mime_type, encoded))
}

fn materialize_attachments(
    attachments: Option<&[ChatAttachment]>,
) -> Result<Option<Vec<ChatAttachment>>, String> {
    let Some(items) = attachments else {
        return Ok(None);
    };
    let mut materialized = Vec::with_capacity(items.len());
    for item in items {
        let mut copy = item.clone();
        if copy.data_base64.is_empty() {
            let data_url = attachment_data_url(item)?;
            let encoded = data_url
                .split_once(",")
                .map(|(_, value)| value)
                .ok_or_else(|| "Не удалось получить base64 вложения".to_string())?;
            copy.data_base64 = encoded.to_string();
        }
        materialized.push(copy);
    }
    Ok(Some(materialized))
}

struct CloudEngine {
    endpoint: CloudEndpoint,
    model: String,
    stream_cb: Arc<dyn Fn(String) + Send + Sync>,
    pending_tools: Mutex<Option<Vec<ToolDefinition>>>,
    requests: Arc<AtomicU64>,
}

impl CloudEngine {
    fn generate<F, L>(
        &self,
        messages: &[LlmMessage],
        max_tokens: usize,
        params: &ModelParams,
        tool_choice: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
        attachments: Option<&[ChatAttachment]>,
        mut progress_cb: F,
        log_cb: L,
    ) -> Result<GenerationResult, String>
    where
        F: FnMut(f32, &str),
        L: Fn(String),
    {
        let tools = match tools {
            Some(tools) => Some(
                tools
                    .into_iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("Не удалось сериализовать tools 9Router: {}", e))?,
            ),
            None => None,
        };
        let mut messages: Vec<ChatMessage> = messages
            .iter()
            .map(|message| ChatMessage {
                role: message.role.clone(),
                content: serde_json::Value::String(message.content.clone()),
                tool_calls: message.tool_calls.clone(),
                tool_call_id: message.tool_call_id.clone(),
            })
            .collect();
        if let Some(items) = attachments.filter(|items| !items.is_empty()) {
            let mut parts = Vec::with_capacity(items.len() + 1);
            let last_user = messages.iter().rposition(|message| message.role == "user");
            if let Some(index) = last_user {
                if let Some(text) = messages[index].content.as_str() {
                    parts.push(serde_json::json!({"type": "text", "text": text}));
                }
            }
            for attachment in items {
                parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": {"url": attachment_data_url(attachment)?}
                }));
            }
            if let Some(index) = last_user {
                messages[index].content = serde_json::Value::Array(parts);
            } else {
                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: serde_json::Value::Array(parts),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }
        let request = ChatRequest {
            model: self.model.clone(),
            messages,
            max_tokens: Some(max_tokens as u32),
            temperature: Some(params.temperature),
            tools,
            tool_choice: tool_choice.map(str::to_string),
            stream: true,
        };
        log_cb(format!("☁️ 9Router: один вызов модели «{}»", self.model));
        let stream_cb = self.stream_cb.clone();
        // Запрос засчитывается ТОЛЬКО по факту ответа роутера: обрыв, таймаут,
        // 4xx/5xx (client.rs отдаёт их как Err) — до провайдера дело не дошло,
        // квота не потрачена. Обрезанный по `length` ответ — реально оплаченный
        // расход, приходит Ok и считается.
        let result = match client::chat_completion_stream_result(
            &self.endpoint.base_url,
            self.endpoint.api_key.as_deref(),
            &request,
            |delta| {
                if !delta.content.is_empty() {
                    stream_cb(delta.content);
                }
                if !delta.reasoning.is_empty() {
                    stream_cb(delta.reasoning);
                }
            },
        ) {
            Ok(result) => result,
            Err(error) => {
                log_cb(format!(
                    "☁️ [cloud-counter] Ответ от облака не получен — запрос НЕ засчитан: {}",
                    error
                ));
                return Err(error);
            }
        };
        let total = self.requests.fetch_add(1, Ordering::Relaxed) + 1;
        log_cb(format!(
            "☁️ [cloud-counter] Засчитано запросов к облаку за этот чат: {}",
            total
        ));
        progress_cb(1.0, "done");
        let tool_calls: Vec<ToolCall> = result
            .tool_calls
            .into_iter()
            .map(|call| ToolCall {
                id: call.id,
                name: call.name,
                arguments: call.arguments,
            })
            .collect();
        let text = if result.reasoning.trim().is_empty() {
            result.content
        } else {
            format!("{}{}", &result.reasoning, &result.content)
        };
        Ok(GenerationResult {
            text,
            stop_reason: if tool_calls.is_empty() {
                result.finish_reason
            } else {
                "tool_calls".to_string()
            },
            reasoning: result.reasoning,
            tool_calls,
            metrics: Default::default(),
        })
    }
}

pub struct LlmEngine {
    backend: LlmBackend,
}

enum LlmBackend {
    Local(LlamaEngine),
    Cloud(CloudEngine),
}

impl LlmEngine {
    pub fn local(engine: LlamaEngine) -> Self {
        Self {
            backend: LlmBackend::Local(engine),
        }
    }

    pub fn cloud(
        endpoint: CloudEndpoint,
        model: String,
        stream_cb: Arc<dyn Fn(String) + Send + Sync>,
        requests: Arc<AtomicU64>,
    ) -> Self {
        Self {
            backend: LlmBackend::Cloud(CloudEngine {
                endpoint,
                model,
                stream_cb,
                pending_tools: Mutex::new(None),
                requests,
            }),
        }
    }

    pub fn is_cloud(&self) -> bool {
        matches!(self.backend, LlmBackend::Cloud(_))
    }

    /// Умеет ли движок ПРИНУДИТЕЛЬНО ограничивать форму ответа (GBNF/JSON-schema).
    ///
    /// Локальный llama-server — да. Облачный роутер — нет: у него нет поля
    /// грамматики в запросе, `set_grammar` для него no-op. Любой механизм,
    /// который держится на принудительной грамматике (двухфазное мышление,
    /// JSON-повтор Phase 2, финальный вердиктный проход), ОБЯЗАН спросить это
    /// здесь. Иначе он молча превращается в платный повтор без механизма —
    /// именно так в облаке появился второй запрос на агента.
    pub fn supports_grammar(&self) -> bool {
        match &self.backend {
            LlmBackend::Local(_) => true,
            LlmBackend::Cloud(_) => false,
        }
    }

    pub fn model_path(&self) -> &str {
        match &self.backend {
            LlmBackend::Local(engine) => &engine.model_path,
            LlmBackend::Cloud(engine) => &engine.model,
        }
    }

    pub fn global_ctx_limit(&self) -> u32 {
        match &self.backend {
            LlmBackend::Local(engine) => engine.global_ctx_limit,
            LlmBackend::Cloud(_) => 128_000,
        }
    }

    pub fn get_tokens_count(
        &self,
        messages: &[LlmMessage],
        format_type: &str,
    ) -> Result<usize, String> {
        match &self.backend {
            LlmBackend::Local(engine) => engine.get_tokens_count(messages, format_type),
            LlmBackend::Cloud(_) => Ok(messages
                .iter()
                .map(|message| message.content.chars().count() / 2 + 4)
                .sum()),
        }
    }

    pub fn engine_mode(&self) -> String {
        match &self.backend {
            LlmBackend::Local(engine) => engine.engine_mode(),
            LlmBackend::Cloud(_) => "cloud".to_string(),
        }
    }

    pub fn tok_per_sec(&self) -> f64 {
        match &self.backend {
            LlmBackend::Local(engine) => engine.tok_per_sec(),
            LlmBackend::Cloud(_) => 0.0,
        }
    }

    pub fn engine_mode_detail(&self) -> String {
        match &self.backend {
            LlmBackend::Local(engine) => engine.engine_mode_detail(),
            LlmBackend::Cloud(_) => "9Router".to_string(),
        }
    }

    pub fn is_alive(&self) -> bool {
        match &self.backend {
            LlmBackend::Local(engine) => engine.is_alive(),
            LlmBackend::Cloud(_) => true,
        }
    }

    pub fn is_multimodal(&self) -> bool {
        match &self.backend {
            LlmBackend::Local(engine) => engine.is_multimodal(),
            LlmBackend::Cloud(_) => false,
        }
    }

    pub fn set_grammar(&self, spec: Option<GrammarSpec>) {
        if let LlmBackend::Local(engine) = &self.backend {
            engine.set_grammar(spec);
        }
    }

    pub fn set_tools(&self, tools: Option<Vec<ToolDefinition>>) {
        match &self.backend {
            LlmBackend::Local(engine) => engine.set_tools(tools),
            LlmBackend::Cloud(engine) => {
                *engine.pending_tools.lock().expect("9Router tools lock poisoned") = tools;
            }
        }
    }

    pub fn generate_chat<F, L>(
        &self,
        messages: &[LlmMessage],
        max_tokens: usize,
        params: &ModelParams,
        format_type: &str,
        disable_reasoning: bool,
        cancel_flag: Arc<AtomicBool>,
        ctx_label: &str,
        tool_choice: Option<&str>,
        progress_cb: F,
        log_cb: L,
    ) -> Result<GenerationResult, String>
    where
        F: FnMut(f32, &str),
        L: Fn(String),
    {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err("Прервано пользователем".to_string());
        }
        match &self.backend {
            LlmBackend::Local(engine) => engine.generate_chat(
                messages,
                max_tokens,
                params,
                format_type,
                disable_reasoning,
                cancel_flag,
                ctx_label,
                tool_choice,
                progress_cb,
                log_cb,
            ),
            LlmBackend::Cloud(engine) => {
                let tools = engine
                    .pending_tools
                    .lock()
                    .expect("9Router tools lock poisoned")
                    .take();
                engine.generate(
                    messages,
                    max_tokens,
                    params,
                    tool_choice,
                    tools,
                    None,
                    progress_cb,
                    log_cb,
                )
            }
        }
    }

    pub fn run_chat_completions<F, L>(
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
        tool_choice: Option<&str>,
        tools: Option<&[ToolDefinition]>,
        progress_cb: F,
        log_cb: L,
    ) -> Result<GenerationResult, String>
    where
        F: FnMut(f32, &str),
        L: Fn(String),
    {
        let materialized_attachments = if matches!(self.backend, LlmBackend::Local(_)) {
            materialize_attachments(attachments)?
        } else {
            None
        };
        match &self.backend {
            LlmBackend::Local(engine) => engine.run_chat_completions(
                messages,
                materialized_attachments.as_deref(),
                max_tokens,
                params,
                stop_words,
                grammar,
                disable_reasoning,
                cancel_flag,
                ctx_label,
                tool_choice,
                tools,
                progress_cb,
                log_cb,
            ),
            LlmBackend::Cloud(engine) => {
                if cancel_flag.load(Ordering::SeqCst) {
                    return Err("Прервано пользователем".to_string());
                }
                let _ = (stop_words, grammar, disable_reasoning, ctx_label);
                engine.generate(
                    messages,
                    max_tokens,
                    params,
                    tool_choice,
                    tools.map(|items| items.to_vec()),
                    attachments,
                    progress_cb,
                    log_cb,
                )
            }
        }
    }

    pub fn generate_chat_multimodal<F, L>(
        &self,
        _messages: &[LlmMessage],
        _attachments: &[ChatAttachment],
        _max_tokens: usize,
        _params: &ModelParams,
        _format_type: &str,
        _cancel_flag: Arc<AtomicBool>,
        _ctx_label: &str,
        _tool_choice: Option<&str>,
        _progress_cb: F,
        _log_cb: L,
    ) -> Result<GenerationResult, String>
    where
        F: FnMut(f32, &str),
        L: Fn(String),
    {
        let materialized_attachments = materialize_attachments(Some(_attachments))?;
        match &self.backend {
            LlmBackend::Local(engine) => engine.generate_chat_multimodal(
                _messages,
                materialized_attachments.as_deref().unwrap_or_default(),
                _max_tokens,
                _params,
                _format_type,
                _cancel_flag,
                _ctx_label,
                _tool_choice,
                _progress_cb,
                _log_cb,
            ),
            LlmBackend::Cloud(engine) => {
                let tools = engine
                    .pending_tools
                    .lock()
                    .expect("9Router tools lock poisoned")
                    .take();
                engine.generate(
                    _messages,
                    _max_tokens,
                    _params,
                    _tool_choice,
                    tools,
                    Some(_attachments),
                    _progress_cb,
                    _log_cb,
                )
            }
        }
    }

    pub fn suspend_for_external_compute(&self) -> bool {
        match &self.backend {
            LlmBackend::Local(engine) => engine.suspend_for_external_compute(),
            LlmBackend::Cloud(_) => false,
        }
    }

    pub fn resume_after_external_compute(&self) -> Result<(), String> {
        match &self.backend {
            LlmBackend::Local(engine) => engine.resume_after_external_compute(),
            LlmBackend::Cloud(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_backend_has_no_local_multimodal_or_vram_mode() {
        let engine = LlmEngine::cloud(
            CloudEndpoint {
                base_url: "http://127.0.0.1:20128".to_string(),
                api_key: None,
            },
            "combo".to_string(),
            Arc::new(|_| {}),
            Arc::new(AtomicU64::new(0)),
        );
        assert!(engine.is_cloud());
        assert_eq!(engine.model_path(), "combo");
        // Облако не умеет принуждать форму ответа — на этой capability держатся
        // все решения оркестратора про грамматикозависимые механизмы.
        assert!(!engine.supports_grammar());
        assert!(!engine.is_multimodal());
        assert_eq!(engine.engine_mode(), "cloud");
        assert_eq!(engine.engine_mode_detail(), "9Router");
        assert_eq!(engine.tok_per_sec(), 0.0);
    }

    /// Конструктор облачного движка для тестов счётчика: считает только то,
    /// что реально ушло в роутер.
    fn cloud_with_counter(base_url: &str, counter: Arc<AtomicU64>) -> LlmEngine {
        LlmEngine::cloud(
            CloudEndpoint {
                base_url: base_url.to_string(),
                api_key: None,
            },
            "combo".to_string(),
            Arc::new(|_| {}),
            counter,
        )
    }

    fn one_user_message() -> Vec<LlmMessage> {
        vec![LlmMessage {
            role: "user".to_string(),
            content: "привет".to_string(),
            ..Default::default()
        }]
    }

    /// Ответ не получен (порт закрыт) → квота не потрачена, счётчик НЕ растёт.
    #[test]
    fn cloud_request_without_response_is_not_counted() {
        let counter = Arc::new(AtomicU64::new(0));
        let engine = cloud_with_counter("http://127.0.0.1:1", counter.clone());

        let result = engine.run_chat_completions(
            &one_user_message(),
            None,
            16,
            &ModelParams::default(),
            &[],
            None,
            false,
            Arc::new(AtomicBool::new(false)),
            "test",
            None,
            None,
            |_, _: &str| {},
            |_: String| {},
        );

        assert!(result.is_err(), "закрытый порт обязан дать ошибку");
        assert_eq!(counter.load(Ordering::Relaxed), 0, "неотвеченный запрос не засчитывается");
    }

    /// Роутер ответил (локальный SSE-стенд) → запрос засчитан ровно один раз.
    #[test]
    fn cloud_request_with_response_is_counted_once() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut sink = [0u8; 4096];
            let _ = stream.read(&mut sink);
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n\
                        data: [DONE]\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });

        let counter = Arc::new(AtomicU64::new(4));
        let engine = cloud_with_counter(&format!("http://{}", addr), counter.clone());
        let result = engine.run_chat_completions(
            &one_user_message(),
            None,
            16,
            &ModelParams::default(),
            &[],
            None,
            false,
            Arc::new(AtomicBool::new(false)),
            "test",
            None,
            None,
            |_, _: &str| {},
            |_: String| {},
        );
        server.join().ok();

        assert!(result.is_ok(), "ответ стенда должен приниматься: {result:?}");
        assert_eq!(counter.load(Ordering::Relaxed), 5, "засчитан ровно один новый запрос");
    }
}
