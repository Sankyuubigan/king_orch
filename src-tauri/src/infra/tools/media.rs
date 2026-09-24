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
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::infra::tools::{Tool, ToolCtx, ToolError};

static OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub struct ImageArtifactRegistry {
    next_id: AtomicU64,
    paths: Mutex<HashMap<String, (String, String)>>,
}

#[derive(Clone, Debug)]
struct ImageRunOutput {
    path: String,
    time_sec: f64,
}

impl ImageArtifactRegistry {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            paths: Mutex::new(HashMap::new()),
        }
    }

    pub fn register(&self, path: String, tool_name: &str) -> Result<String, String> {
        let id = format!("artifact_{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let mut paths = self
            .paths
            .lock()
            .map_err(|_| "Реестр media-артефактов заблокирован".to_string())?;
        paths.insert(id.clone(), (path, tool_name.to_string()));
        Ok(id)
    }

    pub fn resolve(&self, id: &str, tool_name: &str) -> Result<String, String> {
        let entry = self
            .paths
            .lock()
            .map_err(|_| "Реестр media-артефактов заблокирован".to_string())?
            .get(id)
            .cloned();
        let Some((path, registered_tool)) = entry else {
            log::error!("Неизвестный media-артефакт: {}", id);
            return Err("Неизвестный media-артефакт".to_string());
        };
        if registered_tool != tool_name {
            log::error!(
                "Инструмент '{}' попытался приложить артефакт другого типа '{}'",
                tool_name,
                id
            );
            return Err("Артефакт создан другим инструментом".to_string());
        }
        Ok(path)
    }
}

/// Сохранить base64-аттачменты во временные файлы (порядок Vec = порядок -r).
/// Возвращает пути файлов. Папка: системный temp + session_id.
pub fn resolve_ref_paths(
    attachments: &[crate::infra::ChatAttachment],
    session_id: &str,
) -> Result<Vec<String>, String> {
    let legacy: Vec<crate::infra::ChatAttachment> = attachments
        .iter()
        .filter(|attachment| attachment.file_path.is_none())
        .cloned()
        .collect();
    let legacy_paths = if legacy.is_empty() {
        Vec::new()
    } else {
        dump_attachments_to_refs(&legacy, session_id)?
    };
    let mut legacy_index = 0;
    let mut paths = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        if attachment.is_dir.unwrap_or(false) {
            return Err(format!(
                "Для edit_image прикрепите конкретный файл, а не папку: {}",
                attachment.file_name
            ));
        }
        if let Some(path) = attachment.file_path.as_deref() {
            if !std::path::Path::new(path).is_file() {
                return Err(format!("Файл изображения недоступен: {}", path));
            }
            paths.push(path.to_string());
        } else {
            let path = legacy_paths
                .get(legacy_index)
                .ok_or_else(|| "Не удалось подготовить legacy-вложение".to_string())?;
            paths.push(path.clone());
            legacy_index += 1;
        }
    }
    if paths.is_empty() {
        return Err("Для edit_image не выбрано изображение".to_string());
    }
    Ok(paths)
}

pub fn dump_attachments_to_refs(
    attachments: &[crate::infra::ChatAttachment],
    session_id: &str,
) -> Result<Vec<String>, String> {
    let mut dir = std::env::temp_dir();
    dir.push("king_orch_img_refs");
    dir.push(session_id);
    std::fs::create_dir_all(&dir).map_err(|error| {
        log::error!("Не удалось создать папку референсов {:?}: {}", dir, error);
        "Не удалось подготовить временные референсы".to_string()
    })?;
    let entries = std::fs::read_dir(&dir).map_err(|error| {
        log::error!("Не удалось прочитать папку референсов {:?}: {}", dir, error);
        "Не удалось очистить временные референсы".to_string()
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            log::error!("Ошибка чтения временных референсов: {}", error);
            "Не удалось очистить временные референсы".to_string()
        })?;
        std::fs::remove_file(entry.path()).map_err(|error| {
            log::error!("Не удалось удалить временный референс: {}", error);
            "Не удалось очистить временные референсы".to_string()
        })?;
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
            .map_err(|error| {
                log::error!("Ошибка декодирования вложения {:?}: {}", att.file_name, error);
                "Не удалось декодировать вложение".to_string()
            })?;
        std::fs::write(&path, &bytes).map_err(|error| {
            log::error!("Не удалось записать референс {:?}: {}", path, error);
            "Не удалось подготовить временный референс".to_string()
        })?;
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
) -> Result<ImageRunOutput, ToolError> {
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
        .map(|d| d.as_nanos())
        .map_err(|error| ToolError::Io(format!("Некорректное системное время: {}", error)))?;
    let sequence = OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let out_path = out_dir.join(format!("img_{ts}_{sequence}.png"));
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
        Ok(r) => Ok(ImageRunOutput {
            path: r.path,
            time_sec: r.time_sec,
        }),
        Err(e) => Err(ToolError::Io(e)),
    }
}

pub fn extract_saved_image_id(tool_output: &str) -> Option<String> {
    let start = tool_output.find("[IMAGE_SAVED id=")? + "[IMAGE_SAVED id=".len();
    let end = tool_output[start..].find(']')?;
    Some(tool_output[start..start + end].to_string())
}

fn format_image_result(
    output: ImageRunOutput,
    tool_name: &str,
    ctx: &ToolCtx,
) -> Result<String, ToolError> {
    let registry = ctx.image_artifacts.ok_or_else(|| {
        ToolError::Usage("внутренний media-реестр артефактов недоступен".to_string())
    })?;
    let id = registry
        .register(output.path, tool_name)
        .map_err(ToolError::Usage)?;
    Ok(format!(
        "[IMAGE_SAVED id={id}] Готово за {:.1} сек",
        output.time_sec
    ))
}

/// Пост-проход run_chat: ищет маркер [IMAGE_SAVED] в результатах tool_calls
/// (thought-сообщения depth 0 и subcall-отчёты) и цепляет PNG как attachment
/// к последнему agent-сообщению (показ в чате через render.ts уже есть).
/// Возвращает число прицепленных картинок.
pub fn attach_saved_images_from_sub_calls(
    message: &mut crate::infra::ChatMessage,
    sub_calls: &[crate::infra::SubCall],
    registry: &ImageArtifactRegistry,
) -> Result<usize, String> {
    let mut artifacts: Vec<(String, String)> = Vec::new();
    for sub_call in sub_calls {
        for tool_call in &sub_call.tool_calls {
            if !matches!(tool_call.tool_name.as_str(), "generate_image" | "edit_image") {
                continue;
            }
            if let Some(id) = extract_saved_image_id(&tool_call.result) {
                if !artifacts.iter().any(|(known_id, _)| known_id == &id) {
                    artifacts.push((id, tool_call.tool_name.clone()));
                }
            }
        }
    }
    let mut attached = 0;
    for (id, tool_name) in artifacts {
        let path = registry.resolve(&id, &tool_name)?;
        let attachment = image_path_to_attachment(&path)?;
        match &mut message.attachments {
            Some(attachments) => attachments.push(attachment),
            None => message.attachments = Some(vec![attachment]),
        }
        attached += 1;
    }
    Ok(attached)
}

/// Прочитать PNG с диска как ChatAttachment (для показа в чате).
pub fn image_path_to_attachment(
    path: &str,
) -> Result<crate::infra::ChatAttachment, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        log::error!("Не удалось прочитать media-артефакт {:?}: {}", path, error);
        "Не удалось прочитать созданное изображение".to_string()
    })?;
    if bytes.is_empty() {
        log::error!("Media-артефакт пуст: {:?}", path);
        return Err("Созданное изображение пустое".to_string());
    }
    use base64::Engine;
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "image.png".to_string());
    Ok(crate::infra::ChatAttachment {
        file_name,
        mime_type: "image/png".to_string(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
        file_path: None,
        is_dir: Some(false),
    })
}

pub struct GenerateImage;

impl Tool for GenerateImage {
    fn name(&self) -> &str {
        "generate_image"
    }
    fn description(&self) -> &str {
        "Сгенерировать изображение с нуля по английскому промпту. Изображение будет приложено к ответу."
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
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage(
            "generate_image должен исполняться через диспетчер с media-реестром"
                .to_string(),
        ))
    }
}

pub struct EditImage;

impl Tool for EditImage {
    fn name(&self) -> &str {
        "edit_image"
    }
    fn description(&self) -> &str {
        "Отредактировать выбранные изображения из чата по английскому промпту. Обязательно передавай source_image_ids из блока доступных изображений."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt_en": {"type": "string", "description": "Английский промпт правки: что изменить, что сохранить."},
                "source_image_ids": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "ID изображений из блока доступных изображений. Файловые пути запрещены."}
            },
            "required": ["prompt_en", "source_image_ids"]
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage(
            "edit_image требует source_image_ids, разрешённые доменным каталогом изображений"
                .to_string(),
        ))
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
) -> Option<Result<String, ToolError>> {
    match name {
        "generate_image" | "edit_image" => {
            let prompt = args
                .get("prompt_en")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim();
            if prompt.is_empty() {
                return Some(Err(ToolError::Usage("prompt_en пуст".to_string())));
            }
            let refs = if name == "edit_image" {
                match resolve_ref_paths(attachments, ctx.session_id) {
                    Ok(refs) => refs,
                    Err(error) => return Some(Err(ToolError::Usage(error))),
                }
            } else {
                Vec::new()
            };
            Some(
                run_generate(prompt, &refs, ctx)
                    .and_then(|output| format_image_result(output, name, ctx)),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod media_tests;
