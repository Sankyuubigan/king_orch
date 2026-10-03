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
fn detects_missing_condition_router_sequential_target() {
    let (source, workflow) = parse(
        r#"
name: test
nodes:
  - id: router
    type: condition_router
    conditions:
      - field: has_problem
        equals: true
    true_to: yes
    false_to: no
    sequential_to: missing
  - id: yes
    type: note
  - id: no
    type: note
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "NODE_TARGET_MISSING"
            && diagnostic.location == "nodes[id=router].sequential_to"
    }));
}

#[test]
fn detects_dynamic_source_edge_warning() {
    let (source, workflow) = parse(
        r#"
name: test
nodes:
  - id: router
    type: signal_router
    cases_priority:
      - key: A
        to: node_a
  - id: node_a
    type: note
edges:
  - from: router
    to: node_a
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "DYNAMIC_EDGE_NOT_RENDERED"
            && diagnostic.message.contains("router")
            && diagnostic.message.contains("node_a")
    }));
}

#[test]
fn accepts_canonical_workflow() {
    let (source, workflow) = parse(
        r#"
name: test
config: null
entry: user_input
nodes:
  - id: user_input
    type: user_message
  - id: worker
    type: llm_worker
    agent: test
edges:
  - from: user_input
    to: worker
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.is_empty(), "{:?}", diagnostics);
}

#[test]
fn detects_missing_entry() {
    let (source, workflow) = parse(
        r#"
name: test
nodes:
  - id: user_input
    type: user_message
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics
        .iter()
        .any(|d| d.code == "ENTRY_MISSING" && d.location == "entry"));
}

#[test]
fn detects_dangling_entry() {
    let (source, workflow) = parse(
        r#"
name: test
entry: nowhere
nodes:
  - id: user_input
    type: user_message
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics
        .iter()
        .any(|d| d.code == "ENTRY_TARGET_MISSING"));
}

#[test]
fn detects_entry_of_wrong_type_and_incoming_edge() {
    let (source, workflow) = parse(
        r#"
name: test
entry: worker
nodes:
  - id: user_input
    type: user_message
  - id: worker
    type: llm_worker
    agent: test
edges:
  - from: user_input
    to: worker
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics
        .iter()
        .any(|d| d.code == "ENTRY_TYPE_UNEXPECTED"));
    assert!(diagnostics
        .iter()
        .any(|d| d.code == "ENTRY_HAS_INCOMING_EDGE"));
    assert!(diagnostics
        .iter()
        .any(|d| d.code == "USER_MESSAGE_NOT_ENTRY" && d.location == "nodes[id=user_input]"));
}

#[test]
fn detects_template_reference_to_missing_node() {
    let (source, workflow) = parse(
        r#"
name: test
entry: user_input
nodes:
  - id: user_input
    type: user_message
  - id: worker
    type: llm_worker
    agent: test
    task: "Смотри отчёт: {{ nodes.extractor.output.result }}"
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|d| {
        d.code == "TEMPLATE_NODE_REF_MISSING"
            && d.location == "nodes[id=worker].task"
            && d.message.contains("extractor")
    }));
}

#[test]
fn accepts_template_reference_to_existing_node() {
    let (source, workflow) = parse(
        r#"
name: test
entry: user_input
nodes:
  - id: user_input
    type: user_message
  - id: extractor
    type: llm_fact_extractor
    input: "{{ nodes.user_input.output.text }}"
  - id: worker
    type: llm_worker
    agent: test
    task: "Вопрос: {{ nodes.user_input.output.text }} Отчёт: {{ nodes.extractor.output }}"
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(!diagnostics
        .iter()
        .any(|d| d.code == "TEMPLATE_NODE_REF_MISSING"));
}
