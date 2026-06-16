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
    use crate::domain::workflow_engine::parser::FactDef;
    use crate::infra::{ChatMessage, LlamaEngine, ModelParams};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    #[ignore]
    fn test_extractor_has_somatic_false_for_emotional_complaint() {
        let model_path =
            std::env::var("TEST_MODEL_PATH").expect("Set TEST_MODEL_PATH to a GGUF file path");

        let facts = vec![
            FactDef {
                id: "has_somatic".into(),
                criteria: Some(
                    r#"TRUE если: пользователь жалуется на конкретную физическую боль или телесный симптом — "спина болит", "голова раскалывается", "мышцы зажаты", "колено стреляет", "живот крутит", "врач поставил диагноз", "температура", "кашель", "сыпь на коже".
FALSE если: пользователь описывает только эмоциональные/психологические состояния — "тоскливо", "тревога", "апатия", "меланхолия", "потеря смысла", "плохое настроение", "ощущаю себя мертвым", "нет энергии", "депрессия", даже если использует телесные метафоры ("кошки скребут", "ноет на душе", "тяжесть на сердце"). Это эмоциональные симптомы, а не соматика."#
                        .into(),
                ),
            },
            FactDef {
                id: "grounding_dont_exist".into(),
                criteria: Some(
                    r#"Запрос юзера является абстрактным, то есть ни один из данных пунктов не соблюдается:
1. Описан конкретный случай из жизни (кто, где, когда).
2. Детально описано тяжелое текущее физическое или психическое состояние (например: "я в трансе", "тело сковано", "ощущаю себя мертвым"). Само состояние здесь является фактом.
3. Описано конкретное действие, которое человек не может совершить (например: "не могу общаться с девушками", "не могу встать с кровати").
Если текст состоит только из общих фраз, диагнозов, философских рассуждений или жалоб на жизнь в целом ("я неудачник", "у меня прокрастинация", "все люди злые") — запрос АБСТРАКТНЫЙ."#
                        .into(),
                ),
            },
            FactDef {
                id: "user_doesnt_agree".into(),
                criteria: Some(
                    "Пользователь не согласен с ответами команды психотерапии. Юзер не доволен ответами"
                        .into(),
                ),
            },
        ];
        let phases: Vec<FactDef> = vec![
            FactDef {
                id: "data_collection".into(),
                criteria: Some(
                    "Нет сигнала phase, либо последний signals phase сигнал = data_collection"
                        .into(),
                ),
            },
            FactDef {
                id: "datamining".into(),
                criteria: Some("Последний signals phase сигнал = datamining".into()),
            },
            FactDef {
                id: "treatment".into(),
                criteria: Some("Последний signals phase сигнал = treatment".into()),
            },
        ];
        let signals = "[]";
        let user_msg = "User: Я мужчина. У меня состояние медлительное заторможенное трансовое , такое негативное состояние, дискомфорт оно приносит мне. Как будто бы меня затягивает куда-то, какая-то меланхолия без всяких причин, настроения нету. Ощущаю себя мёртвым каким-то. Удовольствия от жизни нету. Я как будто бы не вижу смысла в получении удовольствия. Это странно. По этой причине мне и девушки неинтересны. дискомфорт возникает из за того что нету настроения. как то тоскливо без причины как будто бы кошки скребут ноют внутри.
Session signals: []";

        let prompt = build_default_prompt(&facts, &phases, user_msg, signals);

        // Print the full prompt for inspection
        println!("=== PROMPT ({}) ===", prompt.len());
        println!("{}", prompt);
        println!("=== END PROMPT ===");

        let engine = LlamaEngine::new(&model_path, 8192, false, &|_| {}).unwrap();

        let msgs = vec![
            ChatMessage {
                id: None,
                msg_type: "message".to_string(),
                content: prompt,
                namespace: None,
                sub_calls: None,
                author: Some("system".to_string()),
            },
            ChatMessage {
                id: None,
                msg_type: "message".to_string(),
                content: user_msg.to_string(),
                namespace: None,
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