//! 🧩 Capability rigging (этап 2, PRP «Риггинг skin-tokens.md»): нативный тул
//! `rig_mesh` — авто-скелет + веса скиннинга (JOINTS_0/WEIGHTS_0) на статический
//! GLB через CLI skintokens-cli (skin-tokens.cpp, движок НЕ линкуется —
//! синхронный спавн из mesh-gen-core). Анимацию это НЕ добавляет (анимация —
//! kimodo.cpp, дальнейший этап). Паттерн 1:1 с mesh.rs: общий сессионный
//! реестр `ImageArtifactRegistry` (GLB регистрируется с tool_name="rig_mesh",
//! кросс-тул ЧТЕНИЕ источника — resolve_any), путь-only вложения, гейт
//! источников §00.2: резолвнутые диспетчером пути (артефакты сессии или
//! вложения юзера из каталога {{ mesh_files }}).

use serde_json::Value;
use std::path::PathBuf;

use crate::infra::tools::{Tool, ToolCtx, ToolError};
use plugin_mesh_engine::engine::{rig_mesh, RigMeshParams};

const RIG_MARKER_PREFIX: &str = "[RIGGED_SAVED id=";

/// Папка движка риггинга: SSOT — дверь плагина `rig_dir_early()` (явная папка
/// из `skintokens_dir` в app_config.json, иначе `<exe>/skintokens`; cwd-правило
/// rules.md:53 — `current_exe().parent()`). Дубль резолва в хосте запрещён.
fn rig_dir() -> PathBuf {
    plugin_mesh_engine::engine::rig_dir_early()
}

/// Папка вывода сессии: `<exe>/mesh_outputs/<session_id>` — общий корень с
/// generate_mesh (одна точка записи GLB на сессию, два префикса имён:
/// mesh-N / rigged-N).
fn session_outputs_dir(session_id: &str) -> Result<PathBuf, ToolError> {
    rig_dir()
        .parent()
        .map(|dir| dir.join("mesh_outputs").join(session_id))
        .ok_or_else(|| ToolError::Usage("не удалось вычислить папку вывода риггинга".to_string()))
}

/// Классификация ошибки ядра: детерминированные отказы префлайта (движок
/// риггинга не установлен, DLL/веса отсутствуют) → `NotFound` («Не найдено: …»)
/// — их Display распознаёт существующий механизм честного стопа
/// `vram::is_setup_error` (ретрай не может изменить состояние диска, §00.2).
/// Остальное — Usage (агент может исправить вход: неверный формат/путь).
fn rig_engine_error(error: String) -> ToolError {
    const SETUP_MARKERS: [&str; 2] = ["не найден", "не найдена"];
    if SETUP_MARKERS.iter().any(|marker| error.contains(marker)) {
        ToolError::NotFound(format!("Не найдено: {}", error))
    } else {
        ToolError::Usage(error)
    }
}

/// Пути исходников из вложений, отобранных диспетчером (гейт §00.2 пройден
/// ДО этой функции: артефакты сессии или allow-read вложения юзера).
/// Упрощённый аналог resolve_ref_paths: legacy base64-вложений у GLB нет
/// (path-only с первого дня), поэтому без dump_attachments_to_refs.
fn resolve_mesh_ref_paths(attachments: &[crate::infra::ChatAttachment]) -> Result<Vec<String>, String> {
    let mut paths = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        if attachment.is_dir.unwrap_or(false) {
            return Err(format!(
                "Риггинг требует конкретный файл, а не папку: {}",
                attachment.file_name
            ));
        }
        let path = attachment
            .file_path
            .as_deref()
            .ok_or_else(|| format!("Вложение '{}' без файла на диске", attachment.file_name))?;
        if !std::path::Path::new(path).is_file() {
            return Err(format!("Файл исходного меша недоступен: {}", path));
        }
        paths.push(path.to_string());
    }
    if paths.is_empty() {
        return Err(
            "Не выбран меш: передай source_mesh_ids с одним id из каталога [GLB ЧАТА] \
             или artifact_N из результата generate_mesh"
                .to_string(),
        );
    }
    Ok(paths)
}

/// id из результата тула: `[RIGGED_SAVED id=artifact_N] …`.
pub fn extract_saved_rigged_id(tool_output: &str) -> Option<String> {
    let start = tool_output.find(RIG_MARKER_PREFIX)? + RIG_MARKER_PREFIX.len();
    let end = tool_output[start..].find(']')?;
    Some(tool_output[start..start + end].to_string())
}

/// Результат тула: непрозрачный id вместо пути (путь не утекает в промпт,
/// прецедент format_mesh_result).
fn format_rig_result(
    glb_path: &std::path::Path,
    duration_secs: f64,
    ctx: &ToolCtx,
) -> Result<String, ToolError> {
    let registry = ctx.image_artifacts.ok_or_else(|| {
        ToolError::Usage("внутренний реестр артефактов сессии недоступен".to_string())
    })?;
    let id = registry
        .register(glb_path.to_string_lossy().to_string(), "rig_mesh")
        .map_err(ToolError::Usage)?;
    Ok(format!(
        "[RIGGED_SAVED id={id}] Готово за {duration_secs:.1} сек. Это GLB со скелетом и \
         весами скиннинга (JOINTS_0/WEIGHTS_0) — анимаций в нём нет."
    ))
}

/// Приложить сохранённый GLB к сообщению ТОЛЬКО путём на диск (SSOT-правило
/// path-only: байты не читаются и не энкодятся в base64).
pub fn rigged_path_to_attachment(path: &str) -> Result<crate::infra::ChatAttachment, String> {
    if !std::path::Path::new(path).is_file() {
        log::error!("Rig-артефакт не найден на диске: {:?}", path);
        return Err("Созданная 3D-модель с ригом недоступна на диске".to_string());
    }
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "rigged.glb".to_string());
    log::info!("Rig-вложение path-only: {:?}", path);
    Ok(crate::infra::ChatAttachment {
        file_name,
        mime_type: "model/gltf-binary".to_string(),
        data_base64: String::new(),
        file_path: Some(path.to_string()),
        is_dir: Some(false),
    })
}

/// Пост-проход: ищет маркер [RIGGED_SAVED] в результатах tool_calls тулов
/// `rig_mesh` и цепляет GLB как attachment к сообщению агента.
pub fn attach_saved_rigged_meshes_from_sub_calls(
    message: &mut crate::infra::ChatMessage,
    sub_calls: &[crate::infra::SubCall],
    registry: &crate::infra::ImageArtifactRegistry,
) -> Result<usize, String> {
    let mut artifacts: Vec<String> = Vec::new();
    for sub_call in sub_calls {
        for tool_call in &sub_call.tool_calls {
            if tool_call.tool_name != "rig_mesh" {
                continue;
            }
            if let Some(id) = extract_saved_rigged_id(&tool_call.result) {
                if !artifacts.iter().any(|known_id| known_id == &id) {
                    artifacts.push(id);
                }
            }
        }
    }
    let mut attached = 0;
    for id in artifacts {
        let path = registry.resolve(&id, "rig_mesh")?;
        let attachment = rigged_path_to_attachment(&path)?;
        match &mut message.attachments {
            Some(attachments) => attachments.push(attachment),
            None => message.attachments = Some(vec![attachment]),
        }
        attached += 1;
    }
    Ok(attached)
}

pub struct RigMesh;

impl Tool for RigMesh {
    fn name(&self) -> &str {
        "rig_mesh"
    }
    fn description(&self) -> &str {
        "Поставить на статическую 3D-модель (GLB) скелет и веса скиннинга (авто-риггинг). НЕ добавляет анимацию или движения — только возможность для будущей анимации. Источник — уже существующий GLB (результат generate_mesh или прикреплённый пользователем); работать с изображениями этот инструмент не умеет. Работает на CPU и занимает минуты, на больших моделях — десятки минут. Результат (GLB с ригом) будет приложен к ответу."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "source_mesh_ids": {"type": "array", "items": {"type": "string", "minLength": 1}, "minItems": 1, "maxItems": 1, "description": "ID исходного GLB: artifact_N из результата generate_mesh/rig_mesh ИЛИ путь файла из каталога [GLB ЧАТА]. РОВНО один."},
                "device": {"type": "string", "enum": ["cpu", "vulkan", "auto"], "description": "Устройство инференса: cpu (дефолт), vulkan или auto. Vulkan заметно быстрее, но требует установленного Vulkan-бекенда движка."}
            },
            "required": ["source_mesh_ids"]
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage(
            "риггинг исполняется диспетчером оркестратора".to_string(),
        ))
    }
}

/// Схемы rig-тулов для промпта агента (мета-имя "rig").
pub fn rig_tool_schemas(agent_tools: &[String]) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    if agent_tools.iter().any(|t| t == "rig_mesh") {
        let t = RigMesh;
        out.push(("rig".to_string(), t.name().to_string(), serde_json::json!({
            "name": t.name(), "description": t.description(), "inputSchema": t.parameters(),
        })));
    }
    out
}

/// Исполнить rig-тул по имени. Источник — РОВНО один GLB: пути приходят
/// резолвнутыми из диспетчера (гейт §00.2: артефакты сессии / вложения юзера).
pub fn execute_rig_tool(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
    attachments: &[crate::infra::ChatAttachment],
) -> Option<Result<String, ToolError>> {
    match name {
        "rig_mesh" => {
            let refs = match resolve_mesh_ref_paths(attachments) {
                Ok(refs) => refs,
                Err(error) => return Some(Err(ToolError::Usage(error))),
            };
            if refs.len() != 1 {
                return Some(Err(ToolError::Usage(format!(
                    "rig_mesh принимает РОВНО один исходный GLB ({} передано). Передай source_mesh_ids с одним id.",
                    refs.len()
                ))));
            }
            let device = args.get("device").and_then(|v| v.as_str()).map(str::to_string);
            let rig_dir = rig_dir();
            let out_dir = match session_outputs_dir(ctx.session_id) {
                Ok(dir) => dir,
                Err(error) => return Some(Err(error)),
            };
            let params = RigMeshParams {
                rig_dir,
                out_dir,
                mesh_path: PathBuf::from(&refs[0]),
                device,
            };
            let outcome = match rig_mesh(&params) {
                Ok(outcome) => outcome,
                Err(error) => {
                    log::error!("Ошибка rig_mesh: {}", error);
                    return Some(Err(rig_engine_error(error)));
                }
            };
            Some(format_rig_result(
                &outcome.glb_path,
                outcome.duration_secs,
                ctx,
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "rig_tests.rs"]
mod rig_tests;
