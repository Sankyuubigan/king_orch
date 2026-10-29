//! Юнит-тесты `media_lora` (PRP 29.10.26, фаза 2: per-item сила элемента):
//! извлечение LoRA-запроса из аргументов тула (строки ИЛИ объекты
//! `{name, strength}`, микс форм), резолв через плагинный SSOT,
//! статус-строка и schema-контракты тулов (generate/edit получили
//! опциональные `lora` + `lora_strength`, upscale — принципиально нет).
//!
//! Модуль вложен в media_lora, который в свою очередь вложен в media:
//! `super` = media_lora, `super::super` = media (image_tool_schemas).

use std::path::PathBuf;

use serde_json::Value;

use super::super::image_tool_schemas;
use super::{extract_lora_request, resolve_specs_in, resolve_specs_in_base, status_line, LoraRequest, ToolError};

/// Уникальная temp-подпапка теста (pid + наносекунды: параллельные тесты
/// и повторные прогоны не пересекаются).
fn sandbox(tag: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "media_lora_tests_{}_{tag}_{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp-папка создана");
    dir
}

// ─────────────────────────── extract_lora_request: базовый контракт ───────────────────────────

#[test]
fn extract_absent_args_mean_no_lora() {
    assert!(extract_lora_request(&Value::Object(Default::default())).expect("нет поля — Ok").is_none());
}

#[test]
fn extract_null_fields_mean_no_lora() {
    let args = serde_json::json!({"lora": null, "lora_strength": null});
    assert!(extract_lora_request(&args).expect("null = отсутствие").is_none());
    let only_strength_null = serde_json::json!({"lora_strength": null});
    assert!(
        extract_lora_request(&only_strength_null)
            .expect("null strength без lora = отсутствие")
            .is_none()
    );
}

#[test]
fn extract_valid_names_and_default_strength() {
    let args = serde_json::json!({"lora": ["Style A", "style_b"]});
    let req = extract_lora_request(&args).expect("валидный запрос").expect("некоторые имена");
    // Легаси-форма: сила элемента не задана — общий дефолт применит резолв.
    assert_eq!(req.names(), vec!["Style A".to_string(), "style_b".to_string()]);
    assert!(req.items.iter().all(|i| i.strength.is_none()));
    assert!((req.strength - 1.0).abs() < f32::EPSILON, "дефолт силы = 1.0");
}

#[test]
fn extract_strength_boundaries_are_inclusive() {
    for (raw, expected) in [(0.0, 0.0f32), (2.0, 2.0f32)] {
        let args = serde_json::json!({"lora": ["a"], "lora_strength": raw});
        let req = extract_lora_request(&args)
            .expect("границы вилки валидны")
            .expect("есть имена");
        assert!(
            (req.strength - expected).abs() < f32::EPSILON,
            "strength {raw} → {expected}"
        );
    }
}

#[test]
fn extract_out_of_range_strength_is_usage() {
    for raw in [2.5f64, -0.1f64, 100.0] {
        let args = serde_json::json!({"lora": ["a"], "lora_strength": raw});
        let err = extract_lora_request(&args)
            .expect_err("вне вилки 0–2 — Usage + проверьте PRP");
        assert!(
            matches!(err, ToolError::Usage(_)),
            "raw={raw} → {err:?}"
        );
    }
}

#[test]
fn extract_non_number_strength_is_usage() {
    let args = serde_json::json!({"lora": ["a"], "lora_strength": "middle"});
    let err = extract_lora_request(&args).expect_err("строка вместо числа");
    assert!(matches!(err, ToolError::Usage(_)));
}

#[test]
fn extract_strength_without_lora_is_usage() {
    let args = serde_json::json!({"lora_strength": 1.5});
    let err = extract_lora_request(&args).expect_err("сила без имён бессмысленна");
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("без lora")),
        "сообщение: {err:?}"
    );
}

#[test]
fn extract_empty_array_is_usage() {
    let args = serde_json::json!({"lora": []});
    let err = extract_lora_request(&args).expect_err("пустой массив — Usage");
    assert!(matches!(err, ToolError::Usage(ref m) if m.contains("пустой")), "{err:?}");
}

#[test]
fn extract_empty_name_is_usage() {
    for raw in ["", "   "] {
        let args = serde_json::json!({"lora": [raw]});
        let err = extract_lora_request(&args).expect_err("пустое имя — Usage");
        assert!(matches!(err, ToolError::Usage(_)), "raw={raw:?} — {err:?}");
    }
}

#[test]
fn extract_not_array_is_usage() {
    let args = serde_json::json!({"lora": "style.safetensors"});
    let err = extract_lora_request(&args).expect_err("строка вместо массива");
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("массивом")),
        "проверка формата — {err:?}"
    );
}

#[test]
fn extract_number_item_is_usage() {
    let args = serde_json::json!({"lora": [12]});
    let err = extract_lora_request(&args).expect_err("число в массиве");
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("строка (стем) или объект")),
        "не-строка/не-объект — {err:?}"
    );
}

#[test]
fn extract_duplicates_folded_case_insensitive() {
    let args = serde_json::json!({"lora": ["Style", "style", "STYLE  "]});
    let req = extract_lora_request(&args).expect("дубликаты сворачиваются").expect("");
    // Регистронезависимый свёрт: остаётся первое фактическое имя (+его сила).
    assert_eq!(req.names().len(), 1);
    assert_eq!(req.names()[0], "Style");
}

// ─────────────────────── extract_lora_request: объекты и микс (фаза 2) ───────────────────────

#[test]
fn extract_object_items_carry_per_item_strength() {
    let args = serde_json::json!({
        "lora": [
            {"name": "boring_be_gone_v60", "strength": 1.5},
            {"name": "realism_helper"}
        ]
    });
    let req = extract_lora_request(&args).expect("объектная форма валидна").expect("");
    assert_eq!(req.names(), vec!["boring_be_gone_v60", "realism_helper"]);
    assert!(
        (req.items[0].strength.expect("сила первого элемента") - 1.5).abs() < f32::EPSILON,
        "per-item сила распознана"
    );
    assert!(req.items[1].strength.is_none(), "без strength = общий дефолт");
}

#[test]
fn extract_mixed_string_and_object_formats() {
    let args = serde_json::json!({
        "lora": ["utils_style", {"name": "slider", "strength": 2.0}, {"name": "no_strength_here"}]
    });
    let req = extract_lora_request(&args).expect("микс форм валиден").expect("");
    assert_eq!(req.names(), vec!["utils_style", "slider", "no_strength_here"]);
    assert!(req.items[0].strength.is_none());
    assert!((req.items[1].strength.expect("объектная сила") - 2.0).abs() < f32::EPSILON);
    assert!(req.items[2].strength.is_none());
}

#[test]
fn extract_object_item_without_name_is_usage() {
    let args = serde_json::json!({"lora": [{"strength": 1.0}]});
    let err = extract_lora_request(&args).expect_err("объект без name — Usage");
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("name")),
        "{err:?}"
    );
}

#[test]
fn extract_object_item_with_empty_name_is_usage() {
    for v in [serde_json::json!({}), serde_json::json!({"name": "  "})] {
        let args = serde_json::json!({"lora": [v]});
        let err = extract_lora_request(&args).expect_err("пустое name внутри объекта");
        assert!(matches!(err, ToolError::Usage(_)));
    }
}

#[test]
fn extract_object_item_strength_boundaries_and_range() {
    // Границы включительно.
    for raw in [0.0f64, 2.0f64] {
        let args = serde_json::json!({"lora": [{"name": "a", "strength": raw}]});
        let req = extract_lora_request(&args).expect("").expect("");
        assert!((req.items[0].strength.unwrap() - raw as f32).abs() < f32::EPSILON);
    }
    // Вне вилки / не число — Usage.
    for raw in [serde_json::json!(2.5), serde_json::json!(-0.5), serde_json::json!("big")] {
        let args = serde_json::json!({"lora": [{"name": "a", "strength": raw}]});
        let err = extract_lora_request(&args).expect_err("сила элемента вне контракта");
        assert!(matches!(err, ToolError::Usage(_)), "raw={raw} — {err:?}");
    }
}

#[test]
fn extract_object_item_strength_null_means_default() {
    let args = serde_json::json!({"lora": [{"name": "a", "strength": null}]});
    let req = extract_lora_request(&args).expect("").expect("");
    assert!(req.items[0].strength.is_none(), "null strength = не задана");
}

#[test]
fn extract_duplicate_object_keeps_first_with_its_strength() {
    let args = serde_json::json!({
        "lora": [{"name": "a", "strength": 0.9}, "A", {"name": "a", "strength": 1.5}]
    });
    let req = extract_lora_request(&args).expect("").expect("");
    assert_eq!(req.names().len(), 1, "повторы сворачиваются к первому");
    assert!(
        (req.items[0].strength.expect("сильный первый") - 0.9).abs() < f32::EPSILON,
        "первое объявление выигрывает вместе со своей силой"
    );
}

#[test]
fn extract_item_strength_overrides_only_that_item() {
    // Общий дефолт 1.2; у элемента «a» своя 0.5; у «b» нет — b → 1.2, a → 0.5.
    let args = serde_json::json!({
        "lora": [{"name": "a", "strength": 0.5}, "b"],
        "lora_strength": 1.2
    });
    let req = extract_lora_request(&args).expect("").expect("");
    let dir = sandbox("override_vs_default");
    std::fs::write(dir.join("a.safetensors"), b"stub").expect("стаб-файл");
    std::fs::write(dir.join("b.safetensors"), b"stub").expect("стаб-файл");
    let specs = resolve_specs_in(&dir, &req).expect("резолв");
    assert_eq!(specs.len(), 2);
    assert!((specs[0].multiplier - 0.5).abs() < f32::EPSILON, "a — per-item 0.5");
    assert!((specs[1].multiplier - 1.2).abs() < f32::EPSILON, "b — общий дефолт 1.2");
}

// ─────────────────────────── resolve_specs_in ───────────────────────────

#[test]
fn resolve_unknown_name_lists_available() {
    let dir = sandbox("resolve_unknown");
    std::fs::write(dir.join("the_only.safetensors"), b"stub").expect("стаб-файл");
    let req = LoraRequest {
        items: vec![super::LoraItem {
            name: "ghost".to_string(),
            strength: None,
        }],
        strength: 1.0,
    };
    let err = resolve_specs_in(&dir, &req).expect_err("неизвестное имя — отказ");
    let msg = match err {
        ToolError::NotFound(m) => m,
        other => panic!("ожидался NotFound, получено: {other:?}"),
    };
    assert!(msg.contains("ghost"), "имя запроса в сообщении: {msg}");
    assert!(msg.contains("Доступны"), "перечень доступных в сообщении: {msg}");
    assert!(msg.contains("the_only"), "стем из папки в сообщении: {msg}");
}

#[test]
fn resolve_known_stem_matches_file() {
    let dir = sandbox("resolve_known");
    let file = "FemaledVaginus NSFW.safetensors";
    std::fs::write(dir.join(file), b"stub").expect("стаб-файл");
    let req = LoraRequest {
        items: vec![super::LoraItem {
            name: "FemaledVaginus NSFW".to_string(),
            strength: Some(0.85),
        }],
        strength: 1.0,
    };
    let specs = resolve_specs_in(&dir, &req).expect("валидный стем");
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].path, file);
    assert!((specs[0].multiplier - 0.85).abs() < f32::EPSILON);
}

#[test]
fn resolve_empty_dir_lists_honest_error() {
    let dir = sandbox("resolve_empty");
    let req = LoraRequest {
        items: vec![super::LoraItem {
            name: "ghost".to_string(),
            strength: None,
        }],
        strength: 1.0,
    };
    let err = resolve_specs_in(&dir, &req).expect_err("пустая папка — отказ");
    let msg = match err {
        ToolError::NotFound(m) => m,
        other => panic!("ожидался NotFound, получено: {other:?}"),
    };
    assert!(msg.contains("папка пуста"), "сообщение: {msg}");
}

#[test]
fn resolve_missing_dir_honest_error() {
    let ghost = std::env::temp_dir()
        .join("media_lora_tests_dir_ne_suschestvuet_9182");
    let req = LoraRequest {
        items: vec![super::LoraItem {
            name: "a".to_string(),
            strength: None,
        }],
        strength: 1.0,
    };
    let err = resolve_specs_in(&ghost, &req).expect_err("нет папки — отказ");
    match err {
        ToolError::NotFound(m) => {
            assert!(m.contains("не существует"), "сообщение: {m}");
        }
        other => panic!("ожидался NotFound, получено: {other:?}"),
    }
}

#[test]
fn resolve_clamps_multiplier_on_plugin_boundary() {
    let dir = sandbox("resolve_clamp");
    std::fs::write(dir.join("a.safetensors"), b"stub").expect("стаб-файл");
    // Прямой вызов резолва (обход extract): граница честно клампится плагином.
    let req = LoraRequest {
        items: vec![super::LoraItem {
            name: "a".to_string(),
            strength: Some(9.0),
        }],
        strength: 1.0,
    };
    let specs = resolve_specs_in(&dir, &req).expect("резолв");
    assert!(
        (specs[0].multiplier - tauri_plugin_image_engine::engine::LORA_MULTIPLIER_MAX).abs()
            < f32::EPSILON,
        "кламп до верхней границы"
    );
}

#[test]
fn resolve_multi_item_mixed_strengths() {
    let dir = sandbox("resolve_mixed");
    std::fs::write(dir.join("slider.safetensors"), b"stub").expect("стаб-файл");
    std::fs::write(dir.join("adult.safetensors"), b"stub").expect("стаб-файл");
    std::fs::write(dir.join("free.safetensors"), b"stub").expect("стаб-файл");
    let req = LoraRequest {
        items: vec![
            super::LoraItem { name: "slider".to_string(), strength: Some(1.8) },
            super::LoraItem { name: "adult".to_string(), strength: None },
            super::LoraItem { name: "free".to_string(), strength: Some(0.75) },
        ],
        strength: 1.0,
    };
    let specs = resolve_specs_in(&dir, &req).expect("резолв");
    assert_eq!(specs.len(), 3);
    assert_eq!(specs[0].path, "slider.safetensors");
    assert!((specs[0].multiplier - 1.8).abs() < f32::EPSILON);
    assert!((specs[1].multiplier - 1.0).abs() < f32::EPSILON, "дефолт запроса");
    assert!((specs[2].multiplier - 0.75).abs() < f32::EPSILON);
}

// ─────────────────────────── resolve_specs_in_base (F3, 29.10.26) ───────────────────────────

/// Каталог фазы 3 (F3): лоры разложены по ПОДПАПКАМ на базу. Резолв видит
/// ТОЛЬКО файлы подпапки базы: файлы корня и чужой базы не находятся, а
/// ошибка объясняет структуру (перенести в подпапку) и перечисляет стемы
/// ровно этой базы.
#[test]
fn resolve_base_scopes_folder_cross_base_is_honest_error() {
    let dir = sandbox("resolve_base_scopes");
    // Flat-хвост в корне (юзер ещё не перенёс) — в базу видеться НЕ должен.
    std::fs::write(dir.join("stray_flat.safetensors"), b"stub").expect("стаб-файл");
    let qwen = dir.join("qwen");
    std::fs::create_dir_all(&qwen).expect("подпапка qwen");
    std::fs::write(qwen.join("femaled.safetensors"), b"stub").expect("стаб-файл");
    let krea2 = dir.join("krea2");
    std::fs::create_dir_all(&krea2).expect("подпапка krea2");
    std::fs::write(krea2.join("krea_only.safetensors"), b"stub").expect("стаб-файл");

    // Своё — находится, путь относительно spawn-каталога базы (имя файла).
    let known = LoraRequest {
        items: vec![super::LoraItem { name: "femaled".to_string(), strength: None }],
        strength: 1.0,
    };
    let specs = resolve_specs_in_base(&dir, &known, Some("qwen")).expect("свой стем");
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].path, "femaled.safetensors");

    // Чужая база: krea-стем под qwen — честный отказ; перечень стемов —
    // ТОЛЬКО этой базы, плюс инструкция о структуре папок.
    let cross = LoraRequest {
        items: vec![super::LoraItem { name: "krea_only".to_string(), strength: None }],
        strength: 1.0,
    };
    let err = resolve_specs_in_base(&dir, &cross, Some("qwen")).expect_err("кросс-база");
    let msg = match err {
        ToolError::NotFound(m) => m,
        other => panic!("ожидался NotFound, получено: {other:?}"),
    };
    assert!(msg.contains("krea_only"), "имя запроса в сообщении: {msg}");
    assert!(msg.contains("femaled"), "стемы СВОЕЙ базы в сообщении: {msg}");
    assert!(!msg.contains("krea_only.safetensors") && !msg.contains("stray_flat.safetensors"),
        "чужие стемы не просачиваются: {msg}");
    // Подпапка существует — отказ адресован в каталог именно этой базы.
    // (Инструкция «перенесите в подпапку» появляется, когда подпапки нет —
    // см. resolve_base_missing_folder_honest_error.)
    assert!(msg.contains("qwen"), "каталог базы в сообщении: {msg}");
}

/// Подпапки базы нет — честный отказ с указанием пути и структуры
/// (перенесите лоры в подпапку), не молчаливый «резолв пустого списка».
#[test]
fn resolve_base_missing_folder_honest_error() {
    let dir = sandbox("resolve_base_missing");
    std::fs::write(dir.join("flat.safetensors"), b"stub").expect("стаб-файл");
    let req = LoraRequest {
        items: vec![super::LoraItem { name: "flat".to_string(), strength: None }],
        strength: 1.0,
    };
    let err = resolve_specs_in_base(&dir, &req, Some("ghost_base")).expect_err("базы нет");
    let msg = match err {
        ToolError::NotFound(m) => m,
        other => panic!("ожидался NotFound, получено: {other:?}"),
    };
    assert!(msg.contains("не существует"), "сообщение: {msg}");
    assert!(msg.contains("ghost_base"), "имя базы в сообщении: {msg}");
    assert!(msg.contains("подпапку"), "инструкция о структуре: {msg}");
}

/// Легаси-путь: база не задана (None) — резолв по корню, как раньше.
#[test]
fn resolve_base_none_keeps_legacy_flat() {
    let dir = sandbox("resolve_base_none");
    std::fs::write(dir.join("flat.safetensors"), b"stub").expect("стаб-файл");
    let req = LoraRequest {
        items: vec![super::LoraItem { name: "flat".to_string(), strength: None }],
        strength: 1.0,
    };
    let specs = resolve_specs_in_base(&dir, &req, None).expect("легаси-плоский");
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].path, "flat.safetensors");
}

// ─────────────────────────── status_line ───────────────────────────

#[test]
fn status_line_reports_names_and_resolved_files() {
    let req = LoraRequest {
        items: vec![
            super::LoraItem { name: "a".to_string(), strength: None },
            super::LoraItem { name: "b".to_string(), strength: None },
        ],
        strength: 1.0,
    };
    let specs = vec![
        tauri_plugin_image_engine::engine::LoraSpec {
            path: "a.safetensors".to_string(),
            multiplier: 1.0,
        },
    ];
    let line = status_line(&req, &specs);
    assert!(line.contains("LoRA requested=[a, b]"), "имена запроса: {line}");
    assert!(line.contains("default_strength=1.00"), "общий дефолт: {line}");
    assert!(
        line.contains("resolved=[a.safetensors×1.00]"),
        "фактические файлы: {line}"
    );
    assert!(line.contains("[IMG-TOOL]"), "префикс лога: {line}");
}

#[test]
fn status_line_shows_per_item_resolved_strengths() {
    let req = LoraRequest {
        items: vec![
            super::LoraItem { name: "slider".to_string(), strength: Some(1.8) },
            super::LoraItem { name: "adult".to_string(), strength: None },
        ],
        strength: 0.8,
    };
    let specs = vec![
        tauri_plugin_image_engine::engine::LoraSpec {
            path: "slider.safetensors".to_string(),
            multiplier: 1.8,
        },
        tauri_plugin_image_engine::engine::LoraSpec {
            path: "adult.safetensors".to_string(),
            multiplier: 0.8,
        },
    ];
    let line = status_line(&req, &specs);
    assert!(
        line.contains("slider.safetensors×1.80") && line.contains("adult.safetensors×0.80"),
        "силы в resolved печатаются ПО-ЭЛЕМЕНТНО: {line}"
    );
    assert!(line.contains("default_strength=0.80"), "дефолт назван честно: {line}");
}

// ─────────────────────────── schema-контракты ───────────────────────────

fn first_schema(name: &str) -> Value {
    let schemas = image_tool_schemas(&[
        "generate_image".to_string(),
        "edit_image".to_string(),
        "upscale_image".to_string(),
    ]);
    schemas
        .iter()
        .find(|(_, tool, _)| tool == name)
        .expect("схема тула объявлена")
        .2
        .get("inputSchema")
        .expect("inputSchema")
        .clone()
}

/// Контракт фазы 2: элементы lora — строка ИЛИ объект {name, strength}.
fn assert_lora_items_mixed_contract(lora: &Value) {
    let items = lora.pointer("/items").expect("items есть");
    let variants = items.get("oneOf").and_then(|v| v.as_array()).expect("oneOf есть");
    assert!(
        variants.iter().any(|v| v.get("type").and_then(|t| t.as_str()) == Some("string")),
        "строчная форма описана: {variants:?}"
    );
    let obj = variants
        .iter()
        .find(|v| v.get("type").and_then(|t| t.as_str()) == Some("object"))
        .expect("объектная форма описана");
    assert_eq!(
        obj.pointer("/required/0").and_then(|v| v.as_str()),
        Some("name"),
        "объектная форма требует name"
    );
    assert_eq!(
        obj.pointer("/properties/strength/maximum").and_then(|v| v.as_f64()),
        Some(2.0),
        "strength 0–2 задекларирован"
    );
}

#[test]
fn generate_schema_declares_optional_mixed_lora() {
    let schema = first_schema("generate_image");
    let lora = schema.pointer("/properties/lora").expect("поле lora есть");
    assert_eq!(lora.get("type").and_then(|v| v.as_str()), Some("array"));
    assert_eq!(lora.get("minItems").and_then(|v| v.as_i64()), Some(1));
    assert_lora_items_mixed_contract(lora);
    let strength = schema
        .pointer("/properties/lora_strength")
        .expect("поле lora_strength есть");
    assert_eq!(strength.get("type").and_then(|v| v.as_str()), Some("number"));
    assert_eq!(strength.get("maximum").and_then(|v| v.as_f64()), Some(2.0));
    // Оба поля опциональны: required не расширяется провайдером.
    let required = schema.get("required").and_then(|v| v.as_array()).expect("required");
    assert!(
        !required.iter().any(|v| v.as_str() == Some("lora")),
        "lora обязателен — против PRP: {required:?}"
    );
    assert!(
        !required.iter().any(|v| v.as_str() == Some("lora_strength")),
        "lora_strength обязателен — против PRP: {required:?}"
    );
}

#[test]
fn edit_schema_declares_optional_mixed_lora() {
    let schema = first_schema("edit_image");
    let lora = schema
        .pointer("/properties/lora")
        .expect("edit имеет lora-поле (правило агента может требовать edit-лору)");
    assert_lora_items_mixed_contract(lora);
    assert!(
        schema.pointer("/properties/lora_strength").is_some(),
        "edit имеет lora_strength-поле"
    );
    let required = schema.get("required").and_then(|v| v.as_array()).expect("required");
    assert!(
        !required.iter().any(|v| v.as_str() == Some("lora")),
        "lora необязателен для edit"
    );
}

#[test]
fn upscale_schema_has_no_lora() {
    let schema = first_schema("upscale_image");
    assert!(
        schema.pointer("/properties/lora").is_none(),
        "апскейл LoRA-стек не ведёт (решение PRP)"
    );
    assert!(
        schema.pointer("/properties/lora_strength").is_none(),
        "апскейл LoRA-стек не ведёт (решение PRP)"
    );
    // Прежние контрактные поля апскейла не изменены.
    let required = schema.get("required").and_then(|v| v.as_array()).expect("required");
    assert_eq!(required.len(), 2, "старый required: {required:?}");
}

// ─────────────────────────── каталог лор юзера ({{ video_loras }}) ───────────────────────────

use super::{scan_video_lora_catalog_in, video_loras_prompt_in};

/// Sidecar-описание парсится: description / triggers (в обеих формах) /
/// default_strength / category; сам .md не попадает в каталог как лора.
#[test]
fn scan_catalog_parses_sidecar_frontmatter() {
    let dir = sandbox("catalog_sidecar");
    std::fs::write(dir.join("alpha.safetensors"), b"x").unwrap();
    std::fs::write(
        dir.join("alpha.md"),
        "---\ndescription: Универсальный NSFW-разблокировщик\ntriggers: [nude, topless]\ndefault_strength: 0.5\ncategory: nsfw_universal\n---\nТело игнорируется.\n",
    )
    .unwrap();
    let entries = scan_video_lora_catalog_in(&dir);
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.stem, "alpha");
    assert_eq!(e.file, "alpha.safetensors");
    assert_eq!(e.description, "Универсальный NSFW-разблокировщик");
    assert_eq!(e.triggers, vec!["nude", "topless"]);
    assert_eq!(e.default_strength, Some(0.5));
    assert_eq!(e.category, "nsfw_universal");
}

/// Лора без sidecar — голый стем с пустыми полями; битый frontmatter не роняет
/// скан; посторонние файлы (.md без пары, .json) в каталог не попадают;
/// сортировка по стему стабильна (регистронезависимо).
#[test]
fn scan_catalog_survives_missing_and_broken_sidecars() {
    let dir = sandbox("catalog_broken");
    std::fs::write(dir.join("b_beta.safetensors"), b"x").unwrap();
    std::fs::write(dir.join("a_alpha.safetensors"), b"x").unwrap();
    std::fs::write(dir.join("a_alpha.md"), "не frontmatter, просто текст").unwrap();
    std::fs::write(dir.join("orphan.md"), "---\ndescription: без пары\n---\n").unwrap();
    std::fs::write(dir.join("notes.json"), "{}").unwrap();
    let entries = scan_video_lora_catalog_in(&dir);
    let stems: Vec<&str> = entries.iter().map(|e| e.stem.as_str()).collect();
    assert_eq!(stems, vec!["a_alpha", "b_beta"], "сорт по стему, мусор отфильтрован");
    let alpha = &entries[0];
    assert!(alpha.description.is_empty(), "битый sidecar = пустые поля");
    assert!(alpha.triggers.is_empty());
    assert_eq!(alpha.default_strength, None);
}

/// Блок для агента: по строке на лору со стемом/описанием/триггерами/силой;
/// пустой каталог — честная заглушка, а не тишина.
#[test]
fn prompt_block_lists_entries_and_falls_back_on_empty() {
    let dir = sandbox("catalog_prompt");
    assert!(
        video_loras_prompt_in(&dir).contains("пусто"),
        "пустой каталог — честная заглушка"
    );
    std::fs::write(dir.join("alpha.safetensors"), b"x").unwrap();
    std::fs::write(
        dir.join("alpha.md"),
        "---\ndescription: Кожа и реализм\ntriggers: none\ndefault_strength: 1.2\ncategory: realism\n---\n",
    )
    .unwrap();
    let block = video_loras_prompt_in(&dir);
    assert!(block.contains("- alpha — Кожа и реализм"), "{}", block);
    assert!(block.contains("[realism]"), "{}", block);
    assert!(block.contains("1.20"), "сила в блоке: {}", block);
}
