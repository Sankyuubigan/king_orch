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
