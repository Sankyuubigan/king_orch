use crate::domain::workflow_engine::parser::StatusDef;

/// Возвращает системный промпт для встроенного intent-классификатора.
/// Статусы инжектятся runtime из YAML workflow (config.statuses).
pub fn build_classifier_prompt(statuses: &[StatusDef], user_message: &str) -> String {
    let status_list: String = statuses
        .iter()
        .map(|s| format!("- \"{}\": {}", s.id, s.description))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"Ты — системный анализатор. Прочитай сообщение пользователя.
Твоя единственная задача — определить текущее состояние и выдать ответ СТРОГО в JSON.

Доступные статусы:
{status_list}

Формат ответа (всегда JSON):
{{"status": "<твой выбор>"}}

Если статус "one_problem_incomplete" — добавь поле missing_points:
{{"status": "one_problem_incomplete", "missing_points": ["контекст", "желание", "адаптация"]}}

Сообщение пользователя:
{user_message}

Ответь ТОЛЬКО JSON, без пояснений."#,
        status_list = status_list,
        user_message = user_message
    )
}
