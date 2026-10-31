use std::collections::HashSet;

use super::fact_extractor;
use super::parser::{FactDef, GroupDef, NodeDef, WorkflowDef};

/// Ключ выхода склеенного вызова, в котором группа возвращает текст пользователю.
///
/// Это обычный строковый ключ контракта, а не отдельное поле: грамматика
/// `build_json_object_grammar_with_keys` требует все объявленные ключи, и группа,
/// которая отвечать не должна, честно возвращает пустую строку.
pub const MESSAGE_KEY: &str = "message";

/// Префикс ключа обязательного вывода одного этапа авто-склейки.
///
/// Каждый этап склеенного вызова получает своё строковое поле
/// `out_<node_id>` в JSON-ответе: пропуск этапа перестаёт быть возможным
/// форматом (грамматика требует все ключи), а отчёты этапов разделяются
/// (иначе оба текста замешивались бы в единственный `message`).
pub const STAGE_KEY_PREFIX: &str = "out_";

/// Режим исполнения графа.
///
/// Ровно два значения. Режима `auto` не существует намеренно: любая эвристика
/// «модель достаточно сильная» — это порог без замера (§00.2). Экономию
/// пользователь включает сам и осознанно.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStrategy {
    /// Каждая нода группы выполняется отдельным вызовом (поведение по умолчанию).
    Detailed,
    /// Ноды группы склеиваются в один вызов LLM по `fused_instruction`.
    Fused,
}

impl ExecutionStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutionStrategy::Detailed => "detailed",
            ExecutionStrategy::Fused => "fused",
        }
    }
}

/// Разобрать объявленную стратегию графа.
///
/// Неизвестное значение — ОШИБКА, а не тихий `Detailed`: файл, в котором написано
/// `strategy: fuzed`, обязан сказать об этом пользователю, а не молча работать
/// не так, как автор написал.
pub fn parse_strategy(raw: Option<&str>) -> Result<ExecutionStrategy, String> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None | Some("detailed") => Ok(ExecutionStrategy::Detailed),
        Some("fused") => Ok(ExecutionStrategy::Fused),
        Some(other) => Err(format!(
            "неизвестный `strategy: {}` — допустимы только `detailed` и `fused`",
            other
        )),
    }
}

/// Стратегия исполнения конкретного графа из поля `strategy:`.
pub fn strategy_of(workflow: &WorkflowDef) -> Result<ExecutionStrategy, String> {
    parse_strategy(workflow.strategy.as_deref())
}

/// Итоговый режим исполнения и ЕГО ИСТОЧНИК.
///
/// Приоритет ровно один, и он назван: переопределение из чата → `strategy:` файла
/// → `Detailed`. Явный выбор пользователя в чате важнее файла (он сделал его
/// сейчас, файл он правил когда-то), но переопределение только перекрывает —
/// молча выбрать режим вместо пользователя движок не вправе, поэтому при его
/// отсутствии файл остаётся главным.
///
/// Второе значение — источник. Он же уходит в лог: если режим в чате не тот, что
/// в файле, пользователь обязан это видеть, а не угадывать по результату.
pub fn resolve_strategy(
    workflow: &WorkflowDef,
    override_strategy: Option<ExecutionStrategy>,
) -> Result<(ExecutionStrategy, &'static str), String> {
    if let Some(strategy) = override_strategy {
        return Ok((strategy, "переопределение в чате"));
    }
    match strategy_of(workflow) {
        Ok(strategy) => Ok((strategy, "поле strategy: в файле графа")),
        // Опечатка в файле (`strategy: fuzed`) — ошибка, а не тихий Detailed:
        // автор написал одно, а получил бы другое и не узнал бы.
        Err(why) => Err(why),
    }
}

/// Индекс групп по `id` + проверка уникальности. Пустой для графов без групп.
pub fn group_index(workflow: &WorkflowDef) -> Result<Vec<(String, usize)>, String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::with_capacity(workflow.groups.len());
    for (idx, group) in workflow.groups.iter().enumerate() {
        let id = group.id.trim();
        if id.is_empty() {
            continue;
        }
        if !seen.insert(id) {
            return Err(format!("две группы с одинаковым id '{}'", id));
        }
        out.push((id.to_string(), idx));
    }
    Ok(out)
}

/// Ноды, входящие в группу, в порядке объявления в `nodes:`.
pub fn nodes_of_group<'a>(workflow: &'a WorkflowDef, group_id: &str) -> Vec<&'a NodeDef> {
    workflow
        .nodes
        .iter()
        .filter(|n| n.group.as_deref().map(str::trim) == Some(group_id))
        .collect()
}

/// Ноды группы в порядке ИСПОЛНЕНИЯ (топологическом по внутренним рёбрам).
///
/// Порядок объявления в `nodes:` для этого не годится: редактор переписывает его
/// при каждом сохранении. Склейка промптов обязана идти в том порядке, в каком
/// ноды выполнялись бы по отдельности, иначе модель получит этапы задом наперёд.
pub fn topo_order_of_group<'a>(
    workflow: &'a WorkflowDef,
    group_id: &str,
) -> Result<Vec<&'a NodeDef>, String> {
    let members: Vec<&NodeDef> = nodes_of_group(workflow, group_id);
    let ids: HashSet<&str> = members.iter().map(|n| n.id.as_str()).collect();
    let mut ordered: Vec<&NodeDef> = Vec::with_capacity(members.len());
    let mut placed: HashSet<&str> = HashSet::new();

    // Внутренние предшественники каждой ноды: учитываем только рёбра ВНУТРИ группы.
    let preds = |id: &str| -> Vec<&str> {
        workflow
            .edges
            .iter()
            .filter(|e| e.to == id && ids.contains(e.from.as_str()))
            .map(|e| e.from.as_str())
            .collect()
    };

    while ordered.len() < members.len() {
        let ready: Vec<&NodeDef> = members
            .iter()
            .filter(|n| !placed.contains(n.id.as_str()))
            .filter(|n| preds(n.id.as_str()).iter().all(|p| placed.contains(p)))
            .copied()
            .collect();
        if ready.is_empty() {
            return Err(format!(
                "в группе '{}` цикл по рёбрам — топологического порядка не существует",
                group_id
            ));
        }
        // Стабильность: при нескольких готовых берём порядок объявления, чтобы
        // одинаковый YAML всегда давал одинаковый промпт.
        for node in ready {
            ordered.push(node);
            placed.insert(node.id.as_str());
        }
    }
    Ok(ordered)
}

/// Из чего можно склеить системный промпт группы, если автор его не написал.
///
/// Проверка статическая, по YAML: у ноды должен быть либо агент (его `.md`
/// даст роль этапа), либо собственный `input` (`llm_freeform`). Нода без обоих
/// нечем нарядить — выдумывать роль значило бы отправить модели вымышленную
/// личность.
pub fn check_stitchable(workflow: &WorkflowDef, group: &GroupDef) -> Result<Vec<String>, String> {
    let group_id = group.id.trim();
    let ordered = topo_order_of_group(workflow, group_id)?;
    let mut without_prompt: Vec<String> = Vec::new();
    for node in &ordered {
        let has_agent = node
            .agent
            .as_deref()
            .map(str::trim)
            .is_some_and(|a| !a.is_empty());
        let has_input = node
            .input
            .as_deref()
            .map(str::trim)
            .is_some_and(|i| !i.is_empty());
        if !has_agent && !has_input {
            without_prompt.push(node.id.clone());
        }
    }
    if without_prompt.is_empty() {
        return Ok(ordered.iter().map(|n| n.id.clone()).collect());
    }
    Err(format!(
        "у нод группы ({}) нет ни агента, ни `input` — склеить из них системный промпт нельзя. \
         Задайте группе `fused_instruction` явно",
        without_prompt.join(", ")
    ))
}

/// Детерминированные РОУТЕРЫ внутри группы.
///
/// В режиме Fused группа — неделимый макро-узел: её внутренние ноды вообще не
/// попадают в очередь планировщика. Роутер внутри группы этому противоречит —
/// он решает, какая внутренняя ветка вообще должна выполниться, а макро-вызов
/// такого решения не принимает. Поэтому такая группа нефьюзится и об этом
/// говорится автору (диагностика `GROUP_FUSED_HAS_ROUTER` + честная ошибка в
/// рантайме), а не «схлопывается как-нибудь».
pub fn routers_of_group<'a>(workflow: &'a WorkflowDef, group_id: &str) -> Vec<&'a NodeDef> {
    use super::parser::NodeType;
    nodes_of_group(workflow, group_id)
        .into_iter()
        .filter(|n| {
            matches!(
                n.node_type,
                NodeType::ConditionCheck
                    | NodeType::ConditionRouter
                    | NodeType::Switch
                    | NodeType::LlmSequentialSwitch
                    | NodeType::SignalRouter
            )
        })
        .collect()
}

/// Последняя нода группы — та, из которой макро-вызов уходит наружу.
///
/// ПРОИЗВОДНАЯ величина, а не поле YAML: состав группы задан `nodes[].group`,
/// и второе объявление (`groups[].nodes`) разошлось бы с ним при первом же
/// редактировании (§2.1). Выход — единственная нода группы, из которой НЕТ ребра
/// в другую ноду этой же группы; всё остальное — внутренности макро-узла.
///
/// Их несколько или ноль → макро-нода не определена (разветвление внутри группы
/// или группа без внутренних связей). Это ошибка автора, а не повод угадать.
pub fn exit_node_of_group(workflow: &WorkflowDef, group_id: &str) -> Result<String, String> {
    let members: HashSet<&str> = nodes_of_group(workflow, group_id)
        .iter()
        .map(|n| n.id.as_str())
        .collect();
    if members.is_empty() {
        return Err(format!(
            "группа '{}' пуста: нечего выполнять макро-вызовом",
            group_id
        ));
    }
    let mut exits: Vec<&str> = members
        .iter()
        .filter(|id| {
            !workflow.edges.iter().any(|e| {
                e.from == **id && members.contains(e.to.as_str())
            })
        })
        .copied()
        .collect();
    if exits.len() != 1 {
        let list: Vec<&str> = exits.iter().copied().collect();
        exits.clear();
        return Err(format!(
            "у группы '{}' {} нод без внутренних исходящих ребер ({}): макро-вызову не из чего \
             выходить. Схлопываемая группа — это цепочка: вход → … → выход, без развилок внутри",
            group_id,
            list.len(),
            if list.is_empty() {
                "—".to_string()
            } else {
                list.join(", ")
            }
        ));
    }
    Ok(exits.remove(0).to_string())
}

/// Проверить форму входа макро-узла.
///
/// Разрешён ровно один вход извне — в НАЧАЛО цепочки группы. Второй вход или
/// вход в середину означает, что планировщир дойдёт до внутренней ноды группы
/// другим путём и выполнит её отдельно (групповой вызов уже потрачен) — то есть
/// Fused снова станет дороже Detailed.
///
/// Проверка видит только РЁБРА. Переходы, объявленные полями ноды
/// (`sequential_to`, `true_to`, `false_to`, `cases_priority[].to`), из рёбер не
/// следуют, и пересобирать их здесь означало бы завести в движок ВТОРУЮ копию
/// логики роутинга — расхождение которой стоило бы дороже самой проверки.
/// Поэтому динамический второй вход ловится не здесь, а в рантайме
/// ([`executed_group_of_node`]): ошибкой с именем ноды, а не тихим повторным
/// вызовом.
pub fn ensure_single_entry(workflow: &WorkflowDef, group_id: &str) -> Result<(), String> {
    let members: HashSet<&str> = nodes_of_group(workflow, group_id)
        .iter()
        .map(|n| n.id.as_str())
        .collect();
    if members.is_empty() {
        return Ok(());
    }
    let has_internal_incoming = |id: &str| {
        workflow
            .edges
            .iter()
            .any(|e| e.to == id && members.contains(e.from.as_str()))
    };
    let head = members
        .iter()
        .find(|id| !has_internal_incoming(id))
        .copied()
        .ok_or_else(|| {
            format!(
                "в группе '{}` нет начала: у каждой её ноды есть входящее ребро изнутри группы \
                 (цикл). Схлопываемой группе нужен один вход",
                group_id
            )
        })?;

    let mut foreign: Vec<&str> = members
        .iter()
        .filter(|id| {
            workflow
                .edges
                .iter()
                .any(|e| e.to == ***id && !members.contains(e.from.as_str()))
        })
        .copied()
        .collect();
    foreign.sort();
    if foreign.len() > 1 {
        return Err(format!(
            "в группу '{}` ведут рёбра извне сразу в несколько нод ({}): макро-узел имеет один \
             вход. Оставьте внешним только ребро в '{}'",
            group_id,
            foreign.join(", "),
            head
        ));
    }
    if let Some(only) = foreign.first() {
        if *only != head {
            return Err(format!(
                "внешнее ребро ведёт в середину группы '{}` (в '{}', а начало цепочки — '{}'): \
                 макро-вызов начнётся не с начала, и часть нод группы выполнится отдельно",
                group_id, only, head
            ));
        }
    }
    Ok(())
}

/// Уходит ли ребро из выхода группы по УСЛОВИЮ (`condition:`/`case:`).
///
/// Макро-вызов не имеет `status`/`matched_case` — таких полей у его ответа нет,
/// поэтому условное ребро после прыжка выбрать нечем: `find_next_node` вернул бы
/// `None` и прогон молча оборвался бы. Это отказ, а не деталь реализации.
pub fn exit_has_conditional_edge(workflow: &WorkflowDef, exit_id: &str, group_id: &str) -> bool {
    let members: HashSet<&str> = nodes_of_group(workflow, group_id)
        .iter()
        .map(|n| n.id.as_str())
        .collect();
    workflow.edges.iter().any(|e| {
        e.from == exit_id
            && !members.contains(e.to.as_str())
            && (e.condition.is_some() || e.case.is_some())
    })
}

/// Ноды группы, которые вообще зовут LLM. Именно их количество схлопывает Fused.
pub fn llm_nodes_of_group<'a>(workflow: &'a WorkflowDef, group_id: &str) -> Vec<&'a NodeDef> {
    use super::parser::NodeType;
    nodes_of_group(workflow, group_id)
        .into_iter()
        .filter(|n| {
            matches!(
                n.node_type,
                NodeType::LlmWorker | NodeType::LlmFreeform | NodeType::LlmFactExtractor
            )
        })
        .collect()
}

/// Группа, которую нужно выполнить склеенным вызовом ВМЕСТО ноды `node`.
///
/// Возвращает `None` во всех обычных случаях:
/// - стратегия `Detailed` — группа не фьюзится никогда;
/// - нода вне групп;
/// - группа уже исполнена в этом ходе — планировщик обязан был прыгнуть за её
///   выход; если очередь всё же привела внутрь, это ошибка графа, и её зовёт
///   [`executed_group_of_node`] (тихо выполнить внутреннюю ноду нельзя);
/// - группу нечем склеить: ни явной `fused_instruction`, ни ролей для
///   авто-склейки (см. [`check_stitchable`]).
///
/// Явная `fused_instruction` больше НЕ обязательна: автор может просто объединить
/// ноды в группу, а движок соберёт системный промпт из их агентов
/// ([`build_auto_fused_prompt`]).
pub fn fused_group_for<'a>(
    workflow: &'a WorkflowDef,
    node: &NodeDef,
    strategy: ExecutionStrategy,
    executed_groups: &HashSet<String>,
) -> Option<&'a GroupDef> {
    if strategy != ExecutionStrategy::Fused {
        return None;
    }
    let group_id = node.group.as_deref().map(str::trim).filter(|g| !g.is_empty())?;
    if executed_groups.contains(group_id) {
        return None;
    }
    let group = workflow
        .groups
        .iter()
        .find(|g| g.id.trim() == group_id)?;
    let explicit = group
        .fused_instruction
        .as_deref()
        .map(str::trim)
        .is_some_and(|i| !i.is_empty());
    if !explicit && check_stitchable(workflow, group).is_err() {
        return None;
    }
    Some(group)
}

/// Нода оказалась ВНУТРИ уже выполненной макро-группы.
///
/// Так быть не должно: после макро-вызова планировщик прыгает за выход группы и
/// внутренние ноды не попадают в очередь. Единственный способ дойти до такой ноды —
/// второе ребро извне (`GROUP_FUSED_MULTIPLE_ENTRY` в редакторе) или правка файла
/// руками. Проверка нужна, чтобы такой случай падал с объяснением, а не тихо
/// выполнял ноду вторым отдельным вызовом LLM — то есть не делал вид, что
/// Fused схлопнул группу.
pub fn executed_group_of_node<'a>(
    workflow: &'a WorkflowDef,
    node: &NodeDef,
    strategy: ExecutionStrategy,
    executed_groups: &HashSet<String>,
) -> Option<&'a GroupDef> {
    if strategy != ExecutionStrategy::Fused {
        return None;
    }
    let group_id = node.group.as_deref().map(str::trim).filter(|g| !g.is_empty())?;
    if !executed_groups.contains(group_id) {
        return None;
    }
    workflow.groups.iter().find(|g| g.id.trim() == group_id)
}

/// Ключи `signals`, которые читают ДЕТЕРМИНИРОВАННЫЕ ноды группы.
///
/// Это производная величина: `condition_check`/`switch` читают `field`,
/// `condition_router` — поля из `conditions`, `signal_router` — `cases_priority[].key`.
/// Именно эти ключи склеенный вызов обязан заполнить, иначе роутер группы
/// отправит прогон по ветке «сигнала нет».
pub fn routing_keys_of_group(workflow: &WorkflowDef, group_id: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let mut push = |key: &str| {
        let trimmed = key.trim();
        // `validator_report.e3` — точечный путь до поля объекта-сигнала:
        // в грамматике нужен сам объект `validator_report`, а не его поле.
        let root = trimmed.split('.').next().unwrap_or(trimmed);
        if root.is_empty() {
            return;
        }
        if !keys.iter().any(|k| k == root) {
            keys.push(root.to_string());
        }
    };

    for node in nodes_of_group(workflow, group_id) {
        use super::parser::NodeType;
        match node.node_type {
            NodeType::ConditionCheck | NodeType::Switch => {
                if let Some(field) = node.field.as_deref() {
                    push(field);
                }
            }
            NodeType::ConditionRouter => {
                if let Some(field) = node.field.as_deref() {
                    push(field);
                }
                collect_condition_keys(&node.conditions, &mut push);
            }
            NodeType::SignalRouter => {
                for case in node.cases_priority.iter().flatten() {
                    push(&case.key);
                }
            }
            // LLM-ноды и note в маршрутизации не участвуют.
            _ => {}
        }
    }
    keys
}

/// Рекурсивный сбор полей из дерева условий (`Rule` / вложенная `Group`).
fn collect_condition_keys<'a, F: FnMut(&'a str)>(conditions: &'a [super::parser::ConditionNode], push: &mut F) {
    for condition in conditions {
        match condition {
            super::parser::ConditionNode::Rule { field, .. } => push(field),
            super::parser::ConditionNode::Group { conditions, .. } => {
                collect_condition_keys(conditions, push)
            }
        }
    }
}

/// Есть ли в группе нода-экстрактор фактов.
///
/// Только такая группа обязана вернуть весь факт-контракт графа: остальным
/// факты не нужны, и требовать их означало бы заставлять модель заполнять
/// нерелевантные поля (в фазе исцеления — выдумывать `has_problem`).
fn group_extracts_facts(workflow: &WorkflowDef, group_id: &str) -> bool {
    use super::parser::NodeType;
    nodes_of_group(workflow, group_id)
        .iter()
        .any(|n| n.node_type == NodeType::LlmFactExtractor)
}

/// Контракт выхода группы: `(bool_keys, string_keys)` для грамматики.
///
/// boolean — факты без `values` и `fused_keys` без `values`; строковые — факты
/// с `values` и строковые `fused_keys`; далее для авто-склейки — обязательные
/// этапные ключи `out_<node_id>`, для явной `fused_instruction` — служебный
/// `message`.
/// Ключи дедуплируются по id: один и тот же факт может быть объявлен и в
/// facts.yaml, и в fused_keys.
pub fn output_contract(
    workflow: &WorkflowDef,
    group: &GroupDef,
    workflow_dir: Option<&std::path::Path>,
) -> (Vec<String>, Vec<String>) {
    let config = workflow.config.clone().unwrap_or_default();
    let facts = fact_extractor::resolve_facts(&config, workflow_dir);
    let extracts = group_extracts_facts(workflow, &group.id);

    let mut bool_keys: Vec<String> = facts
        .iter()
        .filter(|f| extracts && f.values.is_empty())
        .map(|f| f.id.clone())
        .collect();
    let mut string_keys: Vec<String> = facts
        .iter()
        .filter(|f| extracts && !f.values.is_empty())
        .map(|f| f.id.clone())
        .collect();

    for key in &group.fused_keys {
        if key.values.is_empty() {
            if !bool_keys.contains(&key.id) {
                bool_keys.push(key.id.clone());
            }
        } else if !string_keys.contains(&key.id) {
            string_keys.push(key.id.clone());
        }
    }

    // Ключи, которые читают роутеры САМОЙ группы, в контракт НЕ входят.
    //
    // Раньше они добавлялись, и это порождало второй источник правды: ключ из
    // `facts.yaml` уже извлекал `extract_facts` в этом же ходу, а склеенный
    // вызов обязан был вернуть его ЕЩЁ РАЗ — уже строкой и без enum-ограничения,
    // и `signals.insert` затирал бы значение экстрактора. Роутеры внутри фьюзной
    // группы теперь запрещены (`GROUP_FUSED_HAS_ROUTER`), а ключ, объявленный в
    // `fused_keys`, группа производит сознательно — вот и весь контракт.
    //
    // Проверка остаётся в диагностике (`GROUP_FUSED_HAS_ROUTER` называет ключи,
    // которые читали роутеры группы), чтобы автор видел, что переносить наружу.

    if is_explicit_fused(group) {
        // Явная инструкция: автор сам объяснил формирование текста, ключ
        // `message` остаётся единственной канавкой итогового текста.
        if !string_keys.iter().any(|k| k == MESSAGE_KEY) {
            string_keys.push(MESSAGE_KEY.to_string());
        }
    } else {
        // Авто-склейка: у КАЖДОГО этапа своя обязательная канавка, чтобы пропуск
        // этапа был невозможен форматом (грамматика требует все ключи). Общий
        // `message` не объявляется: текст пользователю — этапная нода, у которой
        // `output_type: message` (её текст публикуется отдельным сообщением).
        for spec in stage_output_specs(workflow, group) {
            if !string_keys.iter().any(|k| k == &spec.key) {
                string_keys.push(spec.key);
            }
        }
    }
    (bool_keys, string_keys)
}

/// Ключи, которые читали роутеры группы, но которых нет ни в факт-контракте
/// графа, ни в `fused_keys`.
///
/// Нужно только ДЛЯ ДИАГНОСТИКИ: сам контракт группы роутеры больше не пополняют
/// (см. `output_contract`), и смысл этой функции — назвать автору ключи, которые
/// он обязан вынести из группы вместе с роутером.
pub fn undeclared_routing_keys_with_facts(
    workflow: &WorkflowDef,
    group: &GroupDef,
    facts: &[FactDef],
) -> Vec<String> {
    routing_keys_of_group(workflow, &group.id)
        .into_iter()
        .filter(|key| {
            !facts.iter().any(|f| f.id == *key)
                && !group
                    .fused_keys
                    .iter()
                    .any(|k| k.id.trim() == key.trim())
        })
        .collect()
}

/// GBNF-контракт склеенного вызова: think-block + строгий JSON с точными ключами.
///
/// Гибрид (Method 3) — тот же приём, что у `llm_fact_extractor`: think разрешён
/// и НЕ ограничен грамматикой. Подавление размышления — подсказка движка
/// (`disable_reasoning`), бюджет генерации задаёт `max_gen_tokens` из конфига
/// пользователя (как у слоёв воронки — `signals.rs`). Ограничение think
/// в грамматике (300 симв.) существовало 31.10.2026 один день и откатлено
/// решением владельца: перестраховка резала качество мышления на машинах,
/// где бага нет; остаточные риски закрывают сторож живости stderr (плагин)
/// и Phase-2 JSON-only fallback (`orchestrator/mod.rs`).
pub fn build_fused_grammar(
    workflow: &WorkflowDef,
    group: &GroupDef,
    workflow_dir: Option<&std::path::Path>,
) -> String {
    let (bool_keys, string_keys) = output_contract(workflow, group, workflow_dir);
    let base = crate::infra::build_json_object_grammar_with_keys(&bool_keys, &string_keys);
    crate::infra::get_hybrid_grammar(&base)
}

/// Тип ноды, который сам зовёт LLM.
///
/// Только эти ноды заменяются склеенным вызовом группы. Все остальные
/// (`condition_check`, `condition_router`, `signal_router`, `switch`, `note`,
/// `system_condition`) — чистые функции от `signals`: они стоят 0 токенов и
/// ОБЯЗАНЫ выполниться после фьюжинга, иначе маршрутизация потеряет опору.
pub fn node_calls_llm(node: &NodeDef) -> bool {
    use super::parser::NodeType;
    matches!(
        node.node_type,
        NodeType::LlmWorker | NodeType::LlmFreeform | NodeType::LlmFactExtractor
    )
}

/// Один этап авто-склейки: нода группы и роль, которая на ней работает.
#[derive(Debug, Clone, PartialEq)]
pub struct FusedStage {
    /// Квота этапа: ключ выхода, автор публикации и тип сообщения.
    ///
    /// SSOT: строится из тех же нод группы, из которых строится контракт
    /// (`stage_output_specs`), поэтому грамматика, промпт, parse и публикация
    /// не могут разойтись — второго списка этапов не существует.
    pub spec: StageSpec,
    /// Имя агента (или `llm_freeform`, если у ноды свой `input`).
    pub role: String,
    /// Системный промпт роли: содержимое `.md` агента либо текст `input`.
    pub prompt: String,
}

/// Правила вывода ОДНОГО этапа авто-склейки.
///
/// Производная величина от нод группы (`topo_order_of_group` + `agent` +
/// `output_type`): ключ `out_<node_id>` форсируется грамматикой, автора и
/// `msg_type` читает публикация отчётов в `run_fused_group`.
#[derive(Debug, Clone, PartialEq)]
pub struct StageSpec {
    /// Ключ JSON-ответа: `out_<node_id>`.
    pub key: String,
    /// Нода-этап, для диагностики.
    pub node_id: String,
    /// Автор публикации отчёта — id агента ноды. `None` — этап без агента
    /// (`llm_freeform`): его отчёт в сессию не публикуется (как и в detailed).
    pub author: Option<String>,
    /// Тип публикации этапа с агентом: `output_type` ноды или `thought`.
    pub msg_type: String,
}

/// Явная ли fused-группа: у неё собственная инструкция склейки.
///
/// Для таких групп контракт остаётся прежним `{message, ...}` — автор сам
/// объяснил формирование текста, и «канавки этапов» там не производятся.
pub fn is_explicit_fused(group: &GroupDef) -> bool {
    group
        .fused_instruction
        .as_deref()
        .map(str::trim)
        .is_some_and(|i| !i.is_empty())
}

/// Этапные обязательные ключи выхода авто-склейки.
///
/// Один этап = одна нода группы, которая сама зовёт LLM и имеет роль
/// (агент либо собственный `input`). Факт-экстрактор этапом не считается: его
/// выход — не текст, его ключи уже объявлены в факт-контракте. Порядок —
/// топологический, тот же, что и у этапов в склеенном промпте.
pub fn stage_output_specs(workflow: &WorkflowDef, group: &GroupDef) -> Vec<StageSpec> {
    use super::parser::NodeType;
    if is_explicit_fused(group) {
        return Vec::new();
    }
    let ordered = match topo_order_of_group(workflow, group.id.trim()) {
        Ok(ordered) => ordered,
        // Цикл в группе — для сборки промпта всё равно ошибка; здесь честно
        // отдаём пусто, чтобы контракт не выдал провалившейся склейке те же
        // ключи дважды.
        Err(_) => return Vec::new(),
    };
    ordered
        .iter()
        .filter(|n| matches!(n.node_type, NodeType::LlmWorker | NodeType::LlmFreeform))
        .map(|n| StageSpec {
            key: format!("{}{}", STAGE_KEY_PREFIX, n.id),
            node_id: n.id.clone(),
            author: n
                .agent
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(ToOwned::to_owned),
            msg_type: n
                .output_type
                .clone()
                .unwrap_or_else(|| "thought".to_string()),
        })
        .collect()
}

/// Склеить системный промпт макро-ноды из ролей её этапов.
///
/// Автору не нужно переписывать инструкции групп: он объединяет ноды в группу,
/// а движок берёт роль каждой ноды из её агента. Это не «второй источник»:
/// `role`/`prompt` берутся из того же `AgentProfile.system_prompt`, из которого
/// оркестратор строит промпт при обычном вызове.
///
/// Порядок этапов — тот, в котором ноды выполнялись бы по отдельности
/// ([`topo_order_of_group`]), иначе модель получила бы роли задом наперёд.
pub fn build_auto_fused_prompt(
    group: &GroupDef,
    stages: &[FusedStage],
    keys: &[String],
    signals_json: &str,
) -> String {
    let mut prompt = String::from(
        "Тебе предстоит последовательно выполнить задачи цепочки специалистов в рамках одного \
         ответа. Каждый этап — отдельная роль со своим системным промптом; выполни их по порядку, \
         опираясь на результат предыдущего этапа.",
    );
    for (idx, stage) in stages.iter().enumerate() {
        prompt.push_str(&format!(
            "\n\n---\nЭТАП {}: Роль [{} / {}]\n{}\n---",
            idx + 1,
            stage.spec.node_id,
            stage.role,
            stage.prompt.trim()
        ));
    }
    prompt.push_str(
        "\n\nВыполни внутреннюю логику всех этапов последовательно и сформируй итоговый ответ \
         согласно схеме вывода. Не пересказывай этапы и не показывай их служебные рассуждения — \
         пользователю нужен результат.",
    );
    if !group.title.trim().is_empty() {
        prompt.push_str(&format!("\n\n[Группа: {}]", group.title.trim()));
    }
    if !signals_json.trim().is_empty() && signals_json.trim() != "{}" {
        prompt.push_str("\n\n[СИГНАЛЫ И ФАКТЫ ТЕКУЩЕГО ШАГА]\n");
        prompt.push_str(
            "Уже извлечено на этом шаге. Не переизвлекай их и не меняй значения — \
             опирайся на них, они переданы в сообщении пользователя.\n",
        );
        prompt.push_str(signals_json.trim());
    }

    let stage_keys: Vec<&str> = stages.iter().map(|s| s.spec.key.as_str()).collect();
    if !stage_keys.is_empty() {
        // Канавка этапа: полный текст именно этого этапа, а не смесь всех.
        // Пропуск этапа форсируется грамматикой (`out_*` ключи обязательны),
        // но модель ещё и предупреждается: смешивать этапы в одном поле нельзя.
        prompt.push_str("\n\n[РЕЗУЛЬТАТЫ ЭТАПОВ]");
        prompt.push_str(
            "\nДля каждого этапа — своё поле с ПОЛНЫМ готовым ответом этого этапа. \
             Смешивать несколько этапов в одном поле запрещено: каждое поле проходит \
             свою проверку.",
        );
        for (idx, stage) in stages.iter().enumerate() {
            let closing = if stage.spec.msg_type == "message" {
                "; это итоговое сообщение, которое увидит пользователь"
            } else {
                ""
            };
            prompt.push_str(&format!(
                "\n- `{}` — полный ответ этапа {} «{}»{}",
                stage.spec.key,
                idx + 1,
                stage.role,
                closing
            ));
        }
    }

    if keys.iter().any(|k| k == MESSAGE_KEY) {
        if !keys.is_empty() {
            prompt.push_str("\n\n[ОБЯЗАТЕЛЬНЫЕ КЛЮЧИ ОТВЕТА]");
            prompt.push_str(
                "\nКаждый ключ обязателен. Если для ключа нет данных — укажи правдивое \
                 значение по критерию, а не выдумывай.",
            );
            for key in keys {
                if stage_keys.contains(&key.as_str()) {
                    continue;
                }
                prompt.push_str(&format!("\n- {}", key));
            }
        }
        prompt.push_str(&format!(
            "\n- `{}` — текст для пользователя, если в этом шаге есть что сказать; иначе пустая строка.",
            MESSAGE_KEY
        ));
    } else if !keys.is_empty() {
        // Остальные (не этапные, не message) ключи — факты/сигналы: прежние правила.
        prompt.push_str("\n\n[ОБЯЗАТЕЛЬНЫЕ КЛЮЧИ ОТВЕТА]");
        prompt.push_str(
            "\nКаждый ключ обязателен. Если для ключа нет данных — укажи правдивое \
             значение по критерию, а не выдумывай.",
        );
        for key in keys {
            if stage_keys.contains(&key.as_str()) {
                continue;
            }
            prompt.push_str(&format!("\n- {}", key));
        }
    }

    if stage_keys.is_empty() {
        prompt.push_str(
            "\n\n[ФОРМАТ]\nВерни ТОЛЬКО JSON-объект с указанными ключами. \
             Никакого текста вокруг JSON.",
        );
    } else {
        prompt.push_str(
            "\n\n[ФОРМАТ]\nВерни ТОЛЬКО JSON-объект. У каждого этапа — своё поле из \
             раздела [РЕЗУЛЬТАТЫ ЭТАПОВ]; никакой вставки этапов друг в друга.",
        );
    }
    prompt
}

/// Инструкция группы как системный промпт склеенного вызова.
///
/// `fused_agent` НЕ подставляется здесь: его системный промпт строит оркестратор
/// в `run_agent_node`, а подстановка чужого промпта означала бы двойную личность
/// в одном вызове. Пока инструменты не поддержаны, группа с `fused_agent` —
/// это авторское указание на конкретного исполнителя для будущего прохода.
///
/// `signals_json` — факты и сигналы, уже извлечённые В ЭТОМ ХОДУ. Без них
/// склеенный вызов слеп: он видел бы только переписку и был бы обязан заново
/// угадывать то, что `extract_facts` уже посчитал (а «угадал» — значит мог
/// разойтись с роутером по тихому основанию).
pub fn build_fused_prompt(group: &GroupDef, keys: &[String], signals_json: &str) -> String {
    let instruction = group
        .fused_instruction
        .as_deref()
        .unwrap_or_default()
        .trim();
    let mut prompt = String::from(instruction);
    if !group.title.trim().is_empty() {
        prompt.push_str(&format!("\n\n[Группа: {}]", group.title.trim()));
    }
    if !signals_json.trim().is_empty() && signals_json.trim() != "{}" {
        prompt.push_str("\n\n[СИГНАЛЫ И ФАКТЫ ТЕКУЩЕГО ШАГА]\n");
        prompt.push_str(
            "Уже извлечено на этом шаге. Не переизвлекай их и не меняй значения — \
             опирайся на них, они переданы в сообщении пользователя.\n",
        );
        prompt.push_str(signals_json.trim());
    }
    if !keys.is_empty() {
        prompt.push_str("\n\n[ОБЯЗАТЕЛЬНЫЕ КЛЮЧИ ОТВЕТА]");
        prompt.push_str(
            "\nКаждый ключ обязателен. Если для ключа нет данных — укажи правдивое \
             значение по критерию, а не выдумывай.",
        );
        for key in keys {
            prompt.push_str(&format!("\n- {}", key));
        }
    }
    prompt.push_str(
        "\n\n[ФОРМАТ]\nВерни ТОЛЬКО JSON-объект с указанными ключами. \
         Никакого текста вокруг JSON.",
    );
    prompt.push_str(&format!(
        "\n- `{}` — текст для пользователя, если в этом шаге есть что сказать; иначе пустая строка.",
        MESSAGE_KEY
    ));
    prompt
}

/// Ограничения режима, которые исполнятель проверяет ДО вызова модели.
///
/// Инструменты в фьюжене — отдельный механизм (tools-цикл + вердит-проход под
/// грамматикой), и в llama.cpp tools и GBNF взаимоисключающи. Молча вызвать
/// модель без инструментов, объявленных автором группы, означало бы сообщить
/// пользователю «схлопнуто», проигнорировав его же `tools:`.
pub fn ensure_supported(group: &GroupDef) -> Result<(), String> {
    if !group.tools.is_empty() {
        return Err(format!(
            "группа '{}': fused-вызов с инструментами ({}) ещё не поддержан — \
             инструменты требуют отдельного прохода native-tools, а он несовместим с \
             грамматикой JSON-ответа. Работает только режим Detailed.",
            group.id,
            group.tools.join(", ")
        ));
    }
    Ok(())
}

/// Можно ли выполнить эту группу как ОДИН макро-вызов.
///
/// Проверки до вызова модели, а не после: «получилось не то» — худший вид отказа,
/// потому что выглядит как работающая система. Каждый отказ называет, что
/// именно автор должен изменить.
pub fn ensure_macro_possible(workflow: &WorkflowDef, group: &GroupDef) -> Result<(), String> {
    ensure_supported(group)?;

    let routers = routers_of_group(workflow, group.id.trim());
    if !routers.is_empty() {
        let names: Vec<&str> = routers.iter().map(|n| n.id.as_str()).collect();
        let keys = routing_keys_of_group(workflow, group.id.trim());
        let keys_hint = if keys.is_empty() {
            String::new()
        } else {
            format!(" Он читает сигналы: {}.", keys.join(", "))
        };
        return Err(format!(
            "группа '{}' содержит детерминированный роутер ({}): макро-вызов выполняет всю \
             группу одним запросом и не принимает решения о ветвлении.{}. \
             Схлопываемая группа — цепочка нод с одним входом и одним выходом; \
             роутер и читаемые им сигналы вынесите ИЗ группы, а группу оставьте \
             той веткой, которую роутер выбирает.",
            group.id,
            names.join(", "),
            keys_hint
        ));
    }

    if let Err(why) = ensure_single_entry(workflow, group.id.trim()) {
        return Err(why);
    }

    let exit = exit_node_of_group(workflow, group.id.trim())?;
    if exit_has_conditional_edge(workflow, &exit, group.id.trim()) {
        return Err(format!(
            "из выхода группы '{}' (нода '{}') выходит условное ребро (`condition:`/`case:`): \
             у макро-вызова нет `status`/`matched_case`, чтобы его выбрать. \
             Сделайте ребро безусловным или вынесите ветвление в роутер ДО группы.",
            group.id, exit
        ));
    }
    Ok(())
}

/// Разобранный ответ склеенного вызова.
#[derive(Debug, Clone, PartialEq)]
pub struct FusedOutput {
    /// Ключи → значения. Сольются в `context.signals`, откуда их читают роутеры.
    pub signals: Vec<(String, serde_json::Value)>,
    /// Текст для пользователя; заполняется только у ЯВНЫХ `fused_instruction`
    /// групп — у авто-склейки канавки `message` нет, итоговый текст живёт
    /// в этапных полях.
    pub message: String,
    /// Полные тексты этапов в SSOT-порядке (`stage_output_specs`).
    /// Публикует `run_fused_group` в отдельные сообщения по `author`/`msg_type`.
    pub stages: Vec<FusedStageOutput>,
}

/// Текст одного этапа склеенного вызова.
#[derive(Debug, Clone, PartialEq)]
pub struct FusedStageOutput {
    /// Правила этапа (ключ/автор/msg_type) — тот же `StageSpec`, что в грамматике.
    pub spec: StageSpec,
    /// Полный текст этапа.
    pub text: String,
}

/// Разобрать JSON склеенного вызова.
///
/// Пустой/невалидный ответ — ОШИБКА с перечнем ожидаемых ключей, а не «пустые
/// сигналы»: молчаливые all-false сигналы отправляли бы граф по ложной ветке
/// (ровно тот баг, что описана в `build_json_object_grammar_with_keys`).
///
/// Отсутствующий/пустой этапный ключ — тоже ошибка: канавка этапа объявлена
/// грамматикой, значит пропуск этапа — нарушение формы, а не «меньше данных».
pub fn parse_fused_output(
    raw: &str,
    expected: &[String],
    stage_specs: &[StageSpec],
) -> Result<FusedOutput, String> {
    let value: serde_json::Value = serde_json::from_str(raw.trim())
        .map_err(|e| format!("ответ склеенного вызова не JSON: {}", e))?;
    let obj = value
        .as_object()
        .ok_or_else(|| "ответ склеенного вызова — не JSON-объект".to_string())?;

    let mut stages = Vec::with_capacity(stage_specs.len());
    for spec in stage_specs {
        let Some(val) = obj.get(&spec.key) else {
            return Err(format!("в ответе склеенного вызова нет ключей: {}", spec.key));
        };
        let Some(text) = val.as_str() else {
            return Err(format!(
                "ключ '{}' должен быть строкой с полным ответом этапа",
                spec.key
            ));
        };
        let text = text.trim();
        if text.is_empty() {
            return Err(format!(
                "этап '{}' (ключ '{}') пуст — этап обязан вернуть свой полный отчёт",
                spec.node_id, spec.key
            ));
        }
        stages.push(FusedStageOutput {
            spec: spec.clone(),
            text: text.to_string(),
        });
    }

    let missing: Vec<&str> = expected
        .iter()
        .map(|k| k.as_str())
        .filter(|k| !obj.contains_key(*k))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "в ответе склеенного вызова нет ключей: {}",
            missing.join(", ")
        ));
    }

    let is_stage_key = |key: &str| stage_specs.iter().any(|s| s.key == key);
    let mut signals = Vec::new();
    for (key, val) in obj {
        if key == MESSAGE_KEY || is_stage_key(key) {
            continue;
        }
        signals.push((key.clone(), val.clone()));
    }

    let message = obj
        .get(MESSAGE_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    Ok(FusedOutput {
        signals,
        message,
        stages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow_engine::parser::{NodeType, WorkflowConfig};

    fn node(id: &str, node_type: NodeType, group: Option<&str>) -> NodeDef {
        NodeDef {
            id: id.to_string(),
            node_type,
            group: group.map(str::to_string),
            ..Default::default()
        }
    }

    fn group(id: &str, instruction: Option<&str>) -> GroupDef {
        GroupDef {
            id: id.to_string(),
            title: id.to_string(),
            fused_instruction: instruction.map(str::to_string),
            ..Default::default()
        }
    }

    fn workflow(groups: Vec<GroupDef>, nodes: Vec<NodeDef>) -> WorkflowDef {
        WorkflowDef {
            name: "W".to_string(),
            description: None,
            visible: true,
            file_stem: String::new(),
            parent_dir: String::new(),
            config: None,
            entry: None,
            strategy: None,
            groups,
            nodes,
            edges: vec![],
        }
    }

    fn edge(from: &str, to: &str) -> super::super::parser::EdgeDef {
        super::super::parser::EdgeDef {
            from: from.to_string(),
            to: to.to_string(),
            condition: None,
            case: None,
        }
    }

    // ── Форма макро-ноды (вход → цепочка → выход) ───────────────────────────

    /// Цепочка нод группы: выход — та, из которой ребро уходит НАРУЖУ.
    #[test]
    fn exit_of_a_chain_is_the_node_that_leaves_the_group() {
        let mut wf = workflow(
            vec![group("g", Some("сделай"))],
            vec![
                node("a", NodeType::LlmWorker, Some("g")),
                node("b", NodeType::LlmWorker, Some("g")),
            ],
        );
        wf.edges = vec![edge("a", "b"), edge("b", "out")];
        wf.nodes.push(node("out", NodeType::Note, None));

        assert_eq!(exit_node_of_group(&wf, "g").unwrap(), "b");
        assert!(ensure_macro_possible(&wf, &wf.groups[0]).is_ok());
    }

    /// Развилка внутри группы макро-вызов разрешить не может: выхода два.
    #[test]
    fn branching_inside_a_group_has_no_single_exit() {
        let mut wf = workflow(
            vec![group("g", Some("сделай"))],
            vec![
                node("a", NodeType::LlmWorker, Some("g")),
                node("b", NodeType::LlmWorker, Some("g")),
                node("c", NodeType::LlmWorker, Some("g")),
            ],
        );
        wf.edges = vec![edge("a", "b"), edge("a", "c")];

        let err = exit_node_of_group(&wf, "g").expect_err("выход не определён");
        assert!(err.contains("b") && err.contains("c"), "{}: перечислены оба выхода", err);
        assert!(ensure_macro_possible(&wf, &wf.groups[0]).is_err());
    }

    /// Роутер внутри группы — прямой отказ с именем роутера и читаемых ключей.
    #[test]
    fn a_router_inside_a_fusable_group_is_refused_with_names() {
        let mut wf = workflow(
            vec![group("g", Some("сделай"))],
            vec![
                node("gate", NodeType::ConditionCheck, Some("g")),
                node("work", NodeType::LlmWorker, Some("g")),
            ],
        );
        wf.edges = vec![edge("gate", "work")];
        wf.nodes[0].field = Some("triage_complexity".to_string());

        let err = ensure_macro_possible(&wf, &wf.groups[0]).expect_err("роутер внутри группы");
        assert!(err.contains("gate"), "в ошибке нет имени роутера: {}", err);
        assert!(err.contains("triage_complexity"), "в ошибке нет ключа роутера: {}", err);
        assert!(err.contains("вынесите"), "в ошибке нет что делать: {}", err);
    }

    /// Второй внешний вход в группу — макро-узел с двумя входами не имеет смысла.
    #[test]
    fn a_second_entry_from_outside_is_refused() {
        let mut wf = workflow(
            vec![group("g", Some("сделай"))],
            vec![
                node("a", NodeType::LlmWorker, Some("g")),
                node("b", NodeType::LlmWorker, Some("g")),
            ],
        );
        wf.edges = vec![edge("a", "b"), edge("side", "b")];
        wf.nodes.push(node("side", NodeType::LlmWorker, None));

        let err = ensure_macro_possible(&wf, &wf.groups[0]).expect_err("вход не в начало цепочки");
        assert!(err.contains("середину"), "{}", err);

        // Вход в начало цепочки — законная форма.
        wf.edges = vec![edge("a", "b"), edge("side", "a")];
        assert!(ensure_macro_possible(&wf, &wf.groups[0]).is_ok(), "{}", ensure_macro_possible(&wf, &wf.groups[0]).unwrap_err());
    }

    /// Группа из одной ноды с внешним ребром — законный макро-узел.
    #[test]
    fn a_single_node_group_with_one_entry_is_macro_shaped() {
        let mut wf = workflow(
            vec![group("g", Some("сделай"))],
            vec![node("a", NodeType::LlmWorker, Some("g"))],
        );
        wf.edges = vec![edge("side", "a")];
        wf.nodes.push(node("side", NodeType::Note, None));

        assert_eq!(exit_node_of_group(&wf, "g").unwrap(), "a");
        assert!(ensure_macro_possible(&wf, &wf.groups[0]).is_ok());
    }

    /// Условное ребро из выхода выбрать нечем: у макро-ответа нет `status`.
    #[test]
    fn a_conditional_edge_from_the_exit_is_refused() {
        let mut wf = workflow(
            vec![group("g", Some("сделай"))],
            vec![node("a", NodeType::LlmWorker, Some("g"))],
        );
        wf.nodes.push(node("out", NodeType::Note, None));
        let mut conditional = edge("a", "out");
        conditional.case = Some("done".to_string());
        wf.edges = vec![conditional];

        assert!(exit_has_conditional_edge(&wf, "a", "g"));
        let err = ensure_macro_possible(&wf, &wf.groups[0]).expect_err("условное ребро");
        assert!(err.contains("условное ребро"), "{}", err);
    }

    /// Инструменты по-прежнему честный отказ (tools + грамматика несовместимы).
    #[test]
    fn tools_remain_an_explicit_refusal() {
        let mut wf = workflow(
            vec![group("g", Some("сделай"))],
            vec![node("a", NodeType::LlmWorker, Some("g"))],
        );
        wf.groups[0].tools = vec!["ReadSection".to_string()];
        let err = ensure_macro_possible(&wf, &wf.groups[0]).expect_err("инструменты");
        assert!(err.contains("ReadSection"), "{}", err);
    }

    /// Внутренняя нода уже выполненной группы не должна молча выполниться снова.
    #[test]
    fn inner_node_of_an_executed_group_is_detected() {
        let wf = workflow(
            vec![group("g", Some("сделай"))],
            vec![
                node("a", NodeType::LlmWorker, Some("g")),
                node("b", NodeType::LlmWorker, Some("g")),
            ],
        );
        let done: HashSet<String> = ["g".to_string()].into_iter().collect();

        assert!(executed_group_of_node(&wf, &wf.nodes[0], ExecutionStrategy::Fused, &done)
            .is_some());
        // В Detailed та же нода — обычная, никакой ошибки.
        assert!(
            executed_group_of_node(&wf, &wf.nodes[0], ExecutionStrategy::Detailed, &done).is_none()
        );
        assert!(fused_group_for(&wf, &wf.nodes[0], ExecutionStrategy::Fused, &done).is_none());
    }

    /// СИГНАЛЫ ХОДА видны фьюзному вызову — иначе модель переизвлекает факты.
    #[test]
    fn fused_prompt_carries_the_signals_of_this_step() {
        let g = group("g", Some("Ответь пользователю."));
        let prompt = build_fused_prompt(
            &g,
            &[MESSAGE_KEY.to_string()],
            r#"{"has_problem":true,"somatic_presence":"new"}"#,
        );
        assert!(prompt.contains("has_problem"), "факты хода не попали в промпт:\n{}", prompt);
        assert!(prompt.contains("Не переизвлекай"), "нет запрета на переизвлечение:\n{}", prompt);

        // Пустой ход (сигналов ещё нет) — блока быть не должно.
        let empty = build_fused_prompt(&g, &[MESSAGE_KEY.to_string()], "{}");
        assert!(!empty.contains("СИГНАЛЫ И ФАКТЫ"), "пустой блок в промпте:\n{}", empty);
    }

    // ── Стратегия ──────────────────────────────────────────────────────────

    #[test]
    fn default_strategy_is_detailed() {
        assert_eq!(
            parse_strategy(None).unwrap(),
            ExecutionStrategy::Detailed,
            "дефолт обязан быть Detailed: режима auto не существует"
        );
        assert_eq!(parse_strategy(Some("")).unwrap(), ExecutionStrategy::Detailed);
        assert_eq!(
            parse_strategy(Some("  detailed ")).unwrap(),
            ExecutionStrategy::Detailed
        );
    }

    #[test]
    fn fused_strategy_parses_and_auto_is_rejected() {
        assert_eq!(
            parse_strategy(Some("fused")).unwrap(),
            ExecutionStrategy::Fused
        );
        let err = parse_strategy(Some("auto")).expect_err("auto обязан быть отвергнут");
        assert!(err.contains("auto"), "в ошибке нет самого значения: {}", err);
        assert!(parse_strategy(Some("fuzed")).is_err());
    }

    #[test]
    fn strategy_of_reads_workflow_field() {
        let mut wf = workflow(vec![], vec![node("n", NodeType::UserMessage, None)]);
        assert_eq!(strategy_of(&wf).unwrap(), ExecutionStrategy::Detailed);
        wf.strategy = Some("fused".to_string());
        assert_eq!(strategy_of(&wf).unwrap(), ExecutionStrategy::Fused);
    }

    // ── Состав группы ──────────────────────────────────────────────────────

    #[test]
    fn nodes_of_group_selects_by_declared_membership() {
        let wf = workflow(
            vec![group("g1", Some("инструкция")), group("g2", None)],
            vec![
                node("a", NodeType::LlmWorker, Some("g1")),
                node("b", NodeType::ConditionCheck, Some("g1")),
                node("c", NodeType::LlmWorker, Some("g2")),
                node("d", NodeType::LlmWorker, None),
            ],
        );
        let ids: Vec<&str> = nodes_of_group(&wf, "g1").iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        // Fused схлопывает только вызовы LLM; роутеры остаются детерминированными.
        let llm: Vec<&str> = llm_nodes_of_group(&wf, "g1").iter().map(|n| n.id.as_str()).collect();
        assert_eq!(llm, vec!["a"], "condition_check не зовёт LLM и не схлопывается");
    }

    #[test]
    fn duplicate_group_ids_are_rejected_at_index() {
        let wf = workflow(
            vec![group("dup", None), group("dup", None)],
            vec![node("a", NodeType::UserMessage, None)],
        );
        assert!(group_index(&wf).is_err());
    }

    // ── Ключевое правило: что схлопывается ─────────────────────────────────

    #[test]
    fn fused_group_is_not_used_in_detailed_strategy() {
        let wf = workflow(
            vec![group("g1", Some("инструкция"))],
            vec![node("a", NodeType::LlmWorker, Some("g1"))],
        );
        let done = HashSet::new();
        assert!(
            fused_group_for(&wf, &wf.nodes[0], ExecutionStrategy::Detailed, &done).is_none(),
            "в Detailed группа не фьюзится НИКОГДА — это регресс-гейт"
        );
    }

    /// Авто-склейка: автор НЕ пишет инструкцию, движок берёт роли агентов.
    #[test]
    fn group_without_instruction_is_fused_by_stitching_agent_roles() {
        let mut wf = workflow(
            vec![group("g1", None)],
            vec![
                node("a", NodeType::LlmWorker, Some("g1")),
                node("b", NodeType::LlmWorker, Some("g1")),
            ],
        );
        for n in wf.nodes.iter_mut() {
            n.agent = Some(format!("agent_{}", n.id));
        }
        let done = HashSet::new();

        assert!(
            fused_group_for(&wf, &wf.nodes[0], ExecutionStrategy::Fused, &done).is_some(),
            "объединение нод в группу уже делает группу схлопываемой"
        );
    }

    /// Склеивать нечего — группа честно не фьюзится (и не падает).
    #[test]
    fn group_without_instruction_and_without_roles_is_not_fused() {
        let wf = workflow(
            vec![group("g1", None)],
            vec![node("a", NodeType::LlmFreeform, Some("g1"))],
        );
        let done = HashSet::new();

        assert!(fused_group_for(&wf, &wf.nodes[0], ExecutionStrategy::Fused, &done).is_none());
        let err = check_stitchable(&wf, &wf.groups[0]).expect_err("нечем склеивать");
        assert!(err.contains("fused_instruction"), "в ошибке нет подсказки: {}", err);
    }

    /// Этапы склеиваются в порядке ВЫПОЛНЕНИЯ, а не объявления.
    #[test]
    fn stitch_order_is_topological_not_declaration_order() {
        let mut wf = workflow(
            vec![group("g1", None)],
            vec![
                node("second", NodeType::LlmWorker, Some("g1")),
                node("first", NodeType::LlmWorker, Some("g1")),
            ],
        );
        wf.edges = vec![edge("first", "second")];

        let order = topo_order_of_group(&wf, "g1").expect("порядок");
        assert_eq!(
            order.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            vec!["first", "second"]
        );
    }

    /// Цикл внутри группы — топологического порядка не существует.
    #[test]
    fn a_cycle_inside_a_group_has_no_stitch_order() {
        let mut wf = workflow(
            vec![group("g1", None)],
            vec![
                node("a", NodeType::LlmWorker, Some("g1")),
                node("b", NodeType::LlmWorker, Some("g1")),
            ],
        );
        wf.edges = vec![edge("a", "b"), edge("b", "a")];
        let err = topo_order_of_group(&wf, "g1").expect_err("цикл");
        assert!(err.contains("цикл"), "{}", err);
    }

    /// Собранный промпт содержит роль каждого этапа и канавку каждого этапа,
    /// и НЕ обещает `message` (у авто-склейки итоговый текст живёт в этапах).
    #[test]
    fn auto_stitched_prompt_contains_every_stage_role() {
        let stages = vec![
            FusedStage {
                spec: stage_spec("call_decomposer", Some("decomposer"), "thought"),
                role: "Декомпозитор".to_string(),
                prompt: "Разбей запрос на мишени.".to_string(),
            },
            FusedStage {
                spec: stage_spec("call_synthesizer", Some("synthesizer"), "message"),
                role: "Синтезатор".to_string(),
                prompt: "Собери ответ.".to_string(),
            },
        ];
        let keys = vec![
            "out_call_decomposer".to_string(),
            "out_call_synthesizer".to_string(),
        ];
        let prompt = build_auto_fused_prompt(
            &group("g1", None),
            &stages,
            &keys,
            r#"{"has_problem":true}"#,
        );

        assert!(prompt.contains("call_decomposer") && prompt.contains("Декомпозитор"));
        assert!(prompt.contains("call_synthesizer") && prompt.contains("Синтезатор"));
        assert!(prompt.find("Декомпозитор") < prompt.find("Синтезатор"), "этапы переставлены:\n{}", prompt);
        assert!(prompt.contains("has_problem"), "сигналы хода не попали в промпт");
        assert!(
            prompt.contains("[РЕЗУЛЬТАТЫ ЭТАПОВ]"),
            "нет блока этапных канавок:\n{}",
            prompt
        );
        assert!(
            prompt.contains("итоговое сообщение"),
            "у message-этапа не сказано, что он увидит пользователь:\n{}",
            prompt
        );
        assert!(
            !prompt.contains("`message`"),
            "у авто-склейки не должно быть канавки message:\n{}",
            prompt
        );
        assert!(prompt.contains("ТОЛЬКО JSON"), "в промпте нет требования формата");
    }

    #[test]
    fn fused_group_is_used_once_per_turn() {
        let wf = workflow(
            vec![group("g1", Some("инструкция"))],
            vec![
                node("a", NodeType::LlmWorker, Some("g1")),
                node("b", NodeType::LlmWorker, Some("g1")),
            ],
        );
        let mut done = HashSet::new();

        let first = fused_group_for(&wf, &wf.nodes[0], ExecutionStrategy::Fused, &done);
        assert_eq!(first.map(|g| g.id.as_str()), Some("g1"));
        done.insert("g1".to_string());

        assert!(
            fused_group_for(&wf, &wf.nodes[1], ExecutionStrategy::Fused, &done).is_none(),
            "вторая нода той же группы не должна звать LLM повторно"
        );
    }

    #[test]
    fn group_without_instruction_is_never_fused() {
        let wf = workflow(
            vec![group("g1", None)],
            vec![node("a", NodeType::LlmWorker, Some("g1"))],
        );
        let done = HashSet::new();
        assert!(fused_group_for(&wf, &wf.nodes[0], ExecutionStrategy::Fused, &done).is_none());
    }

    #[test]
    fn node_without_group_is_never_fused() {
        let wf = workflow(
            vec![group("g1", Some("инструкция"))],
            vec![node("a", NodeType::LlmWorker, None)],
        );
        let done = HashSet::new();
        assert!(fused_group_for(&wf, &wf.nodes[0], ExecutionStrategy::Fused, &done).is_none());
    }

    #[test]
    fn blank_group_reference_is_treated_as_absent() {
        let wf = workflow(
            vec![group("g1", Some("инструкция"))],
            vec![node("a", NodeType::LlmWorker, Some("  "))],
        );
        let done = HashSet::new();
        assert!(fused_group_for(&wf, &wf.nodes[0], ExecutionStrategy::Fused, &done).is_none());
    }

    // ── Контракт выхода ────────────────────────────────────────────────────

    #[test]
    fn output_contract_includes_facts_group_keys_and_message() {
        let mut wf = workflow(
            vec![GroupDef {
                id: "g1".to_string(),
                fused_keys: vec![
                    FactDef { id: "need_handedness".to_string(), ..Default::default() },
                    FactDef {
                        id: "target_kind".to_string(),
                        values: vec!["one".to_string(), "many".to_string()],
                        ..Default::default()
                    },
                ],
                ..group("g1", Some("инструкция"))
            }],
            vec![node("a", NodeType::LlmFactExtractor, Some("g1"))],
        );
        wf.config = Some(WorkflowConfig {
            facts: vec![
                FactDef { id: "has_problem".to_string(), ..Default::default() },
                FactDef {
                    id: "somatic_presence".to_string(),
                    values: vec!["new".to_string(), "history".to_string()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        });

        let (bools, strings) = output_contract(&wf, &wf.groups[0], None);
        assert!(bools.contains(&"has_problem".to_string()));
        assert!(bools.contains(&"need_handedness".to_string()));
        assert!(strings.contains(&"somatic_presence".to_string()));
        assert!(strings.contains(&"target_kind".to_string()));
        assert!(strings.contains(&MESSAGE_KEY.to_string()));
        assert!(
            !bools.iter().any(|k| strings.contains(k)),
            "ключ не может быть одновременно bool и string"
        );
    }

    #[test]
    fn output_contract_deduplicates_key_declared_twice() {
        let mut wf = workflow(
            vec![GroupDef {
                fused_keys: vec![FactDef { id: "has_problem".to_string(), ..Default::default() }],
                ..group("g1", Some("инструкция"))
            }],
            vec![node("a", NodeType::LlmFactExtractor, Some("g1"))],
        );
        wf.config = Some(WorkflowConfig {
            facts: vec![FactDef { id: "has_problem".to_string(), ..Default::default() }],
            ..Default::default()
        });
        let (bools, _) = output_contract(&wf, &wf.groups[0], None);
        assert_eq!(
            bools.iter().filter(|k| *k == "has_problem").count(),
            1,
            "ключ объявлен и в facts.yaml, и в fused_keys"
        );
    }

    #[test]
    fn grammar_is_built_for_declared_keys() {
        let mut wf = workflow(
            vec![group("g1", Some("инструкция"))],
            vec![node("a", NodeType::LlmFactExtractor, Some("g1"))],
        );
        wf.config = Some(WorkflowConfig {
            facts: vec![FactDef { id: "has_problem".to_string(), ..Default::default() }],
            ..Default::default()
        });
        let grammar = build_fused_grammar(&wf, &wf.groups[0], None);
        assert!(grammar.contains("has_problem"), "грамматика без ключа факта:\n{}", grammar);
        assert!(grammar.contains(MESSAGE_KEY), "грамматика без ключа message:\n{}", grammar);
        assert!(grammar.contains("root ::="), "это не GBNF:\n{}", grammar);
    }

    // ── Этапные канавки авто-склейки ───────────────────────────────────────

    fn agent_node(id: &str, node_type: NodeType, group: Option<&str>, agent: Option<&str>) -> NodeDef {
        NodeDef {
            id: id.to_string(),
            node_type,
            group: group.map(str::to_string),
            agent: agent.map(str::to_string),
            output_type: None,
            ..Default::default()
        }
    }

    /// Этапные канавки — SSOT авто-склейки: топопорядок, авторы, msg_type.
    #[test]
    fn stage_output_specs_are_topological_and_describe_publishing() {
        let mut wf = workflow(
            vec![group("g1", None)],
            vec![
                agent_node("second", NodeType::LlmWorker, Some("g1"), Some("synthesizer")),
                agent_node("first", NodeType::LlmWorker, Some("g1"), Some("decomposer")),
            ],
        );
        wf.edges = vec![edge("first", "second")];
        wf.nodes
            .iter_mut()
            .find(|n| n.id == "second")
            .expect("нода second")
            .output_type = Some("message".to_string());

        let specs = stage_output_specs(&wf, &wf.groups[0]);
        assert_eq!(specs.len(), 2, "каждый этап — своя канавка");
        assert_eq!(specs[0].key, "out_first");
        assert_eq!(specs[0].author.as_deref(), Some("decomposer"));
        assert_eq!(specs[0].msg_type, "thought", "без output_type этап — внутренний отчёт");
        assert_eq!(specs[1].key, "out_second");
        assert_eq!(specs[1].author.as_deref(), Some("synthesizer"));
        assert_eq!(specs[1].msg_type, "message", "output_type ноды задаёт тип публикации");
    }

    /// Явная `fused_instruction`-группа канавок этапов не создаёт: автор сам
    /// объяснил формирование текста.
    #[test]
    fn explicit_group_has_no_stage_specs() {
        let wf = workflow(
            vec![group("g1", Some("сделай всё одним словом"))],
            vec![agent_node("a", NodeType::LlmWorker, Some("g1"), Some("x"))],
        );
        assert!(stage_output_specs(&wf, &wf.groups[0]).is_empty());
    }

    /// Контракт авто-группы требует КАЖДЫЙ этап и не требует message:
    /// пропуск этапа должен быть невозможен форматом, а `message` для юзера
    /// берётся из этапа, у которого `output_type: message`.
    #[test]
    fn auto_contract_requires_stage_keys_and_no_message() {
        let wf = workflow(
            vec![group("g1", None)],
            vec![
                agent_node("call_decomposer", NodeType::LlmWorker, Some("g1"), Some("decomposer")),
                agent_node("call_synthesizer", NodeType::LlmWorker, Some("g1"), Some("synthesizer")),
            ],
        );
        let (_, strings) = output_contract(&wf, &wf.groups[0], None);
        assert!(strings.contains(&"out_call_decomposer".to_string()), "{}", strings.join(","));
        assert!(strings.contains(&"out_call_synthesizer".to_string()), "{}", strings.join(","));
        assert!(
            !strings.iter().any(|k| k == MESSAGE_KEY),
            "у авто-группы не должно быть канавки message: {:?}",
            strings
        );
        // Грамматика собирается из того же контракта — этапные ключи в ней.
        let grammar = build_fused_grammar(&wf, &wf.groups[0], None);
        assert!(grammar.contains("out_call_decomposer"), "{}", grammar);
        assert!(grammar.contains("out_call_synthesizer"), "{}", grammar);
        assert!(
            !grammar.contains(r#" "message" "#),
            "грамматика авто-группы не должна объявлять message:\n{}",
            grammar
        );
    }

    // ── Разбор ответа ─────────────────────────────────────────────────────

    #[test]
    fn parse_fused_output_splits_signals_and_message() {
        let raw = r#"{"has_problem": true, "message": "  текст юзеру  "}"#;
        let out = parse_fused_output(raw, &["has_problem".to_string()], &[]).unwrap();
        assert_eq!(out.message, "текст юзеру", "сообщение должно бытьtrimнутым");
        assert_eq!(out.signals.len(), 1);
        assert_eq!(out.signals[0].0, "has_problem");
        assert_eq!(out.signals[0].1, serde_json::json!(true));
        assert!(out.stages.is_empty(), "stage_specs пуст — этапов нет");
    }

    #[test]
    fn parse_fused_output_reports_missing_keys_instead_of_empty_signals() {
        // Молчаливые all-false сигналы отправили бы граф по ложной ветке.
        let err = parse_fused_output(r#"{"message": ""}"#, &["has_problem".to_string()], &[])
            .expect_err("пустой ответ обязан быть ошибкой");
        assert!(err.contains("has_problem"), "в ошибке нет списка ключей: {}", err);
    }

    #[test]
    fn parse_fused_output_rejects_non_json_and_non_object() {
        assert!(parse_fused_output("просто текст", &[], &[]).is_err());
        assert!(parse_fused_output("[1,2]", &[], &[]).is_err());
    }

    #[test]
    fn message_only_answer_is_allowed_when_no_keys_expected() {
        let out = parse_fused_output(r#"{"message": "готово"}"#, &[], &[]).unwrap();
        assert_eq!(out.message, "готово");
        assert!(out.signals.is_empty());
        assert!(out.stages.is_empty());
    }

    fn stage_spec(node_id: &str, agent: Option<&str>, msg_type: &str) -> StageSpec {
        StageSpec {
            key: format!("{}{}", STAGE_KEY_PREFIX, node_id),
            node_id: node_id.to_string(),
            author: agent.map(ToOwned::to_owned),
            msg_type: msg_type.to_string(),
        }
    }

    #[test]
    fn parse_fused_output_collects_stages_in_ssot_order_and_keeps_signals_separate() {
        let specs = vec![
            stage_spec("call_decomposer", Some("decomposer"), "thought"),
            stage_spec("call_synthesizer", Some("synthesizer"), "message"),
        ];
        let raw = r#"{"out_call_decomposer": "разбор", "out_call_synthesizer": "итог", "has_problem": true}"#;
        let expected = vec![
            "has_problem".to_string(),
            "out_call_decomposer".to_string(),
            "out_call_synthesizer".to_string(),
        ];
        let out = parse_fused_output(raw, &expected, &specs).unwrap();
        assert_eq!(out.stages.len(), 2);
        assert_eq!(out.stages[0].spec.node_id, "call_decomposer");
        assert_eq!(out.stages[0].text, "разбор");
        assert_eq!(out.stages[1].spec.node_id, "call_synthesizer");
        assert_eq!(out.stages[1].text, "итог");
        // Этапные ключи НЕ попадают в signals (иначе роутерам пришёл бы мусор).
        assert_eq!(out.signals.len(), 1);
        assert_eq!(out.signals[0].0, "has_problem");
        assert!(out.message.is_empty(), "у авто-группы канавки message нет");
    }

    #[test]
    fn parse_fused_output_fails_when_a_stage_key_is_missing_or_empty() {
        let specs = vec![
            stage_spec("call_decomposer", Some("decomposer"), "thought"),
            stage_spec("call_synthesizer", Some("synthesizer"), "message"),
        ];
        // Все этапы на месте — разбор успешен.
        let ok = r#"{"out_call_decomposer": "разбор", "out_call_synthesizer": "итог"}"#;
        assert!(
            parse_fused_output(
                ok,
                &["out_call_decomposer".to_string(), "out_call_synthesizer".to_string()],
                &specs,
            )
            .is_ok(),
            "оба этапа доставлены — пропуска нет"
        );

        // Пропущенный ключ этапа — форма, которой не стало быть можно.
        let no = r#"{"out_call_decomposer": "разбор", "hidden": true}"#;
        let err = match parse_fused_output(
            no,
            &["out_call_decomposer".to_string(), "out_call_synthesizer".to_string()],
            &specs,
        ) {
            Ok(_) => panic!("пропуск этапа обязан быть ошибкой"),
            Err(e) => e,
        };
        assert!(err.contains("out_call_synthesizer"), "в ошибке нет пропущенного этапа: {}", err);

        // Пустая строка — тот же пропуск, только замаскированный.
        let empty = r#"{"out_call_decomposer": "разбор", "out_call_synthesizer": "   "}"#;
        let err = match parse_fused_output(
            empty,
            &["out_call_decomposer".to_string(), "out_call_synthesizer".to_string()],
            &specs,
        ) {
            Ok(_) => panic!("пустой этап обязан быть ошибкой"),
            Err(e) => e,
        };
        assert!(err.contains("call_synthesizer"), "в ошибке нет имени этапа: {}", err);
    }

    // ── Контракт не должен требовать от группы чужого ──────────────────────

    #[test]
    fn facts_are_required_only_from_the_group_with_extractor() {
        // Фаза исцеления не должна возвращать `has_problem`: заставлять модель
        // заполнять нерелевантные поля значит провоцировать выдумывание.
        let mut wf = workflow(
            vec![
                group("intake", Some("извлеки факты")),
                group("healing", Some("подбери паттерн")),
            ],
            vec![
                node("extract_facts", NodeType::LlmFactExtractor, Some("intake")),
                node("check_has_problem", NodeType::ConditionCheck, Some("intake")),
                node("call_healer", NodeType::LlmWorker, Some("healing")),
            ],
        );
        wf.config = Some(WorkflowConfig {
            facts: vec![
                FactDef { id: "has_problem".to_string(), ..Default::default() },
                FactDef {
                    id: "somatic_presence".to_string(),
                    values: vec!["new".to_string()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        });

        let intake = output_contract(&wf, &wf.groups[0], None);
        assert!(intake.0.contains(&"has_problem".to_string()));

        let healing = output_contract(&wf, &wf.groups[1], None);
        assert!(
            !healing.0.contains(&"has_problem".to_string()),
            "группа без экстрактора не обязана отдавать факты графа: {:?}",
            healing.0
        );
        assert_eq!(healing.1, vec![MESSAGE_KEY.to_string()]);
    }

    /// КЛЮЧЕВОЙ ГЕЙТ: роутер группы читает `triage_complexity`, который уже
    /// извлёк `extract_facts` в этом же ходу. Склеенный вызов НЕ обязан (и не
    /// должен) возвращать его снова — иначе два писателя одного ключа, причём
    /// фьюзный без enum-ограничения, затирает значение экстрактора.
    #[test]
    fn a_key_already_produced_by_the_extractor_is_not_re_demanded() {
        let mut wf = workflow(
            vec![
                group("intake", Some("извлеки факты")),
                group("cluster_work", Some("разбери запрос на мишени")),
            ],
            vec![
                node("extract_facts", NodeType::LlmFactExtractor, Some("intake")),
                node("check_triage", NodeType::ConditionRouter, Some("cluster_work")),
                node("call_decomposer", NodeType::LlmWorker, Some("cluster_work")),
            ],
        );
        wf.nodes[1].field = Some("triage_complexity".to_string());
        wf.config = Some(WorkflowConfig {
            facts: vec![FactDef {
                id: "triage_complexity".to_string(),
                values: vec!["specific".to_string(), "cluster".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        });

        let cluster = output_contract(&wf, &wf.groups[1], None);
        assert!(
            !cluster.0.contains(&"triage_complexity".to_string())
                && !cluster.1.contains(&"triage_complexity".to_string()),
            "ключ роутера попал в контракт группы: {:?} / {:?}",
            cluster.0,
            cluster.1
        );
        assert_eq!(cluster.1, vec![MESSAGE_KEY.to_string()]);

        // Роутеры группы при этом всё ещё видны для ДИАГНОСТИКИ — функция живая.
        assert_eq!(routing_keys_of_group(&wf, "cluster_work"), vec!["triage_complexity".to_string()]);
    }

    #[test]
    fn routing_keys_are_derived_from_the_groups_own_routers() {
        let mut wf = workflow(
            vec![group("g", Some("сделай"))],
            vec![
                node("checker", NodeType::ConditionCheck, Some("g")),
                node("router", NodeType::SignalRouter, Some("g")),
                node("foreign", NodeType::ConditionCheck, Some("other")),
            ],
        );
        wf.nodes[0].field = Some("has_problem".to_string());
        wf.nodes[1].signal_name = Some("focus_keeper".to_string());
        wf.nodes[1].cases_priority = Some(vec![super::super::parser::PriorityCase {
            key: "Кластер".to_string(),
            to: "call_decomposer".to_string(),
        }]);
        wf.nodes[2].field = Some("чужой_ключ".to_string());

        let keys = routing_keys_of_group(&wf, "g");
        assert!(keys.contains(&"has_problem".to_string()));
        assert!(keys.contains(&"Кластер".to_string()));
        assert!(
            !keys.contains(&"чужой_ключ".to_string()),
            "роутер чужой группы не влияет на контракт: {:?}",
            keys
        );
    }

    #[test]
    fn dotted_condition_field_reduces_to_its_signal_root() {
        // `validator_report.e3` — путь до поля объекта-сигнала; в грамматику
        // должен попасть сам объект, иначе модель вернёт строку вместо объекта.
        let mut wf = workflow(
            vec![group("g", Some("сделай"))],
            vec![node("router", NodeType::ConditionRouter, Some("g"))],
        );
        wf.nodes[0].conditions = vec![super::super::parser::ConditionNode::Rule {
            field: "validator_report.e3".to_string(),
            equals: serde_json::json!(false),
        }];
        assert_eq!(routing_keys_of_group(&wf, "g"), vec!["validator_report".to_string()]);
    }

    #[test]
    fn undeclared_routing_keys_are_reported_to_the_author() {
        let mut wf = workflow(
            vec![group("g", Some("сделай"))],
            vec![node("checker", NodeType::ConditionCheck, Some("g"))],
        );
        wf.nodes[0].field = Some("мистический_сигнал".to_string());

        assert_eq!(
            undeclared_routing_keys_with_facts(&wf, &wf.groups[0], &[]),
            vec!["мистический_сигнал".to_string()]
        );

        // Объявили в fused_keys — предупреждение исчезает.
        wf.groups[0].fused_keys = vec![FactDef {
            id: "мистический_сигнал".to_string(),
            ..Default::default()
        }];
        assert!(undeclared_routing_keys_with_facts(&wf, &wf.groups[0], &[]).is_empty());
    }

    // ── Промпт и ограничения режима ───────────────────────────────────────

    #[test]
    fn node_calls_llm_only_for_llm_nodes() {
        assert!(node_calls_llm(&node("a", NodeType::LlmWorker, None)));
        assert!(node_calls_llm(&node("a", NodeType::LlmFreeform, None)));
        assert!(node_calls_llm(&node("a", NodeType::LlmFactExtractor, None)));
        for deterministic in [
            NodeType::ConditionCheck,
            NodeType::ConditionRouter,
            NodeType::SignalRouter,
            NodeType::Switch,
            NodeType::Note,
            NodeType::UserMessage,
        ] {
            let node_type = deterministic.clone();
            assert!(
                !node_calls_llm(&node("a", node_type, None)),
                "{:?} не зовёт LLM и обязан выполниться после фьюжинга",
                deterministic
            );
        }
    }

    #[test]
    fn prompt_contains_instruction_title_and_required_keys() {
        let g = GroupDef {
            title: "Первичный анализ".to_string(),
            ..group("g", Some("Извлеки факты."))
        };
        let prompt = build_fused_prompt(&g, &["has_problem".to_string(), MESSAGE_KEY.to_string()], "{}");
        assert!(prompt.contains("Извлеки факты."));
        assert!(prompt.contains("Первичный анализ"));
        assert!(prompt.contains("has_problem"));
        assert!(prompt.contains("JSON"));
    }

    #[test]
    fn group_with_tools_is_rejected_honestly_in_fused() {
        let g = GroupDef {
            tools: vec!["ReadSection".to_string()],
            ..group("g", Some("сделай"))
        };
        let err = ensure_supported(&g).expect_err("инструменты в фьюжине не поддержаны");
        assert!(err.contains("ReadSection"), "в ошибке нет имён инструментов: {}", err);
        assert!(err.contains("Detailed"), "в ошибке нет подсказки, что делать: {}", err);

        assert!(ensure_supported(&group("g", Some("сделай"))).is_ok());
    }
}