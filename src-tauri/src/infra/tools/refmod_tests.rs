//! Тесты RefMod-карточек (tasks/261104_0230-db2cb59c, Этап 1 Шаг 5).
//! Герметично: временные каталоги, синтетические PNG-источники; реальный
//! `<KingOrchData>/refmods` не трогается (все проверки через `_in`-варианты).

use super::*;

fn temp_root(tag: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "king_orch_refmod_{}_{}_{}",
        tag,
        std::process::id(),
        stamp
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Синтетический PNG-источник заданного размера (декодер `image` его съест).
fn write_source(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
    let path = dir.join(name);
    image::DynamicImage::new_rgb8(w, h)
        .save_with_format(&path, image::ImageFormat::Png)
        .unwrap();
    path
}

// ── Нормирование габаритов (владелец: ≤1024px по длинной стороне, кратно 32px) ──

#[test]
fn resize_caps_long_side_and_snaps_to_grid() {
    let cases: &[(u32, u32, u32, u32)] = &[
        // 4K-кадр: длинная сторона ровно 1024, обе стороны кратны 32.
        (4096, 2160, 1024, 544),
        // Меньше лимита, но не кратно сетке — выравнивание без роста длинной.
        (830, 1000, 832, 992),
        // Уже в сетке — без изменений.
        (768, 1024, 768, 1024),
        // Микро-кадр — минимальное кратное 32.
        (16, 16, 32, 32),
    ];
    for (w, h, want_w, want_h) in cases {
        let resized = resize_for_card(image::DynamicImage::new_rgb8(*w, *h));
        assert_eq!(
            (resized.width(), resized.height()),
            (*want_w, *want_h),
            "исходник {}x{}",
            w,
            h
        );
        assert!(frame_size_valid(resized.width(), resized.height()));
        assert!(resized.width().max(resized.height()) <= FRAME_LONG_SIDE_MAX);
    }
}

// ── Slug ──

#[test]
fn slug_validation_matrix() {
    assert!(validate_slug("alice_v2").is_ok());
    assert!(validate_slug("a").is_ok());
    assert!(validate_slug("char-3-4").is_ok());
    // Пустой, слишком длинный, недопустимые символы, traversal, крайние разделители.
    assert!(validate_slug("").is_err());
    assert!(validate_slug("   ").is_err());
    assert!(validate_slug(&"a".repeat(49)).is_err());
    assert!(validate_slug("Alice").is_err());
    assert!(validate_slug("алиса").is_err());
    assert!(validate_slug("../evil").is_err());
    assert!(validate_slug("a/b").is_err());
    assert!(validate_slug("-abc").is_err());
    assert!(validate_slug("abc-").is_err());
}

// ── Save / Load roundtrip ──

#[test]
fn save_load_roundtrip_normalizes_frames() {
    let root = temp_root("roundtrip");
    let src_dir = root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let s1 = write_source(&src_dir, "a.png", 100, 64); // → 96x64 (кратно 32)
    let s2 = write_source(&src_dir, "b.png", 4096, 2160); // → 1024x544

    let card = save_refmod_card_in(&root, "test_char", "Тест Чар", "тестовое описание", &[
        s1.clone(),
        s2.clone(),
    ])
    .expect("карточка сохраняется");
    assert_eq!(card.slug, "test_char");
    assert_eq!(card.name, "Тест Чар");
    assert_eq!(card.frames.len(), 2);
    assert!(card.frames[0].ends_with("frame_01.png"));
    assert!(card.frames[1].ends_with("frame_02.png"));

    let loaded = load_refmod_card_in(&root, "test_char").expect("карточка читается");
    assert_eq!(loaded, card);

    // Кадры физически нормированы: 96x64 и 1024x544.
    let d1 = image::image_dimensions(&loaded.frames[0]).unwrap();
    let d2 = image::image_dimensions(&loaded.frames[1]).unwrap();
    assert_eq!((d1.0, d1.1), (96, 64), "меньше лимита — выравнивание кратно 32");
    assert_eq!((d2.0, d2.1), (1024, 544), "4K — даунскейл с кратностью 32");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn save_overwrites_without_stale_frames() {
    let root = temp_root("overwrite");
    let src_dir = root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let sources: Vec<PathBuf> = (0..5)
        .map(|i| write_source(&src_dir, &format!("s{}.png", i), 64, 64))
        .collect();
    save_refmod_card_in(&root, "test_char", "Пять", "", &sources).expect("5 кадров");

    // Перезапись двумя кадрами: frame_03..05 не должны выжить.
    save_refmod_card_in(&root, "test_char", "Два", "", &sources[..2]).expect("перезапись");
    let card = load_refmod_card_in(&root, "test_char").unwrap();
    assert_eq!(card.name, "Два");
    assert_eq!(card.frames.len(), 2, "stale-кадры не должны пережить перезапись");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn save_rejects_bad_inputs() {
    let root = temp_root("rejects");
    let src = write_source(&root, "s.png", 64, 64);
    // >8 кадров.
    let many: Vec<PathBuf> = (0..=FRAMES_MAX)
        .map(|i| write_source(&root, &format!("m{}.png", i), 64, 64))
        .collect();
    let err = save_refmod_card_in(&root, "test_char", "N", "", &many).unwrap_err();
    assert!(matches!(err, ToolError::Usage(ref m) if m.contains("максимум")), "{:?}", err);
    // Пустой список источников.
    let err = save_refmod_card_in(&root, "test_char", "N", "", &[]).unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)), "{:?}", err);
    // Пустое имя.
    let err = save_refmod_card_in(&root, "test_char", "  ", "", &[src.clone()]).unwrap_err();
    assert!(matches!(err, ToolError::Usage(ref m) if m.contains("name")), "{:?}", err);
    // Плохой slug.
    let err = save_refmod_card_in(&root, "Bad Slug", "N", "", &[src]).unwrap_err();
    assert!(matches!(err, ToolError::Usage(ref m) if m.contains("slug")), "{:?}", err);
    // Нечитаемый источник.
    let broken = root.join("broken.png");
    std::fs::write(&broken, b"not a png").unwrap();
    let err = save_refmod_card_in(&root, "test_char", "N", "", &[broken]).unwrap_err();
    assert!(matches!(err, ToolError::Io(ref m) if m.contains("кадр-источник")), "{:?}", err);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn load_missing_card_is_not_found() {
    let root = temp_root("missing");
    let err = load_refmod_card_in(&root, "nobody").unwrap_err();
    assert!(matches!(err, ToolError::NotFound(ref m) if m.contains("nobody")), "{:?}", err);
    // Slug-гейт срабатывает раньше диска.
    let err = load_refmod_card_in(&root, "../evil").unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)), "{:?}", err);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn load_rejects_off_grid_and_oversized_cards() {
    let root = temp_root("corrupt");
    let src = write_source(&root, "s.png", 64, 64);
    // Карточка с кадром, стороны которого НЕ кратны 32 (подмена вручную).
    let card = save_refmod_card_in(&root, "test_char", "N", "", &[src]).unwrap();
    let frame = &card.frames[0];
    image::DynamicImage::new_rgb8(50, 50)
        .save_with_format(frame, image::ImageFormat::Png)
        .unwrap();
    let err = load_refmod_card_in(&root, "test_char").unwrap_err();
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("кратны")),
        "несеточный кадр отклоняется: {:?}",
        err
    );

    // Карточка с 9 кадрами (больше лимита хранения) — честный отказ.
    let frames_dir = root.join("test_char").join("frames");
    std::fs::create_dir_all(&frames_dir).unwrap();
    for i in 0..9 {
        image::DynamicImage::new_rgb8(32, 32)
            .save_with_format(frames_dir.join(format!("frame_{:02}.png", i + 1)), image::ImageFormat::Png)
            .unwrap();
    }
    let err = load_refmod_card_in(&root, "test_char").unwrap_err();
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("повреждена")),
        "{:?}",
        err
    );
    let _ = std::fs::remove_dir_all(root);
}

// ── Каталог и prompt-блок ──

#[test]
fn list_and_prompt_cover_cards_and_skip_broken() {
    let root = temp_root("catalog");
    let src_dir = root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let s1 = write_source(&src_dir, "a.png", 64, 64);
    let s2 = write_source(&src_dir, "b.png", 64, 64);
    save_refmod_card_in(&root, "beta_char", "Бета", "второй", &[s2]).unwrap();
    save_refmod_card_in(&root, "alpha_char", "Альфа", "первый", &[s1]).unwrap();
    // Битая папка без meta.json и файл-не-папка — в каталог не попадают.
    std::fs::create_dir_all(root.join("no_meta")).unwrap();
    std::fs::write(root.join("loose.txt"), b"x").unwrap();

    let cards = list_refmod_cards_in(&root);
    let slugs: Vec<&str> = cards.iter().map(|c| c.slug.as_str()).collect();
    assert_eq!(slugs, vec!["alpha_char", "beta_char"], "сортировка по slug, битое пропущено");

    let prompt = refmods_prompt_in(&root);
    assert!(prompt.contains("alpha_char — Альфа — первый (кадров: 1)"), "{}", prompt);
    assert!(prompt.contains("beta_char"), "{}", prompt);

    // Пустой каталог — честная подсказка тула.
    let empty = temp_root("empty_catalog");
    let hint = refmods_prompt_in(&empty);
    assert!(hint.contains("RefMod-карточек персонажей нет"), "{}", hint);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(empty);
}

// ── Отбор кадров для прогона (владелец: 4–8 кадров НЕ ошибка, берём первые 3) ──

#[test]
fn engine_ref_paths_take_first_three() {
    let root = temp_root("take3");
    let src_dir = root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let sources: Vec<PathBuf> = (0..5)
        .map(|i| write_source(&src_dir, &format!("s{}.png", i), 64, 64))
        .collect();
    let card = save_refmod_card_in(&root, "test_char", "Пять", "", &sources).unwrap();
    assert_eq!(card.frames.len(), 5);
    let paths = engine_ref_paths(&card);
    assert_eq!(paths.len(), FRAMES_PER_RUN, "берём первые 3 из 5");
    assert_eq!(paths[0], card.frames[0].to_string_lossy());
    assert_eq!(paths[2], card.frames[2].to_string_lossy());
    let _ = std::fs::remove_dir_all(root);
}
