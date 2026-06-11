use std::fs;
use std::path::Path;

use crate::domain::workflow_engine::parser::{FactDef, FactsFile, WorkflowConfig};

/// Строит промпт для fact-экстрактора.
/// Если `facts` пуст и указан `facts_file` — загружает факты из внешнего файла лениво.
pub fn build_extractor_prompt(
    config: &WorkflowConfig,
    user_message: &str,
    workflow_dir: Option<&Path>,
) -> String {
    let facts = resolve_facts(config, workflow_dir);
    let prompt = resolve_extractor_prompt(config, workflow_dir);

    if let Some(ref custom_prompt) = prompt {
        let facts_list = build_facts_list(&facts);
        let result = custom_prompt.replace("{{ facts }}", &facts_list);
        return result.replace("{{ user_message }}", user_message);
    }

    build_default_prompt(&facts, user_message)
}

fn resolve_facts(config: &WorkflowConfig, workflow_dir: Option<&Path>) -> Vec<FactDef> {
    if !config.facts.is_empty() {
        return config.facts.clone();
    }
    if let Some(ref facts_file) = config.facts_file {
        if let Some(dir) = workflow_dir {
            let ext_path = dir.join(facts_file);
            if let Ok(content) = fs::read_to_string(&ext_path) {
                if let Ok(ext) = serde_yaml::from_str::<FactsFile>(&content) {
                    return ext.facts;
                }
            }
        }
    }
    vec![]
}

fn resolve_extractor_prompt(config: &WorkflowConfig, workflow_dir: Option<&Path>) -> Option<String> {
    if config.extractor_prompt.is_some() {
        return config.extractor_prompt.clone();
    }
    if let Some(ref facts_file) = config.facts_file {
        if let Some(dir) = workflow_dir {
            let ext_path = dir.join(facts_file);
            if let Ok(content) = fs::read_to_string(&ext_path) {
                if let Ok(ext) = serde_yaml::from_str::<FactsFile>(&content) {
                    return ext.extractor_prompt;
                }
            }
        }
    }
    None
}

fn build_facts_list(facts: &[FactDef]) -> String {
    facts
        .iter()
        .map(|f| {
            let text = f.criteria.as_deref().unwrap_or("");
            format!("- \"{}\": {}", f.id, text)
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
