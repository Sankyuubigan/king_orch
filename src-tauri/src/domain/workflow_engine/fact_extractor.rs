use std::fs;
use std::path::Path;

use crate::domain::workflow_engine::parser::{FactDef, FactsFile, OutputFieldDef, WorkflowConfig};

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
    let output_fields = resolve_output_fields(config, workflow_dir);
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

    build_default_prompt(&facts, &phases, &output_fields, user_message, signals)
}

/// Ожидаемые ключи выхода экстрактора: id boolean-фактов + строковые поля + фаза.
/// Это контракт, из которого строятся и промпт, и грамматика, и валидация.
pub fn expected_output_keys(config: &WorkflowConfig, workflow_dir: Option<&Path>) -> Vec<String> {
    let facts = resolve_facts(config, workflow_dir);
    let output_fields = resolve_output_fields(config, workflow_dir);
    let phases = resolve_phases(config, workflow_dir);
    let mut keys: Vec<String> = facts.iter().map(|f| f.id.clone()).collect();
    keys.extend(output_fields.iter().map(|f| f.id.clone()));
    if !phases.is_empty() {
        keys.push("phase".to_string());
    }
    keys
}

/// Строгая GBNF-грамматика по контракту из facts.yaml (точные ключи, без опций).
pub fn build_facts_grammar(config: &WorkflowConfig, workflow_dir: Option<&Path>) -> String {
    let facts = resolve_facts(config, workflow_dir);
    let output_fields = resolve_output_fields(config, workflow_dir);
    let phases = resolve_phases(config, workflow_dir);
    let bool_keys: Vec<String> = facts.iter().map(|f| f.id.clone()).collect();
    let mut string_keys: Vec<String> = output_fields
        .iter()
        .filter(|f| f.field_type != "boolean")
        .map(|f| f.id.clone())
        .collect();
    if !phases.is_empty() {
        string_keys.push("phase".to_string());
    }
    crate::infra::build_json_object_grammar_with_keys(&bool_keys, &string_keys)
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

fn resolve_output_fields(config: &WorkflowConfig, workflow_dir: Option<&Path>) -> Vec<OutputFieldDef> {
    if let Some(ref facts_file) = config.facts_file {
        if let Some(dir) = workflow_dir {
            let ext_path = dir.join(facts_file);
            if let Ok(content) = fs::read_to_string(&ext_path) {
                if let Ok(ext) = serde_yaml::from_str::<FactsFile>(&content) {
                    return ext.output_fields;
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

fn build_default_prompt(facts: &[FactDef], phases: &[FactDef], output_fields: &[OutputFieldDef], user_message: &str, signals: &str) -> String {
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
    for f in output_fields {
        let t = if f.field_type == "boolean" { "boolean" } else { "string" };
        keys.push(format!("\"{}\": {}", f.id, t));
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
    use crate::infra::{LlamaEngine, ModelParams, LlmMessage};
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
        let user_msg = "User: наблюдаю сниженное настроение и упадок сил, интерес к привычным занятиям пропал, ничего не радует. ощущение безысходности и подавленности без видимой внешней причины.
Session signals: []";

        let prompt = build_default_prompt(&facts, &phases, &[], user_msg, signals);

        // Print the full prompt for inspection
        println!("=== PROMPT ({}) ===", prompt.len());
        println!("{}", prompt);
        println!("=== END PROMPT ===");

        let engine_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .map(|d| crate::infra::llamacpp_installer::default_dir(&d))
            .unwrap_or_else(std::path::PathBuf::new);
        let engine = LlamaEngine::new(&engine_dir, &model_path, 8192, false, false, 0, &|_| {}, |_| {}).unwrap();

        let msgs = vec![
            LlmMessage {
                role: "system".to_string(),
                content: prompt,
            },
            LlmMessage {
                role: "user".to_string(),
                content: user_msg.to_string(),
            },
        ];

        let cancel = Arc::new(AtomicBool::new(false));
        let gen = engine
            .generate_chat(
                &msgs,
                256,
                &ModelParams::default(),
                "Auto",
                cancel,
                "test:fact_extractor",
                |_, _| {},
                |_| {},
            )
            .unwrap();
        let response = gen.text;

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

    #[test]
    #[ignore]
    fn test_extractor_has_somatic_false_when_no_new_somatic_in_history() {
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

        // Имитируем реальный вход узла extract_facts: текущее сообщение (User:) +
        // история сессии (Session history:). В истории УЖЕ есть соматика (msg_0),
        // а текущее сообщение — лишь ответ на сценарии, без НОВЫХ телесных жалоб.
        let history = r#"[{"type":"message","author":"user","content":"периодически болит голова с одной стороны и напряжена шея, бывает дискомфорт в области живота по утрам."},{"type":"message","author":"system","content":"Предложены сценарии проработки."}]"#;
        let user_msg = format!(
            "User: сценарий 1 не подходит. предпочитаю действовать самостоятельно.\nсценарий 2 не про моё.\nсценарий 3 мимо.\nответы:\n1. близких родственников нет в живых.\n2. затрудняюсь ответить.\n3. возможно, сложно принять состояние своего здоровья.\n\nSession history (последние сообщения сессии): {}",
            history
        );

        let prompt = build_default_prompt(&facts, &phases, &[], &user_msg, signals);

        println!("=== PROMPT ({}) ===", prompt.len());
        println!("{}", prompt);
        println!("=== END PROMPT ===");

        let engine_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .map(|d| crate::infra::llamacpp_installer::default_dir(&d))
            .unwrap_or_else(std::path::PathBuf::new);
        let engine = LlamaEngine::new(&engine_dir, &model_path, 8192, false, false, 0, &|_| {}, |_| {}).unwrap();

        let msgs = vec![
            LlmMessage {
                role: "system".to_string(),
                content: prompt,
            },
            LlmMessage {
                role: "user".to_string(),
                content: user_msg.clone(),
            },
        ];

        let cancel = Arc::new(AtomicBool::new(false));
        let gen = engine
            .generate_chat(
                &msgs,
                256,
                &ModelParams::default(),
                "Auto",
                cancel,
                "test:fact_extractor",
                |_, _| {},
                |_| {},
            )
            .unwrap();
        let response = gen.text;

        println!("=== RAW RESPONSE ===");
        println!("{}", response);
        println!("=== END RESPONSE ===");

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
            "has_somatic должен быть false, когда в текущем сообщении НЕТ НОВЫХ соматических жалоб (соматика уже была в истории), но получен true"
        );
    }
}