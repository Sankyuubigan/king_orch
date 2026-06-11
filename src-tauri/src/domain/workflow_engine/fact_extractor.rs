use crate::domain::workflow_engine::parser::{FactDef, WorkflowConfig};

pub fn build_extractor_prompt(config: &WorkflowConfig, user_message: &str) -> String {
    if let Some(ref custom_prompt) = config.extractor_prompt {
        let facts_list = build_facts_list(&config.facts);
        let result = custom_prompt.replace("{{ facts }}", &facts_list);
        return result.replace("{{ user_message }}", user_message);
    }

    build_default_prompt(&config.facts, user_message)
}

fn build_facts_list(facts: &[FactDef]) -> String {
    facts
        .iter()
        .map(|f| {
            let mut entry = format!("- \"{}\": {}", f.id, f.description);
            if let Some(ref criteria) = f.criteria {
                entry.push_str(&format!("\n  Критерии: {}", criteria));
            }
            entry
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_default_prompt(facts: &[FactDef], user_message: &str) -> String {
    let facts_list = build_facts_list(facts);

    format!(
        r#"Ты — системный анализатор. Прочитай сообщение пользователя.
Определи, присутствуют ли в нем следующие факты/смыслы.
Ответь ТОЛЬКО JSON-объектом, где ключ — ID факта, значение — true или false.

Факты:
{facts_list}

Формат ответа:
{{"fact_id": true, "fact_id2": false}}

Сообщение пользователя:
{user_message}

Ответь ТОЛЬКО JSON, без пояснений."#,
        facts_list = facts_list,
        user_message = user_message
    )
}
