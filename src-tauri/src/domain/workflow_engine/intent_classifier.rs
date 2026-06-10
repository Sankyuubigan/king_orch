use crate::domain::workflow_engine::parser::{StatusDef, WorkflowConfig};

/// Возвращает системный промпт для intent-классификатора.
/// Если в config указан `classifier_prompt` — использует его как есть.
/// Иначе собирает дефолтный промпт из статусов (description + criteria).
pub fn build_classifier_prompt(config: &WorkflowConfig, user_message: &str) -> String {
    if let Some(ref custom_prompt) = config.classifier_prompt {
        let status_list = build_status_list(&config.statuses);
        let result = custom_prompt.replace("{{ statuses }}", &status_list);
        return result.replace("{{ user_message }}", user_message);
    }

    build_default_prompt(&config.statuses, user_message)
}

fn build_status_list(statuses: &[StatusDef]) -> String {
    statuses
        .iter()
        .map(|s| {
            let mut entry = format!("- \"{}\": {}", s.id, s.description);
            if let Some(ref criteria) = s.criteria {
                entry.push_str(&format!("\n  Критерии: {}", criteria));
            }
            entry
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_default_prompt(statuses: &[StatusDef], user_message: &str) -> String {
    let status_list = build_status_list(statuses);

    format!(
        r#"Ты — системный анализатор. Прочитай сообщение пользователя.
Твоя задача — определить текущее состояние по критериям ниже.
Ответь ТОЛЬКО JSON, без пояснений.

Доступные статусы:
{status_list}

Формат ответа:
{{"status": "<статус>"}}

При необходимости можешь добавить любые дополнительные поля в JSON.

Сообщение пользователя:
{user_message}

Ответь ТОЛЬКО JSON, без пояснений."#,
        status_list = status_list,
        user_message = user_message
    )
}
