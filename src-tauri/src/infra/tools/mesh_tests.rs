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
    let schemas = mesh_tool_schemas(&["generate_mesh".to_string()]);
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].0, "mesh");
    assert_eq!(schemas[0].1, "generate_mesh");
    let required = schemas[0]
        .2
        .get("inputSchema")
        .unwrap()
        .get("required")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(required[0], serde_json::Value::String("source_image_ids".to_string()));
    let empty = mesh_tool_schemas(&[]);
    assert!(empty.is_empty());
}

#[test]
fn mesh_result_uses_opaque_artifact_id() {
    let registry = ImageArtifactRegistry::new();
    let ctx = test_ctx(&registry);
    let output = format_mesh_result(std::path::Path::new("D:\\private\\mesh-1.glb"), 12.34, &ctx).unwrap();
    assert!(output.starts_with("[MESH_SAVED id=artifact_1]"), "{output}");
    assert!(output.contains("12.3 сек"), "{output}");
    assert!(!output.contains("D:\\private"));
}

#[test]
fn saved_mesh_attachments_are_path_only() {
    let registry = ImageArtifactRegistry::new();
    let path = std::env::temp_dir().join(format!("mesh_test_{}.glb", std::process::id()));
    std::fs::write(&path, b"glTF-binary-stub").unwrap();
    let id = registry
        .register(path.to_string_lossy().to_string(), "generate_mesh")
        .unwrap();
    let mut message = crate::infra::ChatMessage {
        id: Some("msg_0".to_string()),
        msg_type: "message".to_string(),
        content: "готово".to_string(),
        sub_calls: None,
        author: Some("mesh_operator".to_string()),
        model: None,
        time_sec: None,
        attachments: None,
        phase: None,
        author_name: None,
    };
    let sub_call = crate::infra::SubCall {
        agent_name: "Mesh Operator".to_string(),
        prompt: "тест".to_string(),
        response: "готово".to_string(),
        time_sec: 1.0,
        thinking: None,
        tool_calls: vec![crate::infra::ToolCallInfo {
            tool_name: "generate_mesh".to_string(),
            arguments: "{}".to_string(),
            result: format!("[MESH_SAVED id={id}] Готово за 1.0 сек"),
        }],
    };
    let attached = attach_saved_meshes_from_sub_calls(&mut message, &[sub_call], &registry).unwrap();
    assert_eq!(attached, 1);
    let attachments = message.attachments.unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].mime_type, "model/gltf-binary");
    assert_eq!(attachments[0].data_base64, "");
    assert_eq!(attachments[0].file_path.as_deref(), Some(path.to_str().unwrap()));
    std::fs::remove_file(&path).ok();
}

#[test]
fn mesh_artifact_type_check_blocks_foreign_tools() {
    let registry = ImageArtifactRegistry::new();
    let id = registry
        .register("C:\\tmp\\x.glb".to_string(), "generate_mesh")
        .unwrap();
    // attach-путь: чужой тул не может приложить mesh-артефакт
    assert!(registry.resolve(&id, "generate_image").is_err());
    // кросс-тул ЧТЕНИЕ источника легально (edit_image → generate_mesh)
    assert!(registry.resolve_any(&id).is_ok());
}

#[test]
fn execute_requires_exactly_one_source() {
    let registry = ImageArtifactRegistry::new();
    let ctx = test_ctx(&registry);
    // 0 источников: resolve_ref_paths честно отказывает раньше проверки длины
    let zero = execute_mesh_tool("generate_mesh", &serde_json::json!({}), &ctx, &[]).unwrap();
    let err = zero.unwrap_err().to_string();
    assert!(err.contains("Не выбрано изображение"), "{err}");
    // 2 источника: сервер принимает ровно одно изображение (файлы должны
    // существовать — resolve_ref_paths проверяет доступность)
    let dir = std::env::temp_dir();
    let make = |name: &str| {
        let path = dir.join(format!("mesh_src_{}_{}", std::process::id(), name));
        std::fs::write(&path, b"png").unwrap();
        crate::infra::ChatAttachment {
            file_name: name.to_string(),
            mime_type: "image/png".to_string(),
            data_base64: String::new(),
            file_path: Some(path.to_string_lossy().to_string()),
            is_dir: Some(false),
        }
    };
    let a = make("a.png");
    let b = make("b.png");
    let two = execute_mesh_tool(
        "generate_mesh",
        &serde_json::json!({}),
        &ctx,
        &[a, b],
    )
    .unwrap();
    let err = two.unwrap_err().to_string();
    assert!(err.contains("РОВНО один исходник"), "{err}");
    std::fs::remove_file(dir.join(format!("mesh_src_{}_a.png", std::process::id()))).ok();
    std::fs::remove_file(dir.join(format!("mesh_src_{}_b.png", std::process::id()))).ok();
}

#[test]
fn extract_saved_mesh_id_parses_marker() {
    assert_eq!(
        extract_saved_mesh_id("[MESH_SAVED id=artifact_7] Готово"),
        Some("artifact_7".to_string())
    );
    assert_eq!(extract_saved_mesh_id("без маркера"), None);
}

// ── Text→3D: тул mesh_concept_prompt (SSOT шаблонов — core плагина) ──

#[test]
fn concept_prompt_schema_only_when_declared_and_readonly() {
    let schemas = mesh_tool_schemas(&["mesh_concept_prompt".to_string()]);
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].0, "mesh");
    assert_eq!(schemas[0].1, "mesh_concept_prompt");
    let schema = &schemas[0].2;
    let required = schema.get("inputSchema").unwrap().get("required").unwrap();
    let required = required.as_array().unwrap();
    assert!(required.contains(&serde_json::Value::String("subject_en".to_string())));
    assert!(required.contains(&serde_json::Value::String("kind".to_string())));
    assert!(MeshConceptPrompt.is_readonly());
    let both = mesh_tool_schemas(&[
        "generate_mesh".to_string(),
        "mesh_concept_prompt".to_string(),
    ]);
    assert_eq!(both.len(), 2);
}

#[test]
fn concept_prompt_execute_builds_template_prompt() {
    let registry = ImageArtifactRegistry::new();
    let ctx = test_ctx(&registry);
    let result = execute_mesh_tool(
        "mesh_concept_prompt",
        &serde_json::json!({"subject_en": "  an ancient stone chest  ", "kind": "prop"}),
        &ctx,
        &[],
    )
    .unwrap()
    .unwrap();
    assert!(result.contains("CONCEPT_PROMPT:"), "{result}");
    assert!(result.contains("an ancient stone chest"), "{result}");
    // Шаблон специалиста: запреты в теле (cfg 1.0 → негатив не работает)
    assert!(result.contains("#FFFFFF"), "{result}");
    assert!(result.contains("three-quarter isometric perspective"), "{result}");
    // Футер-инструкция: генерация картинки, не модели
    assert!(result.contains("prompt_en"), "{result}");
    assert!(result.contains("generate_image"), "{result}");
}

#[test]
fn concept_prompt_execute_per_kind_and_errors() {
    let registry = ImageArtifactRegistry::new();
    let ctx = test_ctx(&registry);
    let run = |args: serde_json::Value| {
        execute_mesh_tool("mesh_concept_prompt", &args, &ctx, &[]).unwrap()
    };
    // Каждая категория даёт свой шаблон-маркер
    for (kind, marker) in [
        ("tree", "dense chunky sculpted foliage clusters"),
        ("weapon", "45-degree perspective"),
        ("character", "symmetrical A-pose"),
    ] {
        let out = run(serde_json::json!({"subject_en": "a thing", "kind": kind}))
            .unwrap();
        assert!(out.contains(marker), "kind={kind}: {out}");
    }
    // Невалидная категория — честная ошибка с перечнем
    let bad_kind = run(serde_json::json!({"subject_en": "a thing", "kind": "spaceship"}))
        .unwrap_err()
        .to_string();
    assert!(bad_kind.contains("prop, tree, weapon, character"), "{bad_kind}");
    // Отсутствующий kind — честная ошибка
    let no_kind = run(serde_json::json!({"subject_en": "a thing"}))
        .unwrap_err()
        .to_string();
    assert!(no_kind.contains("kind обязателен"), "{no_kind}");
    // Пустой subject — честная ошибка (не дыра в промпте)
    let empty = run(serde_json::json!({"subject_en": "   ", "kind": "prop"}))
        .unwrap_err()
        .to_string();
    assert!(empty.contains("subject_en пуст"), "{empty}");
}
