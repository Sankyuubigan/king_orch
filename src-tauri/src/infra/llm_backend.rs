use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri_plugin_cloud_routers::router::client::{self, ChatMessage, ChatRequest};
use tauri_plugin_cloud_routers::router::client_error::{
    CloudStreamError, CLOUD_CANCELLED_MARKER, CLOUD_EMPTY_STREAM_MARKER, CLOUD_TIMEOUT_MARKER,
};
use tauri_plugin_llama_engine::engine::llm_types::GenerationResult;
use tauri_plugin_llama_engine::engine::{
    ChatAttachment, GrammarSpec, LlmMessage, LlamaEngine, ModelParams, ToolCall, ToolDefinition,
};

#[derive(Clone, Debug)]
pub struct CloudEndpoint {
    pub base_url: String,
    pub api_key: Option<String>,
}

/// Распознавание класса «облако вернуло пустой успех» (HTTP 200 без данных).
///
/// Дверной хелпер (§7 архитектуры): домен обязан работать с инфраструктурой
/// через `crate::infra`, а не импортировать плагин напрямую. Маркер берётся
/// единственной константой из плагина — строковых литералов здесь быть
/// не должно (SSOT).
pub fn is_cloud_empty_stream_error(err: &str) -> bool {
    err.contains(CLOUD_EMPTY_STREAM_MARKER)
}

/// Класс «облако молчало дольше порога тишины» ([cloud_timeout]): живая
/// очередь провайдера под него не попадает — любой байт сбрасывает порог.
/// Распознаётся оркестратором как повторяемый транспортный отказ (тихий
/// ретрай допустим), но НЕ как «мёртвый сервер» (та била по бесполезным
/// ретраям) и НЕ как переполнение контекста.
pub fn is_cloud_timeout_error(err: &str) -> bool {
    err.contains(CLOUD_TIMEOUT_MARKER)
}

/// Штатный исход «юзер нажал Стоп» ([cloud_cancelled]): НЕ ошибка
/// провайдера и НЕ таймаут — ретраи запрещены, это терминальное решение
/// пользователя. Display ошибки начинается с «Прервано пользователем» —
/// детектор оркестратора (mod.rs) опирается на эту фразу.
pub fn is_cloud_cancelled_error(err: &str) -> bool {
    err.contains(CLOUD_CANCELLED_MARKER)
}

/// Максимум повторов запроса при `finish_reason=length` от провайдера
/// (инцидент 24.10.2026, pattern_finder: gemini отдала 217 видимых символов
/// и обрезалась). Инвариант облачной границы: downstream получает либо
/// полный ответ, либо Err — тихо обрезанного текста не существует.
const CLOUD_LENGTH_RETRIES_MAX: u32 = 1;


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
        _max_tokens: usize,
        params: &ModelParams,
        tool_choice: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
        attachments: Option<&[ChatAttachment]>,
        cancel_flag: Option<Arc<AtomicBool>>,
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
        // Инвариант формы запроса (инцидент 20.10.2026): `tool_choice` без
        // непустого `tools[]` — невалидный запрос по контракту OpenAI.
        // Локальный движок поле молча игнорирует, а облако форвардит
        // буквально — и провайдер отдаёт мгновенный пустой стрим
        // (роутер-лог: validator ×3 «IN 8763 · OUT 0» ≈0.6с). Семантику
        // домена принуждения переводим в валидную форму на этой границе:
        // нет тулов на wire — нет и принуждения.
        let tool_choice = match tools.as_deref() {
            Some(items) if !items.is_empty() => tool_choice,
            _ => {
                if tool_choice.is_some() {
                    log::warn!(
                        "☁️ tool_choice проигнорирован: на wire нет tools[] — вместо невалидного запроса уходит обычный выбор модели"
                    );
                    log_cb(
                        "☁️ [cloud-guard] tool_choice сброшен: инструментов в запросе нет".into(),
                    );
                }
                None
            }
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
            // Инвариант wire-формы (инцидент 24.10.2026, pattern_finder): поле
            // `max_tokens` на облачную границу не проходит. Бюджет генерации —
            // концепт локального движка (резерв KV-кэша в VRAM); у облачных
            // thinking-моделей max_tokens накрывает мысли + ответ вместе
            // (gemini: OUT 2044 из бюджета 2048, видимыми — 217 символов,
            // finish_reason=length), поэтому лимит не отправляется вовсе.
            max_tokens: None,
            temperature: Some(params.temperature),
            tools,
            tool_choice: tool_choice.map(str::to_string),
            stream: true,
        };
        log_cb(format!("☁️ 9Router: один вызов модели «{}»", self.model));
        let stream_cb = self.stream_cb.clone();
        // Гейт обрыва по длине (инцидент 24.10.2026): провайдер может вернуть
        // finish_reason=length (дефолтный кап/фильтр). Тихо отдавать обрезанный
        // текст downstream нельзя — воркер уйдёт дальше с полуфабрикатом.
        // Контракт границы: CLOUD_LENGTH_RETRIES_MAX + 1 попыток, затем Err.
        let mut final_result = None;
        for attempt in 0..=CLOUD_LENGTH_RETRIES_MAX {
            let started = std::time::Instant::now();
            // Счётчик = «роутер вернул полезный ответ». Ok стрима и «пустой успех»
            // (HTTP 200 без данных — был подтверждён обмен) засчитываются; ошибке
            // HTTP, внутреннему error-кадру и мусорному стриму — нет. Для «пустого
            // успеха» правда в том, что квота МОГЛА быть списана — это оговорено
            // в warn-логе рядом со счётчиком.
            let attempt_result = match client::chat_completion_stream_result_cancellable(
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
                cancel_flag.clone(),
            ) {
                Ok(result) => {
                    log_cb(format!(
                        "☁️ 9Router: ответ целиком получен за {:.0} с",
                        started.elapsed().as_secs_f32()
                    ));
                    result
                }
                Err(CloudStreamError::EmptySuccess) => {
                    // Провайдер формально ответил 200, но контента не было (инцидент
                    // 18.10.2026: ×5 пустых 200 у validator). Структурно невозможный
                    // раньше пустой Ok пофиксен на уровне плагина (client.rs
                    // `classify_sse_result`) — сюда попадает классифицированный Err.
                    let total = self.requests.fetch_add(1, Ordering::Relaxed) + 1;
                    log_cb(format!(
                        "☁️ [cloud-counter] Пустой успех (HTTP 200 без данных) — засчитано запросов к облаку: {}. Квота могла быть списана.",
                        total
                    ));
                    log::warn!(
                        "☁️ [cloud-counter] EmptySuccess — квота могла быть списана (маркер {})",
                        CLOUD_EMPTY_STREAM_MARKER
                    );
                    return Err(CloudStreamError::EmptySuccess.to_string());
                }
                Err(error) => {
                    let elapsed = started.elapsed();
                    match &error {
                        CloudStreamError::TimedOut { idle_secs, phase } => {
                            log::warn!(
                                "☁️ таймаут тишины: шлюз не откликался ≥{} с (фаза: {}), ожидание длилось {:.0} с",
                                idle_secs,
                                phase,
                                elapsed.as_secs_f32()
                            );
                            log_cb(format!(
                                "☁️ [cloud-timeout] Молчание больше порога тишины ({} с, фаза: {}). Ретрай — по решению оркестратора.",
                                idle_secs, phase
                            ));
                        }
                        CloudStreamError::Cancelled { phase } => {
                            log::info!(
                                "☁️ запрос прерван пользователем (фаза: {}), {:.0} с ожидания",
                                phase,
                                elapsed.as_secs_f32()
                            );
                        }
                        _ => {}
                    }
                    log_cb(format!(
                        "☁️ [cloud-counter] Ответ от облака не получен — запрос НЕ засчитан: {}",
                        error
                    ));
                    return Err(error.to_string());
                }
            };
            let total = self.requests.fetch_add(1, Ordering::Relaxed) + 1;
            // Телеметрия ПОСЛЕ разбора (логирование после трансформации):
            // если провайдер отдал обрезанный/подозрительный ответ — причина станет
            // видимой из last_logs.txt без дополнительных расследований.
            log_cb(format!(
                "☁️ stream: content={} симв, reasoning={} симв, tool_calls={}, finish_reason={}",
                attempt_result.content.chars().count(),
                attempt_result.reasoning.chars().count(),
                attempt_result.tool_calls.len(),
                if attempt_result.finish_reason.is_empty() {
                    "< отсутствует >".to_string()
                } else {
                    attempt_result.finish_reason.clone()
                }
            ));
            log_cb(format!(
                "☁️ [cloud-counter] Засчитано запросов к облаку за этот чат: {}",
                total
            ));
            if attempt_result.finish_reason == "length" {
                if attempt < CLOUD_LENGTH_RETRIES_MAX {
                    log_cb(format!(
                        "☁️ [cloud-length-gate] ответ обрезан лимитом длины (попытка {}/{}), повторяю запрос",
                        attempt + 1,
                        CLOUD_LENGTH_RETRIES_MAX + 1
                    ));
                    continue;
                }
                log::warn!(
                    "☁️ [cloud-length-gate] провайдер {} раз(а) вернул finish_reason=length — отдаю ошибку вместо обрезанного текста",
                    CLOUD_LENGTH_RETRIES_MAX + 1
                );
                return Err(format!(
                    "☁️ провайдер {} раз(а) обрезал ответ по лимиту длины (finish_reason=length) — полный ответ недоступен",
                    CLOUD_LENGTH_RETRIES_MAX + 1
                ));
            }
            final_result = Some(attempt_result);
            break;
        }
        // Инвариант цикла: выход возможен только через Some+break или ранний Err.
        let result = final_result.expect("цикл длины обязан вернуть результат или ошибку");
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
                    Some(cancel_flag),
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
                    Some(cancel_flag),
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
                    Some(_cancel_flag),
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

    fn serve_cloud_body(body: &'static str) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut sink = [0u8; 4096];
            let _ = stream.read(&mut sink);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });
        (addr, server)
    }

    fn run_cloud(engine: &LlmEngine) -> Result<GenerationResult, String> {
        run_cloud_with(engine, None, None)
    }

    /// Вариант `run_cloud` с явными tool_choice/tools (для GP2 — wire-инвариант).
    fn run_cloud_with(
        engine: &LlmEngine,
        tool_choice: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> Result<GenerationResult, String> {
        engine.run_chat_completions(
            &one_user_message(),
            None,
            16,
            &ModelParams::default(),
            &[],
            None,
            false,
            Arc::new(AtomicBool::new(false)),
            "test",
            tool_choice,
            tools.as_deref(),
            |_, _: &str| {},
            |_: String| {},
        )
    }

    /// SSE-стенд, запоминающий фактическое wire-тело запроса: читает заголовки,
    /// потом ровно Content-Length (read-until-0 завис бы — соединение
    /// держится открытым в ожидании ответа).
    fn serve_cloud_capture(
        body: &'static str,
    ) -> (
        std::net::SocketAddr,
        std::thread::JoinHandle<()>,
        Arc<Mutex<String>>,
    ) {
        let capture = Arc::new(Mutex::new(String::new()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let cap = capture.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            *cap.lock().expect("capture lock") = read_request_wire_body(&mut stream);
            write_sse_response(&mut stream, body);
        });
        (addr, server, capture)
    }

    /// SSE-стенд на несколько ПОСЛЕДОВАТЕЛЬНЫХ соединений: каждое новое
    /// соединение получает своё тело из списка по порядку; wire-тела всех
    /// запросов запоминаются в порядке прихода (для гейта finish_reason=length).
    fn serve_cloud_capture_seq(
        bodies: &'static [&'static str],
    ) -> (
        std::net::SocketAddr,
        std::thread::JoinHandle<()>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let capture: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let cap = capture.clone();
        let server = std::thread::spawn(move || {
            for body in bodies {
                let (mut stream, _) = listener.accept().expect("accept");
                cap.lock()
                    .expect("capture lock")
                    .push(read_request_wire_body(&mut stream));
                write_sse_response(&mut stream, body);
            }
        });
        (addr, server, capture)
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
    }

    /// Читает один HTTP-запрос из сокета и возвращает wire-тело (заголовки +
    /// ровно Content-Length — соединение держится открытым в ожидании ответа).
    fn read_request_wire_body(stream: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut target: Option<usize> = None; // конец тела: header_end + content_len
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if target.is_none() {
                if let Some(pos) = find_bytes(&buf, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]);
                    let len = head
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())?
                        });
                    target = len.map(|len| pos + 4 + len);
                }
            }
            if target.is_some_and(|t| buf.len() >= t) {
                break;
            }
        }
        let body_start = find_bytes(&buf, b"\r\n\r\n")
            .map(|pos| pos + 4)
            .unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[body_start..]).to_string()
    }

    /// Отвечает SSE-кадром 200 с переданным телом.
    fn write_sse_response(stream: &mut std::net::TcpStream, body: &str) {
        use std::io::Write;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    /// «Пустой успех» (HTTP 200 без данных — инцидент 18.10.2026) — обмен с
    /// роутером состоялся: счётчик растёт ровно на один запрос, а в тексте
    /// ошибки обязан быть машиночитаемый маркер класса.
    #[test]
    fn cloud_empty_success_counted_once_and_surfaces_marker() {
        let (addr, server) = serve_cloud_body("data: [DONE]\n\n");
        let counter = Arc::new(AtomicU64::new(2));
        let engine = cloud_with_counter(&format!("http://{}", addr), counter.clone());
        let result = run_cloud(&engine);
        server.join().ok();

        match result {
            Err(err) => {
                assert!(
                    err.contains(CLOUD_EMPTY_STREAM_MARKER),
                    "в ошибке обязан быть маркер класса: {err}"
                );
            }
            Ok(_) => panic!("пустой успех обязан быть ошибкой (инвариант G5)"),
        }
        assert_eq!(
            counter.load(Ordering::Relaxed),
            3,
            "обмен состоялся — засчитан ровно один запрос"
        );
    }

    /// Error-кадр внутри 200 — это отказ провайдера: счётчик НЕ растёт
    /// (контракт «не засчитан»), а тело ошибки провайдера видно в тексте.
    #[test]
    fn cloud_error_frame_is_not_counted() {
        let (addr, server) = serve_cloud_body(
            "data: {\"error\":{\"message\":\"You exceeded your current quota\"}}\n\n\
             data: [DONE]\n\n",
        );
        let counter = Arc::new(AtomicU64::new(0));
        let engine = cloud_with_counter(&format!("http://{}", addr), counter.clone());
        let result = run_cloud(&engine);
        server.join().ok();

        match result {
            Err(err) => {
                assert!(
                    err.contains("You exceeded your current quota"),
                    "тело ошибки провайдера обязано дойти до текста: {err}"
                );
            }
            Ok(_) => panic!("error-кадр обязан быть ошибкой, а не «успехом с пустым текстом»"),
        }
        assert_eq!(
            counter.load(Ordering::Relaxed),
            0,
            "error-кадр не считается полезным ответом"
        );
    }

    /// GP2 (wire-инвариант, инцидент 20.10.2026): домен хочет
    /// `tool_choice="required"`, но wire-тулов нет → инфра обязан собрать
    /// ВАЛИДНЫЙ запрос: поле `tool_choice` в фактическом теле отсутствует.
    /// Раньше невалидный запрос уходил на провайдер и получал в ответ
    /// мгновенный пустой стрим (validator ×3 «IN 8763 · OUT 0» ≈0.6с).
    #[test]
    fn cloud_request_without_wire_tools_drops_tool_choice() {
        let (addr, server, capture) =
            serve_cloud_capture("data: {\"choices\":[{\"delta\":{\"content\":\"ок\"},\"finish_reason\":\"stop\"}]}\n\n\
                                 data: [DONE]\n\n");
        let counter = Arc::new(AtomicU64::new(0));
        let engine = cloud_with_counter(&format!("http://{}", addr), counter.clone());
        let result = run_cloud_with(&engine, Some("required"), None);
        server.join().ok();

        result.expect("валидная форма запроса обязана пройти");

        let wire = capture.lock().expect("capture lock").clone();
        let value: serde_json::Value =
            serde_json::from_str(&wire).expect("wire-тело обязано быть JSON");
        assert!(
            value.get("tool_choice").is_none(),
            "tool_choice обязан быть снят с wire при отсутствии tools[]: {wire}"
        );
        assert!(value.get("messages").is_some(), "wire: {wire}");
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

    const SSE_STOP_FULL: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"полный ответ\"},\"finish_reason\":\"stop\"}]}\n\n\
                                 data: [DONE]\n\n";
    const SSE_LENGTH_TRUNCATED: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"нач\"},\"finish_reason\":\"length\"}]}\n\n\
                                        data: [DONE]\n\n";

    /// Wire-инвариант бюджета (инцидент 24.10.2026, pattern_finder: OUT 2044 из
    /// бюджета 2048 — видимо 217 символов, мысли съели ответ): поле `max_tokens`
    /// не имеет права появляться на cloud-wire ни в каком виде. Бюджет —
    /// семантика VRAM локального движка, у провайдера лимит не запрашивается.
    #[test]
    fn cloud_request_carries_no_max_tokens() {
        let (addr, server, capture) = serve_cloud_capture(SSE_STOP_FULL);
        let engine = cloud_with_counter(&format!("http://{}", addr), Arc::new(AtomicU64::new(0)));
        let result = run_cloud(&engine);
        server.join().ok();

        result.expect("ответ стенда обязан приниматься");

        let wire = capture.lock().expect("capture lock").clone();
        let value: serde_json::Value =
            serde_json::from_str(&wire).expect("wire-тело обязано быть JSON");
        assert!(
            value.get("max_tokens").is_none(),
            "бюджет генерации обязан отсутствовать на cloud-wire: {wire}"
        );
        assert!(value.get("messages").is_some(), "wire: {wire}");
    }

    /// Гейт длины: первый ответ finish_reason=length (обрыв), повтор уходит тем
    /// же запросом и приносит stop → отдаётся полный текст. Оба запроса
    /// засчитаны (оба списали квоту), на wire ровно два вызова.
    #[test]
    fn cloud_length_finish_is_retried_once_then_accepted() {
        let (addr, server, capture) = serve_cloud_capture_seq(&[SSE_LENGTH_TRUNCATED, SSE_STOP_FULL]);
        let counter = Arc::new(AtomicU64::new(0));
        let engine = cloud_with_counter(&format!("http://{}", addr), counter.clone());
        let result = run_cloud(&engine);
        server.join().ok();

        let result = result.expect("повтор после length обязан пройти");
        assert_eq!(result.text, "полный ответ", "обрезанный кусок не склеивается");
        assert_eq!(result.stop_reason, "stop");
        assert_eq!(
            counter.load(Ordering::Relaxed),
            2,
            "засчитаны оба запроса — оба обменялись с провайдером"
        );
        assert_eq!(capture.lock().expect("capture lock").len(), 2, "на wire два запроса");
    }

    /// Гейт длины: оба ответа finish_reason=length → Err с диагностикой.
    /// Тихий обрезанный текст downstream не проходит (инвариант границы),
    /// ретрай ровно один — без зацикливания.
    #[test]
    fn cloud_length_twice_is_error_not_truncated_answer() {
        let (addr, server, capture) = serve_cloud_capture_seq(&[SSE_LENGTH_TRUNCATED, SSE_LENGTH_TRUNCATED]);
        let counter = Arc::new(AtomicU64::new(0));
        let engine = cloud_with_counter(&format!("http://{}", addr), counter.clone());
        let result = run_cloud(&engine);
        server.join().ok();

        let err = result.expect_err("дважды обрезанный ответ обязан быть ошибкой");
        assert!(
            err.contains("finish_reason=length"),
            "диагностика обязана называть причину: {err}"
        );
        assert_eq!(
            counter.load(Ordering::Relaxed),
            2,
            "ретрай ровно один, дальше — ошибка"
        );
        assert_eq!(capture.lock().expect("capture lock").len(), 2);
    }
}
