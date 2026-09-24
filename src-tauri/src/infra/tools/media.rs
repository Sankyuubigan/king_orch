//! 🖼️ Инструменты изображений (generate_image / edit_image).
//!
//! Тонкие схемы + исполнение поверх `tauri-plugin-image-engine` (SSOT движка).
//! Исполнение синхронное (spawn_blocking внутри команд плагина не нужен —
//! reqwest::blocking уже синхронный; генерация — долгая операция, минуты).
//!
//! Аттачменты чата (`Vec<ChatAttachment>` в порядке прикрепления) → temp-файлы
//! `image_ref_N.png` → `ref_images[]` HTTP в том же порядке (порядок = порядок
//! conditioning, см. api.md sd.cpp). Пусто = t2i, есть = edit.

use serde_json::Value;

use crate::infra::tools::{Tool, ToolCtx, ToolError};

/// Сохранить base64-аттачменты во временные файлы (порядок Vec = порядок -r).
/// Возвращает пути файлов. Папка: системный temp + session_id.
pub fn dump_attachments_to_refs(
    attachments: &[crate::infra::ChatAttachment],
    session_id: &str,
) -> Result<Vec<String>, String> {
    let mut dir = std::env::temp_dir();
    dir.push("king_orch_img_refs");
    dir.push(session_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Не удалось создать папку референсов: {}", e))?;
    // Чистим старые референсы сессии (идемпотентность повторных вызовов).
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let mut paths = Vec::with_capacity(attachments.len());
    for (i, att) in attachments.iter().enumerate() {
        let ext = match att.mime_type.as_str() {
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => "png",
        };
        let path = dir.join(format!("image_ref_{}.{}", i, ext));
        let raw = att.data_base64.split_once(',').map(|(_, b)| b).unwrap_or(&att.data_base64);
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(raw.trim())
            .map_err(|e| format!("Ошибка декодирования вложения {}: {}", att.file_name, e))?;
        std::fs::write(&path, &bytes)
            .map_err(|e| format!("Не удалось записать референс {}: {}", path.display(), e))?;
        paths.push(path.to_string_lossy().to_string());
    }
    Ok(paths)
}

/// Папка outputs сессии: рядом с exe — `image_outputs/<session_id>/`.
fn session_outputs_dir(session_id: &str) -> std::path::PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    exe_dir.join("image_outputs").join(session_id)
}

/// Результат тула: путь к PNG (агент показывает его + кладёт в attachments ответа).
fn run_generate(
    prompt_en: &str,
    ref_paths: &[String],
    ctx: &ToolCtx,
) -> Result<String, ToolError> {
    let entry = tauri_plugin_image_engine::engine::default_bundle_entry()
        .ok_or_else(|| ToolError::NotFound("В каталоге нет бандла изображений по умолчанию".to_string()))?;
    let cfg = tauri_plugin_image_engine::engine::load_image_config_early();
    let engine_dir = tauri_plugin_image_engine::engine::engine_dir_early();
    let bundle_dir = tauri_plugin_image_engine::engine::bundle_dir_early();

    if !tauri_plugin_image_engine::engine::sdcpp_installer::has_any_installed(&engine_dir) {
        return Err(ToolError::NotFound(
            "Движок изображений не установлен. Откройте Настройки → «Движок изображений» и нажмите «Установить движок».".to_string(),
        ));
    }

    let out_dir = session_outputs_dir(ctx.session_id);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let out_path = out_dir.join(format!("img_{}.png", ts));
    let log_cb = |msg: String| log::info!("[IMG-TOOL] {}", msg);

    let res = if ref_paths.is_empty() {
        tauri_plugin_image_engine::engine::generate_image_simple(
            &engine_dir, &bundle_dir, &entry,
            cfg.image_engine_variant.as_deref(),
            prompt_en,
            entry.preset.width, entry.preset.height, entry.preset.steps,
            entry.preset.cfg_scale, entry.preset.seed,
            &out_path, None, log_cb,
        )
    } else {
        tauri_plugin_image_engine::engine::edit_image_files(
            &engine_dir, &bundle_dir, &entry,
            cfg.image_engine_variant.as_deref(),
            prompt_en, ref_paths,
            entry.preset.width, entry.preset.height, entry.preset.steps,
            entry.preset.cfg_scale, entry.preset.seed,
            &out_path, None, log_cb,
        )
    };
    match res {
        // Маркер пути для пост-прохода run_chat: PNG цепляется как attachment
        // к финальному сообщению агента (показ в чате через render.ts).
        Ok(r) => Ok(format!("[IMAGE_SAVED path={}] Готово за {:.1} сек: {}", r.path, r.time_sec, r.path)),
        Err(e) => Err(ToolError::Io(e)),
    }
}

/// Извлечь путь PNG из результата тула (маркер [IMAGE_SAVED path=...]).
pub fn extract_saved_image_path(tool_output: &str) -> Option<String> {
    let start = tool_output.find("[IMAGE_SAVED path=")? + "[IMAGE_SAVED path=".len();
    let end = tool_output[start..].find(']')?;
    Some(tool_output[start..start + end].to_string())
}

/// Пост-проход run_chat: ищет маркер [IMAGE_SAVED] в результатах tool_calls
/// (thought-сообщения depth 0 и subcall-отчёты) и цепляет PNG как attachment
/// к последнему agent-сообщению (показ в чате через render.ts уже есть).
/// Возвращает число прицепленных картинок.
pub fn attach_saved_images(messages: &mut [crate::infra::ChatMessage]) -> usize {
    let mut paths: Vec<String> = Vec::new();
    for m in messages.iter() {
        if let Some(p) = extract_saved_image_path(&m.content) {
            if !paths.contains(&p) {
                paths.push(p);
            }
        }
        if let Some(subs) = &m.sub_calls {
            for s in subs {
                for tc in &s.tool_calls {
                    if let Some(p) = extract_saved_image_path(&tc.result) {
                        if !paths.contains(&p) {
                            paths.push(p);
                        }
                    }
                }
            }
        }
    }
    if paths.is_empty() {
        return 0;
    }
    // Цель — последнее agent-сообщение (не user/system/thought).
    let target = messages
        .iter_mut()
        .rev()
        .find(|m| {
            m.msg_type == "message"
                && !matches!(m.author.as_deref(), Some("user") | Some("system") | None)
        });
    let Some(msg) = target else { return 0 };
    let mut attached = 0;
    for p in &paths {
        if let Some(att) = image_path_to_attachment(p) {
            match &mut msg.attachments {
                Some(v) => v.push(att),
                None => msg.attachments = Some(vec![att]),
            }
            attached += 1;
        }
    }
    attached
}

/// Прочитать PNG с диска как ChatAttachment (для показа в чате).
pub fn image_path_to_attachment(path: &str) -> Option<crate::infra::ChatAttachment> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    use base64::Engine;
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "image.png".to_string());
    Some(crate::infra::ChatAttachment {
        file_name,
        mime_type: "image/png".to_string(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
    })
}

pub struct GenerateImage;

impl Tool for GenerateImage {
    fn name(&self) -> &str {
        "generate_image"
    }
    fn description(&self) -> &str {
        "Сгенерировать изображение с нуля по английскому промпту (Qwen Image 2.1). Принимает prompt_en (развёрнутый английский промпт). Возвращает путь к PNG."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt_en": {"type": "string", "description": "Развёрнутый английский промпт: объект, композиция, свет, стиль, детали."}
            },
            "required": ["prompt_en"]
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let prompt = args.get("prompt_en").and_then(|v| v.as_str()).unwrap_or("").trim();
        if prompt.is_empty() {
            return Err(ToolError::Usage("prompt_en пуст — опиши, что нарисовать".to_string()));
        }
        run_generate(prompt, &[], ctx)
    }
}

pub struct EditImage;

impl Tool for EditImage {
    fn name(&self) -> &str {
        "edit_image"
    }
    fn description(&self) -> &str {
        "Отредактировать прикреплённые изображения по английскому промпту (Qwen Image 2.1). Референсы подставляются автоматически в порядке прикрепления. Принимает prompt_en (что изменить, остальное сохранить)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt_en": {"type": "string", "description": "Английский промпт правки: что изменить, что сохранить."}
            },
            "required": ["prompt_en"]
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let prompt = args.get("prompt_en").and_then(|v| v.as_str()).unwrap_or("").trim();
        if prompt.is_empty() {
            return Err(ToolError::Usage("prompt_en пуст — опиши, что изменить".to_string()));
        }
        // Референсы берутся из контекста вызова (аттачменты текущего запроса).
        // Диспетчер кладёт их в ToolCtx через image_refs (см. execute_tool_call).
        let refs: Vec<String> = Vec::new();
        if refs.is_empty() {
            return Err(ToolError::Usage(
                "Нет прикреплённых изображений для редактирования. Прикрепи картинку к сообщению (скрепка) и повтори.".to_string(),
            ));
        }
        run_generate(prompt, &refs, ctx)
    }
}

/// Схемы image-тулов для промпта агента (мета-имя "media").
pub fn image_tool_schemas(agent_tools: &[String]) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    if agent_tools.iter().any(|t| t == "generate_image") {
        let t = GenerateImage;
        out.push(("media".to_string(), t.name().to_string(), serde_json::json!({
            "name": t.name(), "description": t.description(), "inputSchema": t.parameters(),
        })));
    }
    if agent_tools.iter().any(|t| t == "edit_image") {
        let t = EditImage;
        out.push(("media".to_string(), t.name().to_string(), serde_json::json!({
            "name": t.name(), "description": t.description(), "inputSchema": t.parameters(),
        })));
    }
    out
}

/// Исполнить image-тул по имени. Референсы — аттачменты текущего запроса
/// (порядок прикрепления = порядок conditioning).
pub fn execute_image_tool(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
    attachments: &[crate::infra::ChatAttachment],
    session_id: &str,
) -> Option<Result<String, ToolError>> {
    match name {
        "generate_image" => Some(GenerateImage.execute(args, ctx)),
        "edit_image" => {
            let refs = match dump_attachments_to_refs(attachments, session_id) {
                Ok(r) => r,
                Err(e) => return Some(Err(ToolError::Io(e))),
            };
            if refs.is_empty() {
                return Some(Err(ToolError::Usage(
                    "Нет прикреплённых изображений для редактирования. Прикрепи картинку к сообщению (скрепка) и повтори.".to_string(),
                )));
            }
            let prompt = args.get("prompt_en").and_then(|v| v.as_str()).unwrap_or("").trim();
            if prompt.is_empty() {
                return Some(Err(ToolError::Usage("prompt_en пуст — опиши, что изменить".to_string())));
            }
            Some(run_generate(prompt, &refs, ctx))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_only_for_declared_tools() {
        let schemas = image_tool_schemas(&["generate_image".to_string()]);
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].1, "generate_image");
        assert_eq!(schemas[0].0, "media");
        let empty = image_tool_schemas(&[]);
        assert!(empty.is_empty());
        let both = image_tool_schemas(&["generate_image".to_string(), "edit_image".to_string()]);
        assert_eq!(both.len(), 2);
    }

    #[test]
    fn unknown_tool_returns_none() {
        let ctx = ToolCtx {
            workspace_root: std::path::Path::new("."),
            write_root: std::path::Path::new("."),
            write_outside: crate::infra::tools::WriteOutside::Prompt,
            session_id: "test",
            approver: crate::infra::permissions::test_approver(),
            agent_id: "test_agent",
            bins_dir: std::path::Path::new("."),
        };
        assert!(execute_image_tool("nope", &serde_json::json!({}), &ctx, &[], "test").is_none());
    }

    #[test]
    fn edit_without_refs_is_usage_error() {
        let ctx = ToolCtx {
            workspace_root: std::path::Path::new("."),
            write_root: std::path::Path::new("."),
            write_outside: crate::infra::tools::WriteOutside::Prompt,
            session_id: "test",
            approver: crate::infra::permissions::test_approver(),
            agent_id: "test_agent",
            bins_dir: std::path::Path::new("."),
        };
        let err = execute_image_tool("edit_image", &serde_json::json!({"prompt_en": "x"}), &ctx, &[], "test")
            .expect("edit_image обрабатывается")
            .unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)));
    }
}
