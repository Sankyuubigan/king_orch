//! Диагностика «мёртвого signal-контракта»: агент с контрактом вызывается
//! графом, но ни один узел/шаблон его сигнальный ключ не читает.
//!
//! Контекст отказа: агент с контрактом работает по Method 3 — ответ модели
//! оформляется конвертом `emit_signal` и попадает в signal bus, но НЕ в чат.
//! Если ключ никто не потребляет, работа выбрасывается: сигнал мёртв,
//! а при `output_type: message` пользователь получает пустое сообщение
//! (исторический кейс — synthesizer 17.10.26).
//!
//! Проверяется только агенты, которых ВЫЗЫВАЕТ данный граф (`llm_worker.agent`),
//! иначе контракты агентов из чужих сценариев ловили бы ложные срабатывания.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::editor_fidelity::GraphDiagnostic;
use super::parser::{ConditionNode, NodeType, WorkflowDef};

/// Диагностика «мёртвый контракт» для графа.
///
/// Для каждого узла `llm_worker`, чей агент объявлен в
/// `signals/root.schema.json`: если его сигнальный ключ не встречается
/// ни в `signal_router.signal_name`, ни в field-путях condition-узлов,
/// ни в шаблонах `{{ signals.<key> }}` — выдаём `SIGNAL_CONTRACT_UNCONSUMED`.
pub fn collect_dead_signal_diagnostics(workflow: &WorkflowDef) -> Vec<GraphDiagnostic> {
    let contracts = signal_contracts(signals_dir_of(workflow).as_deref());
    if contracts.is_empty() {
        return Vec::new();
    }

    let used = consumed_signal_keys(workflow);
    let mut diagnostics = Vec::new();

    for node in &workflow.nodes {
        if node.node_type != NodeType::LlmWorker {
            continue;
        }
        let Some(agent_id) = node.agent.as_deref().map(str::trim).filter(|a| !a.is_empty())
        else {
            continue;
        };
        let Some(key) = contracts.get(agent_id) else {
            continue;
        };
        if used.contains(key.as_str()) {
            continue;
        }

        diagnostics.push(GraphDiagnostic {
            code: "SIGNAL_CONTRACT_UNCONSUMED".to_string(),
            location: format!("nodes[id={}].agent", node.id),
            message: format!(
                "агент '{}' объявляет сигнал \"{}\" (signals/root.schema.json), но граф его не \
                 потребляет: ни signal_router (signal_name), ни condition field-путь \"{}...\", \
                 ни шаблон {{{{ signals.{} }}}}. Такой агент работает по Method 3 — ответ уходит \
                 в сигнал и не попадает в чат: при output_type: message пользователь увидит \
                 ПУСТОЕ сообщение",
                agent_id, key, key, key
            ),
        });
    }

    diagnostics
}

/// Где лежит `signals/root.schema.json` для графа: на уровень выше папки
/// самого графа (`transitions/`) — рядом с агентскими под-папками.
/// Пустой `parent_dir` (юнит-фикстуры) или корень диска — `None`: граф без
/// окружения сигналов обязан проходить без диагностики, а не падать.
fn signals_dir_of(workflow: &WorkflowDef) -> Option<PathBuf> {
    if workflow.parent_dir.trim().is_empty() {
        return None;
    }
    let path = Path::new(workflow.parent_dir.trim());
    path.parent()
        .map(|base| base.join("signals"))
        .filter(|dir| dir != path)
}

/// agent_id → сигнальный key из `signals/root.schema.json`.
/// Файла нет / битый / секции `agents` нет → пустая карта (не отказ).
fn signal_contracts(signals_dir: Option<&Path>) -> HashMap<String, String> {
    let Some(dir) = signals_dir else {
        return HashMap::new();
    };
    let Ok(text) = std::fs::read_to_string(dir.join("root.schema.json")) else {
        return HashMap::new();
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) else {
        return HashMap::new();
    };
    root.get("agents")
        .and_then(|agents| agents.as_object())
        .map(|agents| {
            agents
                .iter()
                .filter_map(|(id, def)| {
                    let key = def.get("key")?.as_str().unwrap_or_default().trim();
                    if key.is_empty() {
                        None
                    } else {
                        Some((id.clone(), key.to_string()))
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Имена сигналов, которые граф реально читает:
/// - `signal_name` у `signal_router` (точное имя);
/// - первый сегмент field-путей condition-узлов (`validator_report.e3` →
///   `validator_report`; `has_problem` → `has_problem` — факт, но пусть
///   просто не совпадёт с чьим-то ключом контракта);
/// - `{{ signals.<key> ... }}` в `input` / `input_object` / `task`
///   и в строковых значениях `args` (`custom_tool_call`).
fn consumed_signal_keys(workflow: &WorkflowDef) -> HashSet<String> {
    let mut used = HashSet::new();

    for node in &workflow.nodes {
        if node.node_type == NodeType::SignalRouter {
            if let Some(name) = node.signal_name.as_deref().map(str::trim) {
                if !name.is_empty() {
                    used.insert(name.to_string());
                }
            }
        }

        for field in rule_fields(node) {
            used.insert(field.split('.').next().unwrap_or_default().trim().to_string());
        }

        for template in [node.input.as_deref(), node.input_object.as_deref(), node.task.as_deref()]
            .into_iter()
            .flatten()
        {
            for key in signals_from_template(template) {
                used.insert(key);
            }
        }

        // args custom_tool_call: значения обязаны быть строками (проверяет
        // custom_tool::build_args_json), шаблоны в них резолвит движок.
        if let Some(args) = &node.args {
            for value in args.values() {
                if let Some(text) = value.as_str() {
                    for key in signals_from_template(text) {
                        used.insert(key);
                    }
                }
            }
        }
    }

    used
}

/// field-пути условия: собственные поля узла + рекурсия по группам условий.
fn rule_fields(node: &super::parser::NodeDef) -> Vec<String> {
    let mut fields = Vec::new();
    for field in [node.field.as_deref(), node.switch_field.as_deref()]
        .into_iter()
        .flatten()
    {
        let trimmed = field.trim();
        if !trimmed.is_empty() {
            fields.push(trimmed.to_string());
        }
    }
    collect_rule_fields(&node.conditions, &mut fields);
    fields
}

fn collect_rule_fields(conditions: &[ConditionNode], out: &mut Vec<String>) {
    for cond in conditions {
        match cond {
            ConditionNode::Rule { field, .. } => out.push(field.clone()),
            ConditionNode::Group { conditions, .. } => collect_rule_fields(conditions, out),
        }
    }
}

/// Идентификаторы сигналов из шаблона: `{{ signals.<ключ> ... }}`.
fn signals_from_template(template: &str) -> Vec<String> {
    const PREFIX: &str = "{{ signals.";
    let mut found = Vec::new();
    let mut rest = template;
    while let Some(pos) = rest.find(PREFIX) {
        rest = &rest[pos + PREFIX.len()..];
        let key: String = rest
            .chars()
            .take_while(|c| !matches!(c, '.' | '}' | ' ' | '\n' | '\t'))
            .collect();
        if !key.is_empty() {
            found.push(key);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use super::super::editor_fidelity::analyze_workflow_fidelity;
    use super::super::parser;

    /// Структура временного окружения сигналов: `<tmp>/agents/work/tests`
    /// как папка графа, `<tmp>/agents/signals` — с root.schema.json.
    fn temp_signals_workspace(tag: &str, schema_json: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ko_signal_check_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let signals = root.join("signals");
        let graph_dir = root.join("work");
        fs::create_dir_all(&signals).unwrap();
        fs::create_dir_all(&graph_dir).unwrap();
        fs::write(signals.join("root.schema.json"), schema_json).unwrap();
        graph_dir
    }

    /// Граф: worker вызывает контрактного агента; потребитель — опционально.
    fn graph_yaml(with_consumer: bool) -> String {
        format!(
            r#"
name: test
nodes:
  - id: call_test_signal_agent
    type: llm_worker
    agent: test_signal_agent
    output_type: message
  {consumer}
edges: []
"#,
            consumer = if with_consumer {
                r#"- id: router
    type: signal_router
    signal_name: test_signal_key
    cases_priority:
      - key: V1
        to: call_test_signal_agent
"#
            } else {
                ""
            }
        )
    }

    fn parse_workflow(yaml: &str, parent_dir: &Path) -> WorkflowDef {
        let mut wf: WorkflowDef =
            serde_yaml::from_str(yaml).expect("фикстурный YAML обязан парситься");
        wf.parent_dir = parent_dir.to_string_lossy().to_string();
        wf
    }

    const SCHEMA: &str = r#"{
        "agents": {
            "test_signal_agent": {
                "key": "test_signal_key",
                "value": { "type": "string", "enum": ["V1", "V2"] }
            }
        }
    }"#;

    #[test]
    fn dead_contract_is_reported_when_nothing_consumes_the_key() {
        let graph_dir = temp_signals_workspace("dead", SCHEMA);
        let wf = parse_workflow(&graph_yaml(false), &graph_dir);

        let diagnostics = collect_dead_signal_diagnostics(&wf);

        assert_eq!(diagnostics.len(), 1, "единственный мёртвый контракт");
        assert_eq!(diagnostics[0].code, "SIGNAL_CONTRACT_UNCONSUMED");
        assert_eq!(diagnostics[0].location, "nodes[id=call_test_signal_agent].agent");
        assert!(
            diagnostics[0].message.contains("test_signal_key"),
            "сообщение должно называть ключ сигнала"
        );
        let _ = fs::remove_dir_all(graph_dir.parent().unwrap());
    }

    #[test]
    fn contract_with_signal_router_consumer_is_alive() {
        let graph_dir = temp_signals_workspace("alive", SCHEMA);
        let wf = parse_workflow(&graph_yaml(true), &graph_dir);

        assert!(
            collect_dead_signal_diagnostics(&wf).is_empty(),
            "signal_router с этим signal_name обязан закрывать контракт"
        );
        let _ = fs::remove_dir_all(graph_dir.parent().unwrap());
    }

    /// Потребителем считается и field-путь condition-узла, и шаблон
    /// `{{ signals.<key> }}` — как только ключ встречен, контракт жив.
    #[test]
    fn contract_survives_via_condition_path_or_template() {
        let graph_dir = temp_signals_workspace("consumers", SCHEMA);
        let base = r#"
name: test
nodes:
  - id: worker
    type: llm_worker
    agent: test_signal_agent
  "#;

        let via_condition = parse_workflow(
            &format!(
                "{base}- id: check_route
    type: condition_router
    conditions:
      - field: test_signal_key.sub
        equals: x
    true_to: worker
    false_to: worker
edges: []
"
            ),
            &graph_dir,
        );
        assert!(
            collect_dead_signal_diagnostics(&via_condition).is_empty(),
            "field-путь test_signal_key.sub обязан потреблять ключ"
        );

        let via_template = parse_workflow(
            &format!(
                "{base}- id: dst
    type: note
  - id: reader
    type: note
    input: '{{{{ signals.test_signal_key }}}}'
edges: []
"
            ),
            &graph_dir,
        );
        assert!(
            collect_dead_signal_diagnostics(&via_template).is_empty(),
            "шаблон {{{{ signals.<key> }}}} обязан потреблять ключ"
        );

        // Потребление через args custom_tool_call: сигнал подаётся на вход тула.
        let via_tool_args = parse_workflow(
            &format!(
                "{base}- id: read_section
    type: custom_tool_call
    tool_file: ../tools/read.ts
    args:
      file: some/db.md
      heading: '{{{{ signals.test_signal_key }}}}'
edges: []
"
            ),
            &graph_dir,
        );
        assert!(
            collect_dead_signal_diagnostics(&via_tool_args).is_empty(),
            "шаблон {{{{ signals.<key> }}}} в args custom_tool_call обязан потреблять ключ"
        );

        let _ = fs::remove_dir_all(graph_dir.parent().unwrap());
    }

    /// Пропущенный/битый root.schema.json — НЕ отказ: граф без окружения
    /// сигналов обязан проходить без диагностики и без паники.
    #[test]
    fn missing_or_broken_schema_is_diagnostics_free() {
        let root = std::env::temp_dir().join(format!("ko_signal_check_broken_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let signals = root.join("signals");
        let graph_dir = root.join("work");
        fs::create_dir_all(&signals).unwrap();
        fs::create_dir_all(&graph_dir).unwrap();
        fs::write(signals.join("root.schema.json"), "не json вовсе").unwrap();

        let wf = parse_workflow(&graph_yaml(false), &graph_dir);
        assert!(collect_dead_signal_diagnostics(&wf).is_empty());

        // Нет файла вовсе.
        fs::remove_file(signals.join("root.schema.json")).unwrap();
        assert!(collect_dead_signal_diagnostics(&wf).is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    /// Боевой YAML после шага 1 не должен ловить ложных срабатываний:
    /// валидатор потребляется field-путями `validator_report.*`,
    /// soma_translator — signal_router `check_need_handedness`.
    #[test]
    fn production_graph_has_no_dead_contracts() {
        let signals_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../agents/psychotherapist/transitions");
        if !signals_root.exists() {
            panic!("боевая папка transitions не найдена: {}", signals_root.display());
        }

        let yaml_path = signals_root.join("main_conversation_flow.yaml");
        let source = fs::read_to_string(&yaml_path).expect("боевой YAML прочитать");
        let wf = parse_workflow(&source, &yaml_path.parent().unwrap());

        let (source_value, workflow) = {
            let v: serde_yaml::Value = serde_yaml::from_str(&source).expect("YAML");
            (v, wf)
        };
        assert!(
            collect_dead_signal_diagnostics(&workflow).is_empty(),
            "боевой граф: мёртвых контрактов быть не должно"
        );

        // Интеграция с поверхностью редактора: полный анализ тоже чист.
        let full = analyze_workflow_fidelity(&source_value, &workflow).expect("fidelity");
        assert!(
            full.iter()
                .all(|d| d.code != "SIGNAL_CONTRACT_UNCONSUMED"),
            "editor_fidelity не должен дополнять список мёртвых контрактов на боевом графе"
        );
    }

    /// Регрессия удаления шага 1: у контракта synthesizer больше нет записи —
    /// вызов синтетика в боевом графе не должен рождать диагностику.
    #[test]
    fn synthesizer_is_not_treated_as_dead_contract_after_step_1() {
        let signals_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../agents/psychotherapist/signals");
        let schema = fs::read_to_string(signals_dir.join("root.schema.json"))
            .expect("боевой root.schema.json прочитать");
        let root: serde_json::Value =
            serde_json::from_str(&schema).expect("root.schema.json валиден");
        assert!(
            root.get("agents")
                .and_then(|a| a.get("synthesizer"))
                .is_none(),
            "synthesizer обязан быть удалён из контрактов (шаг 1)"
        );
    }
}
