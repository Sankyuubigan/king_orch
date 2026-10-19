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
    collect_group_diagnostics(workflow, &mut diagnostics);
    collect_entry_diagnostics(workflow, &mut diagnostics);
    collect_template_reference_diagnostics(workflow, &mut diagnostics);
    collect_node_target_diagnostics(workflow, &mut diagnostics);
    collect_edge_diagnostics(workflow, &mut diagnostics);
    collect_agent_frame_diagnostics(workflow, &mut diagnostics);
    // «Мёртвый signal-контракт»: агент вызывает контракта, ключ никто
    // не читает — Method 3 отправит ответ в сигнал, а чат останется пустым.
    diagnostics.extend(super::signal_consumer_check::collect_dead_signal_diagnostics(
        workflow,
    ));
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

/// Контракт фрейма агента (`NodeDef.system_prompt_file`). Файл живёт вне YAML,
/// поэтому без диска проверяется только семантика:
/// - слот доставки без файла;
/// - неизвестное значение слота (валидация через единый
///   `agent_frame::FrameDelivery::parse`);
/// - поле на ноде, чей рантайм фрейм не читает (не `llm_worker`);
/// - нода внутри группы с ЯВНОЙ `fused_instruction`: склеенный вызов
///   объединяется этапами с системными промптами агентов, per-node фрейм
///   на таком этапе неприменим (авто-склейка без инструкции предупреждается
///   в рантайме — `workflow_engine::build_fused_system_prompt`).
fn collect_agent_frame_diagnostics(
    workflow: &WorkflowDef,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    let fused_group_ids: HashSet<&str> = workflow
        .groups
        .iter()
        .filter(|g| {
            g.fused_instruction
                .as_deref()
                .map_or(false, |s| !s.trim().is_empty())
        })
        .map(|g| g.id.as_str())
        .collect();
    for node in &workflow.nodes {
        let has_file = node
            .system_prompt_file
            .as_deref()
            .map_or(false, |s| !s.trim().is_empty());
        let delivery_raw = node.system_prompt_delivery.as_deref();
        let has_delivery = delivery_raw.map_or(false, |s| !s.trim().is_empty());
        if !has_file && !has_delivery {
            continue;
        }
        if has_delivery {
            if let Err(e) = super::agent_frame::FrameDelivery::parse(delivery_raw) {
                push_diagnostic(
                    diagnostics,
                    "SYSTEM_PROMPT_DELIVERY_UNKNOWN",
                    &format!("nodes[id={}]", node.id),
                    e,
                );
            }
        }
        if !has_file {
            push_diagnostic(
                diagnostics,
                "SYSTEM_PROMPT_DELIVERY_WITHOUT_FILE",
                &format!("nodes[id={}]", node.id),
                "задан system_prompt_delivery без system_prompt_file — нода не получит фрейм"
                    .to_string(),
            );
            continue;
        }
        if node.node_type != NodeType::LlmWorker {
            push_diagnostic(
                diagnostics,
                "SYSTEM_PROMPT_FILE_UNSUPPORTED_NODE",
                &format!("nodes[id={}]", node.id),
                format!(
                    "system_prompt_file поддерживается только нодами llm_worker (тип этой ноды: {:?})",
                    node.node_type,
                ),
            );
            continue;
        }
        if node
            .group
            .as_deref()
            .map_or(false, |id| fused_group_ids.contains(id.trim()))
        {
            push_diagnostic(
                diagnostics,
                "SYSTEM_PROMPT_FILE_IN_GROUP",
                &format!("nodes[id={}]", node.id),
                "нода принадлежит группе с fused_instruction — склеенный вызов не читает фрейм ноды"
                    .to_string(),
            );
        }
    }
}

fn collect_duplicate_node_ids(    workflow: &WorkflowDef,
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

/// Проверки объявленной точки входа.
///
/// Движок требует `entry` жёстко (см. `parser::resolve_entry`), поэтому редактор
/// показывает проблему ДО сохранения, а не после падения запуска.
fn collect_entry_diagnostics(
    workflow: &WorkflowDef,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    let Some(entry) = workflow
        .entry
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
    else {
        push_diagnostic(
            diagnostics,
            "ENTRY_MISSING",
            "entry",
            "не объявлена точка входа: добавь ноду типа user_message и поле `entry:` верхнего уровня со ссылкой на её id".to_string(),
        );
        return;
    };

    let Some(node) = workflow.nodes.iter().find(|n| n.id == entry) else {
        push_diagnostic(
            diagnostics,
            "ENTRY_TARGET_MISSING",
            "entry",
            format!("entry '{}' не существует среди nodes[].id", entry),
        );
        return;
    };

    if node.node_type != NodeType::UserMessage {
        push_diagnostic(
            diagnostics,
            "ENTRY_TYPE_UNEXPECTED",
            "entry",
            format!(
                "вход '{}' имеет тип {:?}, а не user_message — текст пользователя не попадёт в граф явно",
                entry, node.node_type
            ),
        );
    }

    for (index, edge) in workflow.edges.iter().enumerate() {
        if edge.to == entry {
            push_diagnostic(
                diagnostics,
                "ENTRY_HAS_INCOMING_EDGE",
                &format!("edges[{}]", index),
                format!(
                    "в '{}' приходит ребро '{}' → точка входа не может иметь входов",
                    entry, edge.from
                ),
            );
        }
    }

    for other in &workflow.nodes {
        if other.node_type == NodeType::UserMessage && other.id != entry {
            push_diagnostic(
                diagnostics,
                "USER_MESSAGE_NOT_ENTRY",
                &format!("nodes[id={}]", other.id),
                "второй узел user_message, не являющийся точкой входа: вход в граф должен быть один".to_string(),
            );
        }
    }
}

    /// Статическая сверка `groups:` с реальными нодами графа: группа должна
    /// быть выполнима МАКРО-НОДОЙ (один вход, цепочка, один выход, без роутеров).
    ///
    /// Группа — МЕТАДАННЫЕ (рамка на холсте + инструкция фьюжинга), поэтому
    /// большинство проблем здесь не ломают DETAILED-прогон, а делают заявленную
    /// автором схлопываемость недостижимой. Проверяем их ДО запуска, чтобы
    /// «Fused включён, а группа не схлопнулась» не выглядело как «Fused сломался».
    fn collect_group_diagnostics(
    workflow: &WorkflowDef,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    let workflow_dir = std::path::Path::new(&workflow.parent_dir);
    // Факт-контракт вычисляется ОДИН раз на весь граф: `resolve_facts` при пустом
    // `config.facts` читает `facts_file` с диска, и на каждый файл с группами
    // иначе выходило бы N одинаковых чтений одного файла.
    let facts = super::fact_extractor::resolve_facts(
        &workflow.config.clone().unwrap_or_default(),
        Some(workflow_dir),
    );
    let mut ids: HashMap<&str, usize> = HashMap::new();
    for group in &workflow.groups {
        let trimmed = group.id.trim();
        if trimmed.is_empty() {
            push_diagnostic(
                diagnostics,
                "GROUP_ID_MISSING",
                "groups[]",
                "у группы не задан `id`: группы без id невозможно удалить и переименовать".to_string(),
            );
            continue;
        }
        let count = ids.entry(trimmed).or_insert(0);
        *count += 1;
        if *count > 1 {
            push_diagnostic(
                diagnostics,
                "GROUP_ID_DUPLICATE",
                &format!("groups[id={}]", trimmed),
                "две группы с одинаковым id: нода с `group:` попадёт сразу в обе".to_string(),
            );
        }
    }

    for node in &workflow.nodes {
        let Some(group) = node.group.as_deref().map(str::trim).filter(|g| !g.is_empty()) else {
            continue;
        };
        if !ids.contains_key(group) {
            push_diagnostic(
                diagnostics,
                "GROUP_REF_MISSING",
                &format!("nodes[id={}].group", node.id),
                format!("группа '{}' объявлена в `group:`, но её нет в секции `groups:`", group),
            );
        }
    }

    for group in &workflow.groups {
        let id = group.id.trim();
        if id.is_empty() {
            continue;
        }
        let location = format!("groups[id={}]", id);
        let members = workflow
            .nodes
            .iter()
            .filter(|n| n.group.as_deref().map(str::trim) == Some(id))
            .count();

        if members == 0 {
            push_diagnostic(
                diagnostics,
                "GROUP_EMPTY",
                &location,
                "в группу не входит ни одна нода: рамка на холсте будет пустой, схлопывать нечего"
                    .to_string(),
            );
        }

        let instruction = group.fused_instruction.as_deref().map(str::trim);
        match instruction {
            None | Some("") => {
                // Инструкция больше не обязательна: без неё движок склеивает
                // системный промпт из ролей нод (`build_auto_fused_prompt`).
                // Но если склеивать НЕЧЕГО, группа молча не схлопнется — это
                // и есть то, о чём автор должен узнать ДО запуска.
                if let Err(why) = super::groups::check_stitchable(workflow, group) {
                    push_diagnostic(
                        diagnostics,
                        "GROUP_FUSED_NOT_STITCHABLE",
                        &location,
                        why,
                    );
                }
                if !group.tools.is_empty() {
                    push_diagnostic(
                        diagnostics,
                        "GROUP_FUSED_WITHOUT_INSTRUCTION",
                        &location,
                        "у группы объявлены `tools`, но нет `fused_instruction`: инструменты \
                         группы и так не поддержаны, а авто-склейка их не воспроизводит"
                            .to_string(),
                    );
                }
            }
Some(text) => {
                // Инструменты группы резолвятся из `mcp_servers` её агента. Без
                // агента их неоткуда взять, и объявление было бы ложью: пользователь
                // прочитал бы «инструмент доступен», а вызова не произойдёт.
                if !group.tools.is_empty()
                    && group
                        .fused_agent
                        .as_deref()
                        .map(str::trim)
                        .filter(|a| !a.is_empty())
                        .is_none()
                {
                    push_diagnostic(
                        diagnostics,
                        "GROUP_FUSED_TOOLS_WITHOUT_AGENT",
                        &location,
                        "у группы объявлены `tools`, но нет `fused_agent`: инструменты группы \
                         берутся из `mcp_servers` агента, объявлять их без него бессмысленно"
                            .to_string(),
                    );
                }
                // Ноды вне группы, чьи отчёты внедряются внутрь неё
                // (`inject_response`). В Fused внутренние ноды
                // не выполняются, а склеенный вызов получает историю БЕЗ
                // thought-сообщений (`llm_history` их отфильтровывает) — отчёт
                // внешнего агента до модели просто не дойдёт.
                let members: HashSet<&str> = super::groups::nodes_of_group(workflow, id)
                    .iter()
                    .map(|n| n.id.as_str())
                    .collect();
                let produced_inside: HashSet<&str> = members
                    .iter()
                    .filter_map(|member| {
                        workflow
                            .nodes
                            .iter()
                            .find(|n| n.id.as_str() == *member)
                            .and_then(|n| n.agent.as_deref())
                    })
                    .collect();
                for node in &workflow.nodes {
                    if !members.contains(node.id.as_str()) {
                        continue;
                    }
                    let mut injected: Vec<&String> = Vec::new();
                    injected.extend(node.inject_response.iter().flatten());
                    let lost: Vec<&str> = injected
                        .iter()
                        .map(|a| a.trim())
                        .filter(|a| !a.is_empty() && !produced_inside.contains(a))
                        .collect();
                    if let Some(first) = lost.first() {
                        push_diagnostic(
                            diagnostics,
                            "GROUP_FUSED_INJECT_LOST",
                            &format!("nodes[id={}].inject_response", node.id),
                            format!(
                                "нода группы внедряет отчёт агента '{}` (всего: {}), а он производится \
                                 ВНЕ группы. В FUSED склеенный вызов получает историю без \
                                 thought-сообщений, поэтому отчёт до модели не дойдёт. Либо уберите \
                                 зависимость, либо опишите её в `fused_instruction`",
                                first,
                                lost.join(", ")
                            ),
                        );
                    }
                }

                // ФОРМА макро-ноды. Группа схлопывается в ОДИН вызов, поэтому у
                // неё обязан быть один вход, одна цепочка и один выход. Каждое
                // нарушение — отдельный отказ с текстом «что делать», а не
                // «Fused почему-то не схлопнулся».
                let routers = super::groups::routers_of_group(workflow, id);
                if !routers.is_empty() {
                    let names: Vec<&str> = routers.iter().map(|n| n.id.as_str()).collect();
                    let undeclared = super::groups::undeclared_routing_keys_with_facts(
                        workflow,
                        group,
                        &facts,
                    );
                    let keys = super::groups::routing_keys_of_group(workflow, id);
                    let mut hint = format!(
                        "в группе есть детерминированный роутер ({}): макро-вызов выполняет всю \
                         группу одним запросом и ветвление не принимает. Вынесите роутер и \
                         читаемые им сигналы ({}) ИЗ группы, а в группу оставьте ту ветку, \
                         которую роутер выбирает",
                        names.join(", "),
                        if keys.is_empty() {
                            "—".to_string()
                        } else {
                            keys.join(", ")
                        }
                    );
                    if !undeclared.is_empty() {
                        hint.push_str(&format!(
                            ". Часть этих сигналов не объявлена ни в фактах графа, ни в \
                             `fused_keys`: {}",
                            undeclared.join(", ")
                        ));
                    }
                    push_diagnostic(
                        diagnostics,
                        "GROUP_FUSED_HAS_ROUTER",
                        &location,
                        hint,
                    );
                }

                if let Err(why) = super::groups::ensure_single_entry(workflow, id) {
                    push_diagnostic(
                        diagnostics,
                        "GROUP_FUSED_MULTIPLE_ENTRY",
                        &location,
                        why,
                    );
                }

                if let Err(why) = super::groups::exit_node_of_group(workflow, id) {
                    push_diagnostic(
                        diagnostics,
                        "GROUP_FUSED_EXIT_AMBIGUOUS",
                        &location,
                        why,
                    );
                } else if let Ok(exit) = super::groups::exit_node_of_group(workflow, id) {
                    if super::groups::exit_has_conditional_edge(workflow, &exit, id) {
                        push_diagnostic(
                            diagnostics,
                            "GROUP_FUSED_EXIT_CONDITIONAL",
                            &format!("{}.exit", location),
                            format!(
                                "из выхода группы (нода '{}') выходит условное ребро \
                                 (`condition:`/`case:`): у макро-вызова нет \
                                 `status`/`matched_case`, чтобы его выбрать. Сделайте ребро \
                                 безусловным или вынесите ветвление в роутер ДО группы",
                                exit
                            ),
                        );
                    }
                }

                // В FUSED внутренние ноды группы НЕ выполняются, поэтому ссылка
                // `{{ nodes.X.output }}` в инструкции не разрешится и уедет в промпт
                // литералом (`resolve_template` молча оставляет неизвестные плейсхолдеры).
                let targets = referenced_node_ids(text);
                if let Some(first) = targets.first() {
                    push_diagnostic(
                        diagnostics,
                        "GROUP_FUSED_NODE_REF",
                        &format!("{}.fused_instruction", location),
                        format!(
                            "ссылка {{{{ nodes.{}.output }}}} в инструкции фьюжинга не разрешится: \
                             в FUSED эти ноды не выполняются по отдельности (всего: {})",
                            first,
                            targets.join(", ")
                        ),
                    );
                }

                // То же для потребителей СНАРУЖИ группы: нода вне группы читает
                // выход внутренней ноды — в Fused такого выхода не будет.
                for node in &workflow.nodes {
                    if members.contains(node.id.as_str()) {
                        continue;
                    }
                    for (field, template) in [
                        ("input", node.input.as_deref()),
                        ("input_object", node.input_object.as_deref()),
                        ("task", node.task.as_deref()),
                    ] {
                        let Some(template) = template else { continue };
                        let referenced = referenced_node_ids(template);
                        let inside: Vec<&str> = referenced
                            .iter()
                            .filter(|t| members.contains(t.as_str()))
                            .map(|t| t.as_str())
                            .collect();
                        if let Some(first) = inside.first() {
                            push_diagnostic(
                                diagnostics,
                                "GROUP_FUSED_OUTPUT_CONSUMED",
                                &format!("nodes[id={}].{}", node.id, field),
                                format!(
                                    "нода читает выход '{}' — внутренней ноды группы '{}': в FUSED \
                                     группа выполняется одним вызовом и такой выход не создаётся \
                                     (плейсхолдер уедет в промпт литералом)",
                                    first, id
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Статическая сверка `{{ nodes.X.output }}` со списком `nodes[].id`.
///
/// Шаблоны разрешаются текстовой заменой, и неразрешённый плейсхолдер МОЛЧА
/// уезжает в промпт модели (см. `WorkflowContext::resolve_template`). Эта проверка
/// превращает такой отказ в видимую диагностику редактора, без запуска графа.
fn collect_template_reference_diagnostics(
    workflow: &WorkflowDef,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    let ids: HashSet<&str> = workflow.nodes.iter().map(|node| node.id.as_str()).collect();
    for node in &workflow.nodes {
        for (field, template) in [
            ("input", node.input.as_deref()),
            ("input_object", node.input_object.as_deref()),
            ("task", node.task.as_deref()),
        ] {
            let Some(template) = template else { continue };
            for target in referenced_node_ids(template) {
                if !ids.contains(target.as_str()) {
                    push_diagnostic(
                        diagnostics,
                        "TEMPLATE_NODE_REF_MISSING",
                        &format!("nodes[id={}].{}", node.id, field),
                        format!(
                            "ссылка {{{{ nodes.{}.output }}}} не разрешится: ноды с id '{}' нет в графе",
                            target, target
                        ),
                    );
                }
            }
        }
    }
}

/// Вынимает id нод из шаблонов вида `{{ nodes.<id>.output }}` / `{{ nodes.<id>.output.<key> }}`.
fn referenced_node_ids(template: &str) -> Vec<String> {
    const PREFIX: &str = "{{ nodes.";
    let mut found = Vec::new();
    let mut rest = template;
    while let Some(pos) = rest.find(PREFIX) {
        rest = &rest[pos + PREFIX.len()..];
        let id: String = rest
            .chars()
            .take_while(|c| !matches!(c, '.' | ' ' | '}' | '\n' | '\t'))
            .collect();
        if !id.is_empty() {
            found.push(id);
        }
    }
    found
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
