//! Тесты refmod-тулов хоста (Этап 2, Шаг 1): capability-схемы, гейты, roundtrip
//! create → edit на временных корнях (реальный `<KingOrchData>/refmods` не
//! трогается — все проверки через `*_in`-варианты с root-параметром).
//! Тесты SSOT формата (save/load/normalize/thumbnail) живут в плагине
//! (engine/refmod_tests.rs) — там же, где код.

use super::*;

fn temp_root(tag: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "king_orch_refmod_tools_{}_{}_{}",
        tag,
        std::process::id(),
        stamp
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ctx() -> ToolCtx<'static> {
    ToolCtx {
        workspace_root: std::path::Path::new("."),
        write_root: std::path::Path::new("."),
        write_outside: crate::infra::tools::WriteOutside::Prompt,
        session_id: "refmod-tools-test",
        approver: crate::infra::permissions::test_approver(),
        agent_id: "test_agent",
        bins_dir: std::path::Path::new("."),
        image_artifacts: None,
    }
}

/// PNG-файл заданного размера + аттачмент с file_path (гейт resolve_ref_paths).
fn png_attachment(dir: &Path, name: &str, w: u32, h: u32) -> crate::infra::ChatAttachment {
    let path = dir.join(name);
    image::DynamicImage::new_rgb8(w, h)
        .save_with_format(&path, image::ImageFormat::Png)
        .unwrap();
    crate::infra::ChatAttachment {
        file_name: name.to_string(),
        mime_type: "image/png".to_string(),
        data_base64: String::new(),
        file_path: Some(path.to_string_lossy().to_string()),
        is_dir: Some(false),
    }
}

// ── Capability-схемы ──

#[test]
fn refmod_schemas_included_only_for_declared_tools() {
    let none = refmod_tool_schemas(&[]);
    assert!(none.is_empty(), "без declared-тулов схем нет");

    let both = refmod_tool_schemas(&["refmod_create".to_string(), "refmod_edit".to_string()]);
    assert_eq!(both.len(), 2);
    assert!(both.iter().all(|(meta, _, _)| meta == "refmod"));
    let names: Vec<&str> = both.iter().map(|(_, name, _)| name.as_str()).collect();
    assert_eq!(names, vec!["refmod_create", "refmod_edit"]);
    for (_, _, schema) in &both {
        assert!(schema.get("inputSchema").is_some(), "inputSchema обязателен");
    }
}

#[test]
fn is_refmod_tool_matrix() {
    assert!(is_refmod_tool("refmod_create"));
    assert!(is_refmod_tool("refmod_edit"));
    assert!(!is_refmod_tool("generate_video"));
    assert!(!is_refmod_tool("excel_apply"));
    assert!(!is_refmod_tool(""));
}

#[test]
fn execute_unknown_refmod_tool_returns_none() {
    let args = serde_json::json!({});
    assert!(execute_refmod_tool("refmod_rename", &args, &ctx(), &[], &[]).is_none());
}

// ── Гейты refmod_create ──

#[test]
fn create_requires_slug_name_and_sources() {
    let root = temp_root("gates");
    let ctx = ctx();
    // Без slug.
    let err = create_card(
        &root,
        &serde_json::json!({"name": "Имя"}),
        &ctx,
        &[],
        &[],
    )
    .unwrap_err();
    assert!(matches!(err, ToolError::Usage(ref m) if m.contains("slug")), "{:?}", err);
    // Без name.
    let err = create_card(
        &root,
        &serde_json::json!({"slug": "char"}),
        &ctx,
        &[],
        &[],
    )
    .unwrap_err();
    assert!(matches!(err, ToolError::Usage(ref m) if m.contains("name")), "{:?}", err);
    // Без вложений — честный отказ гейта источников.
    let err = create_card(
        &root,
        &serde_json::json!({"slug": "char", "name": "Имя"}),
        &ctx,
        &[],
        &[],
    )
    .unwrap_err();
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("Не выбрано изображение")),
        "пустые вложения не проходят гейт источников: {:?}",
        err
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn create_rejects_bad_slug_before_disk_write() {
    let root = temp_root("bad_slug");
    let src = temp_root("bad_slug_src");
    let att = png_attachment(&src, "face.png", 64, 64);
    let err = create_card(
        &root,
        &serde_json::json!({"slug": "Bad Slug!", "name": "Имя"}),
        &ctx(),
        std::slice::from_ref(&att),
        &[],
    )
    .unwrap_err();
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("slug")),
        "невалидный slug отсекается валидатором SSOT: {:?}",
        err
    );
    assert!(!root.join("Bad Slug!").exists(), "на диске ничего не создано");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(src);
}

// ── ⚠-блок нормализации в результате refmod_create (PRP 07.11.26) ──

#[test]
fn create_reports_compression_and_silent_for_aligned() {
    let root = temp_root("warn_compress");
    let src = temp_root("warn_compress_src");
    // Оверсайз-фото (кейс vlad_main: телефонное 3024×4032) — тул обязан
    // ВИДИМО сообщить о сжатии (§00.2: молчаливая потеря пикселей недопустима).
    let big = png_attachment(&src, "big.png", 3024, 4032);
    let marker = create_card(
        &root,
        &serde_json::json!({"slug": "big_char", "name": "Биг"}),
        &ctx(),
        std::slice::from_ref(&big),
        &[],
    )
    .expect("оверсайз-карточка сохраняется");
    assert!(marker.contains("[REFMOD_SAVED slug=big_char]"), "{}", marker);
    // SSOT 4906735: бюджет площади вместо лимита длинной стороны →
    // 1216×1632 (обе стороны кратны 32), тест не обновили в том коммите.
    assert!(
        marker.contains("⚠ СЖАТИЕ frame_01.png: 3024×4032 → 1216×1632"),
        "сжатие названо явно с размерами: {}",
        marker
    );
    assert!(marker.contains("Совет: прикладывайте кропы"), "{}", marker);

    // Уже выровненный набор: отчёт пуст → ⚠-блока нет (не спамим).
    let root2 = temp_root("warn_clean");
    let small = png_attachment(&src, "small.png", 320, 288);
    let clean = create_card(
        &root2,
        &serde_json::json!({"slug": "grid_char", "name": "Грид"}),
        &ctx(),
        std::slice::from_ref(&small),
        &[],
    )
    .expect("сеточная карточка сохраняется");
    assert!(clean.contains("[REFMOD_SAVED slug=grid_char]"), "{}", clean);
    assert!(
        !clean.contains("СЖАТИЕ") && !clean.contains("РАСТЯЖЕНИЕ"),
        "сеточные кадры не порождают предупреждений: {}",
        clean
    );

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
    let _ = std::fs::remove_dir_all(src);
}

// ── Roundtrip create → edit (временный корень) ──

#[test]
fn create_then_edit_roundtrip_keeps_grid_and_original() {
    let root = temp_root("roundtrip");
    let src = temp_root("roundtrip_src");
    let atts = vec![
        png_attachment(&src, "face.png", 128, 96),
        png_attachment(&src, "pose.png", 512, 384),
    ];
    let marker = create_card(
        &root,
        &serde_json::json!({
            "slug": "test_char",
            "name": "Тест Чар",
            "description": "создан тестом"
        }),
        &ctx(),
        &atts,
        &[],
    )
    .expect("карточка создаётся из вложений");
    assert!(marker.contains("[REFMOD_SAVED slug=test_char]"), "{}", marker);

    // Правка: hue −20 → НОВАЯ карточка test_char_v2, исходник не тронут.
    let edit_marker = edit_card(
        &root,
        &serde_json::json!({"slug": "test_char", "ops": [{"op": "hue", "degrees": -20}]}),
        &[],
    )
    .expect("правка проходит");
    assert!(
        edit_marker.contains("[REFMOD_SAVED slug=test_char_v2]"),
        "результат edit — новая карточка: {}",
        edit_marker
    );
    let original = load_refmod_card_in(&root, "test_char").expect("исходник жив");
    assert_eq!(original.frames.len(), 2, "исходник не изменён");
    let variant = load_refmod_card_in(&root, "test_char_v2").expect("вариант существует");
    assert_eq!(variant.frames.len(), 2);
    for frame in &variant.frames {
        let (w, h) = image::image_dimensions(frame).unwrap();
        assert_eq!(w % FRAME_SIZE_MULTIPLE, 0, "{} на сетке по ширине", frame.display());
        assert_eq!(h % FRAME_SIZE_MULTIPLE, 0, "{} на сетке по высоте", frame.display());
    }

    // Вторая правка → v3 (next_free_slug).
    let edit_marker = edit_card(
        &root,
        &serde_json::json!({"slug": "test_char", "ops": [{"op": "warp", "factor": 0.85}]}),
        &[],
    )
    .expect("вторая правка проходит");
    assert!(
        edit_marker.contains("[REFMOD_SAVED slug=test_char_v3]"),
        "следующий свободный вариант: {}",
        edit_marker
    );

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(src);
}

#[test]
fn edit_honest_gates_before_disk() {
    let root = temp_root("edit_gates");
    // Пустой slug.
    let err =
        edit_card(&root, &serde_json::json!({"ops": [{"op": "hue", "degrees": 5}]}), &[])
            .unwrap_err();
    assert!(matches!(err, ToolError::Usage(ref m) if m.contains("slug")), "{:?}", err);
    // ops-гейт срабатывает ДО чтения карточки (неизвестный slug + битые ops → Usage про ops).
    let err = edit_card(
        &root,
        &serde_json::json!({"slug": "nobody", "ops": []}),
        &[],
    )
    .unwrap_err();
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("ops")),
        "пустые ops отсекаются до диска: {:?}",
        err
    );
    // Неизвестная карточка — NotFound с причиной.
    let err = edit_card(
        &root,
        &serde_json::json!({"slug": "nobody", "ops": [{"op": "hue", "degrees": 5}]}),
        &[],
    )
    .unwrap_err();
    assert!(
        matches!(err, ToolError::NotFound(ref m) if m.contains("nobody")),
        "{:?}",
        err
    );
    let _ = std::fs::remove_dir_all(root);
}

// ── Voice-ветки тулов (Этап 3) ──

/// WAV-файл (синтетический тон заданной частоты) + аттачмент с file_path —
/// сэмпл голоса юзера. Контейнер строится SSOT-фн плагина (to_wav_bytes),
/// symphonia его читает. Разные freq → разные байты (смена голоса отличима).
fn wav_attachment(dir: &Path, name: &str, freq: f64) -> crate::infra::ChatAttachment {
    let wav = tauri_plugin_image_engine::engine::VoiceWav {
        // Амплитуда 12000: синус [-1, 1] кастом в i16 даёт почти все нули
        // (sin не достигает ±1.0 в этих точках) — v1 и v2 становятся
        // неразличимыми, и тест смены голоса сравнивает пустышки.
        samples: (0..4410)
            .map(|i| (((i as f64 / 100.0) * freq).sin() * 12_000.0) as i16)
            .collect(),
        sample_rate: 44_100,
        channels: 1,
    }
    .to_wav_bytes();
    let path = dir.join(name);
    std::fs::write(&path, &wav).unwrap();
    crate::infra::ChatAttachment {
        file_name: name.to_string(),
        mime_type: "audio/wav".to_string(),
        data_base64: String::new(),
        file_path: Some(path.to_string_lossy().to_string()),
        is_dir: Some(false),
    }
}

#[test]
fn create_with_voice_attaches_sample() {
    let root = temp_root("create_voice");
    let src = temp_root("create_voice_src");
    let atts = vec![png_attachment(&src, "face.png", 128, 96)];
    let wav = wav_attachment(&src, "voice.wav", 6000.0);
    let marker = create_card(
        &root,
        &serde_json::json!({"slug": "voiced", "name": "Голос"}),
        &ctx(),
        &atts,
        std::slice::from_ref(&wav),
    )
    .expect("карточка с голосом создаётся");
    assert!(marker.contains("🎤 голос задан"), "{}", marker);
    let card = load_refmod_card_in(&root, "voiced").expect("карточка жива");
    let voice = card.voice.expect("голос записан в карточку");
    let on_disk = std::fs::read(&voice).unwrap();
    assert!(on_disk.starts_with(b"RIFF"), "voice.wav — канонический WAV");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(src);
}

#[test]
fn create_rejects_multiple_voice_samples() {
    let root = temp_root("multi_voice");
    let src = temp_root("multi_voice_src");
    let atts = vec![png_attachment(&src, "face.png", 128, 96)];
    let wavs = vec![
        wav_attachment(&src, "a.wav", 6000.0),
        wav_attachment(&src, "b.wav", 9000.0),
    ];
    let err = create_card(
        &root,
        &serde_json::json!({"slug": "char", "name": "Имя"}),
        &ctx(),
        &atts,
        &wavs,
    )
    .unwrap_err();
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("РОВНО ОДИН")),
        "мульти-голос структурно запрещён: {:?}",
        err
    );
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(src);
}

#[test]
fn edit_voice_change_is_in_place() {
    let root = temp_root("edit_voice");
    let src = temp_root("edit_voice_src");
    let atts = vec![png_attachment(&src, "face.png", 128, 96)];
    create_card(
        &root,
        &serde_json::json!({"slug": "char", "name": "Имя"}),
        &ctx(),
        &atts,
        &[],
    )
    .unwrap();
    let v1 = wav_attachment(&src, "v1.wav", 6000.0);
    let out = edit_card(
        &root,
        &serde_json::json!({"slug": "char", "voice_desc": "низкий"}),
        std::slice::from_ref(&v1),
    )
    .expect("голос добавлен");
    assert!(out.contains("без смены кадров"), "{}", out);
    let before = load_refmod_card_in(&root, "char").unwrap();
    assert_eq!(before.voice_desc.as_deref(), Some("низкий"));
    // Байты v1 читаются ДО второй записи: путь voice.wav один и тот же,
    // после смены там уже v2 — сравнивать было бы нечего.
    let a = std::fs::read(before.voice.expect("v1 был")).unwrap();

    // Смена голоса: та же карточка (IN-PLACE, без v2), файл перезаписан.
    // freq другой — сэмплы обязаны отличаться (иначе тест не о том).
    let v2 = wav_attachment(&src, "v2.wav", 11_500.0);
    let out = edit_card(&root, &serde_json::json!({"slug": "char"}), std::slice::from_ref(&v2))
        .expect("голос сменён");
    assert!(out.contains("без смены кадров"), "{}", out);
    let after = load_refmod_card_in(&root, "char").unwrap();
    assert_eq!(after.slug, "char");
    assert!(
        !root.join("char_v2").join("meta.json").exists(),
        "IN-PLACE — v2 не создаётся"
    );
    let b = std::fs::read(after.voice.expect("v2 записан")).unwrap();
    assert_ne!(a, b, "voice.wav перезаписан новым сэмплом");
    assert_eq!(
        after.voice_desc, None,
        "старое описание сменившегося тембра не живёт"
    );
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(src);
}

#[test]
fn edit_voice_desc_only_and_missing_voice() {
    let root = temp_root("desc_only");
    let src = temp_root("desc_only_src");
    let atts = vec![png_attachment(&src, "face.png", 128, 96)];
    create_card(
        &root,
        &serde_json::json!({"slug": "char", "name": "Имя"}),
        &ctx(),
        &atts,
        &[],
    )
    .unwrap();

    // desc без голоса — описывать нечего (честный Usage, не тихий no-op).
    let err = edit_card(
        &root,
        &serde_json::json!({"slug": "char", "voice_desc": "хриплый"}),
        &[],
    )
    .unwrap_err();
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("голос")),
        "{:?}",
        err
    );

    // Голос есть — desc-only правит описание, файл не трогает.
    let wav = wav_attachment(&src, "v.wav", 6000.0);
    edit_card(
        &root,
        &serde_json::json!({"slug": "char"}),
        std::slice::from_ref(&wav),
    )
    .unwrap();
    let before = load_refmod_card_in(&root, "char").unwrap();
    let on_disk = std::fs::read(before.voice.as_ref().unwrap()).unwrap();
    let out = edit_card(
        &root,
        &serde_json::json!({"slug": "char", "voice_desc": "хриплый"}),
        &[],
    )
    .expect("desc правится");
    assert!(out.contains("описание голоса обновлено"), "{}", out);
    let after = load_refmod_card_in(&root, "char").unwrap();
    assert_eq!(after.voice_desc.as_deref(), Some("хриплый"));
    assert_eq!(
        std::fs::read(after.voice.as_ref().unwrap()).unwrap(),
        on_disk,
        "desc-only не трогает voice.wav"
    );
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(src);
}

#[test]
fn edit_ops_and_voice_are_two_calls() {
    let root = temp_root("xor_voice");
    let src = temp_root("xor_voice_src");
    let atts = vec![png_attachment(&src, "face.png", 128, 96)];
    create_card(
        &root,
        &serde_json::json!({"slug": "char", "name": "Имя"}),
        &ctx(),
        &atts,
        &[],
    )
    .unwrap();
    // ops + аудио → Usage (двумя вызовами).
    let wav = wav_attachment(&src, "v.wav", 6000.0);
    let err = edit_card(
        &root,
        &serde_json::json!({"slug": "char", "ops": [{"op": "hue", "degrees": 10}]}),
        std::slice::from_ref(&wav),
    )
    .unwrap_err();
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("двумя вызовами")),
        "{:?}",
        err
    );
    // ops + voice_desc → тот же гейт.
    let err = edit_card(
        &root,
        &serde_json::json!({"slug": "char", "ops": [{"op": "hue", "degrees": 10}], "voice_desc": "низкий"}),
        &[],
    )
    .unwrap_err();
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("двумя вызовами")),
        "{:?}",
        err
    );
    // Пустой вызов (только slug) → Usage с подсказкой про режимы.
    let err = edit_card(&root, &serde_json::json!({"slug": "char"}), &[]).unwrap_err();
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("пустой вызов")),
        "{:?}",
        err
    );
    // Голос при XOR-гейте не потрачен: карточка всё ещё без голоса.
    let card = load_refmod_card_in(&root, "char").unwrap();
    assert!(card.voice.is_none(), "XOR-гейт до записи голоса");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(src);
}
