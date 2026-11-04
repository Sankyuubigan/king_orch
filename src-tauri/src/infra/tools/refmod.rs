//! 🧩 RefMod-тулы хоста: `refmod_create` (карточка из фото юзера) и
//! `refmod_edit` (warp/hue/splice → новая карточка), capability-паттерн
//! медиа-тулов (tasks/261104_0230-db2cb59c, Этап 2).
//!
//! **SSOT формата карточки — плагин** (`tauri_plugin_image_engine::engine::refmod`):
//! одна реализация меты/кадров/нормализации потребляется и тулами хоста,
//! и командами плашки «Рефмодс». Этот модуль держит только:
//! 1) тонкие обёртки над SSOT (маппинг `RefModError` → `ToolError`, чтобы
//!    диспетчер и тесты сохраняли семантику Usage/NotFound/Io);
//! 2) tool-слой: схемы, гейты аргументов, исполнение через `execute_refmod_tool`.
//!
//! Габариты кадров нормируются при сохранении (длинная ≤1024px, обе стороны
//! кратны 32px) — отказ «не тот размер» невозможен по построению (§00.2):
//! фото юзера могут быть ЛЮБОГО размера/формата (png/jpeg/webp).

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::infra::tools::media::resolve_ref_paths;
use crate::infra::tools::refmod_ops::{apply_ops, parse_ops};
use crate::infra::tools::{Tool, ToolCtx, ToolError};

// ── SSOT-реэкспорт (типы и константы без ошибок) ──
pub use tauri_plugin_image_engine::engine::{
    engine_ref_paths, normalize_frame, RefModBundle, RefModMeta, FRAME_LONG_SIDE_MAX,
    FRAME_SIZE_MULTIPLE, FRAMES_MAX, FRAMES_PER_RUN,
};

/// Маппинг ошибки SSOT в ошибку тул-слоя (вариант в вариант — семантика честных
/// отказов Usage/NotFound сохраняется для диспетчера и ретрай-логики).
fn map_error(error: tauri_plugin_image_engine::engine::RefModError) -> ToolError {
    match error {
        tauri_plugin_image_engine::engine::RefModError::Usage(msg) => ToolError::Usage(msg),
        tauri_plugin_image_engine::engine::RefModError::NotFound(msg) => ToolError::NotFound(msg),
        tauri_plugin_image_engine::engine::RefModError::Io(msg) => ToolError::Io(msg),
    }
}

/// Папка карточек по действующему конфигу: `<KingOrchData>/refmods`.
pub fn refmods_dir() -> PathBuf {
    tauri_plugin_image_engine::engine::refmods_dir()
}

/// Валидный slug папки карточки (1–48 символов `[a-z0-9_-]`, без traversal).
pub fn validate_slug(slug: &str) -> Result<(), ToolError> {
    tauri_plugin_image_engine::engine::validate_slug(slug).map_err(map_error)
}

/// Сохранить RefMod-карточку из файлов-источников (перезаписывает существующую).
pub fn save_refmod_card(
    slug: &str,
    name: &str,
    description: &str,
    sources: &[PathBuf],
) -> Result<RefModBundle, ToolError> {
    save_refmod_card_in(&refmods_dir(), slug, name, description, sources)
}

pub fn save_refmod_card_in(
    refmods_root: &Path,
    slug: &str,
    name: &str,
    description: &str,
    sources: &[PathBuf],
) -> Result<RefModBundle, ToolError> {
    tauri_plugin_image_engine::engine::save_refmod_card_in(refmods_root, slug, name, description, sources)
        .map_err(map_error)
}

/// Сохранить карточку из кадров в памяти (операции редактирования).
pub fn save_refmod_card_images_in(
    refmods_root: &Path,
    slug: &str,
    name: &str,
    description: &str,
    images: &[image::DynamicImage],
) -> Result<RefModBundle, ToolError> {
    tauri_plugin_image_engine::engine::save_refmod_card_images_in(
        refmods_root,
        slug,
        name,
        description,
        images,
    )
    .map_err(map_error)
}

/// Загрузить карточку по slug (с валидацией габаритов кадров).
pub fn load_refmod_card(slug: &str) -> Result<RefModBundle, ToolError> {
    load_refmod_card_in(&refmods_dir(), slug)
}

pub fn load_refmod_card_in(refmods_root: &Path, slug: &str) -> Result<RefModBundle, ToolError> {
    tauri_plugin_image_engine::engine::load_refmod_card_in(refmods_root, slug).map_err(map_error)
}

/// Список карточек (битые пропускаются с warn-логом в SSOT).
pub fn list_refmod_cards() -> Vec<RefModBundle> {
    list_refmod_cards_in(&refmods_dir())
}

pub fn list_refmod_cards_in(refmods_root: &Path) -> Vec<RefModBundle> {
    tauri_plugin_image_engine::engine::list_refmod_cards_in(refmods_root)
}

/// Блок «Доступные RefMod-карточки» для графа (`{{ video_refmods }}`).
pub fn refmods_prompt() -> String {
    refmods_prompt_in(&refmods_dir())
}

pub fn refmods_prompt_in(refmods_root: &Path) -> String {
    tauri_plugin_image_engine::engine::refmods_prompt_in(refmods_root)
}

// ── Tool-слой (capability-паттерн медиа-тулов) ──

/// Все имена refmod-тулов (фильтр capability-гейта).
pub const REFMOD_TOOL_NAMES: [&str; 2] = ["refmod_create", "refmod_edit"];

pub fn is_refmod_tool(name: &str) -> bool {
    REFMOD_TOOL_NAMES.contains(&name)
}

pub struct RefmodCreate;

impl Tool for RefmodCreate {
    fn name(&self) -> &str {
        "refmod_create"
    }
    fn description(&self) -> &str {
        "Собрать RefMod-карточку постоянного персонажа для генерации видео из ПРИКРЕПЛЁННЫХ фото \
         (любых размеров и форматов png/jpeg/webp — габариты нормируются автоматически). \
         Обязательно передавай source_image_ids из блока доступных изображений ПОРЯДКОМ КОНВЕНЦИИ: \
         1-й id — лицо крупно в камеру без перекрытий, 2-й — ракурс 3/4 или профиль, 3-й — фигура/одежда, \
         дальше — дополнительные ракурсы (до 8). Все фото — ОДИН И ТОТ ЖЕ человек. Не смешивай разных людей."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source_image_ids": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 8,
                    "description": "ID изображений из блока доступных изображений, по порядку конвенции кадров: лицо → ракурс 3/4 → фигура. Файловые пути запрещены."},
                "slug": {"type": "string", "minLength": 1, "maxLength": 48,
                    "description": "Короткий латинский идентификатор персонажа (a-z, 0-9, _, -; начинается и заканчивается буквой/цифрой), например 'masha_red'. Одинаковый slug ПЕРЕЗАПИШЕТ существующую карточку."},
                "name": {"type": "string", "minLength": 1,
                    "description": "Человекочитаемое имя персонажа для каталога, например 'Маша (рыжая)'."},
                "description": {"type": "string",
                    "description": "Короткое описание внешности (2–3 фразы) для каталога персонажей."}
            },
            "required": ["source_image_ids", "slug", "name"]
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage(
            "refmod_create должен исполняться через диспетчер (гейт вложений запроса)".to_string(),
        ))
    }
}

pub struct RefmodEdit;

impl Tool for RefmodEdit {
    fn name(&self) -> &str {
        "refmod_edit"
    }
    fn description(&self) -> &str {
        "Отредактировать существующую RefMod-карточку персонажа фиксированными операциями \
         и сохранить результат в НОВУЮ карточку (<slug>_v2, …) — исходник не меняется; \
         перезапись исходника — только по overwrite:true. Операции: \
         {\"op\":\"warp\",\"factor\":0.7–1.3} — телосложение (горизонтальный масштаб), \
         {\"op\":\"hue\",\"degrees\":-40..40} — цвет волос/кожи/одежды, \
         {\"op\":\"splice\",\"other_slug\":\"...\",\"position\":\"top|bottom\"} — сборка из двух \
         карточек (вертикальная склейка кадров попарно с растушёвкой шва)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "slug": {"type": "string", "minLength": 1,
                    "description": "Slug существующей карточки из блока «Доступные RefMod-карточки»."},
                "ops": {"type": "array", "minItems": 1, "maxItems": 8, "items": {"type": "object"},
                    "description": "Операции применяются по порядку: [{\"op\":\"warp\",\"factor\":0.85}, {\"op\":\"hue\",\"degrees\":-20}, {\"op\":\"splice\",\"other_slug\":\"x\",\"position\":\"bottom\"}]."},
                "overwrite": {"type": "boolean",
                    "description": "true — перезаписать карточку slug результатом; false (по умолчанию) — создать <slug>_v2 (или следующий свободный вариант)."},
                "name": {"type": "string",
                    "description": "Опционально: новое имя карточки-результата (по умолчанию наследуется)."},
                "description": {"type": "string",
                    "description": "Опционально: новое описание карточки-результата (по умолчанию наследуется)."}
            },
            "required": ["slug", "ops"]
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage(
            "refmod_edit должен исполняться через диспетчер".to_string(),
        ))
    }
}

/// Схемы refmod-тулов для промпта агента (мета-имя "refmod").
pub fn refmod_tool_schemas(agent_tools: &[String]) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    if agent_tools.iter().any(|t| t == "refmod_create") {
        let t = RefmodCreate;
        out.push(("refmod".to_string(), t.name().to_string(), json!({
            "name": t.name(), "description": t.description(), "inputSchema": t.parameters(),
        })));
    }
    if agent_tools.iter().any(|t| t == "refmod_edit") {
        let t = RefmodEdit;
        out.push(("refmod".to_string(), t.name().to_string(), json!({
            "name": t.name(), "description": t.description(), "inputSchema": t.parameters(),
        })));
    }
    out
}

/// Исполнить refmod-тул по имени. `refmod_create` читает ТОЛЬКО вложения текущего
/// запроса (гейт источников §00.2: диспетчер резолвит source_image_ids в
/// аттачменты; произвольные пути диска структурно недоступны).
pub fn execute_refmod_tool(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
    attachments: &[crate::infra::ChatAttachment],
) -> Option<Result<String, ToolError>> {
    match name {
        "refmod_create" => Some(create_card(&refmods_dir(), args, ctx, attachments)),
        "refmod_edit" => Some(edit_card(&refmods_dir(), args)),
        _ => None,
    }
}

fn create_card(
    refmods_root: &Path,
    args: &Value,
    ctx: &ToolCtx,
    attachments: &[crate::infra::ChatAttachment],
) -> Result<String, ToolError> {
    let slug = args
        .get("slug")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if slug.is_empty() {
        return Err(ToolError::Usage(
            "refmod_create: обязателен slug (короткий латинский id персонажа)".to_string(),
        ));
    }
    if name.is_empty() {
        return Err(ToolError::Usage(
            "refmod_create: обязательно name (человекочитаемое имя персонажа)".to_string(),
        ));
    }
    // Порядок конвенции фиксируется ПОРЯДКОМ id: агент расставил кадры
    // (лицо → ракурс → фигура) — здесь ничего не переупорядочиваем.
    let paths: Vec<PathBuf> = resolve_ref_paths(attachments, ctx.session_id)
        .map_err(ToolError::Usage)?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    log::info!(
        "[REFMOD-TOOL] create start: slug={}, name={}, источников={}",
        slug,
        name,
        paths.len()
    );
    let card = save_refmod_card_in(refmods_root, slug, name, description, &paths)?;
    log::info!(
        "[REFMOD-TOOL] create done: slug={}, кадров={}, dir={}",
        card.slug,
        card.frames.len(),
        refmods_root.join(&card.slug).display()
    );
    Ok(format!(
        "[REFMOD_SAVED slug={}] Карточка персонажа сохранена: {} кадр(ов), {} кадров доступно \
         видео-агенту в первом прогоне. Теперь её можно использовать в генерации видео \
         аргументом refmod=\"{}\".",
        card.slug,
        card.frames.len(),
        FRAMES_PER_RUN,
        card.slug
    ))
}

fn edit_card(refmods_root: &Path, args: &Value) -> Result<String, ToolError> {
    let slug = args
        .get("slug")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if slug.is_empty() {
        return Err(ToolError::Usage(
            "refmod_edit: обязателен slug существующей карточки".to_string(),
        ));
    }
    let ops = parse_ops(args.get("ops"))?;
    let overwrite = args.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false);
    let new_name = args.get("name").and_then(|v| v.as_str()).map(str::trim);
    let new_desc = args
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim);

    // Кадры читаются В ПАМЯТЬ до сохранения: save перезаписывает frames/,
    // при overwrite исходник должен быть уже не нужен на диске.
    let source = load_refmod_card_in(refmods_root, slug)?;
    let mut frames: Vec<image::DynamicImage> = source
        .frames
        .iter()
        .map(|path| {
            image::open(path).map_err(|e| {
                ToolError::Io(format!(
                    "refmod_edit: не удалось прочитать кадр {}: {}",
                    path.display(),
                    e
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    log::info!(
        "[REFMOD-TOOL] edit start: slug={}, ops={:?}, overwrite={}",
        slug,
        ops,
        overwrite
    );
    frames = apply_ops(frames, &ops, &|other_slug| {
        crate::infra::tools::refmod_ops::load_other_frames_in(refmods_root, other_slug)
    })?;

    let target_slug = if overwrite {
        slug.to_string()
    } else {
        crate::infra::tools::refmod_ops::next_free_slug_in(refmods_root, slug)?
    };
    let name = new_name.unwrap_or(&source.name).to_string();
    let description = new_desc.unwrap_or(&source.description).to_string();
    let card =
        save_refmod_card_images_in(refmods_root, &target_slug, &name, &description, &frames)?;
    log::info!(
        "[REFMOD-TOOL] edit done: {} → {}, кадров={}",
        slug,
        card.slug,
        card.frames.len()
    );
    Ok(format!(
        "[REFMOD_SAVED slug={}] Карточка сохранена: {} кадр(ов) (операций: {}). Исходная карточка '{}'.",
        card.slug,
        card.frames.len(),
        ops.len(),
        slug
    ))
}

/// Загрузчик кадров другой карточки для splice живёт в refmod_ops
/// (`load_other_frames_in`) — полномочие чтения ограничено каталогом карточек.

#[cfg(test)]
#[path = "refmod_tests.rs"]
mod refmod_tests;
