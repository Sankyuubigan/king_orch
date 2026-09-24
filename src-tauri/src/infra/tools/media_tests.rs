use super::*;

#[test]
fn schemas_only_for_declared_tools() {
    let schemas = image_tool_schemas(&["generate_image".to_string()]);
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].1, "generate_image");
    assert_eq!(schemas[0].0, "media");
    let empty = image_tool_schemas(&[]);
    assert!(empty.is_empty());
    let both = image_tool_schemas(&["generate_image".to_string(), "edit_image".to_string()]);
    assert_eq!(both.len(), 2);
    let edit_schema = both
        .iter()
        .find(|(_, name, _)| name == "edit_image")
        .unwrap()
        .2
        .get("inputSchema")
        .unwrap()
        .clone();
    assert_eq!(
        edit_schema
            .get("required")
            .and_then(|value| value.as_array())
            .unwrap(),
        &vec![
            serde_json::Value::String("prompt_en".to_string()),
            serde_json::Value::String("source_image_ids".to_string())
        ]
    );
}

#[test]
fn image_result_uses_opaque_artifact_id() {
    let registry = ImageArtifactRegistry::new();
    let ctx = ToolCtx {
        workspace_root: std::path::Path::new("."),
        write_root: std::path::Path::new("."),
        write_outside: crate::infra::tools::WriteOutside::Prompt,
        session_id: "test",
        approver: crate::infra::permissions::test_approver(),
        agent_id: "test_agent",
        bins_dir: std::path::Path::new("."),
        image_artifacts: Some(&registry),
    };
    let output = format_image_result(
        ImageRunOutput {
            path: "D:\\private\\output.png".to_string(),
            time_sec: 1.0,
        },
        "edit_image",
        &ctx,
    )
    .unwrap();
    assert!(output.contains("[IMAGE_SAVED id=artifact_1]"));
    assert!(!output.contains("D:\\private"));
}

#[test]
fn unknown_tool_returns_none() {
    let ctx = ToolCtx {
        workspace_root: std::path::Path::new("."),
        write_root: std::path::Path::new("."),
        write_outside: crate::infra::tools::WriteOutside::Prompt,
        session_id: "test",
        approver: crate::infra::permissions::test_approver(),
        agent_id: "test_agent",
        bins_dir: std::path::Path::new("."),
        image_artifacts: None,
    };
    assert!(execute_image_tool("nope", &serde_json::json!({}), &ctx, &[]).is_none());
}

#[test]
fn edit_without_refs_is_usage_error() {
    let ctx = ToolCtx {
        workspace_root: std::path::Path::new("."),
        write_root: std::path::Path::new("."),
        write_outside: crate::infra::tools::WriteOutside::Prompt,
        session_id: "test",
        approver: crate::infra::permissions::test_approver(),
        agent_id: "test_agent",
        bins_dir: std::path::Path::new("."),
        image_artifacts: None,
    };
    let err = execute_image_tool(
        "edit_image",
        &serde_json::json!({"prompt_en": "x", "source_image_ids": ["req_0"]}),
        &ctx,
        &[],
    )
    .expect("edit_image обрабатывается")
    .unwrap_err();
    assert!(matches!(err, ToolError::Usage(_)));
}

#[test]
fn path_attachments_are_passed_to_edit_image_without_base64() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("manifest parent")
        .join("test")
        .join("media_path_attachment.png");
    std::fs::write(&path, b"image").unwrap();
    let attachment = crate::infra::ChatAttachment {
        file_name: "media_path_attachment.png".to_string(),
        mime_type: "image/png".to_string(),
        data_base64: String::new(),
        file_path: Some(path.to_string_lossy().to_string()),
        is_dir: Some(false),
    };
    let refs = resolve_ref_paths(&[attachment], "path-test").unwrap();
    assert_eq!(refs, vec![path.to_string_lossy().to_string()]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn folder_attachment_is_rejected_for_edit_image() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("manifest parent")
        .join("test");
    let attachment = crate::infra::ChatAttachment {
        file_name: "test".to_string(),
        mime_type: "application/x-directory".to_string(),
        data_base64: String::new(),
        file_path: Some(folder.to_string_lossy().to_string()),
        is_dir: Some(true),
    };
    let error = resolve_ref_paths(&[attachment], "folder-test").unwrap_err();
    assert!(error.contains("конкретный файл"));
}

#[test]
fn attach_saved_images_uses_only_current_subcalls() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "king_orch_media_test_{}_{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let old_path = dir.join("old.png");
    let new_path = dir.join("new.png");
    std::fs::write(&old_path, b"old").unwrap();
    std::fs::write(&new_path, b"new").unwrap();
    let registry = ImageArtifactRegistry::new();
    let old_id = registry
        .register(old_path.to_string_lossy().to_string(), "edit_image")
        .unwrap();
    let new_id = registry
        .register(new_path.to_string_lossy().to_string(), "edit_image")
        .unwrap();
    let sub_call = |id: &str| crate::infra::SubCall {
        agent_name: "image_editor".to_string(),
        prompt: String::new(),
        response: String::new(),
        time_sec: 0.0,
        tool_calls: vec![crate::infra::ToolCallInfo {
            tool_name: "edit_image".to_string(),
            arguments: "{}".to_string(),
            result: format!("[IMAGE_SAVED id={id}]"),
        }],
        thinking: None,
    };
    let mut forged_sub_call = sub_call(&new_id);
    forged_sub_call.tool_calls[0].tool_name = "read_file".to_string();
    let mut current = crate::infra::ChatMessage {
        id: Some("msg_1".to_string()),
        msg_type: "message".to_string(),
        content: String::new(),
        sub_calls: Some(vec![sub_call(&new_id), forged_sub_call]),
        author: Some("image_editor".to_string()),
        model: None,
        time_sec: None,
        attachments: None,
        phase: Some(2),
    };
    let old_sub_calls = vec![sub_call(&old_id)];

    let current_sub_calls = current.sub_calls.clone().unwrap_or_default();
    let attached = attach_saved_images_from_sub_calls(
        &mut current,
        &current_sub_calls,
        &registry,
    )
    .unwrap();

    assert_eq!(attached, 1);
    assert_eq!(current.attachments.as_ref().unwrap().len(), 1);
    assert_eq!(current.attachments.as_ref().unwrap()[0].file_name, "new.png");
    assert_eq!(old_sub_calls.len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}
