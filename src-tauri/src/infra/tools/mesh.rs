//! 🧊 Capability mesh (3D-модели, §1 AGENTS.md): нативный тул `generate_mesh`
//! (image→GLB через trellis-server, паттерн медиа-тулов). Реестр артефактов —
//! общий сессионный `ImageArtifactRegistry` (excel/video уже передают его в
//! свои attach-функции): GLB регистрируется с `tool_name="generate_mesh"`, и
//! проверка типа в `resolve` работает сама. Источники — гейт §00.2: только
//! резолвнутые диспетчером вложения (каталог изображений / реестр артефактов),
//! произвольные пути диска структурно недоступны.

use serde_json::Value;
use std::path::PathBuf;

use crate::infra::tools::{Tool, ToolCtx, ToolError};
use plugin_mesh_engine::engine::{
    generate_mesh, GenerateMeshParams, MeshPreset, MeshQuant, DEFAULT_QUANT, DEFAULT_RESOLUTION,
};

const MESH_MARKER_PREFIX: &str = "[MESH_SAVED id=";

/// Корень движка: SSOT — дверь плагина `engine_dir_early()` (явная папка из
/// `trellis_dir` в app_config.json, иначе `<exe>/trellis`; cwd-правило
/// rules.md:53 — `current_exe().parent()`, не `app.path().executable_dir()`).
/// Дубль резолва в хосте запрещён.
fn engine_dir() -> PathBuf {
    plugin_mesh_engine::engine::config::engine_dir_early()
}

/// Папка вывода сессии: `<exe>/mesh_outputs/<session_id>` (те же точки записи,
/// что у image_outputs, но отдельный корень под GLB).
fn session_outputs_dir(session_id: &str) -> Result<PathBuf, ToolError> {
    engine_dir()
        .parent()
        .map(|dir| dir.join("mesh_outputs").join(session_id))
        .ok_or_else(|| ToolError::Usage("не удалось вычислить папку движка".to_string()))
}

/// Классификация ошибки ядра: детерминированные отказы префлайта (движок не
/// установлен, весов нет) → `NotFound` («Не найдено: …») — их Display
/// распознаёт существующий механизм честного стопа `vram::is_setup_error`
/// (ретрай не может изменить состояние диска, §00.2). Остальное — Usage.
fn mesh_engine_error(error: String) -> ToolError {
    const SETUP_MARKERS: [&str; 3] = ["не установлен", "не найдены", "нет *.gguf"];
    if SETUP_MARKERS.iter().any(|marker| error.contains(marker)) {
        ToolError::NotFound(format!("Не найдено: {}", error))
    } else {
        ToolError::Usage(error)
    }
}

/// id из результата тула: `[MESH_SAVED id=artifact_N] …`.
pub fn extract_saved_mesh_id(tool_output: &str) -> Option<String> {
    let start = tool_output.find(MESH_MARKER_PREFIX)? + MESH_MARKER_PREFIX.len();
    let end = tool_output[start..].find(']')?;
    Some(tool_output[start..start + end].to_string())
}

/// Результат тула: непрозрачный id вместо пути (путь не утекает в промпт,
/// прецедент format_image_result).
fn format_mesh_result(
    glb_path: &std::path::Path,
    duration_secs: f64,
    ctx: &ToolCtx,
) -> Result<String, ToolError> {
    let registry = ctx.image_artifacts.ok_or_else(|| {
        ToolError::Usage("внутренний реестр артефактов сессии недоступен".to_string())
    })?;
    let id = registry
        .register(glb_path.to_string_lossy().to_string(), "generate_mesh")
        .map_err(ToolError::Usage)?;
    Ok(format!("[MESH_SAVED id={id}] Готово за {duration_secs:.1} сек"))
}

/// Приложить сохранённый GLB к сообщению ТОЛЬКО путём на диск (SSOT-правило
/// path-only: байты не читаются и не энкодятся в base64; тест-страж ниже).
pub fn mesh_path_to_attachment(path: &str) -> Result<crate::infra::ChatAttachment, String> {
    if !std::path::Path::new(path).is_file() {
        log::error!("Mesh-артефакт не найден на диске: {:?}", path);
        return Err("Созданная 3D-модель недоступна на диске".to_string());
    }
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "model.glb".to_string());
    log::info!("Mesh-вложение path-only: {:?}", path);
    Ok(crate::infra::ChatAttachment {
        file_name,
        mime_type: "model/gltf-binary".to_string(),
        data_base64: String::new(),
        file_path: Some(path.to_string()),
        is_dir: Some(false),
    })
}

/// Пост-проход: ищет маркер [MESH_SAVED] в результатах tool_calls тулов
/// `generate_mesh` и цепляет GLB как attachment к сообщению агента.
pub fn attach_saved_meshes_from_sub_calls(
    message: &mut crate::infra::ChatMessage,
    sub_calls: &[crate::infra::SubCall],
    registry: &crate::infra::ImageArtifactRegistry,
) -> Result<usize, String> {
    let mut artifacts: Vec<String> = Vec::new();
    for sub_call in sub_calls {
        for tool_call in &sub_call.tool_calls {
            if tool_call.tool_name != "generate_mesh" {
                continue;
            }
            if let Some(id) = extract_saved_mesh_id(&tool_call.result) {
                if !artifacts.iter().any(|known_id| known_id == &id) {
                    artifacts.push(id);
                }
            }
        }
    }
    let mut attached = 0;
    for id in artifacts {
        let path = registry.resolve(&id, "generate_mesh")?;
        let attachment = mesh_path_to_attachment(&path)?;
        match &mut message.attachments {
            Some(attachments) => attachments.push(attachment),
            None => message.attachments = Some(vec![attachment]),
        }
        attached += 1;
    }
    Ok(attached)
}

pub struct GenerateMesh;

impl Tool for GenerateMesh {
    fn name(&self) -> &str {
        "generate_mesh"
    }
    fn description(&self) -> &str {
        "Построить 3D-модель (GLB) из одного изображения. Изображение — уже существующее (прикреплённое пользователем или из каталога доступных изображений); текстовый промпт моделью не принимается. Модель будет приложена к ответу."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "source_image_ids": {"type": "array", "items": {"type": "string", "minLength": 1}, "minItems": 1, "maxItems": 1, "description": "ID изображения-исходника из списка доступных изображений. РОВНО одно изображение."},
                "model": {"type": "string", "enum": ["trellis2", "pixal3d"], "description": "Семейство весов: trellis2 (TRELLIS.2, дефолт) или pixal3d (многовидовая текстуризация с UV-атласом)."},
                "quant": {"type": "string", "enum": ["q4", "q8", "f16"], "description": "Квант весов движка: q4 (слабые GPU ~6.4 ГБ), q8 (дефолт, ~9.8 ГБ), f16 (16 ГБ+)."},
                "resolution": {"type": "integer", "enum": [512, 1024, 1536], "description": "Разрешение генерации (дефолт 512; 1024/1536 заметно медленнее и рискованнее по памяти)."},
                "seed": {"type": "integer", "minimum": 0, "description": "Опционально: seed для воспроизводимого результата."},
                "faces": {"type": "string", "description": "Опционально: 'auto' или целое число ≥ 1000 (бюджет треугольников)."},
                "bg_removal": {"type": "string", "enum": ["threshold", "birefnet"], "description": "Опционально: удаление фона. threshold (дефолт) работает везде; birefnet качественнее, но только CUDA."}
            },
            "required": ["source_image_ids"]
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage(
            "генерация 3D исполняется диспетчером оркестратора".to_string(),
        ))
    }
}

/// Тул сборки промпта концепт-изображения (text→3D, решение владельца
/// 17.11.26 «шаблоны должны быть в плагине лежать»): подставляет английское
/// описание объекта в проверенный шаблон категории (SSOT — core плагина,
/// `prompting.rs`). Read-only строковая функция: ничего не пишет на диск,
/// VRAM не трогает; результат агент использует как `prompt_en` в
/// `generate_image` (sd.cpp), а модель строится позже по концепту
/// (`generate_mesh`).
pub struct MeshConceptPrompt;

impl Tool for MeshConceptPrompt {
    fn name(&self) -> &str {
        "mesh_concept_prompt"
    }
    fn description(&self) -> &str {
        "Собрать промпт концепт-изображения объекта для маршрута «текст → концепт → 3D»: подставляет английское описание в проверенный шаблон категории (белый фон #FFFFFF, широкие поля, 3/4 ракурс, без теней/боке; деревья — плотные «chunky» кроны; персонажи — A-pose фронтально). Верни результат как prompt_en в generate_image ЦЕЛИКОМ, без редактирования."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subject_en": {"type": "string", "minLength": 1, "description": "Английское описание ОБЪЕКТА (без фона, сцены и рамок кадра): «an ancient stone chest covered in moss». Переведи запрос пользователя на английский сам."},
                "kind": {"type": "string", "enum": ["prop", "tree", "weapon", "character"], "description": "Категория шаблона: prop (камни, сундуки, бочки, дома, заборы, руины, мебель) / tree (деревья, кусты, растения) / weapon (оружие, щиты, инструменты, инвентарь) / character (персонажи, монстры, существа)."}
            },
            "required": ["subject_en", "kind"]
        })
    }
    fn is_readonly(&self) -> bool {
        true
    }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage(
            "сборка концепт-промпта исполняется диспетчером оркестратора".to_string(),
        ))
    }
}

/// Схемы mesh-тулов для промпта агента (мета-имя "mesh").
pub fn mesh_tool_schemas(agent_tools: &[String]) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    if agent_tools.iter().any(|t| t == "generate_mesh") {
        let t = GenerateMesh;
        out.push(("mesh".to_string(), t.name().to_string(), serde_json::json!({
            "name": t.name(), "description": t.description(), "inputSchema": t.parameters(),
        })));
    }
    if agent_tools.iter().any(|t| t == "mesh_concept_prompt") {
        let t = MeshConceptPrompt;
        out.push(("mesh".to_string(), t.name().to_string(), serde_json::json!({
            "name": t.name(), "description": t.description(), "inputSchema": t.parameters(),
        })));
    }
    out
}

/// Исполнить mesh-тул по имени. Источник — РОВНО один (сервер принимает одно
/// изображение): пути приходят резолвнутыми из диспетчера (гейт §00.2).
pub fn execute_mesh_tool(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
    attachments: &[crate::infra::ChatAttachment],
) -> Option<Result<String, ToolError>> {
    match name {
        "mesh_concept_prompt" => {
            // Text→3D (§1 AGENTS.md): SSOT шаблонов — core плагина
            // (`prompting.rs`); хост только подставляет аргументы. Read-only:
            // attachments не нужны, диск не пишется.
            let subject = args.get("subject_en").and_then(|v| v.as_str()).unwrap_or("");
            let kind = match args.get("kind").and_then(|v| v.as_str()) {
                Some(raw) => match plugin_mesh_engine::engine::ConceptKind::parse(raw) {
                    Ok(kind) => kind,
                    Err(error) => return Some(Err(ToolError::Usage(error))),
                },
                None => {
                    return Some(Err(ToolError::Usage(
                        "kind обязателен: prop | tree | weapon | character".to_string(),
                    )))
                }
            };
            match plugin_mesh_engine::engine::build_concept_prompt(subject, kind) {
                Ok(prompt) => {
                    let catalog: Vec<String> = plugin_mesh_engine::engine::CONCEPT_KIND_CATALOG
                        .iter()
                        .map(|(key, label)| format!("{} — {}", key, label))
                        .collect();
                    Some(Ok(format!(
                        "CONCEPT_PROMPT:\n{prompt}\n\nИспользуй эту строку ЦЕЛИКОМ как prompt_en в generate_image — не редактируй, не сокращай и не добавляй негативный промпт (запреты уже в тексте). Категории для будущего выбора: {}",
                        catalog.join("; ")
                    )))
                }
                Err(error) => Some(Err(ToolError::Usage(error))),
            }
        }
        "generate_mesh" => {
            let refs = match crate::infra::tools::media::resolve_ref_paths(attachments, ctx.session_id) {
                Ok(refs) => refs,
                Err(error) => return Some(Err(ToolError::Usage(error))),
            };
            if refs.len() != 1 {
                return Some(Err(ToolError::Usage(format!(
                    "generate_mesh принимает РОВНО один исходник ({} передано). Передай source_image_ids с одним id.",
                    refs.len()
                ))));
            }
            let model = match args.get("model").and_then(|v| v.as_str()) {
                Some(raw) => match MeshPreset::parse(raw) {
                    Ok(preset) => preset,
                    Err(error) => return Some(Err(ToolError::Usage(error))),
                },
                None => MeshPreset::Trellis,
            };
            let quant = match args.get("quant").and_then(|v| v.as_str()) {
                Some(raw) => match MeshQuant::parse(raw) {
                    Ok(quant) => quant,
                    Err(error) => return Some(Err(ToolError::Usage(error))),
                },
                None => DEFAULT_QUANT,
            };
            let resolution = args
                .get("resolution")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .unwrap_or(DEFAULT_RESOLUTION);
            let seed = args.get("seed").and_then(|v| v.as_u64()).map(|v| v as u32);
            let bg_removal = args
                .get("bg_removal")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let faces = match args.get("faces") {
                Some(Value::String(raw)) => Some(raw.clone()),
                Some(Value::Number(n)) => Some(n.to_string()),
                _ => None,
            };
            let engine_dir = engine_dir();
            let out_dir = match session_outputs_dir(ctx.session_id) {
                Ok(dir) => dir,
                Err(error) => return Some(Err(error)),
            };
            let params = GenerateMeshParams {
                engine_dir,
                out_dir,
                image_path: PathBuf::from(&refs[0]),
                model,
                quant,
                backend: None,
                gpu: None,
                seed,
                resolution,
                bg_removal,
                faces,
            };
            let outcome = match generate_mesh(&params) {
                Ok(outcome) => outcome,
                Err(error) => {
                    log::error!("Ошибка generate_mesh: {}", error);
                    return Some(Err(mesh_engine_error(error)));
                }
            };
            Some(format_mesh_result(
                &outcome.glb_path,
                outcome.duration.as_secs_f64(),
                ctx,
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "mesh_tests.rs"]
mod mesh_tests;
