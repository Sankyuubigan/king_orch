use crate::domain::workflow_engine::context::WorkflowContext;
use crate::domain::workflow_engine::parser::{EdgeDef, NodeDef, NodeType, WorkflowConfig, WorkflowDef};
use crate::domain::workflow_engine::WorkflowRunner;
use crate::infra::{ChatMessage, SubCall};

/// Результат выполнения узла
#[derive(Debug, Clone)]
pub struct NodeResult {
    pub output: serde_json::Value,
    pub next_node: Option<String>,
    /// Дополнительные следующие узлы (для SequentialSwitch)
    pub next_nodes: Vec<String>,
}

/// Выполняет один узел графа и возвращает результат + id следующего узла
pub fn execute_node<L, S, C>(
    node: &NodeDef,
    workflow: &WorkflowDef,
    context: &mut WorkflowContext,
    runner: &mut WorkflowRunner<L, S, C>,
) -> Result<NodeResult, String>
where
    L: Fn(String) + Clone + Send + Sync + 'static,
    S: Fn(String, u8) + Clone + Send + Sync + 'static,
    C: Fn(&SubCall) + Clone + Send + Sync + 'static,
{
    match node.node_type {
        NodeType::LlmFactExtractor => {
            let config = workflow
                .config
                .as_ref()
                .cloned()
                .unwrap_or(WorkflowConfig::default());
            let input = context.resolve_template(node.input.as_deref().unwrap_or("{{ user_message }}"));
            let signals = context.resolve_template("{{ signals }}");
            let workflow_dir = std::path::Path::new(&workflow.parent_dir);
            let prompt =
                super::fact_extractor::build_extractor_prompt(&config, &input, &signals, Some(workflow_dir));

            let llm_response = runner.call_llm_direct(&prompt, &input)?;

            let parsed: serde_json::Value =
                serde_json::from_str(&llm_response).unwrap_or_else(|_| {
                    extract_json(&llm_response)
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_else(|| serde_json::json!({}))
                });

            (runner.log_cb)(format!(
                "[fact_extractor] Сырой ответ: {}",
                llm_response.chars().take(500).collect::<String>()
            ));

            Ok(NodeResult {
                output: parsed,
                next_node: None,
            next_nodes: vec![],
            })
        }

        NodeType::LlmWorker => {
            let agent_id = node
                .agent
                .as_deref()
                .ok_or_else(|| "llm_worker: не указан agent".to_string())?;
            let agent = runner
                .agents
                .iter()
                .find(|a| a.id == agent_id)
                .ok_or_else(|| format!("llm_worker: агент '{}' не найден", agent_id))?;

            let mut task = context.resolve_template(node.task.as_deref().unwrap_or(""));

            // ── inject_reports: Push отчётов коллег ──
            if let Some(ref reports_to_inject) = node.inject_reports {
                if !reports_to_inject.is_empty() {
                    let mut injected = String::from("\n\n### [ОТЧЕТЫ КОЛЛЕГ ДЛЯ АНАЛИЗА]\n");
                    for aid in reports_to_inject {
                        let report = context.messages.iter().rev()
                            .find(|m| m.msg_type == "thought" && m.author.as_deref() == Some(aid.as_str()))
                            .map(|m| m.content.clone())
                            .unwrap_or_else(|| "[Отчет не найден]".to_string());
                        injected.push_str(&format!("--- Отчет от {} ---\n{}\n\n", aid, report));
                    }
                    task.push_str(&injected);
                }
            }

            (runner.log_cb)(format!(
                "[worker] Вызов '{}': {}",
                agent_id,
                task.chars().take(200).collect::<String>()
            ));

            let start_len = runner.all_sub_calls.len();
            let result = runner.call_agent(agent, &task, &mut context.messages)?;
            let end_len = runner.all_sub_calls.len();

            let node_sub_calls = if start_len < end_len {
                Some(runner.all_sub_calls[start_len..end_len].to_vec())
            } else {
                None
            };

            // Тип сохранения: message (в чат юзеру) или thought (внутренний отчёт)
            let msg_type = node.output_type.as_deref().unwrap_or("thought");
            let msg = ChatMessage {
                id: Some(format!("msg_{}", runner.msg_counter)),
                msg_type: msg_type.to_string(),
                content: result.clone(),
            sub_calls: node_sub_calls,
            author: Some(agent_id.to_string()),
        };
        context.messages.push(msg);
        *runner.msg_counter += 1;
        context.output_emitted = node.output_type.as_deref() == Some("message");

        Ok(NodeResult {
            output: serde_json::json!({"result": result, "agent": agent_id}),
                next_node: None,
            next_nodes: vec![],
            })
        }

        NodeType::SystemCondition => {
            let action = node.action.as_deref().unwrap_or("check");

            match action {
                "get_missing_reports" => {
                    let required = node.required.as_ref().ok_or_else(|| {
                        "get_missing_reports: нет required".to_string()
                    })?;
                    let missing: Vec<String> = required
                        .iter()
                        .filter(|agent_id| {
                            !context.messages.iter().any(|m| {
                                m.author.as_deref() == Some(agent_id.as_str())
                            })
                        })
                        .cloned()
                        .collect();
                    let all_present = missing.is_empty();
                    Ok(NodeResult {
                        output: serde_json::json!({
                            "status": if all_present { "all_done" } else { "missing" },
                            "missing": missing
                        }),
                        next_node: None,
                        next_nodes: vec![],
                    })
                }

                "has_reports" => {
                    let required = node
                        .required
                        .as_ref()
                        .ok_or_else(|| "has_reports: нет required".to_string())?;
                    let all_present = required.iter().all(|agent_id| {
                        context.messages.iter().any(|m| {
                            m.author.as_deref() == Some(agent_id.as_str())
                        })
                    });
                    Ok(NodeResult {
                        output: serde_json::json!({
                            "status": if all_present { "present" } else { "missing" }
                        }),
                        next_node: None,
                        next_nodes: vec![],
                    })
                }

                "all_problems_analyzed" => {
                    let has_report = context.messages.iter().any(|m| {
                        m.author.as_deref() == Some("pattern_finder_by_double_bind")
                    });
                    Ok(NodeResult {
                        output: serde_json::json!({
                            "status": if has_report { "all_done" } else { "has_unanalyzed" },
                        }),
                        next_node: None,
                        next_nodes: vec![],
                    })
                }

                "aggregate_reports" => {
                    let required = node.required.as_ref().ok_or_else(|| {
                        "aggregate_reports: нет required".to_string()
                    })?;
                    let mut reports = String::new();
                    for agent_id in required {
                        if let Some(msg) = context.messages.iter().rev()
                            .find(|m| m.author.as_deref() == Some(agent_id.as_str()))
                        {
                            reports.push_str(&format!(
                                "--- {} ---\n{}\n\n",
                                agent_id,
                                &msg.content
                            ));
                        } else {
                            reports.push_str(&format!(
                                "--- {} ---\n[отчёт не найден]\n\n",
                                agent_id
                            ));
                        }
                    }
                    Ok(NodeResult {
                        output: serde_json::json!({"reports": reports}),
                        next_node: None,
                        next_nodes: vec![],
                    })
                }

                "check_protocol_state" => {
                    let required = node.required.as_ref().ok_or_else(|| {
                        "check_protocol_state: нет required".to_string()
                    })?;
                    let all_present = required.iter().all(|agent_id| {
                        context.messages.iter().any(|m| {
                            m.author.as_deref() == Some(agent_id.as_str())
                        })
                    });

                    if all_present {
                        Ok(NodeResult {
                            output: serde_json::json!({"status": "ready"}),
                            next_node: None,
                            next_nodes: vec![],
                        })
                    } else {
                        let missing: Vec<String> = required
                            .iter()
                            .filter(|agent_id| {
                                !context.messages.iter().any(|m| {
                                    m.author.as_deref() == Some(agent_id.as_str())
                                })
                            })
                            .cloned()
                            .collect();
                        Ok(NodeResult {
                            output: serde_json::json!({
                                "status": "need_more_data",
                                "missing_points": missing
                            }),
                            next_node: None,
                            next_nodes: vec![],
                        })
                    }
                }

                "aggregate_and_output" => {
                    let required = node.required.as_ref().ok_or_else(|| {
                        "aggregate_and_output: нет required".to_string()
                    })?;
                    let mut reports = String::new();
                    for agent_id in required {
                        if let Some(msg) = context.messages.iter().rev()
                            .find(|m| m.author.as_deref() == Some(agent_id.as_str()))
                        {
                            if !reports.is_empty() {
                                reports.push_str("\n\n");
                            }
                            reports.push_str(&msg.content);
                        }
                    }

                    let msg = ChatMessage {
                        id: Some(format!("msg_{}", runner.msg_counter)),
                        msg_type: "message".to_string(),
                        content: reports.clone(),
                        sub_calls: None,
                        author: Some("system".to_string()),
                    };
                    context.messages.push(msg);
                    *runner.msg_counter += 1;
                    context.output_emitted = true;

                    Ok(NodeResult {
                        output: serde_json::json!({"reports": reports}),
                        next_node: None,
                        next_nodes: vec![],
                    })
                }

                _ => Err(format!(
                    "Неизвестное действие system_condition: {}",
                    action
                )),
            }
        }

        NodeType::SubWorkflow => {
            let wf_name = node
                .workflow
                .as_deref()
                .ok_or_else(|| "sub_workflow: не указан workflow".to_string())?;

            // Ищем загруженный workflow по file_stem (имя файла без .yaml)
            let clean = wf_name
                .trim_end_matches(".yaml")
                .trim_end_matches(".yml");
            let sub_wf = runner
                .workflows
                .iter()
                .find(|w| w.file_stem == clean)
                .ok_or_else(|| {
                    format!(
                        "sub_workflow: '{}' (file_stem='{}') не найден среди {} загруженных workflow",
                        wf_name,
                        clean,
                        runner.workflows.len()
                    )
                })?;

            (runner.log_cb)(format!(
                "[sub_workflow] Запуск '{}'",
                sub_wf.name
            ));

            let mut sub_ctx = WorkflowContext::new(
                context.user_message.clone(),
                context.messages.clone(),
                context.history.clone(),
            );

            let sub_result = super::run_workflow(sub_wf, &mut sub_ctx, runner)?;

            // Синхронизируем messages обратно в родительский контекст
            context.messages = sub_ctx.messages;

            Ok(NodeResult {
                output: serde_json::json!({"result": sub_result}),
                next_node: None,
            next_nodes: vec![],
            })
        }

        NodeType::LlmFreeform => {
            let user_text = context.resolve_template(node.input.as_deref().unwrap_or("{{ user_message }}"));
            let result = runner.call_llm_freeform(&user_text, &context.history)?;
            Ok(NodeResult {
                output: serde_json::json!({"result": result}),
                next_node: None,
            next_nodes: vec![],
            })
        }

        NodeType::Switch => {
            // Маршрутизация по switch_field (строгое соответствие значения)
            if let Some(ref input_obj) = node.input_object {
                let resolved = context.resolve_template(input_obj);
                let json_val: serde_json::Value = serde_json::from_str(&resolved)
                    .unwrap_or(serde_json::Value::Null);

                if let Some(ref sf) = node.switch_field {
                    let field_val = json_val.get(sf).and_then(|v| v.as_str());
                    if let Some(val) = field_val {
                        if let Some(ref priority_cases) = node.cases_priority {
                            if let Some(pc) = priority_cases.iter().find(|pc| pc.key == val) {
                                (runner.log_cb)(format!(
                                    "[switch] switch_field '{}' = '{}' → {}",
                                    sf, val, pc.to
                                ));
                                return Ok(NodeResult {
                                    output: serde_json::json!({"matched_case": val, "target": &pc.to}),
                                    next_node: Some(pc.to.clone()),
                                    next_nodes: vec![],
                                });
                            }
                        }
                    }
                    if let Some(ref default) = node.default {
                        (runner.log_cb)(format!(
                            "[switch] switch_field '{}' не совпал, default → {}",
                            sf, default
                        ));
                        return Ok(NodeResult {
                            output: serde_json::json!({"matched_case": "__default__", "target": default}),
                            next_node: Some(default.clone()),
                            next_nodes: vec![],
                        });
                    }
                    return Ok(NodeResult {
                        output: serde_json::json!({"matched_case": "__none__", "target": null}),
                        next_node: None,
                        next_nodes: vec![],
                    });
                }

                // Приоритетная маршрутизация по input_object + cases_priority
                if let Some(obj) = json_val.as_object() {
                    if let Some(ref priority_cases) = node.cases_priority {
                        for pc in priority_cases {
                            if let Some(val) = obj.get(&pc.key) {
                                if val.as_bool().unwrap_or(false) {
                                    (runner.log_cb)(format!(
                                        "[switch] Приоритет: '{}' = true → {}",
                                        pc.key, pc.to
                                    ));
                                    return Ok(NodeResult {
                                        output: serde_json::json!({"matched_case": pc.key, "target": pc.to}),
                                        next_node: Some(pc.to.clone()),
                                        next_nodes: vec![],
                                    });
                                }
                            }
                        }
                        // Ни один приоритет не совпал
                        if let Some(ref default) = node.default {
                        return Ok(NodeResult {
                            output: serde_json::json!({"matched_case": "__default__", "target": default}),
                            next_node: Some(default.clone()),
                            next_nodes: vec![],
                        });
                        }
                        return Ok(NodeResult {
                            output: serde_json::json!({"matched_case": "__none__", "target": null}),
                            next_node: None,
                            next_nodes: vec![],
                        });
                    }
                }
            }

            // Стандартная маршрутизация по input
            let input = context.resolve_template(node.input.as_deref().unwrap_or(""));
            let status = serde_json::from_str::<serde_json::Value>(&input)
                .ok()
                .and_then(|v| {
                    v.get("status")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| input.trim_matches('"').to_string());

            let target = node.cases_priority.as_ref()
                .and_then(|cp| cp.iter().find(|pc| pc.key == status).map(|pc| &pc.to))
                .cloned()
                .or_else(|| node.default.clone());

            Ok(NodeResult {
                output: serde_json::json!({"matched_case": status, "target": target}),
                next_node: target,
                next_nodes: vec![],
            })
        }

        NodeType::LlmSequentialSwitch => {
            let input_obj = node.input_object.as_deref().unwrap_or("{{ user_message }}");
            let resolved = context.resolve_template(input_obj);
            let json_val: serde_json::Value = serde_json::from_str(&resolved)
                .unwrap_or(serde_json::Value::Null);

            let mut matched: Vec<String> = vec![];

            if let Some(obj) = json_val.as_object() {
                if let Some(ref priority_cases) = node.cases_priority {
                    for pc in priority_cases {
                        if let Some(val) = obj.get(&pc.key) {
                            if val.as_bool().unwrap_or(false) {
                                (runner.log_cb)(format!(
                                    "[seq_switch] '{}' = true → {}",
                                    pc.key, pc.to
                                ));
                                matched.push(pc.to.clone());
                            }
                        }
                    }
                }
            }

            if matched.is_empty() {
                if let Some(ref default) = node.default {
                    (runner.log_cb)(format!(
                        "[seq_switch] Ни одного true, default → {}",
                        default
                    ));
                    matched.push(default.clone());
                }
            }

            let output = serde_json::json!({"matched_cases": matched});

            if matched.is_empty() {
                Ok(NodeResult {
                    output,
                    next_node: None,
                    next_nodes: vec![],
                })
            } else {
                let first = matched.remove(0);
                Ok(NodeResult {
                    output,
                    next_node: Some(first),
                    next_nodes: matched,
                })
            }
        }

        NodeType::SignalRouter => {
            let signal_name = node.signal_name.as_deref().unwrap_or("");
            let field = node.field.as_deref().unwrap_or("");
            let mut field_val: Option<String> = None;

            // Ищем последний signal-месседж с нужным ключом
            for msg in context.messages.iter().rev() {
                if msg.msg_type != "signal" { continue; }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&msg.content) {
                    if let Some(signal) = val.get(signal_name) {
                        if field.is_empty() {
                            field_val = signal.as_str().map(|s| s.to_string());
                        } else if let Some(nested) = signal.get(field) {
                            field_val = nested.as_str().map(|s| s.to_string());
                        }
                        if field_val.is_some() { break; }
                    }
                }
            }

            let matched = field_val.as_deref().unwrap_or("");

            (runner.log_cb)(format!(
                "[signal_router] signal '{}' field '{}' = '{}'",
                signal_name, field, matched
            ));

            // Поиск по cases_priority
            let target = node.cases_priority.as_ref()
                .and_then(|cp| cp.iter().find(|pc| pc.key == matched).map(|pc| &pc.to))
                .cloned()
                .or_else(|| node.default.clone());

            if let Some(ref t) = target {
                (runner.log_cb)(format!("[signal_router] → {}", t));
            } else {
                (runner.log_cb)("[signal_router] → нет target".to_string());
            }

            Ok(NodeResult {
                output: serde_json::json!({
                    "matched": matched,
                    "signal_name": signal_name,
                    "field": field,
                    "target": target
                }),
                next_node: target,
                next_nodes: vec![],
            })
        }

        NodeType::Return => Ok(NodeResult {
            output: serde_json::json!({"done": true}),
            next_node: Some("__END__".to_string()),
            next_nodes: vec![],
        }),
    }
}

const END_SENTINEL: &str = "END";

/// Находит следующий узел по списку edges.
/// Возвращает `None` когда нужно остановить workflow (дошли до END).
pub fn find_next_node(
    current_id: &str,
    edges: &[EdgeDef],
    node_result: &NodeResult,
) -> Option<String> {
    if let Some(ref next) = node_result.next_node {
        if next == "__END__" || next == END_SENTINEL {
            return None;
        }
        return Some(next.clone());
    }

    for edge in edges {
        if edge.from != current_id {
            continue;
        }
        let target = edge.to.clone();

        if edge.condition.is_none() && edge.case.is_none() {
            if target == END_SENTINEL {
                return None;
            }
            return Some(target);
        }
        if let Some(ref condition) = edge.condition {
            if let Some(status) = node_result
                .output
                .get("status")
                .and_then(|v| v.as_str())
            {
                let mapped = match status {
                    "has_unanalyzed" | "missing" => "has_unanalyzed",
                    "all_done" | "present" => "all_done",
                    s => s,
                };
                if mapped == condition || status == condition {
                    if target == END_SENTINEL {
                        return None;
                    }
                    return Some(target);
                }
            }
        }
        if let Some(ref case) = edge.case {
            if let Some(matched) = node_result
                .output
                .get("matched_case")
                .and_then(|v| v.as_str())
            {
                if matched == case {
                    if target == END_SENTINEL {
                        return None;
                    }
                    return Some(target);
                }
            }
        }
    }

    None
}

/// Извлекает JSON из произвольного текста (ищет первый `{...}` блок)
fn extract_json(text: &str) -> Option<String> {
    let text = if let Some(start) = text.find("```json") {
        let cs = start + 7;
        if let Some(end) = text[cs..].find("```") {
            text[cs..cs + end].to_string()
        } else {
            text[cs..].to_string()
        }
    } else {
        text.to_string()
    };

    text.find('{')
        .and_then(|start| text[start..].rfind('}').map(|end| text[start..start + end + 1].to_string()))
}
