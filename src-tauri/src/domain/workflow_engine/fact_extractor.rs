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
            let mut lines: Vec<&str> = text.lines().collect();
            if lines.len() <= 1 {
                return format!("- \"{}\": {}", f.id, text);
            }
            let first = lines.remove(0);
            let rest: Vec<String> = lines
                .iter()
                .map(|l| {
                    if l.trim().is_empty() {
                        l.to_string()
                    } else {
                        format!("  {}", l)
                    }
                })
                .collect();
            format!("- \"{}\": {}\n{}", f.id, first, rest.join("\n"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_default_prompt(facts: &[FactDef], phases: &[FactDef], user_message: &str, signals: &str) -> String {
    let facts_list = build_list(facts);
    let phases_list = build_list(phases);

    let mut prompt = r#"Ты — системный анализатор. Прочитай сообщение пользователя и сигналы сессии."#
        .to_string();

    if !phases_list.is_empty() {
        prompt.push_str(&format!(
            "\n\n### Фазы (выбери ОДНУ на основе сигналов сессии, а НЕ на основе сообщения)\n{}\n\nПравило выбора фазы: если в сигналах нет поля \"phase\" — ставь \"data_collection\".",
            phases_list
        ));
    }
    if !facts_list.is_empty() {
        prompt.push_str(&format!("\n\n### Факты (true/false, определяй по сообщению пользователя)\n{}", facts_list));
    }

    let signals_trimmed = signals.trim();
    if !signals_trimmed.is_empty() && signals_trimmed != "[]" && signals_trimmed != "null" {
        prompt.push_str(&format!("\n\nСигналы сессии (используй для выбора фазы):\n{}", signals));
    }

    prompt.push_str("\n\nФормат ответа (ТОЛЬКО JSON, без пояснений):\n{");
    let mut keys = Vec::new();
    for f in facts {
        keys.push(format!("\"{}\": boolean", f.id));
    }
    if !phases.is_empty() {
        keys.push("\"phase\": \"название_фазы\"".to_string());
    }
    prompt.push_str(&keys.join(", "));
    prompt.push_str("}\n\nСообщение пользователя:\n");
    prompt.push_str(user_message);
    prompt.push_str("\n\nОтветь ТОЛЬКО JSON, без пояснений.");

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::{ChatMessage, LlamaEngine, ModelParams};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    #[ignore]
    fn test_extractor_has_somatic_false_for_emotional_complaint() {
        let model_path =
            std::env::var("TEST_MODEL_PATH").expect("Set TEST_MODEL_PATH to a GGUF file path");

        // Единый источник правды — реальный facts.yaml. Никаких копий критериев в тесте.
        let workflow_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("agents/psychotherapist/transitions");
        let config = WorkflowConfig {
            facts_file: Some("facts.yaml".into()),
            ..Default::default()
        };
        let facts = resolve_facts(&config, Some(&workflow_dir));
        let phases = resolve_phases(&config, Some(&workflow_dir));
        assert!(!facts.is_empty(), "facts.yaml не загрузился из {:?}", workflow_dir);
        let signals = "[]";
        let user_msg = "User: Я мужчина. У меня состояние медлительное заторможенное трансовое , такое негативное состояние, дискомфорт оно приносит мне. Как будто бы меня затягивает куда-то, какая-то меланхолия без всяких причин, настроения нету. Ощущаю себя мёртвым каким-то. Удовольствия от жизни нету. Я как будто бы не вижу смысла в получении удовольствия. Это странно. По этой причине мне и девушки неинтересны. дискомфорт возникает из за того что нету настроения. как то тоскливо без причины как будто бы кошки скребут ноют внутри.
Session signals: []";

        let prompt = build_default_prompt(&facts, &phases, user_msg, signals);

        // Print the full prompt for inspection
        println!("=== PROMPT ({}) ===", prompt.len());
        println!("{}", prompt);
        println!("=== END PROMPT ===");

        let engine = LlamaEngine::new(&model_path, 8192, false, false, &|_| {}, |_| {}).unwrap();

        let msgs = vec![
            ChatMessage {
                id: None,
                msg_type: "message".to_string(),
                content: prompt,
                sub_calls: None,
                author: Some("system".to_string()),
            },
            ChatMessage {
                id: None,
                msg_type: "message".to_string(),
                content: user_msg.to_string(),
                sub_calls: None,
                author: Some("user".to_string()),
            },
        ];

        let cancel = Arc::new(AtomicBool::new(false));
        let response = engine
            .generate_chat(
                &msgs,
                256,
                &ModelParams::default(),
                "Auto",
                cancel,
                |_, _| {},
                |_| {},
            )
            .unwrap();

        println!("=== RAW RESPONSE ===");
        println!("{}", response);
        println!("=== END RESPONSE ===");

        // Extract JSON from response — find first { and last }
        let cleaned: String = {
            let s = response.trim();
            let start = s.find('{').unwrap_or(0);
            let end = s.rfind('}').map(|i| i + 1).unwrap_or(s.len());
            s[start..end].to_string()
        };

        let parsed: serde_json::Value = serde_json::from_str(&cleaned).unwrap_or_else(|e| {
            panic!("Failed to parse JSON from response '{}': {}", cleaned, e)
        });

        println!("=== PARSED JSON ===");
        println!("{:#}", parsed);
        println!("=== END JSON ===");

        let has_somatic = parsed
            .get("has_somatic")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        assert!(
            !has_somatic,
            "has_somatic должен быть false для чисто эмоциональной жалобы, но получен true"
        );
    }
}