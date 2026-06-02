use crate::domain::workflow_engine::context::WorkflowContext;
use crate::domain::workflow_engine::parser::{EdgeDef, NodeDef, NodeType, WorkflowDef};
use crate::domain::workflow_engine::WorkflowRunner;
use crate::infra::{ChatMessage, SubCall};

/// Результат выполнения узла
#[derive(Debug, Clone)]
pub struct NodeResult {
    pub output: serde_json::Value,
    pub next_node: Option<String>,
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
        NodeType::LlmClassifier => {
            let statuses = workflow
                .config
                .as_ref()
                .map(|c| c.statuses.clone())
                .unwrap_or_default();
            let input = context.resolve_template(node.input.as_deref().unwrap_or(""));
            let prompt =
                super::intent_classifier::build_classifier_prompt(&statuses, &input);

            let llm_response = runner.call_llm_direct(&prompt, &input)?;

            // Парсим JSON из ответа LLM
            let parsed: serde_json::Value =
                serde_json::from_str(&llm_response).unwrap_or_else(|_| {
                    // fallback: ищем JSON-блок в тексте
                    extract_json(&llm_response)
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_else(|| serde_json::json!({"status": "greeting"}))
                });

            let status = parsed
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("greeting")
                .to_string();
            let missing = parsed.get("missing_points");

            (runner.log_cb)(format!(
                "[classifier] Статус: {}, ответ: {}",
                status,
                llm_response.chars().take(200).collect::<String>()
            ));

            let mut output = serde_json::json!({"status": status});
            if let Some(mp) = missing {
                output["missing_points"] = mp.clone();
            }

            Ok(NodeResult {
                output,
                next_node: None,
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

            let task = context.resolve_template(node.task.as_deref().unwrap_or(""));
            let ns = node
                .namespace
                .as_deref()
                .unwrap_or(&context.namespace);

            (runner.log_cb)(format!(
                "[worker] Вызов '{}' (ns: {}): {}",
                agent_id,
                ns,
                task.chars().take(200).collect::<String>()
            ));

            let result = runner.call_agent(agent, &task, ns, &mut context.messages)?;

            // Сохраняем результат в messages как thought
            let msg = ChatMessage {
                id: Some(format!("msg_{}", runner.msg_counter)),
                msg_type: "thought".to_string(),
                content: result.clone(),
                namespace: Some(ns.to_string()),
                sub_calls: None,
                author: Some(agent_id.to_string()),
            };
            context.messages.push(msg);
            *runner.msg_counter += 1;

            Ok(NodeResult {
                output: serde_json::json!({"result": result, "agent": agent_id}),
                next_node: None,
            })
        }

        NodeType::SystemCondition => {
            let action = node.action.as_deref().unwrap_or("check");
            let ns = node
                .namespace
                .as_deref()
                .unwrap_or(&context.namespace);

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
                                    && m.namespace.as_deref() == Some(ns)
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
                                && m.namespace.as_deref() == Some(ns)
                        })
                    });
                    Ok(NodeResult {
                        output: serde_json::json!({
                            "status": if all_present { "present" } else { "missing" }
                        }),
                        next_node: None,
                    })
                }

                "all_problems_analyzed" => {
                    // Пробегаем по problem_1, problem_2... пока не найдём перерыв
                    let mut next_index = 1;
                    loop {
                        let ns_check = format!("problem_{}", next_index);
                        let has_report = context.messages.iter().any(|m| {
                            m.author.as_deref() == Some("pattern_finder_by_double_bind")
                                && m.namespace.as_deref() == Some(&ns_check)
                        });
                        if !has_report {
                            break;
                        }
                        next_index += 1;
                    }
                    // Если problem_1 не существует — значит ни одной проблемы нет
                    let all_done = next_index == 1;
                    Ok(NodeResult {
                        output: serde_json::json!({
                            "status": if all_done { "all_done" } else { "has_unanalyzed" },
                            "next_index": next_index
                        }),
                        next_node: None,
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

            let ns = node
                .namespace
                .as_deref()
                .unwrap_or(&context.namespace);

            (runner.log_cb)(format!(
                "[sub_workflow] Запуск '{}' (ns: {})",
                sub_wf.name, ns
            ));

            let mut sub_ctx = WorkflowContext::new(
                context.user_message.clone(),
                ns.to_string(),
                context.messages.clone(),
                context.history.clone(),
            );

            let sub_result = super::run_workflow(sub_wf, &mut sub_ctx, runner)?;

            // Синхронизируем messages обратно в родительский контекст
            context.messages = sub_ctx.messages;

            Ok(NodeResult {
                output: serde_json::json!({"result": sub_result}),
                next_node: None,
            })
        }

        NodeType::Switch => {
            let input = context.resolve_template(node.input.as_deref().unwrap_or(""));
            let status = serde_json::from_str::<serde_json::Value>(&input)
                .ok()
                .and_then(|v| {
                    v.get("status")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| input.trim_matches('"').to_string());

            let cases = node
                .cases
                .as_ref()
                .ok_or_else(|| "Switch node без cases".to_string())?;
            let target = cases
                .get(&status)
                .or_else(|| cases.values().next())
                .cloned();

            Ok(NodeResult {
                output: serde_json::json!({"matched_case": status, "target": target}),
                next_node: target,
            })
        }

        NodeType::Return => Ok(NodeResult {
            output: serde_json::json!({"done": true}),
            next_node: Some("__END__".to_string()),
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
