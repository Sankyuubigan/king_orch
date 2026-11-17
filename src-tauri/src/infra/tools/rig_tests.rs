use super::*;
use crate::infra::ImageArtifactRegistry;

fn test_ctx(registry: &ImageArtifactRegistry) -> ToolCtx<'_> {
    ToolCtx {
        workspace_root: std::path::Path::new("."),
        write_root: std::path::Path::new("."),
        write_outside: crate::infra::tools::WriteOutside::Prompt,
        session_id: "test",
        approver: crate::infra::permissions::test_approver(),
        agent_id: "test_agent",
        bins_dir: std::path::Path::new("."),
        image_artifacts: Some(registry),
    }
}

#[test]
fn schemas_only_for_declared_tools() {
    let schemas = rig_tool_schemas(&["rig_mesh".to_string()]);
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].0, "rig");
    assert_eq!(schemas[0].1, "rig_mesh");
    let required = schemas[0]
        .2
        .get("inputSchema")
        .unwrap()
        .get("required")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(required[0], serde_json::Value::String("source_mesh_ids".to_string()));
    let empty = rig_tool_schemas(&[]);
    assert!(empty.is_empty());
}

#[test]
fn rig_result_uses_opaque_artifact_id() {
    let registry = ImageArtifactRegistry::new();
    let ctx = test_ctx(&registry);
    let output =
        format_rig_result(std::path::Path::new("D:\\private\\rigged-1.glb"), 12.34, &ctx).unwrap();
    assert!(output.starts_with("[RIGGED_SAVED id=artifact_1]"), "{output}");
    assert!(output.contains("12.3 сек"), "{output}");
    assert!(!output.contains("D:\\private"));
    // Честный отчёт: результат — риг, не анимация.
    assert!(output.contains("анимац"), "{output}");
}

#[test]
fn saved_rigged_attachments_are_path_only() {
    let registry = ImageArtifactRegistry::new();
    let path = std::env::temp_dir().join(format!("rig_test_{}.glb", std::process::id()));
    std::fs::write(&path, b"glTF-binary-stub").unwrap();
    let id = registry
        .register(path.to_string_lossy().to_string(), "rig_mesh")
        .unwrap();
    let mut message = crate::infra::ChatMessage {
        id: Some("msg_0".to_string()),
        msg_type: "message".to_string(),
        content: "готово".to_string(),
        sub_calls: None,
        author: Some("rig_operator".to_string()),
        model: None,
        time_sec: None,
        attachments: None,
        phase: None,
        author_name: None,
    };
    let sub_call = crate::infra::SubCall {
        agent_name: "Rig Operator".to_string(),
        prompt: "тест".to_string(),
        response: "готово".to_string(),
        time_sec: 1.0,
        thinking: None,
        tool_calls: vec![crate::infra::ToolCallInfo {
            tool_name: "rig_mesh".to_string(),
            arguments: "{}".to_string(),
            result: format!("[RIGGED_SAVED id={id}] Готово за 1.0 сек"),
        }],
    };
    let attached = attach_saved_rigged_meshes_from_sub_calls(&mut message, &[sub_call], &registry)
        .unwrap();
    assert_eq!(attached, 1);
    let attachments = message.attachments.unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].mime_type, "model/gltf-binary");
    assert_eq!(attachments[0].data_base64, "");
    assert_eq!(attachments[0].file_path.as_deref(), Some(path.to_str().unwrap()));
    std::fs::remove_file(&path).ok();
}

#[test]
fn rigged_artifact_type_check_blocks_foreign_tools() {
    let registry = ImageArtifactRegistry::new();
    let id = registry
        .register("C:\\tmp\\x.glb".to_string(), "rig_mesh")
        .unwrap();
    // attach-путь: чужой тул не может приложить rig-артефакт
    assert!(registry.resolve(&id, "generate_mesh").is_err());
    // кросс-тул ЧТЕНИЕ источника легально (generate_mesh → rig_mesh)
    assert!(registry.resolve_any(&id).is_ok());
}

#[test]
fn execute_requires_exactly_one_source() {
    let registry = ImageArtifactRegistry::new();
    let ctx = test_ctx(&registry);
    // 0 источников: resolve_mesh_ref_paths честно отказывает раньше всего
    let zero = execute_rig_tool("rig_mesh", &serde_json::json!({}), &ctx, &[]).unwrap();
    let err = zero.unwrap_err().to_string();
    assert!(err.contains("Не выбран меш"), "{err}");
    // 2 источника: РОВНО один (файлы должны существовать — resolve проверяет)
    let dir = std::env::temp_dir();
    let make = |name: &str| {
        let path = dir.join(format!("rig_src_{}_{}", std::process::id(), name));
        std::fs::write(&path, b"glb").unwrap();
        crate::infra::ChatAttachment {
            file_name: name.to_string(),
            mime_type: "model/gltf-binary".to_string(),
            data_base64: String::new(),
            file_path: Some(path.to_string_lossy().to_string()),
            is_dir: Some(false),
        }
    };
    let a = make("a.glb");
    let b = make("b.glb");
    let two = execute_rig_tool("rig_mesh", &serde_json::json!({}), &ctx, &[a, b]).unwrap();
    let err = two.unwrap_err().to_string();
    assert!(err.contains("РОВНО один"), "{err}");
    std::fs::remove_file(dir.join(format!("rig_src_{}_a.glb", std::process::id()))).ok();
    std::fs::remove_file(dir.join(format!("rig_src_{}_b.glb", std::process::id()))).ok();
}

#[test]
fn execute_rejects_attachment_without_file_path() {
    let registry = ImageArtifactRegistry::new();
    let ctx = test_ctx(&registry);
    let legacy = crate::infra::ChatAttachment {
        file_name: "old.glb".to_string(),
        mime_type: "model/gltf-binary".to_string(),
        data_base64: String::new(),
        file_path: None,
        is_dir: Some(false),
    };
    let err = execute_rig_tool("rig_mesh", &serde_json::json!({}), &ctx, &[legacy])
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(err.contains("без файла на диске"), "{err}");
}

#[test]
fn extract_saved_rigged_id_parses_marker() {
    assert_eq!(
        extract_saved_rigged_id("[RIGGED_SAVED id=artifact_7] Готово"),
        Some("artifact_7".to_string())
    );
    assert_eq!(extract_saved_rigged_id("без маркера"), None);
}

#[test]
fn engine_error_classification_is_honest() {
    // Preflight-отказ → NotFound (честный стоп, ретрай не изменит диск)
    let setup = rig_engine_error("skintokens-cli.exe не найден: D:\\x\\skintokens-cli.exe".into());
    assert!(setup.to_string().starts_with("Не найдено:"), "{setup}");
    let weights = rig_engine_error("Вес риггинга не найден: w.gguf".into());
    assert!(weights.to_string().starts_with("Не найдено:"), "{weights}");
    // Ошибка входа агента → Usage (исправимо ретраем с другим аргументом)
    let bad_ext = rig_engine_error(
        "Риггинг принимает .glb (или .t2mesh), получено .obj: duck.obj".into(),
    );
    assert!(!bad_ext.to_string().starts_with("Не найдено:"), "{bad_ext}");
    let bad_device = rig_engine_error("device 'cuda' не поддержан: допустимо cpu|vulkan|auto".into());
    assert!(!bad_device.to_string().starts_with("Не найдено:"), "{bad_device}");
}
