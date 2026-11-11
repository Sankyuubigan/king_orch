//! Тесты ffmpeg-тулов: гейты источников, валидаторы аргументов, билдер
//! выходного пути, фильтр схем. ЗАПРЕЩЕНО тестировать путь, ведущий к
//! ensure/спавну: в тест-среде движка нет, докачка недопустима — все
//! исполнения здесь останавливаются на гейтах ДО ensure_engine (§00.3.1:
//! гейты зафиксированы до прогона).

use std::path::{Path, PathBuf};

use super::*; // tools::ffmpeg — private-гейты доступны дочернему модулю тестов
use crate::infra::tools::media::ImageArtifactRegistry;
use crate::infra::tools::{ToolCtx, ToolError};

fn ctx_with_registry() -> (ToolCtx<'static>, &'static ImageArtifactRegistry) {
    // lease-паттерн из excel/mod.rs: static-строки живут дольше ctx.
    let registry: &'static ImageArtifactRegistry = Box::leak(Box::new(ImageArtifactRegistry::new()));
    let ctx = ToolCtx {
        workspace_root: Path::new("."),
        write_root: Path::new("."),
        write_outside: crate::infra::WriteOutside::Prompt,
        session_id: Box::leak("test_session".to_string().into_boxed_str()),
        approver: crate::infra::test_approver(),
        agent_id: "video_editor",
        bins_dir: Path::new("."),
        image_artifacts: Some(registry),
    };
    (ctx, registry)
}

fn sandbox(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test_tmp")
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_stub_video(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"FAKE-VIDEO-BYTES").unwrap();
    path
}

// ─────────────────────────── Схемы и имена ───────────────────────────

#[test]
fn schemas_filter_by_grant_and_meta() {
    let all = ffmpeg_tool_schemas(&[
        "ffmpeg_probe".to_string(),
        "ffmpeg_run".to_string(),
    ]);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].0, "ffmpeg");
    assert_eq!(all[0].1, "ffmpeg_probe");
    assert_eq!(all[1].1, "ffmpeg_run");
    assert!(all[1].2["inputSchema"]["properties"]["input_args"].is_object());
    assert!(
        all[1].2["inputSchema"]["properties"]["extra_inputs"].is_object(),
        "схема ffmpeg_run обязана объявлять extra_inputs (мульти-вход)"
    );

    // Без гранта в frontmatter агента схем нет; чужие тула не подтягиваются.
    assert!(ffmpeg_tool_schemas(&[]).is_empty());
    assert!(ffmpeg_tool_schemas(&["generate_video".to_string()]).is_empty());
}

#[test]
fn is_ffmpeg_tool_only_two_names() {
    assert!(is_ffmpeg_tool("ffmpeg_probe"));
    assert!(is_ffmpeg_tool("ffmpeg_run"));
    assert!(!is_ffmpeg_tool("generate_video"));
    assert!(!is_ffmpeg_tool("excel_run"));
    assert!(!is_ffmpeg_tool("ffmpeg"));
}

#[test]
fn unknown_name_is_none() {
    let (ctx, _reg) = ctx_with_registry();
    assert!(execute_ffmpeg_tool("excel_apply", &serde_json::json!({}), &ctx, &[]).is_none());
}

// ─────────────────────────── Валидаторы ───────────────────────────

#[test]
fn suffix_default_and_charset() {
    assert_eq!(sanitize_suffix(None).unwrap(), "_edited");
    assert_eq!(sanitize_suffix(Some("  ")).unwrap(), "_edited");
    assert_eq!(sanitize_suffix(Some("_cut")).unwrap(), "_cut");
    assert_eq!(sanitize_suffix(Some("v2-final")).unwrap(), "v2-final");
    for bad in ["с_русс", "two words", "dot.dot", "slash/esc", "back\\slash"] {
        assert!(sanitize_suffix(Some(bad)).is_err(), "'{bad}' обязан быть отклонён");
    }
}

#[test]
fn ext_default_from_source_and_charset() {
    let mp4 = Path::new("D:\\v\\clip.mp4");
    assert_eq!(sanitize_ext(None, mp4).unwrap(), "mp4");
    assert_eq!(sanitize_ext(Some("MKV"), mp4).unwrap(), "mkv");
    assert_eq!(sanitize_ext(Some("webm"), mp4).unwrap(), "webm");
    assert!(sanitize_ext(Some("a.b"), mp4).is_err(), "точка в ext запрещена");
    assert!(sanitize_ext(Some("w m"), mp4).is_err(), "пробел в ext запрещён");
    // У исходника без расширения — честный дефолт mp4.
    assert_eq!(sanitize_ext(None, Path::new("D:\\v\\clip")).unwrap(), "mp4");
}

#[test]
fn forbidden_token_catches_i_and_y_only() {
    assert_eq!(forbidden_token(&["-i".into()]), Some("-i"));
    assert_eq!(forbidden_token(&["-I".into()]), Some("-i"));
    assert_eq!(forbidden_token(&[" -y ".into()]), Some("-y"));
    // Легитимные аргументы, содержащие буквы i/y префиксом, не флагаются.
    assert_eq!(forbidden_token(&["-itsoffset".into(), "1".into()]), None);
    assert_eq!(forbidden_token(&["-vf".into(), "scale=1280:-1".into()]), None);
    assert_eq!(forbidden_token(&["-c:v".into(), "libx264".into()]), None);
    assert_eq!(forbidden_token(&[]), None);
}

#[test]
fn parse_args_list_types() {
    let absent = serde_json::json!({});
    assert!(parse_args_list(&absent, "input_args").unwrap().is_empty());
    let null = serde_json::json!({"input_args": null});
    assert!(parse_args_list(&null, "input_args").unwrap().is_empty());
    let ok = serde_json::json!({"input_args": ["-ss", "2", "-to", "5"]});
    assert_eq!(parse_args_list(&ok, "input_args").unwrap(), vec!["-ss", "2", "-to", "5"]);
    // Строка целиком — не массив токенов: честный отказ, не тихий спавн.
    let glued = serde_json::json!({"input_args": "-ss 2 -to 5"});
    let err = parse_args_list(&glued, "input_args").unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)));
    let mixed = serde_json::json!({"output_args": ["-c", 3]});
    assert!(parse_args_list(&mixed, "output_args").is_err());
}

#[test]
fn output_path_next_to_source_with_suffix() {
    let out = build_output_path(Path::new("D:\\v\\clip.mp4"), "_edited", "mp4");
    assert_eq!(out, PathBuf::from("D:\\v\\clip_edited.mp4"));
    let out2 = build_output_path(Path::new("D:\\v\\клип.webm"), "-cut", "mkv");
    assert_eq!(out2, PathBuf::from("D:\\v\\клип-cut.mkv"));
    // Суффикс исключает совпадение с исходником даже при пустом stem-хвосте.
    let out3 = build_output_path(Path::new("D:\\v\\_edited.mp4"), "_edited", "mp4");
    assert_ne!(out3, PathBuf::from("D:\\v\\_edited.mp4"));
}

// ─────────────────────────── Гейт источника ───────────────────────────

#[test]
fn source_gate_allows_attachment_and_session_artifact_only() {
    let (ctx, registry) = ctx_with_registry();
    let dir = sandbox("ffmpeg_gate");
    let video = write_stub_video(&dir, "clip.mp4");
    let allowed = vec![video.to_string_lossy().to_string()];

    // Вложение юзера — ок.
    assert!(ensure_allowed_source(&video.to_string_lossy(), &ctx, &allowed).is_ok());

    // Чужой путь диска — Forbidden (не NotFound): гейт срабатывает ДО stat.
    let err = ensure_allowed_source("C:\\Windows\\notepad.exe", &ctx, &allowed).unwrap_err();
    assert!(matches!(err, ToolError::Forbidden(_)), "{err}");

    // Артефакт сессии (результат предыдущего ffmpeg_run в этом ходе) — ок,
    // даже если пути нет в allow-read.
    let produced = dir.join("clip_edited.mp4");
    std::fs::write(&produced, b"FAKE").unwrap();
    registry
        .register(produced.to_string_lossy().to_string(), "ffmpeg_run")
        .unwrap();
    assert!(ensure_allowed_source(&produced.to_string_lossy(), &ctx, &allowed).is_ok());

    // Гейтнутый, но удалённый файл — честный NotFound.
    std::fs::remove_file(&video).unwrap();
    let err = ensure_allowed_source(&video.to_string_lossy(), &ctx, &allowed).unwrap_err();
    assert!(matches!(err, ToolError::NotFound(_)), "{err}");

    // Относительный путь и '..' — Usage.
    assert!(ensure_allowed_source("clip.mp4", &ctx, &allowed).is_err());
    assert!(ensure_allowed_source("D:\\v\\..\\clip.mp4", &ctx, &allowed).is_err());
}

// ─────────────────── Диспетчер (до ensure — без докачки) ───────────────────

#[test]
fn run_rejects_dash_i_before_any_engine_work() {
    let (ctx, _reg) = ctx_with_registry();
    let dir = sandbox("ffmpeg_run_dash_i");
    let video = write_stub_video(&dir, "clip.mp4");
    let args = serde_json::json!({
        "path": video.to_string_lossy(),
        "input_args": ["-i", "C:\\evil.mp4"],
    });
    let err = execute_ffmpeg_tool("ffmpeg_run", &args, &ctx, &[video.to_string_lossy().to_string()])
        .expect("ffmpeg_run резолвится")
        .unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)));
    assert!(err.to_string().contains("-i"), "сообщение: {err}");
}

#[test]
fn run_rejects_existing_output_without_engine() {
    let (ctx, _reg) = ctx_with_registry();
    let dir = sandbox("ffmpeg_run_existing");
    let video = write_stub_video(&dir, "clip.mp4");
    std::fs::write(dir.join("clip_edited.mp4"), b"EXISTS").unwrap();
    let args = serde_json::json!({ "path": video.to_string_lossy() });
    let err = execute_ffmpeg_tool("ffmpeg_run", &args, &ctx, &[video.to_string_lossy().to_string()])
        .expect("ffmpeg_run резолвится")
        .unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)));
    assert!(err.to_string().contains("уже существует"), "сообщение: {err}");
}

#[test]
fn run_rejects_outside_source_without_engine() {
    let (ctx, _reg) = ctx_with_registry();
    let args = serde_json::json!({ "path": "C:\\Windows\\notepad.exe" });
    let err = execute_ffmpeg_tool("ffmpeg_run", &args, &ctx, &[])
        .expect("ffmpeg_run резолвится")
        .unwrap_err();
    assert!(matches!(err, ToolError::Forbidden(_)), "{err}");
}

#[test]
fn probe_rejects_outside_source_without_engine() {
    let (ctx, _reg) = ctx_with_registry();
    let args = serde_json::json!({ "path": "D:\\ любой чужой путь.mp4" });
    let err = execute_ffmpeg_tool("ffmpeg_probe", &args, &ctx, &[])
        .expect("ffmpeg_probe резолвится")
        .unwrap_err();
    assert!(matches!(err, ToolError::Forbidden(_)), "{err}");
}

#[test]
fn probe_missing_path_is_usage() {
    let (ctx, _reg) = ctx_with_registry();
    let err = execute_ffmpeg_tool("ffmpeg_probe", &serde_json::json!({}), &ctx, &[])
        .expect("ffmpeg_probe резолвится")
        .unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)));
}

#[test]
fn registry_contains_path_normalized() {
    let registry = ImageArtifactRegistry::new();
    registry.register("D:\\V\\clip_edited.mp4".to_string(), "ffmpeg_run").unwrap();
    assert!(registry.contains_path("D:\\V\\clip_edited.mp4"));
    assert!(registry.contains_path("d:/v/CLIP_EDITED.mp4"), "сепаратор и регистр не значимы");
    assert!(!registry.contains_path("D:\\V\\other.mp4"));
    // Чужой тул не открывает путь через реестр другого артефакта-типа.
    assert!(!registry.contains_path("D:\\несуществующий\\путь.xlsx"));
}

// ─────────────────────── extra_inputs (мульти-вход) ───────────────────────

#[test]
fn extra_inputs_duplicate_of_path_is_usage() {
    let (ctx, _reg) = ctx_with_registry();
    let dir = sandbox("ffmpeg_extra_dup");
    let video = write_stub_video(&dir, "clip.mp4");
    let allowed = vec![video.to_string_lossy().to_string()];
    // Гейт пройден (вложение юзера), затем дубль-проверка — Usage ДО ensure_engine.
    let err = parse_extra_inputs(
        &serde_json::json!({"extra_inputs": [video.to_string_lossy()]}),
        &video,
        &ctx,
        &allowed,
    )
    .unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)), "{err}");
    assert!(err.to_string().contains("дублирует path"), "сообщение: {err}");
}

#[test]
fn extra_inputs_outside_is_forbidden() {
    let (ctx, _reg) = ctx_with_registry();
    let dir = sandbox("ffmpeg_extra_gate");
    let video = write_stub_video(&dir, "clip.mp4");
    let allowed = vec![video.to_string_lossy().to_string()];
    let err = parse_extra_inputs(
        &serde_json::json!({"extra_inputs": ["C:\\Windows\\notepad.exe"]}),
        &video,
        &ctx,
        &allowed,
    )
    .unwrap_err();
    assert!(matches!(err, ToolError::Forbidden(_)), "{err}");
}

#[test]
fn extra_inputs_gated_but_missing_is_not_found() {
    let (ctx, _reg) = ctx_with_registry();
    let dir = sandbox("ffmpeg_extra_missing");
    let video = write_stub_video(&dir, "clip.mp4");
    let ghost = dir.join("ghost.mp3");
    // Путь в allow-списке (гейт пройден), файла нет — честный NotFound.
    let allowed = vec![video.to_string_lossy().to_string(), ghost.to_string_lossy().to_string()];
    let err = parse_extra_inputs(
        &serde_json::json!({"extra_inputs": [ghost.to_string_lossy()]}),
        &video,
        &ctx,
        &allowed,
    )
    .unwrap_err();
    assert!(matches!(err, ToolError::NotFound(_)), "{err}");
}

#[test]
fn extra_inputs_attachment_and_session_artifact_pass_gate() {
    let (ctx, registry) = ctx_with_registry();
    let dir = sandbox("ffmpeg_extra_ok");
    let video = write_stub_video(&dir, "clip.mp4");
    let audio = write_stub_video(&dir, "voice.mp3");
    let allowed = vec![video.to_string_lossy().to_string(), audio.to_string_lossy().to_string()];
    // Наблюдаемая точка: выход «уже существует» наступает ПОСЛЕ гейта
    // extra_inputs, но ДО ensure_engine — значит этот Usage доказывает,
    // что оба допвхода гейт пройден, и докачки движка в тесте нет.
    std::fs::write(dir.join("clip_edited.mp4"), b"EXISTS").unwrap();
    let args = serde_json::json!({
        "path": video.to_string_lossy(),
        "extra_inputs": [audio.to_string_lossy()],
    });
    let err = execute_ffmpeg_tool("ffmpeg_run", &args, &ctx, &allowed)
        .expect("ffmpeg_run резолвится")
        .unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)), "{err}");
    assert!(err.to_string().contains("уже существует"), "сообщение: {err}");

    // Артефакт сессии (не в allow-read) — тот же проход гейта.
    let artifact = dir.join("clip_recut.mp4");
    std::fs::write(&artifact, b"FAKE").unwrap();
    registry.register(artifact.to_string_lossy().to_string(), "ffmpeg_run").unwrap();
    let args2 = serde_json::json!({
        "path": video.to_string_lossy(),
        "extra_inputs": [artifact.to_string_lossy()],
    });
    let err2 = execute_ffmpeg_tool("ffmpeg_run", &args2, &ctx, &allowed)
        .expect("ffmpeg_run резолвится")
        .unwrap_err();
    assert!(err2.to_string().contains("уже существует"), "сообщение: {err2}");
}

#[test]
fn extra_inputs_type_and_limit_errors() {
    let (ctx, _reg) = ctx_with_registry();
    let dir = sandbox("ffmpeg_extra_types");
    let video = write_stub_video(&dir, "clip.mp4");
    let allowed: Vec<String> = vec![];

    // Склеенная строка вместо массива — Usage (спавн идёт args-массивом).
    let err = parse_extra_inputs(
        &serde_json::json!({"extra_inputs": "D:\\v\\audio.mp3"}),
        &video,
        &ctx,
        &allowed,
    )
    .unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)), "{err}");

    // Элемент не строка — Usage.
    let err2 = parse_extra_inputs(
        &serde_json::json!({"extra_inputs": [3]}),
        &video,
        &ctx,
        &allowed,
    )
    .unwrap_err();
    assert!(matches!(err2, ToolError::Usage(_)), "{err2}");

    // Больше лимита — Usage.
    let five = [1, 2, 3, 4, 5].iter().map(|i| format!("D:\\v\\a{i}.mp3")).collect::<Vec<_>>();
    let err3 = parse_extra_inputs(
        &serde_json::json!({"extra_inputs": five}),
        &video,
        &ctx,
        &allowed,
    )
    .unwrap_err();
    assert!(matches!(err3, ToolError::Usage(_)), "{err3}");
    assert!(err3.to_string().contains("максимум 4"), "сообщение: {err3}");

    // absent/null — пустой список (обратная совместимость: старые вызовы как раньше).
    assert!(parse_extra_inputs(&serde_json::json!({}), &video, &ctx, &allowed).unwrap().is_empty());
    assert!(
        parse_extra_inputs(&serde_json::json!({"extra_inputs": null}), &video, &ctx, &allowed)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn build_edit_argv_places_extras_between_inputs_and_outputs() {
    let argv = build_edit_argv(
        vec!["-ss".to_string(), "25".to_string()],
        Path::new("D:\\v\\clip.mp4"),
        &[PathBuf::from("D:\\v\\valkiria.mp3"), PathBuf::from("D:\\v\\subs.srt")],
        vec!["-map".to_string(), "0:v".to_string(), "-map".to_string(), "1:a".to_string()],
        Path::new("D:\\v\\clip_edited.mp4"),
    );
    let i_positions: Vec<usize> = argv
        .iter()
        .enumerate()
        .filter(|(_, a)| a.as_str() == "-i")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        argv.iter().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "-hide_banner",
            "-ss", "25",
            "-i", "D:\\v\\clip.mp4",
            "-i", "D:\\v\\valkiria.mp3",
            "-i", "D:\\v\\subs.srt",
            "-map", "0:v", "-map", "1:a",
            "D:\\v\\clip_edited.mp4",
        ],
        "порядок: [input_args] -i path -i extras… [output_args] output"
    );
    assert_eq!(i_positions, vec![3, 5, 7], "все -i строит хост между входными и выходными опциями");
}
