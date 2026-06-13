use std::fs;
use std::path::Path;

use crate::domain::workflow_engine::parser::{FactDef, FactsFile, WorkflowConfig};

/// Строит промпт для fact-экстрактора.
/// Если `facts` пуст и указан `facts_file` — загружает факты из внешнего файла лениво.
pub fn build_extractor_prompt(
    config: &WorkflowConfig,
    user_message: &str,
    signals: &str,
    workflow_dir: Option<&Path>,
) -> String {
    let facts = resolve_facts(config, workflow_dir);
    let phases = resolve_phases(config, workflow_dir);
    let prompt = resolve_extractor_prompt(config, workflow_dir);

    if let Some(ref custom_prompt) = prompt {
        let facts_list = build_list(&facts);
        let phases_list = build_list(&phases);
        let result = custom_prompt
            .replace("{{ facts }}", &facts_list)
            .replace("{{ phases }}", &phases_list)
            .replace("{{ signals }}", signals)
            .replace("{{ user_message }}", user_message);
        return result;
    }

    build_default_prompt(&facts, &phases, user_message, signals)
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

fn resolve_phases(config: &WorkflowConfig, workflow_dir: Option<&Path>) -> Vec<FactDef> {
    if !config.phases.is_empty() {
        return config.phases.clone();
    }
    if let Some(ref facts_file) = config.facts_file {
        if let Some(dir) = workflow_dir {
            let ext_path = dir.join(facts_file);
            if let Ok(content) = fs::read_to_string(&ext_path) {
                if let Ok(ext) = serde_yaml::from_str::<FactsFile>(&content) {
                    return ext.phases;
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

fn build_list(items: &[FactDef]) -> String {
    items
        .iter()
        .map(|f| {
            let text = f.criteria.as_deref().unwrap_or("");
            format!("- \"{}\": {}", f.id, text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_default_prompt(facts: &[FactDef], phases: &[FactDef], user_message: &str, signals: &str) -> String {
    let facts_list = build_list(facts);
    let phases_list = build_list(phases);

    let mut prompt = r#"Ты — системный анализатор. Прочитай сообщение пользователя.
Определи, присутствуют ли в нем следующие факты/смыслы.
Ответь ТОЛЬКО JSON-объектом, где ключ — ID факта, значение — true или false."#
        .to_string();

    if !phases_list.is_empty() {
        prompt.push_str(&format!("\n\nФазы:\n{}", phases_list));
    }
    if !facts_list.is_empty() {
        prompt.push_str(&format!("\n\nФакты:\n{}", facts_list));
    }

    let signals_trimmed = signals.trim();
    if !signals_trimmed.is_empty() && signals_trimmed != "[]" && signals_trimmed != "null" {
        prompt.push_str(&format!("\n\nСигналы сессии:\n{}", signals));
    }

    prompt.push_str(&format!("\n\nФормат ответа:\n{{\"fact_id\": true, \"fact_id2\": false"));
    if !phases_list.is_empty() {
        prompt.push_str(", \"phase\": \"название_фазы\"");
    }
    prompt.push_str("}\n\nСообщение пользователя:\n");
    prompt.push_str(user_message);
    prompt.push_str("\n\nОтветь ТОЛЬКО JSON, без пояснений.");

    prompt
}
