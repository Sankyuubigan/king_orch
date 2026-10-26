use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::{AppHandle, State};

use super::AppState;
use crate::infra;
use crate::infra::llm::LlamaEngine;
use crate::infra::llm_types::{GenerationResult, LlmMessage};

/// Переводит переданный текст выбранной моделью-переводчиком строго на
/// указанный язык. В историю НЕ попадает ничего, кроме самого сообщения
/// (ни переписка, ни другие сообщения чата) — на вход подаётся ровно один
/// user-текст. Используется для «причёсывания» галлюцинированных/поломанных
/// ответов модели на чистый целевой язык.
#[tauri::command]
pub async fn translate_message(
    app: AppHandle,
    state: State<'_, AppState>,
    model_path: String,
    text: String,
    target_lang: String,
) -> Result<String, String> {
    // Локальный движок — один в VRAM: переводчик берёт слот общего гейта,
    // как и чат-ходы (иначе параллельный чат + перевод = два llama-server).
    let permit = state
        .local_engine_gate
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| format!("Гейт локального движка: {e}"))?;

    // Генерация синхронная и тяжёлая — в blocking-пул, чтобы не занять воркер
    // async-рантайма на минуты. Пермит уезжает в замыкание: слот живёт ровно
    // до конца генерации.
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        translate_sync(app, model_path, text, target_lang)
    })
    .await
    .map_err(|e| format!("Поток генерации перевода: {e}"))?
}

/// Синхронное тело перевода (выполняется в spawn_blocking).
fn translate_sync(
    app: AppHandle,
    model_path: String,
    text: String,
    target_lang: String,
) -> Result<String, String> {
    let cfg = infra::load_config(&app);
    let engine_dir = infra::get_engine_dir(&app);
    let ctx_limit = cfg.context_size;
    let max_tokens = cfg.max_gen_tokens as usize;
    let params =
        tauri_plugin_llama_engine::commands::get_model_params(app.clone(), model_path.clone());

    let target_name = match target_lang.as_str() {
        "en" => "English",
        _ => "Russian",
    };

    let engine = LlamaEngine::new(
        &engine_dir,
        &model_path,
        ctx_limit,
        cfg.kv_quant_keys,
        cfg.kv_quant_values,
        cfg.reasoning_budget,
        |_: String| {},
        |_: String| {},
    )?;

    let messages = vec![
        LlmMessage {
            role: "system".into(),
            content: format!(
                "You are a professional translator. Translate the user's message into {target_name}. \
Output only the translated text — no commentary, no explanations, no quotes around the result."
            ),
         ..Default::default()},
        LlmMessage {
            role: "user".into(),
            content: text,
         ..Default::default()},
    ];

    let cancel = Arc::new(AtomicBool::new(false));
    let result: GenerationResult = engine.generate_chat(
        &messages,
        max_tokens,
        &params,
        "Auto",
        true,
        cancel,
        "translate",
        None,
        |_, _: &str| {},
        |_: String| {},
    )?;

    Ok(result.text)
}
