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
    copy_card_voice_in, engine_ref_paths, normalize_frame, set_card_voice_in,
    update_voice_desc_in, NormalizeReport, RefModBundle, FRAME_LONG_SIDE_MAX,
    FRAME_SIZE_MULTIPLE, FRAMES_PER_RUN,
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
/// Второй элемент — отчёт нормализации (SSOT — плагин): хост только форматирует.
pub fn save_refmod_card(
    slug: &str,
    name: &str,
    description: &str,
    sources: &[PathBuf],
) -> Result<(RefModBundle, NormalizeReport), ToolError> {
    save_refmod_card_in(&refmods_dir(), slug, name, description, sources)
}

pub fn save_refmod_card_in(
    refmods_root: &Path,
    slug: &str,
    name: &str,
    description: &str,
    sources: &[PathBuf],
) -> Result<(RefModBundle, NormalizeReport), ToolError> {
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
) -> Result<(RefModBundle, NormalizeReport), ToolError> {
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
         Порядок source_image_ids = порядок кадров карточки (1-й id станет Picture 1 для движка видео). \
         Рекомендуется первым передавать самый чёткий крупный план лица без перекрытий, \
         дальше — другие ракурсы (до 8). Все фото — ОДИН И ТОТ ЖЕ человек. Не смешивай разных людей. \
         Опционально задай голос персонажа: source_audio_ids (сэмпл голоса из блока доступных аудио, \
         ровно один файл) + voice_desc (краткое описание тембра)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source_image_ids": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 8,
                    "description": "ID изображений из блока доступных изображений. Порядок = порядок кадров карточки; первым рекомендуется чёткий крупный план лица. Файловые пути запрещены."},
                "slug": {"type": "string", "minLength": 1, "maxLength": 48,
                    "description": "Короткий латинский идентификатор персонажа (a-z, 0-9, _, -; начинается и заканчивается буквой/цифрой), например 'masha_red'. Одинаковый slug ПЕРЕЗАПИШЕТ существующую карточку."},
                "name": {"type": "string", "minLength": 1,
                    "description": "Человекочитаемое имя персонажа для каталога, например 'Маша (рыжая)'."},
                "description": {"type": "string",
                    "description": "Короткое описание внешности (2–3 фразы) для каталога персонажей."},
                "source_audio_ids": {"type": "array", "items": {"type": "string"}, "maxItems": 1,
                    "description": "Опционально: ровно ОДИН id аудио-сэмпла голоса из блока [ДОСТУПНЫЕ АУДИО-САМПЛЫ] — голос персонажа карточки. mp3/wav/m4a/ogg-vorbis/flac; записи webm/opus не декодируются."},
                "voice_desc": {"type": "string",
                    "description": "Опционально: краткое описание тембра голоса (низкий, звонкий, хриплый…) для каталога — видео-агент использует его при озвучке."}
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
        "Отредактировать RefMod-карточку персонажа. ДВА РЕЖИМА (не смешивать в одном вызове): \
         (1) ОПЕРАЦИИ над кадрами — фиксированные ops (warp 0.7–1.3 телосложение, hue ±40 цвет, \
         splice сборка из двух карточек) → результат в НОВУЮ карточку <slug>_v2 (исходник не меняется; \
         перезапись исходника — overwrite:true); голос карточки наследуется. \
         (2) ГОЛОС — добавить голос (если его нет) или ПОМЕНЯТЬ (если есть): source_audio_ids \
         (сэмпл из блока доступных аудио, ровно один) + опционально voice_desc; правка IN-PLACE, \
         карточка и кадры не меняются. voice_desc без аудио — правка описания УЖЕ заданного голоса. \
         Операции и голос в одном вызове запрещены — двумя вызовами."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "slug": {"type": "string", "minLength": 1,
                    "description": "Slug существующей карточки из блока «Доступные RefMod-карточки»."},
                "ops": {"type": "array", "items": {"type": "object"},
                    "description": "Режим (1): операции применяются по порядку: [{\"op\":\"warp\",\"factor\":0.85}, {\"op\":\"hue\",\"degrees\":-20}, {\"op\":\"splice\",\"other_slug\":\"x\",\"position\":\"bottom\"}]. НЕ совмещать с голосом (source_audio_ids/voice_desc)."},
                "overwrite": {"type": "boolean",
                    "description": "Только для режима операций: true — перезаписать карточку slug результатом; false (по умолчанию) — создать <slug>_v2 (или следующий свободный вариант)."},
                "name": {"type": "string",
                    "description": "Только для режима операций: новое имя карточки-результата (по умолчанию наследуется)."},
                "description": {"type": "string",
                    "description": "Только для режима операций: новое описание карточки-результата (по умолчанию наследуется)."},
                "source_audio_ids": {"type": "array", "items": {"type": "string"}, "maxItems": 1,
                    "description": "Режим (2): ровно ОДИН id аудио-сэмпла из блока [ДОСТУПНЫЕ АУДИО-САМПЛЫ] — задать/сменить голос карточки (IN-PLACE, кадры не меняются). mp3/wav/m4a/ogg-vorbis/flac; webm/opus не декодируются."},
                "voice_desc": {"type": "string",
                    "description": "Режим (2): описание тембра. Без source_audio_ids — правка описания уже заданного голоса."}
            },
            "required": ["slug"]
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
/// аттачменты; произвольные пути диска структурно недоступны). `audio_attachments`
/// — резолвнутые source_audio_ids (Этап 3: сэмпл голоса).
pub fn execute_refmod_tool(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
    attachments: &[crate::infra::ChatAttachment],
    audio_attachments: &[crate::infra::ChatAttachment],
) -> Option<Result<String, ToolError>> {
    match name {
        "refmod_create" => Some(create_card(&refmods_dir(), args, ctx, attachments, audio_attachments)),
        "refmod_edit" => Some(edit_card(&refmods_dir(), args, audio_attachments)),
        _ => None,
    }
}

/// Байты голосового сэмпла из резолвнутых аудио-вложений. Ровно один сэмпл на
/// карточку (структурный запрет мульти-голосов — H3 2+ ref_audios путает):
/// 0 → None (без голоса), 1 → байты (file_path → чтение, иначе legacy b64),
/// 2+ → Usage — лишние не отбрасываем молча.
fn voice_bytes(
    attachments: &[crate::infra::ChatAttachment],
) -> Result<Option<Vec<u8>>, ToolError> {
    match attachments.len() {
        0 => Ok(None),
        1 => {
            let attachment = &attachments[0];
            if attachment.is_dir.unwrap_or(false) {
                return Err(ToolError::Usage(format!(
                    "Голос персонажа требует конкретный аудио-файл, а не папку: {}",
                    attachment.file_name
                )));
            }
            if let Some(path) = attachment.file_path.as_deref() {
                let bytes = std::fs::read(path).map_err(|error| {
                    log::error!("Не удалось прочитать аудио-сэмпл {}: {}", path, error);
                    ToolError::Usage(format!("Аудио-сэмпл недоступен: {}", path))
                })?;
                Ok(Some(bytes))
            } else {
                let raw = attachment
                    .data_base64
                    .split_once(',')
                    .map(|(_, b)| b)
                    .unwrap_or(&attachment.data_base64);
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(raw.trim())
                    .map_err(|error| {
                        log::error!("Ошибка декодирования аудио-вложения {:?}: {}", attachment.file_name, error);
                        ToolError::Usage("Не удалось декодировать аудио-вложение".to_string())
                    })?;
                Ok(Some(bytes))
            }
        }
        _ => Err(ToolError::Usage(format!(
            "Голос персонажа — РОВНО ОДИН сэмпл (передано {}). Второй голос H3 путает; \
             карточка несёт один voice.wav.",
            attachments.len()
        ))),
    }
}

fn create_card(
    refmods_root: &Path,
    args: &Value,
    ctx: &ToolCtx,
    attachments: &[crate::infra::ChatAttachment],
    audio_attachments: &[crate::infra::ChatAttachment],
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
    let (card, report) = save_refmod_card_in(refmods_root, slug, name, description, &paths)?;
    // Голос опционален (Этап 3): сэмпл декодируется ДО записи (битый аудио —
    // Usage; карточка с кадрами уже сохранена и остаётся без голоса).
    let voice_desc = args
        .get("voice_desc")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let voice_line = match voice_bytes(audio_attachments)? {
        Some(bytes) => {
            let voiced =
                set_card_voice_in(refmods_root, &card.slug, &bytes, voice_desc).map_err(map_error)?;
            format!(
                ", 🎤 голос задан{}",
                voiced
                    .voice_desc
                    .as_deref()
                    .map(|d| format!(" ({})", d))
                    .unwrap_or_default()
            )
        }
        None => String::new(),
    };
    log::info!(
        "[REFMOD-TOOL] create done: slug={}, кадров={}, dir={}",
        card.slug,
        card.frames.len(),
        refmods_root.join(&card.slug).display()
    );
    let normalize_note = format_normalize_warning(&report);
    Ok(format!(
        "[REFMOD_SAVED slug={}] Карточка персонажа сохранена: {} кадр(ов), {} кадров доступно \
         видео-агенту в первом прогоне{}{}. Теперь её можно использовать в генерации видео \
         аргументом refmod=\"{}\".{}",
        card.slug,
        card.frames.len(),
        FRAMES_PER_RUN,
        voice_line,
        if card.voice.is_some() {
            " (голос уйдёт в ref_audios при генерации видео)".to_string()
        } else {
            String::new()
        },
        card.slug,
        normalize_note
    ))
}

/// ⚠-блок о нормализации кадров в тексте результата `refmod_create`
/// (PRP 07.11.26): тул молча сжимал фото пользователя до ≤1024px — потеря
/// пикселей лица была невидимой (кейс vlad_main: 3024×4032 → 768×1024).
/// Хост НЕ пересчитывает размеры — только классифицирует и форматирует
/// отчёт плагина: площадь ужалась → СЖАТИЕ, выросла → РАСТЯЖЕНИЕ (сетка 32).
fn format_normalize_warning(report: &NormalizeReport) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut compressed = false;
    for r in &report.resized {
        let src_area = r.src_w as u64 * r.src_h as u64;
        let dst_area = r.dst_w as u64 * r.dst_h as u64;
        if dst_area < src_area {
            compressed = true;
            lines.push(format!(
                "⚠ СЖАТИЕ {}: {}×{} → {}×{} (детали потеряны)",
                r.frame_file, r.src_w, r.src_h, r.dst_w, r.dst_h
            ));
        } else if dst_area > src_area {
            lines.push(format!(
                "⚠ РАСТЯЖЕНИЕ {}: {}×{} → {}×{} (выравнивание к сетке 32px)",
                r.frame_file, r.src_w, r.src_h, r.dst_w, r.dst_h
            ));
        } else {
            lines.push(format!(
                "⚠ ИЗМЕНЕНИЕ {}: {}×{} → {}×{} (выравнивание к сетке 32px)",
                r.frame_file, r.src_w, r.src_h, r.dst_w, r.dst_h
            ));
        }
    }
    if compressed {
        lines.push(
            "Совет: прикладывайте кропы лица из ОРИГИНАЛОВ со сторонами ≤1024px — \
             иначе лицо теряет пиксели ещё до видео-генерации."
                .to_string(),
        );
    }
    if lines.is_empty() {
        return String::new();
    }
    format!("\n{}", lines.join("\n"))
}

fn edit_card(
    refmods_root: &Path,
    args: &Value,
    audio_attachments: &[crate::infra::ChatAttachment],
) -> Result<String, ToolError> {
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
    let ops_arg = args.get("ops");
    // ops ОПЦИОНАЛЕН (Этап 3): отсутствие поля = voice-режим (правка голоса
    // без кадров); присутствующее поле валидируется как раньше (битый/пустой
    // массив — честный Usage до диска).
    let ops = match ops_arg {
        None => Vec::new(),
        Some(value) => parse_ops(Some(value))?,
    };
    let voice_desc = args
        .get("voice_desc")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let has_audio = !audio_attachments.is_empty();
    let has_voice_desc = voice_desc.is_some();
    let has_ops = !ops.is_empty();

    // Режимы XOR (поправка владельца 05.11.26): операции над кадрами и правка
    // голоса — разные операции, смешивание молча порождало бы одновременно и
    // новую карточку v2, и смену голоса исходника. Только явная граница.
    if has_ops && (has_audio || has_voice_desc) {
        return Err(ToolError::Usage(
            "refmod_edit: операции кадров и правка голоса в одном вызове запрещены — \
             двумя вызовами (сначала голос, потом ops, или наоборот)."
                .to_string(),
        ));
    }
    if !has_ops && !has_audio && !has_voice_desc {
        return Err(ToolError::Usage(
            "refmod_edit: пустой вызов — передай ops (операции над кадрами) либо \
             source_audio_ids/voice_desc (голос карточки)."
                .to_string(),
        ));
    }

    // ── Режим (2): голос — IN-PLACE, кадры и slug не меняются ──
    if !has_ops {
        let voice_line = if has_audio {
            let bytes = voice_bytes(audio_attachments)?;
            let bundle = match bytes {
                Some(bytes) => {
                    set_card_voice_in(refmods_root, slug, &bytes, voice_desc).map_err(map_error)?
                }
                None => {
                    return Err(ToolError::Usage(
                        "refmod_edit: аудио-вложение пустое".to_string(),
                    ))
                }
            };
            format!(
                "🎤 голос задан/сменён{}",
                bundle
                    .voice_desc
                    .as_deref()
                    .map(|d| format!(" ({})", d))
                    .unwrap_or_default()
            )
        } else {
            let bundle = update_voice_desc_in(refmods_root, slug, voice_desc.expect("desc"))
                .map_err(map_error)?;
            format!(
                "🎤 описание голоса обновлено{}",
                bundle
                    .voice_desc
                    .as_deref()
                    .map(|d| format!(" → «{}»", d))
                    .unwrap_or_default()
            )
        };
        log::info!("[REFMOD-TOOL] edit voice in-place: slug={}", slug);
        return Ok(format!(
            "[REFMOD_SAVED slug={}] Карточка обновлена без смены кадров: {}. \
             Она продолжает использоваться аргументом refmod=\"{}\".",
            slug, voice_line, slug
        ));
    }

    // ── Режим (1): ops над кадрами → новая карточка / overwrite ──
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
    let (card, _) =
        save_refmod_card_images_in(refmods_root, &target_slug, &name, &description, &frames)?;
    // Наследование голоса в новую карточку (v2): при overwrite голос уже
    // наследует save_refmod_card_images_in — копирование не нужно.
    if target_slug != slug {
        copy_card_voice_in(refmods_root, slug, &target_slug).map_err(map_error)?;
    }
    log::info!(
        "[REFMOD-TOOL] edit done: {} → {}, кадров={}",
        slug,
        card.slug,
        card.frames.len()
    );
    Ok(format!(
        "[REFMOD_SAVED slug={}] Карточка сохранена: {} кадр(ов) (операций: {}). Исходная карточка '{}'. \
         Голос: {}.",
        card.slug,
        card.frames.len(),
        ops.len(),
        slug,
        if card.voice.is_some() { "унаследован из исходной карточки" } else { "у исходной карточки голоса не было" }
    ))
}

/// Загрузчик кадров другой карточки для splice живёт в refmod_ops
/// (`load_other_frames_in`) — полномочие чтения ограничено каталогом карточек.

#[cfg(test)]
#[path = "refmod_tests.rs"]
mod refmod_tests;
