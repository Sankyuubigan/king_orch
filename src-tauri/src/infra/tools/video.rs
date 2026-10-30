//! 🎬 Инструмент видео-генерации (`generate_video`).
//!
//! Тонкая схема + исполнение поверх `tauri-plugin-image-engine::engine` (SSOT
//! движка: дефолты размеров/seed/fps/лимит seconds — из пресета каталога,
//!Никаких локальных копий констант). По образцу `media.rs` (generate_image),
//! но: один тул, артефакт прикрепляется к ответу ПО ПУТИ (`file_path`), потому
//! что webm-клипы десятки МБ — base64-энкачить их в `messages[]` нельзя.
//!
//! Файловая точка сохранения: `<exe_dir>/video_outputs/<session_id>/vid_<ts>_<seq>.ext`.

use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::infra::tools::media::ImageArtifactRegistry;
use crate::infra::tools::{Tool, ToolCtx, ToolError};

static VIDEO_OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct VideoRunOutput {
    path: String,
    time_sec: f64,
}

/// Mime контейнера по расширению (дублирует логику plugin `ext_for_mime`,
/// которая не публична; расширения совпадают: webm/webp/avi).
fn mime_for_ext(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "webp" => "image/webp",
        "avi" => "video/x-msvideo",
        _ => "video/webm",
    }
}

/// Папка outputs сессии: рядом с exe — `video_outputs/<session_id>/`
/// (правило cwd: current_exe().parent(), НЕ app.path()).
fn video_session_outputs_dir(session_id: &str) -> std::path::PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    exe_dir.join("video_outputs").join(session_id)
}

/// Активный видео-бандл: выбор юзера из конфига → дефолт каталога. НЕ тихий
/// фолбэк: отсутствующее имя или недокачанные файлы — честная ошибка
/// с инструкцией (по образцу media.rs resolve_bundle_for_tools).
fn resolve_video_bundle_for_tools() -> Result<
    tauri_plugin_image_engine::engine::VideoBundleEntry,
    ToolError,
> {
    let cfg = tauri_plugin_image_engine::engine::load_image_config_early();
    let selected = cfg.selected_video_bundle.clone().unwrap_or_default();
    let entry = if selected.is_empty() {
        tauri_plugin_image_engine::engine::default_video_bundle_entry()
            .ok_or_else(|| ToolError::NotFound(
                "В каталоге нет видео-бандла по умолчанию".to_string(),
            ))?
    } else {
        tauri_plugin_image_engine::engine::video_bundle_by_name(&selected)
            .ok_or_else(|| ToolError::NotFound(format!(
                "Активный видео-бандл «{}» отсутствует в каталоге — выберите другой в Настройки → «Модели ImageGEN».",
                selected
            )))?
    };
    let dir = tauri_plugin_image_engine::engine::video_bundle_dir_early();
    let missing: Vec<&str> = entry
        .files
        .iter()
        .filter(|f| !dir.join(&f.filename).exists())
        .map(|f| f.filename.as_str())
        .collect();
    if !missing.is_empty() {
        log::warn!(
            "[VID-TOOL] активный видео-бандл «{}» (label={}): нет файлов {}",
            entry.name,
            entry.label,
            missing.join(", ")
        );
        return Err(ToolError::NotFound(format!(
            "Активный видео-бандл «{}» скачан не полностью: нет файлов {}. Скачайте его в Настройки → «Модели ImageGEN» (вкладка «Видео»).",
            entry.name,
            missing.join(", ")
        )));
    }
    Ok(entry)
}

/// Гейт режима бандла (декларативный, SSOT каталога): запрос режима, которого
/// нет в `modes` бандла, — честный отказ с инструкцией, а НЕ тихая деградация
/// (молчаливое отклонение картинок юзера = ложь). Референсы (ref2va) и кадры
/// (i2va) ловятся здесь ДО обращения к движку. Выделен в чистую функцию для
/// герметичного теста на синтетическом entry (без реального конфига машины).
fn ensure_bundle_supports_mode(
    entry: &tauri_plugin_image_engine::engine::VideoBundleEntry,
    mode: &str,
) -> Result<(), ToolError> {
    if entry.modes.iter().any(|m| m == mode) {
        return Ok(());
    }
    Err(ToolError::Usage(format!(
        "Активный видео-бандл «{}» не поддерживает режим «{}» (modes: {}). \
         Активируйте бандл с нужным режимом в Настройки → «Модели ImageGEN» (вкладка «Видео») \
         и повторите.",
        entry.name,
        mode,
        entry.modes.join(", ")
    )))
}

fn run_generate_video(
    prompt_en: &str,
    seconds: u32,
    final_quality: bool,
    init_path: Option<&str>,
    ref_paths: &[String],
    loras: &[tauri_plugin_image_engine::engine::LoraSpec],
    ctx: &ToolCtx,
) -> Result<VideoRunOutput, ToolError> {
    let entry = resolve_video_bundle_for_tools()?;
    // ref2va и i2va взаимоисключающие (api.md: Ref2VA нельзя комбинировать
    // с init_image) — отказ ДО чтения файлов, зеркало resolve_mode плагина.
    let has_refs = ref_paths.iter().any(|p| !p.trim().is_empty());
    if has_refs && init_path.is_some() {
        return Err(ToolError::Usage(
            "generate_video принимает ЛИБО картинку-первый-кадр (first_frame), \
             ЛИБО референсы (reference, 1–3), но не одновременно."
                .to_string(),
        ));
    }
    // Режим — из заполненных полей (SSOT имен — VideoMode плагина).
    let mode: &str = if has_refs {
        "ref2va"
    } else if init_path.is_some() {
        "i2va"
    } else {
        "t2va"
    };
    ensure_bundle_supports_mode(&entry, mode)?;
    let engine_dir = tauri_plugin_image_engine::engine::engine_dir_early();
    let bundle_dir = tauri_plugin_image_engine::engine::video_bundle_dir_early();

    if !tauri_plugin_image_engine::engine::sdcpp_installer::has_any_installed(&engine_dir) {
        return Err(ToolError::NotFound(
            "Движок изображений не установлен. Откройте Настройки → «Движок запуска ImageGEN моделей» и нажмите «Установить движок».".to_string(),
        ));
    }

    // Честный прогон: движок spawn'ится на прогон и его же Drop убивает.
    log::info!(
        "[VID-TOOL] generate_video start: bundle={}, seconds={}, quality={}, mode={}",
        entry.name,
        seconds,
        if final_quality { "final" } else { "draft" },
        mode
    );
    let out_dir = video_session_outputs_dir(ctx.session_id);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .map_err(|error| ToolError::Io(format!("Некорректное системное время: {}", error)))?;
    let sequence = VIDEO_OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let out_path = out_dir.join(format!("vid_{}_{}.webm", ts, sequence));
    let log_cb = |msg: String| log::info!("[VID-TOOL] {}", msg);

    tauri_plugin_image_engine::engine::generate_video_simple(
        &engine_dir,
        &bundle_dir,
        &entry,
        prompt_en,
        seconds,
        final_quality,
        init_path,
        ref_paths,
        &out_path,
        None,
        loras,
        log_cb,
    )
    .map(|r| {
        log::info!("[VID-TOOL] generate_video done: {} ({:.1} сек)", r.path, r.time_sec);
        VideoRunOutput { path: r.path, time_sec: r.time_sec }
    })
    .map_err(ToolError::Io)
}

pub fn extract_saved_video_id(tool_output: &str) -> Option<String> {
    let marker = "[VIDEO_SAVED id=";
    let start = tool_output.find(marker)? + marker.len();
    let end = tool_output[start..].find(']')?;
    Some(tool_output[start..start + end].to_string())
}

fn format_video_result(
    output: VideoRunOutput,
    ctx: &ToolCtx,
) -> Result<String, ToolError> {
    let registry = ctx.image_artifacts.ok_or_else(|| {
        ToolError::Usage("внутренний media-реестр артефактов недоступен".to_string())
    })?;
    let id = registry
        .register(output.path, "generate_video")
        .map_err(ToolError::Usage)?;
    Ok(format!(
        "[VIDEO_SAVED id={id}] Готово за {:.1} сек",
        output.time_sec
    ))
}

/// Пост-проход run_chat: маркер [VIDEO_SAVED] в результатах tool_calls →
/// цепляет клип к финальному сообщению агента по `file_path` (webm в base64
/// не энкодится — клипы десятки МБ, messages[] не резины). Возвращает число
/// прицепленных видео (частичная ошибка одного артефакта не валит остальные).
pub fn attach_saved_videos_from_sub_calls(
    message: &mut crate::infra::ChatMessage,
    sub_calls: &[crate::infra::SubCall],
    registry: &ImageArtifactRegistry,
) -> Result<usize, String> {
    let mut artifacts: Vec<String> = Vec::new();
    for sub_call in sub_calls {
        for tool_call in &sub_call.tool_calls {
            if tool_call.tool_name != "generate_video" {
                continue;
            }
            if let Some(id) = extract_saved_video_id(&tool_call.result) {
                if !artifacts.contains(&id) {
                    artifacts.push(id);
                }
            }
        }
    }
    let mut attached = 0;
    for id in artifacts {
        let path = registry.resolve(&id, "generate_video")?;
        match video_path_to_attachment(&path) {
            Ok(attachment) => {
                match &mut message.attachments {
                    Some(attachments) => attachments.push(attachment),
                    None => message.attachments = Some(vec![attachment]),
                }
                attached += 1;
            }
            Err(error) => {
                log::error!("[VID-TOOL] артефакт {} не приложен: {}", id, error);
            }
        }
    }
    Ok(attached)
}

/// Прочитать webm с диска как ChatAttachment (только путь — без base64).
pub fn video_path_to_attachment(
    path: &str,
) -> Result<crate::infra::ChatAttachment, String> {
    let bytes_len = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|error| {
            log::error!("[VID-TOOL] Не удалось прочитать параметры клипа {:?}: {}", path, error);
            "Созданный клип недоступен".to_string()
        })?;
    if bytes_len == 0 {
        return Err("Созданный клип пустой".to_string());
    }
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "clip.webm".to_string());
    Ok(crate::infra::ChatAttachment {
        file_name,
        mime_type: mime_for_ext(path).to_string(),
        data_base64: String::new(),
        file_path: Some(path.to_string()),
        is_dir: Some(false),
    })
}

/// Гейт роли изображения для generate_video (§00.2: раньше состояния «роль
/// картинки» не существовало вовсе — диспетчер при любом вложении принуждал
/// i2va, и эскиз-референс становился первым кадром). Теперь выбор явный.
/// Возвращает Some(текст ошибки), когда вызов надо отклонить ДО резолва ID;
/// None — вызов продолжается (resolve ID или t2va).
pub fn video_image_gate_error(
    image_usage: Option<&str>,
    has_source_image_ids: bool,
    has_current_attachments: bool,
) -> Option<String> {
    match image_usage {
        Some("first_frame") | None => {
            if has_source_image_ids {
                // ID есть (текущий запрос или история) — resolve решит.
                // first_frame допускает РОВНО ОДИН id — счёт проверяет тул.
                return None;
            }
            if image_usage == Some("first_frame") {
                return Some(
                    "image_usage:\"first_frame\" требует source_image_ids с ID картинки — она станет первым кадром видео.".to_string(),
                );
            }
            if has_current_attachments {
                // Вложения есть, роль не выбрана — молча терять данные юзера
                // запрещено, но и молча делать картинку первым кадром нельзя.
                return Some(
                    "К текущему запросу прикреплены изображения — выбери их роль явно. Хочешь картинку ПЕРВЫМ КАДРОМ (оживить/продолжить её): передай 'source_image_ids' с её ID и image_usage:\"first_frame\". Картинки — РЕФЕРЕНСЫ сцены/персонажа (движок учтёт их как образцы): передай их ID (1–3) и image_usage:\"reference\".".to_string(),
                );
            }
            None
        }
        Some("reference") => {
            // reference теперь идёт в движок как ref_images (режим ref2va):
            // 1–3 ID разрешены. reference БЕЗ ID при прикреплённых картинках —
            // отказ: иначе картинки юзера тихо не дойдут до движка (ложь).
            // Счёт (≤3) проверяет тул структурно.
            if !has_source_image_ids && has_current_attachments {
                return Some(
                    "image_usage:\"reference\" требует source_image_ids (1–3 ID картинок) — \
                     референсы уходят в движок как образцы сцены/персонажа. \
                     Если картинка должна стать ПЕРВЫМ КАДРОМ — используйте first_frame с одним ID."
                        .to_string(),
                );
            }
            None
        }
        Some(other) => Some(format!(
            "Неизвестное image_usage:\"{}\" — допустимы \"first_frame\" (картинка = первый кадр) или \"reference\" (картинки = референсы сцены).",
            other
        )),
    }
}

pub struct GenerateVideo;

impl Tool for GenerateVideo {
    fn name(&self) -> &str {
        "generate_video"
    }
    fn description(&self) -> &str {
        "Сгенерировать короткое видео (+аудио) по английскому промпту: либо с нуля по тексту, либо ПО ПРИКРЕПЛЁННОЙ КАРТИНКЕ. Роль картинки задаёт image_usage: \"first_frame\" — картинка станет ПЕРВЫМ КАДРОМ видео (передай её ID в source_image_ids); \"reference\" — картинки-референсы (персонаж, стиль, позы): передай их ID (1–3) в source_image_ids, и движок учтёт их при генерации (речь персонажа референса задаётся в промпте). Клип будет приложен к ответу как файл."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt_en": {"type": "string", "description": "Развёрнутый английский промпт: объект, действие/движение, окружение, свет, стиль. Описывай ДВИЖЕНИЕ сцены и камеры, а не статичную картинку. При i2va описывай, КАК сцена на картинке оживает и движется; при reference — желаемую сцену с учётом референсов (что взять с каждой). Если пользователь просит РЕЧЬ/озвучку/диалог — включи реплику ДОСЛОВНО на языке пользователя, не переводя: The woman looks at the camera and says in Russian: <реплика> (реплика ≤10–12 слов, без кавычек и спецсимволов), и добавь в конец промпта: No subtitles, no captions, no on-screen text."},
                "image_usage": {"type": "string", "enum": ["first_frame", "reference"], "description": "Роль прикреплённых картинок. \"first_frame\" — картинка станет ПЕРВЫМ КАДРОМ видео (оживить/продолжить само изображение): требует ровно один source_image_ids. \"reference\" — картинки (персонаж, character sheet, эскиз, пример позы/стиля) — референсы сцены: передай их ID (1–3), движок учтёт их как образцы; без ID сцена описывается только текстом. Передавай всегда, когда к запросу прикреплены изображения."},
                "source_image_ids": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 3, "description": "ID картинок из блока доступных изображений. first_frame: РОВНО ОДИН ID — картинка станет первым кадром (image-to-video). reference: 1–3 ID — картинки пойдут в движок референсами (консистентный персонаж/стиль); больше 3 не принимается. Опционально: без него видео генерируется с нуля по тексту. Файловые пути запрещены."},
                "seconds": {"type": "integer", "minimum": 1, "maximum": 15, "description": "Длительность клипа в секундах (по умолчанию 5). Лимит зависит от выбранного набора моделей: больше — только если пользователь прямо попросил."},
                "final_quality": {"type": "boolean", "description": "false (по умолчанию) = черновик (быстро, минуты). true = финальное качество (медленно: 10+ минут на 5 секунд — предупреждай пользователя о времени)."},
                "lora": {"type": "array", "minItems": 1, "items": {"oneOf": [{"type": "string", "minLength": 1}, {"type": "object", "required": ["name"], "properties": {"name": {"type": "string", "minLength": 1}, "strength": {"type": "number", "minimum": 0, "maximum": 2}}}]}, "description": "Опционально: элементы — строка (стем файла LoRA без расширения) ИЛИ объект {\"name\": стем, \"strength\": 0–2}. Смешивать формы можно. Сила элемента переопределяет lora_strength; без strength применяется lora_strength (общий дефолт, по умолчанию 1.0). Какие лоры и с какой силой ставить — правило вашего агента (например, категория «взрослый контент»; обычные запросы не получают LoRA). Файлы LoRA лежат в подпапке базы активного видео-движка (…\\video_loras\\<база>\\): ищите стемы только этой базы — лора чужой базы найдена не будет."},
                "lora_strength": {"type": "number", "minimum": 0, "maximum": 2, "description": "Общая сила-дефолт LoRA (0–2, по умолчанию 1.0): применяется к элементам lora БЕЗ собственной strength. Передавать только вместе с lora."}
            },
            "required": ["prompt_en"]
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage(
            "generate_video должен исполняться через диспетчер с media-реестром"
                .to_string(),
        ))
    }
}

/// Схема video-тула для промпта агента (мета-имя "media").
pub fn video_tool_schemas(agent_tools: &[String]) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    if agent_tools.iter().any(|t| t == "generate_video") {
        let t = GenerateVideo;
        out.push(("media".to_string(), t.name().to_string(), serde_json::json!({
            "name": t.name(), "description": t.description(), "inputSchema": t.parameters(),
        })));
    }
    out
}

/// Исполнить video-тул по имени (диспетчер; VRAM-suspend делает вызывающий).
/// `attachments` — ресолвнутые по source_image_ids картинки текущего запроса.
/// Роль из args (`image_usage`) решает трактовку: first_frame = ровно одна
/// картинка-первый-кадр (i2va); reference = 1–3 референса (ref2va, идут в
/// движок как ref_images); без роли и картинок — t2va.
pub fn execute_video_tool(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
    attachments: &[crate::infra::ChatAttachment],
) -> Option<Result<String, ToolError>> {
    match name {
        "generate_video" => {
            let prompt = args
                .get("prompt_en")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim();
            if prompt.is_empty() {
                return Some(Err(ToolError::Usage("prompt_en пуст".to_string())));
            }
            // Роль картинок решает, что означает вложение/ID: first_frame =
            // РОВНО один первый кадр (i2va); reference = 1–3 референса (ref2va,
            // структурный лимит: референсов >6 модель ломает следование, лимит
            // 3 зашит сюда и в схему); без роли при вложениях диспетчер уже
            // отказал (video_image_gate_error) — здесь её отсутствие = t2va.
            // Счётные проверки — ДО resolve (отказ без обращения к диску).
            let role = args
                .get("image_usage")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            match role {
                "first_frame" if attachments.len() != 1 => {
                    return Some(Err(ToolError::Usage(format!(
                        "first_frame принимает РОВНО одну картинку (первый кадр), передано {}. \
                         Режим «первый + последний кадр» в этой задаче не открыт.",
                        attachments.len()
                    ))));
                }
                "reference" if attachments.len() > 3 => {
                    return Some(Err(ToolError::Usage(format!(
                        "reference принимает от 1 до 3 референсов (больше модель не выдерживает: \
                         ломается следование), передано {}.",
                        attachments.len()
                    ))));
                }
                _ if attachments.len() > 1 => {
                    // Вложения без роли сюда дойти не должны (диспетчерский
                    // гейт), но молча терять лишние запрещено и здесь.
                    return Some(Err(ToolError::Usage(format!(
                        "Передано {} картинок без роли — укажи image_usage \
                         (\"first_frame\" для первого кадра, \"reference\" для референсов).",
                        attachments.len()
                    ))));
                }
                _ => {}
            }
            // Пустой запрос без картинок — чистый t2va: resolve не зовём
            // (он на пустом списке честно ошибается «Не выбрано изображение»).
            let paths = if attachments.is_empty() {
                Vec::new()
            } else {
                match crate::infra::tools::media::resolve_ref_paths(attachments, ctx.session_id) {
                    Ok(paths) => paths,
                    Err(error) => return Some(Err(ToolError::Usage(error))),
                }
            };
            let (init_path, ref_paths): (Option<String>, Vec<String>) = match role {
                "first_frame" => (Some(paths[0].clone()), Vec::new()),
                "reference" => (None, paths),
                _ => (paths.into_iter().next(), Vec::new()),
            };
            let seconds = args
                .get("seconds")
                .and_then(|value| value.as_u64())
                .map(|s| s as u32)
                .unwrap_or(5);
            let final_quality = args
                .get("final_quality")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            // LoRA-прокидка (зеркало media.rs): опциональные поля; невалидные
            // имена/сила — честный отказ Usage/NotFound ДО обращения к движку.
            // Пусто/отсутствие — прежний путь без лор.
            let loras = match crate::infra::tools::media::media_lora::extract_lora_request(args) {
                Ok(None) => Vec::new(),
                Ok(Some(req)) => {
                    let specs = match crate::infra::tools::media::media_lora::resolve_video_specs(&req) {
                        Ok(specs) => specs,
                        Err(e) => return Some(Err(e)),
                    };
                    log::info!(
                        "{}",
                        crate::infra::tools::media::media_lora::video_status_line(&req, &specs)
                    );
                    specs
                }
                Err(e) => return Some(Err(e)),
            };
            Some(
                run_generate_video(
                    prompt,
                    seconds,
                    final_quality,
                    init_path.as_deref(),
                    &ref_paths,
                    &loras,
                    ctx,
                )
                .and_then(|output| format_video_result(output, ctx)),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "video_tests.rs"]
mod video_tests;
