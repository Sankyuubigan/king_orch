use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_yaml::{Mapping, Value};

use super::parser::{NodeDef, NodeType, WorkflowDef};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphDiagnostic {
    pub code: String,
    pub location: String,
    pub message: String,
}

pub fn analyze_workflow_fidelity(
    source: &Value,
    workflow: &WorkflowDef,
) -> Result<Vec<GraphDiagnostic>, String> {
    let normalized = serde_yaml::to_value(workflow)
        .map_err(|e| format!("Не удалось проверить полноту YAML workflow: {}", e))?;
    let mut diagnostics = Vec::new();
    collect_value_differences("workflow", source, &normalized, &mut diagnostics);
    collect_duplicate_node_ids(workflow, &mut diagnostics);
    collect_node_target_diagnostics(workflow, &mut diagnostics);
    collect_edge_diagnostics(workflow, &mut diagnostics);
    deduplicate(&mut diagnostics);
    Ok(diagnostics)
}

fn collect_value_differences(
    path: &str,
    source: &Value,
    normalized: &Value,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    match (source, normalized) {
        (Value::Mapping(source), Value::Mapping(normalized)) => {
            collect_mapping_differences(path, source, normalized, diagnostics);
        }
        (Value::Sequence(source), Value::Sequence(normalized)) => {
            if source.len() != normalized.len() {
                push_diagnostic(
                    diagnostics,
                    "YAML_SEQUENCE_CHANGED",
                    path,
                    format!(
                        "число элементов изменится при нормализации: {} → {}",
                        source.len(),
                        normalized.len()
                    ),
                );
                return;
            }
            for (index, (source_item, normalized_item)) in
                source.iter().zip(normalized.iter()).enumerate()
            {
                collect_value_differences(
                    &format!("{}[{}]", path, index),
                    source_item,
                    normalized_item,
                    diagnostics,
                );
            }
        }
        _ if source != normalized => {
            push_diagnostic(
                diagnostics,
                "YAML_VALUE_CHANGED",
                path,
                "значение изменится при нормализации редактором".to_string(),
            );
        }
        _ => {}
    }
}

fn collect_mapping_differences(
    path: &str,
    source: &Mapping,
    normalized: &Mapping,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    for (key, source_value) in source {
        let Some(normalized_value) = normalized.get(key) else {
            push_diagnostic(
                diagnostics,
                "YAML_FIELD_DROPPED",
                &mapping_path(path, key),
                "поле не поддерживается моделью редактора и будет потеряно при сохранении"
                    .to_string(),
            );
            continue;
        };
        collect_value_differences(
            &mapping_path(path, key),
            source_value,
            normalized_value,
            diagnostics,
        );
    }

    for key in normalized.keys() {
        if source.contains_key(key) {
            continue;
        }
        push_diagnostic(
            diagnostics,
            "YAML_FIELD_ADDED",
            &mapping_path(path, key),
            "поле будет добавлено при нормализации редактором".to_string(),
        );
    }
}

fn mapping_path(path: &str, key: &Value) -> String {
    match key {
        Value::String(key) => format!("{}.{}", path, key),
        _ => format!("{}.{:?}", path, key),
    }
}

fn collect_duplicate_node_ids(
    workflow: &WorkflowDef,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    let mut counts = HashMap::new();
    for node in &workflow.nodes {
        *counts.entry(node.id.as_str()).or_insert(0usize) += 1;
    }
    for (id, count) in counts {
        if count > 1 {
            push_diagnostic(
                diagnostics,
                "DUPLICATE_NODE_ID",
                &format!("nodes[id={}]", id),
                format!("ID ноды повторяется {} раз; визуальный редактор оставит только последнюю ноду", count),
            );
        }
    }
}

fn collect_node_target_diagnostics(
    workflow: &WorkflowDef,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    let ids: HashSet<&str> = workflow.nodes.iter().map(|node| node.id.as_str()).collect();
    for node in &workflow.nodes {
        for (index, case) in node.cases_priority.iter().flatten().enumerate() {
            push_missing_target(
                diagnostics,
                &ids,
                &format!(
                    "nodes[id={}].cases_priority[{}]",
                    node.id, index
                ),
                &case.to,
            );
        }
        for (field, target) in [
            ("default", node.default.as_deref()),
            ("sequential_to", node.sequential_to.as_deref()),
            ("true_to", node.true_to.as_deref()),
            ("false_to", node.false_to.as_deref()),
        ] {
            if let Some(target) = target {
                push_missing_target(
                    diagnostics,
                    &ids,
                    &format!("nodes[id={}].{}", node.id, field),
                    target,
                );
            }
        }

        let mut keys = HashSet::new();
        for (index, case) in node.cases_priority.iter().flatten().enumerate() {
            if !keys.insert(case.key.as_str()) {
                push_diagnostic(
                    diagnostics,
                    "DUPLICATE_CASE_KEY",
                    &format!(
                        "nodes[id={}].cases_priority[{}]",
                        node.id, index
                    ),
                    format!("ключ маршрута '{}' повторяется", case.key),
                );
            }
        }
    }
}

fn push_missing_target(
    diagnostics: &mut Vec<GraphDiagnostic>,
    ids: &HashSet<&str>,
    location: &str,
    target: &str,
) {
    if target.is_empty() {
        push_diagnostic(
            diagnostics,
            "NODE_TARGET_MISSING",
            location,
            "цель маршрута пустая и не будет отображена".to_string(),
        );
    } else if !ids.contains(target) {
        push_diagnostic(
            diagnostics,
            "NODE_TARGET_MISSING",
            location,
            format!("цель '{}' не существует среди nodes[].id", target),
        );
    }
}

fn collect_edge_diagnostics(
    workflow: &WorkflowDef,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    let nodes: HashMap<&str, &NodeDef> = workflow
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    for (index, edge) in workflow.edges.iter().enumerate() {
        let location = format!("edges[{}]", index);
        if edge.from.is_empty() || edge.to.is_empty() {
            push_diagnostic(
                diagnostics,
                "EDGE_ENDPOINT_MISSING",
                &location,
                format!(
                    "ребро имеет пустой endpoint: from='{}', to='{}'",
                    edge.from, edge.to
                ),
            );
        } else {
            if !nodes.contains_key(edge.from.as_str()) {
                push_missing_edge_endpoint(diagnostics, &location, "from", &edge.from);
            }
            if !nodes.contains_key(edge.to.as_str()) {
                push_missing_edge_endpoint(diagnostics, &location, "to", &edge.to);
            }
        }

        let Some(from) = nodes.get(edge.from.as_str()) else {
            continue;
        };
        if is_dynamic_node(from) {
            push_diagnostic(
                diagnostics,
                "DYNAMIC_EDGE_NOT_RENDERED",
                &location,
                format!(
                    "ребро '{}' → '{}' из динамической ноды нельзя точно отобразить и сохранить: маршрут хранится в полях ноды",
                    edge.from, edge.to
                ),
            );
        }

        if edge.condition.is_some() || edge.case.is_some() {
            push_diagnostic(
                diagnostics,
                "EDGE_ROUTING_FIELD_NOT_RENDERED",
                &location,
                "поля condition/case ребра не сохраняются визуальным редактором"
                    .to_string(),
            );
        }
    }
}

fn push_missing_edge_endpoint(
    diagnostics: &mut Vec<GraphDiagnostic>,
    location: &str,
    field: &str,
    value: &str,
) {
    push_diagnostic(
        diagnostics,
        "EDGE_ENDPOINT_MISSING",
        &format!("{}.{}", location, field),
        format!("{}='{}' не существует среди nodes[].id", field, value),
    );
}

fn is_dynamic_node(node: &NodeDef) -> bool {
    matches!(
        &node.node_type,
        NodeType::Switch
            | NodeType::LlmSequentialSwitch
            | NodeType::SignalRouter
            | NodeType::ConditionCheck
            | NodeType::ConditionRouter
    )
}

fn push_diagnostic(
    diagnostics: &mut Vec<GraphDiagnostic>,
    code: &str,
    location: &str,
    message: String,
) {
    diagnostics.push(GraphDiagnostic {
        code: code.to_string(),
        location: location.to_string(),
        message,
    });
}

fn deduplicate(diagnostics: &mut Vec<GraphDiagnostic>) {
    let mut seen = HashSet::new();
    diagnostics.retain(|diagnostic| {
        seen.insert((
            diagnostic.code.clone(),
            diagnostic.location.clone(),
            diagnostic.message.clone(),
        ))
    });
}

#[cfg(test)]
#[path = "editor_fidelity_tests.rs"]
mod tests;
