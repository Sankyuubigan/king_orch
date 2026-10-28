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

#[test]
fn accepts_agent_frame_on_llm_worker_without_fused_group() {
    let (source, workflow) = parse(
        r#"
name: test
entry: user_input
groups:
  - id: plain
    title: Место работы рамки
nodes:
  - id: user_input
    type: user_message
  - id: worker
    type: llm_worker
    agent: test
    system_prompt_file: ../system_frame.md
    system_prompt_delivery: user_prefix
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(!diagnostics
        .iter()
        .any(|d| d.code.starts_with("SYSTEM_PROMPT_")));
}

#[test]
fn rejects_agent_frame_delivery_without_file() {
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
    system_prompt_delivery: user_prefix
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|d| {
        d.code == "SYSTEM_PROMPT_DELIVERY_WITHOUT_FILE"
            && d.location == "nodes[id=worker]"
    }));
}

#[test]
fn rejects_agent_frame_unknown_delivery() {
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
    system_prompt_file: ../system_frame.md
    system_prompt_delivery: в_воздухе
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|d| {
        d.code == "SYSTEM_PROMPT_DELIVERY_UNKNOWN"
            && d.location == "nodes[id=worker]"
            && d.message.contains("неизвестное значение")
    }));
}

#[test]
fn rejects_agent_frame_on_non_worker_node() {
    let (source, workflow) = parse(
        r#"
name: test
entry: user_input
nodes:
  - id: user_input
    type: user_message
  - id: memo
    type: note
    input: заметка
    system_prompt_file: ../system_frame.md
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|d| {
        d.code == "SYSTEM_PROMPT_FILE_UNSUPPORTED_NODE" && d.location == "nodes[id=memo]"
    }));
}

#[test]
fn rejects_agent_frame_inside_explicit_fused_group() {
    let (source, workflow) = parse(
        r#"
name: test
entry: user_input
groups:
  - id: fused_group
    title: Склеенная группа
    fused_instruction: одним вызовом
nodes:
  - id: user_input
    type: user_message
  - id: worker
    type: llm_worker
    group: fused_group
    agent: test
    system_prompt_file: ../system_frame.md
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|d| {
        d.code == "SYSTEM_PROMPT_FILE_IN_GROUP" && d.location == "nodes[id=worker]"
    }));
}

// --- Графовый дефолт фрейма (`config.default_system_prompt_*`) ---

#[test]
fn accepts_agent_frame_from_graph_default() {
    // Гейт G1 (tasks/21.10.26): ноды наследуют дефолт — диагностик быть не должно.
    let (source, workflow) = parse(
        r#"
name: test
entry: user_input
config:
  default_system_prompt_file: ../system_frame.md
  default_system_prompt_delivery: user_prefix
nodes:
  - id: user_input
    type: user_message
  - id: worker
    type: llm_worker
    agent: test
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(!diagnostics
        .iter()
        .any(|d| d.code.starts_with("SYSTEM_PROMPT_")));
}

#[test]
fn accepts_node_delivery_merging_with_default_file() {
    // Merge по полю: нода меняет только слот, файл наследуется из дефолта —
    // легальная комбинация, без диагностики DELIVERY_WITHOUT_FILE.
    let (source, workflow) = parse(
        r#"
name: test
entry: user_input
config:
  default_system_prompt_file: ../system_frame.md
nodes:
  - id: user_input
    type: user_message
  - id: worker
    type: llm_worker
    agent: test
    system_prompt_delivery: system
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(!diagnostics
        .iter()
        .any(|d| d.code.starts_with("SYSTEM_PROMPT_")));
}

#[test]
fn rejects_config_default_delivery_unknown() {
    let (source, workflow) = parse(
        r#"
name: test
entry: user_input
config:
  default_system_prompt_file: ../system_frame.md
  default_system_prompt_delivery: в_воздухе
nodes:
  - id: user_input
    type: user_message
  - id: worker
    type: llm_worker
    agent: test
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|d| {
        d.code == "SYSTEM_PROMPT_DELIVERY_UNKNOWN"
            && d.location == "config"
            && d.message.contains("неизвестное значение")
    }));
}

#[test]
fn rejects_config_default_delivery_without_file() {
    // Парная семантика дефолта: слот без файла в config — диагностика location config.
    let (source, workflow) = parse(
        r#"
name: test
entry: user_input
config:
  default_system_prompt_delivery: user_prefix
nodes:
  - id: user_input
    type: user_message
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|d| {
        d.code == "SYSTEM_PROMPT_DELIVERY_WITHOUT_FILE" && d.location == "config"
    }));
}

#[test]
fn custom_tool_without_file_is_diagnosed() {
    let (source, workflow) = parse(
        r#"
name: test
nodes:
  - id: tool
    type: custom_tool_call
  - id: target
    type: note
edges:
  - from: tool
    to: target
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics
        .iter()
        .any(|d| d.code == "CUSTOM_TOOL_FILE_MISSING" && d.location == "nodes[id=tool]"));
}

#[test]
fn custom_tool_output_message_requires_attachments_dir() {
    let (source, workflow) = parse(
        r#"
name: test
nodes:
  - id: tool
    type: custom_tool_call
    tool_file: ../tools/x.ts
    output_message: true
  - id: target
    type: note
edges:
  - from: tool
    to: target
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics
        .iter()
        .any(|d| d.code == "CUSTOM_TOOL_ATTACHMENTS_DIR_MISSING" && d.location == "nodes[id=tool]"));
}

#[test]
fn custom_tool_in_group_is_diagnosed() {
    let (source, workflow) = parse(
        r#"
name: test
nodes:
  - id: tool
    type: custom_tool_call
    tool_file: ../tools/x.ts
    group: g
  - id: target
    type: note
edges:
  - from: tool
    to: target
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics
        .iter()
        .any(|d| d.code == "CUSTOM_TOOL_IN_GROUP" && d.location == "nodes[id=tool]"));
}

#[test]
fn valid_custom_tool_node_has_no_custom_diagnostics() {
    let (source, workflow) = parse(
        r#"
name: test
entry: user_input
nodes:
  - id: user_input
    type: user_message
  - id: tool
    type: custom_tool_call
    tool_file: ../tools/x.ts
    attachments_dir: ../database/cards
    output_message: true
  - id: target
    type: note
edges:
  - from: user_input
    to: tool
  - from: tool
    to: target
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(!diagnostics
        .iter()
        .any(|d| d.code.starts_with("CUSTOM_TOOL")), "лишние диагностики: {:?}", diagnostics);
}

// --- Сентинел END удалён из формата (конец ветки = отсутствие исходящих рёбер) ---

#[test]
fn edge_to_end_sentinel_is_deprecated_not_missing() {
    let (source, workflow) = parse(
        r#"
name: test
nodes:
  - id: worker
    type: llm_worker
    agent: test
edges:
  - from: worker
    to: END
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    let deprecated = diagnostics
        .iter()
        .find(|d| d.code == "END_SENTINEL_DEPRECATED")
        .expect("нет END_SENTINEL_DEPRECATED");
    assert_eq!(deprecated.location, "edges[0].to");
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code == "EDGE_ENDPOINT_MISSING"),
        "END-цель не должна выглядеть повреждением: {:?}",
        diagnostics
    );
}

#[test]
fn cases_priority_to_end_sentinel_is_deprecated() {
    let (source, workflow) = parse(
        r#"
name: test
nodes:
  - id: facts
    type: note
  - id: router
    type: switch
    input_object: "{{ nodes.facts.output }}"
    cases_priority:
      - key: ready
        to: END
  - id: worker
    type: llm_worker
    agent: test
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|d| {
        d.code == "END_SENTINEL_DEPRECATED"
            && d.location == "nodes[id=router].cases_priority[0]"
    }));
    assert!(
        !diagnostics.iter().any(|d| d.code == "NODE_TARGET_MISSING"),
        "END-цель не должна выглядеть отсутствующей: {:?}",
        diagnostics
    );
}

#[test]
fn edge_from_end_sentinel_stays_missing_endpoint() {
    let (source, workflow) = parse(
        r#"
name: test
nodes:
  - id: worker
    type: llm_worker
    agent: test
edges:
  - from: END
    to: worker
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|d| {
        d.code == "EDGE_ENDPOINT_MISSING" && d.location == "edges[0].from"
    }));
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code == "END_SENTINEL_DEPRECATED"),
        "`from: END` — настоящее повреждение, а не легаси-цель: {:?}",
        diagnostics
    );
}

#[test]
fn detects_unreachable_node_from_entry() {
    let (source, workflow) = parse(
        r#"
name: test
entry: start
nodes:
  - id: start
    type: user_message
  - id: orphan
    type: note
edges: []
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(diagnostics.iter().any(|d| {
        d.code == "NODE_UNREACHABLE_FROM_ENTRY" && d.location == "nodes[id=orphan]"
    }));
}

#[test]
fn node_reachable_through_routing_fields_is_not_flagged() {
    let (source, workflow) = parse(
        r#"
name: test
entry: start
nodes:
  - id: start
    type: user_message
  - id: gate
    type: condition_check
    field: ok
    true_to: via_true
    false_to: via_cases
  - id: via_true
    type: note
  - id: via_cases
    type: note
  - id: orphan
    type: note
edges:
  - from: start
    to: gate
"#,
    );

    // via_true/via_cases достижимы ТОЛЬКО через поля маршрутизации
    // condition_check — достижимостия обязана их видеть; orphan не достижим никем.
    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");
    let unreachable: Vec<&str> = diagnostics
        .iter()
        .filter(|d| d.code == "NODE_UNREACHABLE_FROM_ENTRY")
        .filter_map(|d| d.location.strip_prefix("nodes[id=").and_then(|s| s.strip_suffix("]")))
        .collect();
    assert_eq!(unreachable, vec!["orphan"]);
}

#[test]
fn group_members_are_reachable_together() {
    let (source, workflow) = parse(
        r#"
name: test
entry: start
nodes:
  - id: start
    type: user_message
  - id: member_a
    type: llm_worker
    agent: test
    group: fused_block
  - id: member_b
    type: llm_worker
    agent: test
    group: fused_block
groups:
  - id: fused_block
edges:
  - from: start
    to: member_a
"#,
    );

    let diagnostics = analyze_workflow_fidelity(&source, &workflow).expect("analysis");

    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code == "NODE_UNREACHABLE_FROM_ENTRY"),
        "достигнув одного члена группы, движок исполняет всю группу: {:?}",
        diagnostics
    );
}
