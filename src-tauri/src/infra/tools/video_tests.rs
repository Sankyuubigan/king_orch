use super::*;

fn test_ctx(registry: Option<&ImageArtifactRegistry>) -> ToolCtx<'_> {
    ToolCtx {
        workspace_root: std::path::Path::new("."),
        write_root: std::path::Path::new("."),
        write_outside: crate::infra::tools::WriteOutside::Prompt,
        session_id: "test",
        approver: crate::infra::permissions::test_approver(),
        agent_id: "test_agent",
        bins_dir: std::path::Path::new("."),
        image_artifacts: registry,
    }
}

#[test]
fn video_schema_only_for_declared_tool() {
    let schemas = video_tool_schemas(&["generate_video".to_string()]);
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].0, "media");
    assert_eq!(schemas[0].1, "generate_video");
    let required = schemas[0]
        .2
        .pointer("/inputSchema/required")
        .and_then(|value| value.as_array())
        .unwrap()
        .clone();
    assert_eq!(
        required,
        vec![serde_json::Value::String("prompt_en".to_string())]
    );
    // generate_image-агент НЕ получает видео-тул.
    assert!(video_tool_schemas(&[]).is_empty());
    assert!(video_tool_schemas(&["generate_image".to_string()]).is_empty());
}

#[test]
fn video_schema_documents_motion_and_quality() {
    let tool = GenerateVideo;
    assert!(tool.description().contains("видео"), "{}", tool.description());
    let params = tool.parameters();
    let seconds = params.pointer("/properties/seconds").unwrap();
    assert_eq!(
        seconds.get("maximum").and_then(|v| v.as_i64()),
        Some(15),
        "лимит агента в схеме = 15 сек, финальный кламп — в плагине по max_seconds каталога"
    );
    assert_eq!(
        seconds.get("minimum").and_then(|v| v.as_i64()),
        Some(1)
    );
    let fq = params.pointer("/properties/final_quality").unwrap();
    assert_eq!(fq.get("type").and_then(|v| v.as_str()), Some("boolean"));
}

#[test]
fn video_result_uses_opaque_artifact_id() {
    let registry = ImageArtifactRegistry::new();
    let ctx = test_ctx(Some(&registry));
    let output = format_video_result(
        VideoRunOutput {
            path: "D:\\private\\clip.webm".to_string(),
            time_sec: 12.5,
        },
        &ctx,
    )
    .unwrap();
    assert!(output.contains("[VIDEO_SAVED id=artifact_1]"), "{}", output);
    assert!(!output.contains("D:\\private"), "путь не должен светиться в промпте");
}

#[test]
fn unknown_tool_returns_none_and_empty_prompt_is_usage() {
    let ctx = test_ctx(None);
    assert!(execute_video_tool("nope", &serde_json::json!({}), &ctx, &[]).is_none());
    let err = execute_video_tool(
        "generate_video",
        &serde_json::json!({"prompt_en": "   "}),
        &ctx,
        &[],
    )
    .expect("generate_video обрабатывается")
    .unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)));
}

/// Схема тула объявляет i2va-вход: source_image_ids (ровно один ID) — опционален,
/// t2va остаётся путём по умолчанию (образец media_tests.rs).
#[test]
fn video_schema_declares_source_image_ids() {
    let tool = GenerateVideo;
    let params = tool.parameters();
    let ids = params
        .pointer("/properties/source_image_ids")
        .expect("source_image_ids объявлен в схеме");
    assert_eq!(ids.get("type").and_then(|v| v.as_str()), Some("array"));
    assert_eq!(ids.get("minItems").and_then(|v| v.as_i64()), Some(1));
    assert_eq!(ids.get("maxItems").and_then(|v| v.as_i64()), Some(1));
    // Не в required: видео с нуля по тексту — легальный вызов без картинок.
    let required = params
        .pointer("/required")
        .and_then(|v| v.as_array())
        .unwrap();
    assert!(!required.iter().any(|v| v == "source_image_ids"));
    assert!(tool.description().contains("source_image_ids"));
}

/// LoRA-стек в схеме тула: поля lora/lora_strength задекларированы,
/// prompt_en документирует прокидку реплики речи (русская речь — дословно).
#[test]
fn video_schema_declares_lora_stack_and_speech() {
    let tool = GenerateVideo;
    let params = tool.parameters();
    let lora = params
        .pointer("/properties/lora")
        .expect("lora объявлен в схеме");
    assert_eq!(lora.get("type").and_then(|v| v.as_str()), Some("array"));
    assert_eq!(lora.get("minItems").and_then(|v| v.as_i64()), Some(1));
    let strength = params
        .pointer("/properties/lora_strength")
        .expect("lora_strength объявлен в схеме");
    assert_eq!(strength.get("maximum").and_then(|v| v.as_f64()), Some(2.0));
    let prompt_en = params
        .pointer("/properties/prompt_en")
        .and_then(|v| v.get("description"))
        .and_then(|v| v.as_str())
        .expect("описание prompt_en");
    assert!(
        prompt_en.contains("says in Russian"),
        "схема несёт формат речи с маркером языка: {}",
        prompt_en
    );
    assert!(prompt_en.contains("No subtitles"), "схема несёт запрет субтитров");
}

/// lora_strength без lora — Usage ДО всякого обращения к бандлу/движку
/// (зеркало media; герметично: ошибка на границе extract_lora_request).
#[test]
fn video_lora_strength_without_lora_is_usage() {
    let ctx = test_ctx(None);
    let err = execute_video_tool(
        "generate_video",
        &serde_json::json!({"prompt_en": "тест", "lora_strength": 1.5}),
        &ctx,
        &[],
    )
    .expect("generate_video обрабатывается")
    .unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)));
}

/// Пустой массив lora — Usage: поле без имён бессмысленно, опустите его.
#[test]
fn video_empty_lora_array_is_usage() {
    let ctx = test_ctx(None);
    let err = execute_video_tool(
        "generate_video",
        &serde_json::json!({"prompt_en": "тест", "lora": []}),
        &ctx,
        &[],
    )
    .expect("generate_video обрабатывается")
    .unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)));
}

/// Две и больше картинок — честный Usage-отказ ДО всякого обращения к бандлу
/// (fl2va/ref2va в туле не открыты; молча взять первую — потеря данных юзера).
#[test]
fn video_refuses_multiple_attachments_honestly() {
    let ctx = test_ctx(None);
    let attachments = vec![
        crate::infra::ChatAttachment {
            file_name: "a.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: String::new(),
            file_path: Some("D:\\no\\a.png".to_string()),
            is_dir: Some(false),
        },
        crate::infra::ChatAttachment {
            file_name: "b.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: String::new(),
            file_path: Some("D:\\no\\b.png".to_string()),
            is_dir: Some(false),
        },
    ];
    let err = execute_video_tool(
        "generate_video",
        &serde_json::json!({"prompt_en": "тест"}),
        &ctx,
        &attachments,
    )
    .expect("generate_video обрабатывается")
    .unwrap_err();
    match err {
        ToolError::Usage(msg) => {
            assert!(msg.contains("РОВНО одну"), "отказ называет лимит: {}", msg);
        }
        other => panic!("ожидался Usage, получен: {:?}", other),
    }
}

/// Гейт режима бандла (Шаг 4): i2va-запрос на бандле без «i2va» в modes —
/// честный Usage-отказ с инструкцией, а НЕ тихий t2va. Синтетический entry
/// из встроенного каталога (герметично, без реального конфига машины).
#[test]
fn video_refuses_i2va_on_bundle_without_mode() {
    let mut ref2va = tauri_plugin_image_engine::engine::video_bundle_by_name(
        "minimax_h3_ref2va_pruned-q3k",
    )
    .expect("ref2va-бандл в каталоге");
    assert!(!ref2va.modes.iter().any(|m| m == "i2va"));
    let err = ensure_bundle_supports_i2va(&ref2va).unwrap_err();
    match err {
        ToolError::Usage(msg) => {
            assert!(msg.contains("не поддерживает генерацию по картинке"), "{}", msg);
            assert!(msg.contains("modes"), "отказ перечисляет modes: {}", msg);
            assert!(msg.contains("Настройки"), "отказ даёт инструкцию: {}", msg);
        }
        other => panic!("ожидался Usage, получен: {:?}", other),
    }
    // fl2va-бандл с i2va — гейт пропускает.
    let mut fl2va = tauri_plugin_image_engine::engine::video_bundle_by_name(
        "minimax_h3_fl2va_pruned-q4km",
    )
    .expect("fl2va-бандл в каталоге");
    assert!(fl2va.modes.iter().any(|m| m == "i2va"));
    assert!(ensure_bundle_supports_i2va(&fl2va).is_ok());
    // Бандл вообще без modes (добавленный кодом) — гейт отказывает.
    ref2va.modes.clear();
    assert!(ensure_bundle_supports_i2va(&ref2va).is_err());
}

/// Гейт роли картинки (§00.2): раньше состояния «роль» не существовало и
/// диспетчер при любом вложении принуждал i2va (эскиз-референс становился
/// первым кадром; сессия 1793324737511 — 2 попытки впустую). Теперь выбор явный.
#[test]
fn video_image_gate_matrix() {
    // Без вложений и без роли — t2va разрешён.
    assert!(video_image_gate_error(None, false, false).is_none());
    // Вложения есть + source_image_ids без роли — легаси-путь i2va разрешён.
    assert!(video_image_gate_error(None, true, true).is_none());
    // Явный reference — t2va разрешён; source_image_ids при reference запрещён.
    assert!(video_image_gate_error(Some("reference"), false, true).is_none());
    assert!(video_image_gate_error(Some("reference"), true, true).is_some());
    // first_frame без источника — отказ; с источником — ок (в т.ч. из истории).
    assert!(video_image_gate_error(Some("first_frame"), false, false).is_some());
    assert!(video_image_gate_error(Some("first_frame"), true, false).is_none());
    // Вложения есть, роль не выбрана, ID нет — принудительный выбор с обоими режимами.
    let err = video_image_gate_error(None, false, true).expect("выбор роли обязателен");
    assert!(err.contains("first_frame") && err.contains("reference"), "{}", err);
    // Неизвестное значение — отказ с перечнем допустимых.
    let err = video_image_gate_error(Some("canvas"), false, false).expect("неизвестная роль");
    assert!(err.contains("first_frame") && err.contains("reference"), "{}", err);
}

/// Схема тула объявляет image_usage (обе роли) и не делает его required:
/// t2va без вложений остаётся легальным вызовом.
#[test]
fn video_schema_declares_image_usage() {
    let tool = GenerateVideo;
    let params = tool.parameters();
    let usage = params
        .pointer("/properties/image_usage")
        .expect("image_usage объявлен в схеме");
    assert_eq!(usage.get("type").and_then(|v| v.as_str()), Some("string"));
    let variants = usage
        .get("enum")
        .and_then(|v| v.as_array())
        .expect("enum ролей");
    assert_eq!(variants.len(), 2);
    assert!(variants.iter().any(|v| v == "first_frame"));
    assert!(variants.iter().any(|v| v == "reference"));
    let required = params
        .pointer("/required")
        .and_then(|v| v.as_array())
        .unwrap();
    assert!(!required.iter().any(|v| v == "image_usage"));
    assert!(tool.description().contains("image_usage"));
    assert!(tool.description().contains("ПЕРВЫМ КАДРОМ"));
}

#[test]
fn video_artifact_resolves_only_for_generate_video() {
    let registry = ImageArtifactRegistry::new();
    let id = registry
        .register("D:\\private\\a.webm".to_string(), "generate_video")
        .unwrap();
    assert!(registry.resolve(&id, "generate_video").is_ok());
    // Кросс-инструментальный доступ запрещён (тот же реестр, что у image-тулов).
    assert!(registry.resolve(&id, "generate_image").is_err());
    assert!(registry.resolve("artifact_999", "generate_video").is_err());
}

#[test]
fn extract_saved_video_id_parses_marker() {
    assert_eq!(
        extract_saved_video_id("[VIDEO_SAVED id=artifact_7] Готово за 1.0 сек"),
        Some("artifact_7".to_string())
    );
    assert_eq!(extract_saved_video_id("нет маркера"), None);
}

#[test]
fn attach_videos_uses_current_subcalls_and_file_path_no_base64() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("king_orch_video_{}_{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("vid.webm");
    std::fs::write(&clip, [0x1Au8, 0x45u8, 0xDFu8, 0xA3u8]).unwrap(); // EBML-заголовок
    let path_str = clip.to_string_lossy().to_string();

    let registry = ImageArtifactRegistry::new();
    let id = registry.register(path_str.clone(), "generate_video").unwrap();
    let sub_call = crate::infra::SubCall {
        agent_name: "video_maker".to_string(),
        prompt: String::new(),
        response: String::new(),
        time_sec: 0.0,
        tool_calls: vec![crate::infra::ToolCallInfo {
            tool_name: "generate_video".to_string(),
            arguments: "{}".to_string(),
            result: format!("[VIDEO_SAVED id={id}] Готово за 9.9 сек"),
        }],
        thinking: None,
    };
    let mut message = crate::infra::ChatMessage {
        id: Some("msg_1".to_string()),
        msg_type: "message".to_string(),
        content: String::new(),
        sub_calls: Some(vec![sub_call]),
        author: Some("video_maker".to_string()),
        model: None,
        time_sec: None,
        attachments: None,
        phase: Some(2),
        author_name: None,
    };
    let current_sub_calls = message.sub_calls.clone().unwrap_or_default();
    let attached = attach_saved_videos_from_sub_calls(&mut message, &current_sub_calls, &registry)
        .unwrap();
    assert_eq!(attached, 1);
    let att = &message.attachments.as_ref().unwrap()[0];
    assert_eq!(att.file_name, "vid.webm");
    assert_eq!(att.mime_type, "video/webm");
    // Главный инвариант: клип идёт ПО ПУТИ, base64 пуст (webm — десятки МБ).
    assert_eq!(att.data_base64, "");
    assert_eq!(att.file_path.as_deref(), Some(path_str.as_str()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn attach_videos_skips_other_tools_and_unknown_ids_survive() {
    let registry = ImageArtifactRegistry::new();
    let image_id = registry
        .register("D:\\no\\img.png".to_string(), "generate_image")
        .unwrap();
    // Маркер image-tула в sub_call generate_video НЕ резолвится (кросс-тул) —
    // остальный артефакт отсутствует, attach вернуть 0 без паники.
    let sub_call = crate::infra::SubCall {
        agent_name: "video_maker".to_string(),
        prompt: String::new(),
        response: String::new(),
        time_sec: 0.0,
        tool_calls: vec![crate::infra::ToolCallInfo {
            tool_name: "generate_video".to_string(),
            arguments: "{}".to_string(),
            result: format!("[IMAGE_SAVED id={image_id}]"),
        }],
        thinking: None,
    };
    let mut message = crate::infra::ChatMessage {
        id: Some("msg_1".to_string()),
        msg_type: "message".to_string(),
        content: String::new(),
        sub_calls: Some(vec![sub_call]),
        author: Some("video_maker".to_string()),
        model: None,
        time_sec: None,
        attachments: None,
        phase: Some(2),
        author_name: None,
    };
    let current_sub_calls = message.sub_calls.clone().unwrap_or_default();
    // IMAGE_SAVED-маркер не парсится extract_saved_video_id → 0 без реестра-инъекции.
    let attached = attach_saved_videos_from_sub_calls(&mut message, &current_sub_calls, &registry);
    assert_eq!(attached.unwrap(), 0);
    assert!(message.attachments.is_none());
}
