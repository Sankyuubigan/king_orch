//! 🎬 Инструмент видео-генерации (`generate_video`).
//!
//! Тонкая схема + исполнение поверх `tauri-plugin-image-engine::engine` (SSOT
//! движка: дефолты размеров/seed/fps/лимит seconds — из пресета каталога,
//! никаких локальных копий констант). По образцу `media.rs` (generate_image),
//! но: один тул, артефакт прикрепляется к ответу ПО ПУТИ (`file_path`), потому
//! что webm-клипы десятки МБ — base64-энкачить их в `messages[]` нельзя.
//!
//! Файловая точка сохранения: `<exe_dir>/video_outputs/<session_id>/vid_<ts>_<seq>.ext`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri_plugin_image_engine::engine::{RefModBundle, VideoSamplingPlan, sampling_plan_for_quality};

use crate::infra::tools::media::ImageArtifactRegistry;
use crate::infra::tools::{Tool, ToolCtx, ToolError};

static VIDEO_OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct VideoRunOutput {
    path: String,
    time_sec: f64,
    /// Сид прогона (клиентский) — уходит в маркер для replay-рефайна.
    seed: i64,
    /// Прогон — рефайн последнего видео (приписка в маркере результата).
    refined: bool,
}

/// Память последнего успешного видео-запроса сессии — основа replay-рефайна:
/// движок видео-латенты не хранит, поэтому рефайн = перегенерация драфта с
/// тем же сидом и теми же параметрами (R0.4 seed-replay: тот же сид+драфт →
/// визуально идентичный ролик). Кэш in-memory + файл на диске
/// (`video_outputs/<session>/last_video_req.json`, рядом с клипами сессии):
/// после рестарта приложения память дочитывается с диска при первом
/// обращении — рестарт больше не делает рефайн невозможным (баг 06.11.26:
/// стёртая память вела к отказу, а отказ — к «самодеятельной» новой
/// генерации с чужим персонажем). Записи нет → честный отказ (молча
/// генерировать новое видео нельзя).
#[derive(Clone, Serialize, Deserialize)]
struct LastVideoReq {
    prompt: String,
    seconds: u32,
    /// Референсы (пути кадров карточки/вложений) — при refmod перечитываются
    /// с диска, иначе переиспользуются сохранённые пути.
    refmod_slug: Option<String>,
    ref_paths: Vec<String>,
    ref_image_size_max: bool,
    ref_audio_paths: Vec<String>,
    /// Путь первого кадра (i2va). Рефайн i2va бенчем не проверен → honest отказ.
    init_path: Option<String>,
    loras: Vec<tauri_plugin_image_engine::engine::LoraSpec>,
    seed: i64,
    /// Честный реплей (2.3 Euler-PRP): разрешённый план ДРАФТА на момент его
    /// генерации (шаги/сэмплер/шедулер/shift/вес turbo-лоры). Рефайн реплеит
    /// ИМЕННО его — смена каталога не переписывает уже снятые видео. None =
    /// память старого формата (serde default) — план берётся из текущего
    /// каталога (задокументированное поведение).
    #[serde(default)]
    draft_plan: Option<VideoSamplingPlan>,
    /// Видео уже прошло рефайн — повторный даст тот же результат (honest отказ).
    refined: bool,
}

static LAST_VIDEO_REQUESTS: Mutex<Option<HashMap<String, LastVideoReq>>> = Mutex::new(None);

fn lock_last_video() -> std::sync::MutexGuard<'static, Option<HashMap<String, LastVideoReq>>> {
    LAST_VIDEO_REQUESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn remember_last_video(session_id: &str, req: LastVideoReq) {
    lock_last_video()
        .get_or_insert_with(HashMap::new)
        .insert(session_id.to_string(), req.clone());
    // Диск: рефайн должен переживать рестарт приложения (баг 06.11.26:
    // рестарт стёр память → отказ → агент «выкрутился» новой генерацией
    // без карточки — чужой персонаж). Ошибка записи не валит успешную
    // генерацию: рефайн после рестарта тогда честно откажет.
    let dir = video_session_outputs_dir(session_id);
    if let Err(err) = persist_last_video(&dir, &req) {
        log::warn!(
            "[VID-TOOL] память рефайна не записана на диск ({}): {} — после рестарта рефайн откажет",
            dir.join(LAST_VIDEO_REQ_FILE).display(),
            err
        );
    }
}

fn last_video_request(session_id: &str) -> Option<LastVideoReq> {
    let mut guard = lock_last_video();
    if let Some(req) = guard.as_ref().and_then(|map| map.get(session_id)) {
        return Some(req.clone());
    }
    // Промах кэша (рестарт приложения / первое обращение): дочитываем с диска.
    let req = load_last_video(&video_session_outputs_dir(session_id))?;
    guard
        .get_or_insert_with(HashMap::new)
        .insert(session_id.to_string(), req.clone());
    Some(req)
}

/// Файл памяти рефайна в каталоге outputs сессии.
const LAST_VIDEO_REQ_FILE: &str = "last_video_req.json";

/// Записать память рефайна в каталог `dir` (чистая функция — тестируется
/// напрямую на произвольном каталоге).
fn persist_last_video(dir: &std::path::Path, req: &LastVideoReq) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(req).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(LAST_VIDEO_REQ_FILE), json)
}

/// Дочитать память рефайна из каталога `dir`; файла нет — None (штатно:
/// драфта ещё не было), битый JSON — None + warn (не отказ генерации).
fn load_last_video(dir: &std::path::Path) -> Option<LastVideoReq> {
    let path = dir.join(LAST_VIDEO_REQ_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str(&text) {
        Ok(req) => Some(req),
        Err(err) => {
            log::warn!(
                "[VID-TOOL] файл памяти рефайна повреждён ({}): {}",
                path.display(),
                err
            );
            None
        }
    }
}

/// Клиентский сид (i64 > 0): наносекунды времени ^ счётчик прогонов,
/// смешанный константой Кнута. Сервер сид -1 не возвращает (рандомит молча),
/// поэтому сид всегда генерирует хост — иначе replay-рефайн невозможен.
fn client_seed() -> i64 {
    static SEED_SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mix = nanos
        ^ SEED_SEQ
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    // Снимаем знаковый бит: сид для CLI -s — положительное число.
    (mix & 0x7FFF_FFFF_FFFF_FFFF) as i64
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
        "mp4" | "m4v" | "mov" => "video/mp4",
        "mkv" => "video/x-matroska",
        "ts" => "video/mp2t",
        "flv" => "video/x-flv",
        "wmv" => "video/x-ms-wmv",
        // ffmpeg_run умеет извлекать аудио-дорожку в файл — карточка должна
        // нести честный mime аудио, а не дефолтный video/webm.
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" | "opus" => "audio/ogg",
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

/// Пути ref_audios из голоса RefMod-карточки (Этап 3, voice cloning H3):
/// голос Some → РОВНО один путь; карточки нет / голоса нет → пусто.
/// Источник голоса — только карточка (у картинок-вложений голоса нет
/// по построению). Выделена в чистую функцию для герметичного теста.
fn refmod_voice_paths(bundle: Option<&RefModBundle>) -> Vec<String> {
    bundle
        .and_then(|bundle| bundle.voice.as_ref())
        .map(|voice| vec![voice.to_string_lossy().into_owned()])
        .unwrap_or_default()
}

fn run_generate_video(
    prompt_en: &str,
    seconds: u32,
    keep_last_seed: bool,
    init_path: Option<&str>,
    ref_paths: &[String],
    ref_image_size_max: bool,
    ref_audio_paths: &[String],
    loras: &[tauri_plugin_image_engine::engine::LoraSpec],
    refmod_slug: Option<String>,
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

    // Клиентский сид на каждый прогон: сервер сид -1 не возвращает (рандомит
    // молча) — сид всегда генерирует хост, иначе replay-рефайн невозможен.
    // keep_last_seed (2.7 Euler-PRP): «повтори то же видео, но …» — сид
    // последнего видео сессии даёт ту же сцену; памяти нет — новый случайный.
    let seed = if keep_last_seed {
        last_video_request(ctx.session_id)
            .map(|req| req.seed)
            .unwrap_or_else(client_seed)
    } else {
        client_seed()
    };
    log::info!(
        "[VID-TOOL] generate_video start: bundle={}, seconds={}, mode={}, refs={}, audio={}, ref_image_size_max={}, keep_seed={}, seed={}",
        entry.name,
        seconds,
        mode,
        ref_paths.len(),
        ref_audio_paths.len(),
        ref_image_size_max,
        keep_last_seed,
        seed
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
        init_path,
        ref_paths,
        ref_image_size_max,
        &ref_audio_paths,
        Some(seed),
        &out_path,
        None,
        loras,
        log_cb,
    )
    .map(|r| {
        log::info!("[VID-TOOL] generate_video done: {} ({:.1} сек)", r.path, r.time_sec);
        // Память последнего запроса сессии (основа replay-рефайна): сид + все
        // параметры драфта. RefMod-слаг перезапоминается: карточку рефайн
        // перечитывает с диска (кадры/голос актуальны на момент рефайна).
        remember_last_video(
            ctx.session_id,
            LastVideoReq {
                prompt: prompt_en.to_string(),
                seconds,
                refmod_slug,
                ref_paths: ref_paths.to_vec(),
                ref_image_size_max,
                ref_audio_paths: ref_audio_paths.to_vec(),
                init_path: init_path.map(str::to_string),
                loras: loras.to_vec(),
                seed,
                // План драфта замораживается на момент генерации (2.3): рефайн
                // реплеит его, а не текущий каталог.
                draft_plan: Some(sampling_plan_for_quality(&entry, &bundle_dir)),
                refined: false,
            },
        );
        VideoRunOutput { path: r.path, time_sec: r.time_sec, seed, refined: false }
    })
    .map_err(ToolError::Io)
}

/// Рефайн последнего видео сессии: перегенерация драфта с тем же сидом и
/// параметрами + латент-апскейл 1.4× + второй проход (конфиг каталога
/// `preset.refine`, бенч-паритет кейса E — 412 с на 5-сек клип). Промпт,
/// рефы и голос берутся ИЗ ПАМЯТИ последнего запроса — аргументы текущего
/// вызова тула игнорируются (гейт честности: любые conflicting args — отказ).
fn run_refine_video(ctx: &ToolCtx) -> Result<VideoRunOutput, ToolError> {
    let Some(saved) = last_video_request(ctx.session_id) else {
        return Err(ToolError::Usage(
            "Недавнего видео для рефайна нет: сначала сгенерируй видео в этой сессии \
             (draft), потом попроси улучшить его."
                .to_string(),
        ));
    };
    // Рефайн = replay драфта: повторный рефайн того же сида даст тот же
    // результат — honest отказ вместо бесполезного 7-минутного прогона.
    if saved.refined {
        return Err(ToolError::Usage(
            "Это видео уже улучшено (рефайн применён). Повторный рефайн того же драфта \
             даст тот же результат — попроси новое видео вместо повтора."
                .to_string(),
        ));
    }
    if !saved.loras.is_empty() {
        return Err(ToolError::Usage(
            "Рефайн не воспроизводит пользовательские LoRA (CLI движка грузит лоры только \
             целым каталогом, одиночной силы нет) — рефайн с другим LoRA-стеком дал бы \
             ДРУГОЕ видео. Попроси новое видео с этими лорами вместо рефайна."
                .to_string(),
        ));
    }
    if saved.init_path.is_some() {
        return Err(ToolError::Usage(
            "Рефайн проверен для видео по тексту и по референсам (RefMod/вложения); \
             для видео из первого кадра (first_frame) механика replay бенчем не проверена — \
             отказ вместо угадывания. Попроси новое видео."
                .to_string(),
        ));
    }
    let entry = resolve_video_bundle_for_tools()?;
    let Some(refine) = entry.preset.refine.clone() else {
        return Err(ToolError::Usage(format!(
            "Активный видео-бандл «{}» не объявляет параметры рефайна (preset.refine) — \
             рефайн недоступен. Обновите набор моделей.",
            entry.name
        )));
    };
    let engine_dir = tauri_plugin_image_engine::engine::engine_dir_early();
    let bundle_dir = tauri_plugin_image_engine::engine::video_bundle_dir_early();
    if !tauri_plugin_image_engine::engine::sdcpp_installer::has_any_installed(&engine_dir) {
        return Err(ToolError::NotFound(
            "Движок изображений не установлен. Откройте Настройки → «Движок запуска ImageGEN моделей» и нажмите «Установить движок».".to_string(),
        ));
    }
    // RefMod-карточка перечитывается с диска: кадры/голос актуальны на момент
    // рефайна (юзер мог добавить голос после драфта — он попадёт в рефайн).
    let (ref_paths, ref_audio_paths, ref_image_size_max) = match &saved.refmod_slug {
        Some(slug) => {
            let bundle = crate::infra::tools::refmod::load_refmod_card(slug).map_err(
                |error| ToolError::NotFound(format!("RefMod-карточка «{}» недоступна: {}", slug, error)),
            )?;
            (
                crate::infra::tools::refmod::engine_ref_paths(&bundle),
                refmod_voice_paths(Some(&bundle)),
                // false → ключ ref_image_args НЕ ставится: движок ужмёт рефы
                // штатным `match` (до площади драфт-холста 864×480 ≈ 640px).
                // Раньше `max` (2048px) раздувал активации: +2 ГБ VRAM и
                // +100 с (X3a/X3c, tasks/261105_1528-92e5af13). Качество лица
                // не страдает (X3a≈X3c, X7≈P4_1152).
                false,
            )
        }
        None => (
            saved.ref_paths.clone(),
            saved.ref_audio_paths.clone(),
            saved.ref_image_size_max,
        ),
    };
    log::info!(
        "[VID-TOOL] refine start: bundle={}, seed={}, refs={}, audio={}, refmod={}",
        entry.name,
        saved.seed,
        ref_paths.len(),
        ref_audio_paths.len(),
        saved.refmod_slug.as_deref().unwrap_or("-")
    );
    let out_dir = video_session_outputs_dir(ctx.session_id);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .map_err(|error| ToolError::Io(format!("Некорректное системное время: {}", error)))?;
    let sequence = VIDEO_OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    // Новый файл: драфт сохраняется, рефайн рядом (не перезапись драфта).
    let out_path = out_dir.join(format!("vid_{}_{}_refined.webm", ts, sequence));
    let log_cb = |msg: String| log::info!("[VID-TOOL] {}", msg);

    tauri_plugin_image_engine::engine::generate_video_refine(
        &engine_dir,
        &bundle_dir,
        &entry,
        &refine,
        // План драфта из памяти (2.3): реплей тем же планом, что снял драфт.
        saved.draft_plan.clone(),
        &saved.prompt,
        saved.seconds,
        saved.seed,
        &ref_paths,
        ref_image_size_max,
        &ref_audio_paths,
        &out_path,
        None,
        log_cb,
    )
    .map(|r| {
        log::info!("[VID-TOOL] refine done: {} ({:.1} сек)", r.path, r.time_sec);
        // Память хранит параметры драфта: флаг refined=true закрывает повторный
        // рефайн того же сида (результат воспроизводим и бесполезен).
        remember_last_video(
            ctx.session_id,
            LastVideoReq { refined: true, ..saved },
        );
        VideoRunOutput { path: r.path, time_sec: r.time_sec, seed: r.seed, refined: true }
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
        "[VIDEO_SAVED id={id}] Готово за {:.1} сек (seed={}{})",
        output.time_sec,
        output.seed,
        if output.refined {
            ", улучшено (рефайн: выше разрешение, второй проход)"
        } else {
            ""
        }
    ))
}

/// Пост-проход run_chat: маркер [VIDEO_SAVED] в результатах tool_calls →
/// цепляет клип к финальному сообщению агента по `file_path` (webm/mp4 в
/// base64 не энкодятся — клипы десятки МБ, messages[] не резины). Маркер
/// ставят и media-тулы (`generate_video`), и ffmpeg-тулы (`ffmpeg_run` —
/// результат монтажа рядом с исходником). Возвращает число прицепленных
/// видео (частичная ошибка одного артефакта не валит остальные).
pub fn attach_saved_videos_from_sub_calls(
    message: &mut crate::infra::ChatMessage,
    sub_calls: &[crate::infra::SubCall],
    registry: &ImageArtifactRegistry,
) -> Result<usize, String> {
    let mut artifacts: Vec<(String, String)> = Vec::new(); // (id, tool_name)
    for sub_call in sub_calls {
        for tool_call in &sub_call.tool_calls {
            if tool_call.tool_name != "generate_video" && tool_call.tool_name != "ffmpeg_run" {
                continue;
            }
            if let Some(id) = extract_saved_video_id(&tool_call.result) {
                let key = (id, tool_call.tool_name.clone());
                if !artifacts.contains(&key) {
                    artifacts.push(key);
                }
            }
        }
    }
    let mut attached = 0;
    for (id, tool_name) in artifacts {
        let path = registry.resolve(&id, &tool_name)?;
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
        "Сгенерировать короткое видео (+аудио) по английскому промпту: либо с нуля по тексту, либо ПО ПРИКРЕПЛЁННОЙ КАРТИНКЕ. Роль картинки задаёт image_usage: \"first_frame\" — картинка станет ПЕРВЫМ КАДРОМ видео (передай её ID в source_image_ids); \"reference\" — картинки-референсы (персонаж, стиль, позы): передай их ID (1–3) в source_image_ids, и движок учтёт их при генерации (речь персонажа референса задаётся в промпте). Персонажа из каталога RefMod-карточек бери аргументом refmod (кадры карточки станут референсами движка). Улучшение уже готового видео — ОТДЕЛЬНЫЙ тул refine_last_video. Клип будет приложен к ответу как файл."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt_en": {"type": "string", "description": "Развёрнутый английский промпт: объект, действие/движение, окружение, свет, стиль. Описывай ДВИЖЕНИЕ сцены и камеры, а не статичную картинку. При i2va описывай, КАК сцена на картинке оживает и движется; при reference — желаемую сцену с учётом референсов (что взять с каждой). Если пользователь просит РЕЧЬ/озвучку/диалог — включи реплику ДОСЛОВНО на языке пользователя, не переводя: The woman looks at the camera and says in Russian: <реплика> (реплика ≤10–12 слов, без кавычек и спецсимволов), и добавь в конец промпта: No subtitles, no captions, no on-screen text."},
                "image_usage": {"type": "string", "enum": ["first_frame", "reference"], "description": "Роль прикреплённых картинок. \"first_frame\" — картинка станет ПЕРВЫМ КАДРОМ видео (оживить/продолжить само изображение): требует ровно один source_image_ids. \"reference\" — картинки (персонаж, character sheet, эскиз, пример позы/стиля) — референсы сцены: передай их ID (1–3), движок учтёт их как образцы; без ID сцена описывается только текстом. Передавай всегда, когда к запросу прикреплены изображения."},
                "refmod": {"type": "string", "description": "Slug RefMod-карточки персонажа из блока «Доступные RefMod-карточки» контекста: кадры карточки пойдут референсами движка (Ref2VA) — внешность/персонаж возьмутся с карточки, сцена описывается prompt_en. НЕ сочетается с source_image_ids и image_usage (референсы либо из карточки, либо из вложений). В движок уходят ВСЕ кадры карточки (до 8), порядок первых трёх — конвенция лицо → ракурс 3/4 → фигура; кадры 4–8 могут быть любыми (все лица, лица+фигура и т.п.). Если у карточки есть голос (🎤 в каталоге) — он уйдёт в ref_audios движка: персонаж заговорит этим голосом, реплику задай в prompt_en в кавычках. Лоры при этом не нужны."},
                "source_image_ids": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 3, "description": "ID картинок из блока доступных изображений. first_frame: РОВНО ОДИН ID — картинка станет первым кадром (image-to-video). reference: 1–3 ID — картинки пойдут в движок референсами (консистентный персонаж/стиль); больше 3 не принимается. Опционально: без него видео генерируется с нуля по тексту. Файловые пути запрещены."},
                "seconds": {"type": "integer", "minimum": 1, "maximum": 15, "description": "Длительность клипа в секундах (по умолчанию 5). Лимит зависит от выбранного набора моделей: больше — только если пользователь прямо попросил."},
                "max_quality": {"type": "boolean", "description": "true = сразу в максимальном качестве: автоматическая цепочка «черновик → улучшение» двумя проходами внутри одного вызова (в ответ придёт ОДИН готовый улучшенный клип). Долго: ~15 минут на 5-сек клип — предупреждай пользователя о времени. НЕ сочетается с first_frame и пользовательскими LoRA (механика улучшения для них не проверена — отказ). По умолчанию false = черновик (быстро, минуты)."},
                "keep_last_seed": {"type": "boolean", "description": "true = взять сид последнего видео сессии вместо нового случайного: «повтори то же видео, но …» даст ту же сцену с изменением, а не другого персонажа. Видео в сессии ещё не было — новый случайный сид. По умолчанию false."},
                "lora": {"type": "array", "items": {"oneOf": [{"type": "string", "minLength": 1}, {"type": "object", "required": ["name"], "properties": {"name": {"type": "string", "minLength": 1}, "strength": {"type": "number", "minimum": 0, "maximum": 2}}}]}, "description": "Опционально: элементы — строка (стем файла LoRA без расширения) ИЛИ объект {\"name\": стем, \"strength\": 0–2}. Смешивать формы можно. Пустой массив или опущенное поле = генерация БЕЗ лор (легально). Сила элемента переопределяет lora_strength; без strength применяется lora_strength (общий дефолт, по умолчанию 1.0). Какие лоры и с какой силой ставить — правило вашего агента (например, категория «взрослый контент»; обычные запросы не получают LoRA). Стемы доступных лор перечислены в каталоге LoRA вашего контекста — используй ТОЛЬКО их; файлы лежат в подпапке базы активного видео-движка (…\\video_loras\\<база>\\): лора чужой базы найдена не будет."},
                "lora_strength": {"type": "number", "minimum": 0, "maximum": 2, "description": "Общая сила-дефолт LoRA (0–2, по умолчанию 1.0): применяется к элементам lora БЕЗ собственной strength. Без lora (или с пустым lora) — игнорируется."}
            }
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

/// Улучшение последнего видео сессии — ОТДЕЛЬНЫЙ тул с ПУСТОЙ схемой (2.6
/// Euler-PRP, структурная защита §00.2): ни промпта, ни рефов, ни сида
/// передать невозможно — нода рефайна физически не может сгенерировать новое
/// видео взамен улучшаемого.
pub struct RefineLastVideo;

impl Tool for RefineLastVideo {
    fn name(&self) -> &str {
        "refine_last_video"
    }
    fn description(&self) -> &str {
        "Улучшить ПОСЛЕДНЕЕ видео этой сессии: тот же промпт, референсы, голос и сид перегенерируются в повышенном разрешении (1152×640) со вторым проходом уточнения. НИКАКИХ аргументов — всё берётся из последнего видео сессии. Откажет, если: видео в этой сессии ещё не было; оно уже улучшалось; оно делалось из первого кадра; в нём были пользовательские LoRA. Долго: ~10 минут на 5-сек клип — предупреждай пользователя о времени. Клип будет приложен к ответу как файл."
    }
    fn parameters(&self) -> Value {
        // Пустая схема — структурная защита: передавать нечего.
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }
    fn is_readonly(&self) -> bool {
        false
    }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage(
            "refine_last_video должен исполняться через диспетчер с media-реестром"
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
    if agent_tools.iter().any(|t| t == "refine_last_video") {
        let t = RefineLastVideo;
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
            // RefMod-карточка: рефы движка из каталога карточек вместо вложений
            // (единая лёгкая архитектура без лор, tasks/261104_0230-db2cb59c).
            let refmod_bundle = match args
                .get("refmod")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(slug) => {
                    // Гейт (§00.2): рефы ЛИБО из карточки, ЛИБО из вложений —
                    // состояние «те и другие» не существует: Ref2VA нельзя
                    // комбинировать с init-image (api.md), reference — это
                    // вложения. Отказ ДО обращения к диску.
                    if !attachments.is_empty() || !role.is_empty() {
                        return Some(Err(ToolError::Usage(
                            "refmod не сочетается с вложениями и image_usage: референсы \
                             либо из RefMod-карточки (аргумент refmod), либо из картинок \
                             (source_image_ids + image_usage). Передай что-то одно."
                                .to_string(),
                        )));
                    }
                    match crate::infra::tools::refmod::load_refmod_card(slug) {
                        Ok(bundle) => {
                            log::info!(
                                "[VID-TOOL] refmod '{}': кадров в карточке {}, уйдут в движок все {}",
                                bundle.slug,
                                bundle.frames.len(),
                                crate::infra::tools::refmod::FRAMES_PER_RUN
                            );
                            if bundle.voice.is_some() {
                                log::info!(
                                    "[VID-TOOL] refmod '{}': 🎤 голос карточки уйдёт в ref_audios (voice cloning H3)",
                                    bundle.slug
                                );
                            }
                            Some(bundle)
                        }
                        Err(e) => return Some(Err(e)),
                    }
                }
                None => None,
            };
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
            let (init_path, ref_paths): (Option<String>, Vec<String>) =
                if let Some(bundle) = &refmod_bundle {
                    // Карточка: ВСЕ кадры идут референсами (05.11.26 — «мёртвых»
                    // кадров нет; формат ≤8, потолок ref2va по бюджету токенов).
                    (None, crate::infra::tools::refmod::engine_ref_paths(bundle))
                } else {
                    let paths = if attachments.is_empty() {
                        Vec::new()
                    } else {
                        match crate::infra::tools::media::resolve_ref_paths(
                            attachments,
                            ctx.session_id,
                        ) {
                            Ok(paths) => paths,
                            Err(error) => return Some(Err(ToolError::Usage(error))),
                        }
                    };
                    match role {
                        "first_frame" => (Some(paths[0].clone()), Vec::new()),
                        "reference" => (None, paths),
                        _ => (paths.into_iter().next(), Vec::new()),
                    }
                };
            // Этап 3: голос карточки → ref_audios движка (voice cloning H3).
            // Источник — ТОЛЬКО карточка refmod: у картинок-вложений голоса нет
            // по построению (состояние «у вложения есть голос» не существует).
            let ref_audio_paths = refmod_voice_paths(refmod_bundle.as_ref());
            let seconds = args
                .get("seconds")
                .and_then(|value| value.as_u64())
                .map(|s| s as u32)
                .unwrap_or(5);
            let keep_last_seed = args
                .get("keep_last_seed")
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
            // Цепочка «сразу в максимальном качестве» (2.5 Euler-PRP): драфт →
            // рефайн внутри одного вызова тула, в ответ — ОДИН улучшенный клип.
            // Гейты ДО драфта: честный отказ вместо сгоревших минут драфта и
            // гарантированного отказа на середине цепочки.
            let max_quality = args
                .get("max_quality")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if max_quality {
                if !loras.is_empty() {
                    return Some(Err(ToolError::Usage(
                        "Цепочка max_quality (черновик → улучшение) не воспроизводит \
                         пользовательские LoRA: улучшение грузит лоры только целым \
                         каталогом — стек изменится, это будет ДРУГОЕ видео. \
                         Сгенерируй черновик или убери лоры."
                            .to_string(),
                    )));
                }
                if init_path.is_some() {
                    return Some(Err(ToolError::Usage(
                        "Цепочка max_quality проверена для видео по тексту/референсам; \
                         для видео из первого кадра (first_frame) механика улучшения \
                         бенчем не проверена — отказ вместо угадывания. Сгенерируй \
                         черновик."
                            .to_string(),
                    )));
                }
            }
            let run = run_generate_video(
                prompt,
                seconds,
                keep_last_seed,
                init_path.as_deref(),
                &ref_paths,
                refmod_bundle.is_some(),
                &ref_audio_paths,
                &loras,
                refmod_bundle.as_ref().map(|b| b.slug.clone()),
                ctx,
            );
            Some(match run {
                // max_quality: драфт уже в памяти (refined:false) — рефайн
                // реплеит его тем же сидом и тем же планом драфта.
                Ok(output) if max_quality => {
                    log::info!(
                        "[VID-TOOL] max_quality: драфт готов ({:.1} сек) — запускаю улучшение",
                        output.time_sec
        );
                    run_refine_video(ctx)
                        .and_then(|refined| format_video_result(refined, ctx))
                }
                Ok(output) => format_video_result(output, ctx),
                Err(e) => Err(e),
            })
        }
        "refine_last_video" => {
            // Схема пустая по построению (2.6), но чужие ключи не молчим:
            // аргументов не предусмотрено ВООБЩЕ — структурная защита.
            if let Some(obj) = args.as_object() {
                let extra: Vec<&String> = obj.keys().collect();
                if !extra.is_empty() {
                    return Some(Err(ToolError::Usage(format!(
                        "refine_last_video не принимает аргументов (получено: {}) — \
                         всё берётся из последнего видео этой сессии.",
                        extra
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))));
                }
            }
            Some(run_refine_video(ctx).and_then(|output| format_video_result(output, ctx)))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "video_tests.rs"]
mod video_tests;
