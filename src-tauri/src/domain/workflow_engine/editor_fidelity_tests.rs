use super::*;
use serde_yaml::Value;

fn parse(yaml: &str) -> (Value, WorkflowDef) {
    let source: Value = serde_yaml::from_str(yaml).expect("source YAML");
    let workflow = serde_yaml::from_value(source.clone()).expect("workflow");
    (source, workflow)
}

#[test]
fn detects_dynamic_source_edge() {
    let (source, workflow) = parse(
        r#"
name: test
visible: true
nodes:
  - id: router
    type: signal_router
    signal_name: source
    cases_priority:
      - key: first
        to: target
  - id: target
    type: note
edges:
  - from: router
    to: target
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "DYNAMIC_EDGE_NOT_RENDERED"
            && diagnostic.location == "edges[0]"
    }));
}

#[test]
fn detects_edge_metadata_and_missing_endpoint() {
    let (source, workflow) = parse(
        r#"
name: test
visible: true
nodes:
  - id: worker
    type: llm_worker
    agent: test
edges:
  - from: worker
    to: missing
    condition: ready
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "EDGE_ENDPOINT_MISSING"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "EDGE_ROUTING_FIELD_NOT_RENDERED"));
}

#[test]
fn detects_fields_dropped_by_typed_parser() {
    let (source, workflow) = parse(
        r#"
name: test
visible: true
custom_top_level: value
nodes:
  - id: worker
    type: llm_worker
    agent: test
    custom_node_field: value
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.location == "workflow.custom_top_level"));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.location == "workflow.nodes[0].custom_node_field"
    }));
}

#[test]
fn detects_current_psychotherapist_dynamic_edge() {
    let source: Value = serde_yaml::from_str(include_str!(
        "../../../../agents/psychotherapist/transitions/main_conversation_flow.yaml"
    ))
    .expect("psychotherapist workflow YAML");
    let workflow = serde_yaml::from_value(source.clone()).expect("psychotherapist workflow");
    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "DYNAMIC_EDGE_NOT_RENDERED"
            && diagnostic.message.contains("check_need_handedness")
            && diagnostic.message.contains("call_focus_keeper")
    }));
}

#[test]
fn accepts_canonical_workflow() {
    let (source, workflow) = parse(
        r#"
name: test
visible: true
config: null
nodes:
  - id: worker
    type: llm_worker
    agent: test
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.is_empty(), "{:?}", diagnostics);
}
