//! Правила System-1: универсальный парсер файла критериев.
//!
//! # Почему это здесь, а не в `domain/system1.rs`
//!
//! Движок workflow — универсальный рантайм для любых агентов, поэтому он не
//! знает ни про психотерапевта, ни про девять элементов. Файл критериев и
//! набор правил приходят из YAML-узла (`rules_file`), ровно как `facts_file`
//! приходит в `LlmFactExtractor` (`parser::WorkflowConfig::facts_file`).
//!
//! Количество правил и их идентификаторы — данные графа, а не константа движка:
//! сегодня это `e1..e9`, завтра это может быть любой набор.
//!
//! # Где правило решения
//!
//! Порог `p_true >= threshold` применяется здесь, в хосте: порог — свойство
//! критериев конкретного агента, а не механика модели. Сам плагин
//! (`tauri-plugin-system1`) о порогах не знает и возвращает вероятности.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use tauri_plugin_system1::contract::{QuestionType, TypedQuestion};

/// Одно правило System-1 в том виде, как оно записано в YAML.
///
/// `#[serde(default)]` на каждом поле — не лень, а формат файла: правило без
/// `name` или без `false_criteria` обязано читаться, а не ломать разбор всего
/// файла. Проверка обязательных полей ниже, с внятной ошибкой.
#[derive(Debug, Clone, Deserialize)]
struct RawRule {
    #[serde(default)]
    name: String,
    #[serde(default)]
    question: String,
    #[serde(default)]
    true_criteria: String,
    #[serde(default)]
    false_criteria: String,
    #[serde(default)]
    threshold: Option<f32>,
}

/// Правило элемента, готовое к инференсу.
#[derive(Debug, Clone)]
pub struct System1Rule {
    /// Идентификатор правила. Приходит из ключа элемента в YAML, поэтому
    /// плагин возвращает вероятность ровно под этим именем.
    pub id: String,
    /// Имя правила для UI и логов.
    pub name: String,
    /// Формулировка вопроса модели.
    pub question: String,
    /// Текст положительного варианта.
    pub true_criteria: String,
    /// Текст отрицательного варианта.
    pub false_criteria: String,
    /// Порог решения. `None` → [`DEFAULT_THRESHOLD`].
    pub threshold: Option<f32>,
}

/// Порог по умолчанию, если правило его не задало.
pub const DEFAULT_THRESHOLD: f32 = 0.5;

impl System1Rule {
    /// Порог с подстановкой значения по умолчанию.
    pub fn threshold_or_default(&self) -> f32 {
        self.threshold.unwrap_or(DEFAULT_THRESHOLD)
    }

    /// Превратить правило в вопрос для модели.
    ///
    /// Порядок вариантов `[отрицательный, положительный]` обязателен: плагин
    /// читает вероятность положительного из слота 1, и перестановка здесь
    /// тихо инвертировала бы все вердикты.
    pub fn to_typed_question(&self) -> TypedQuestion {
        TypedQuestion {
            id: self.id.clone(),
            qtype: QuestionType::Noul,
            instructions: self.question.clone(),
            options: [self.false_criteria.clone(), self.true_criteria.clone()],
        }
    }
}

/// Формат файла критериев, который понимает этот парсер.
///
/// Принимаются оба варианта, потому что формат исторически менялся, а ломать
/// чужой файл из-за смены записи — это регресс, а не чистота:
/// - `elements` как отображение `1 -> {...}` (текущий боевой формат);
/// - `elements` как список `- id: e1 ...` (явные идентификаторы).
#[derive(Debug, Deserialize)]
struct RuleFile {
    elements: RulesRoot,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RulesRoot {
    Mapping(serde_yaml::Mapping),
    Sequence(Vec<serde_yaml::Mapping>),
}

impl RulesRoot {
    fn len(&self) -> usize {
        match self {
            RulesRoot::Mapping(map) => map.len(),
            RulesRoot::Sequence(list) => list.len(),
        }
    }
}

/// Прочитать правила System-1 из файла критериев.
pub fn load_rules(path: &Path) -> Result<Vec<System1Rule>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("не прочитать правила System-1 {}: {}", path.display(), error))?;
    parse_rules(&text, path)
}

/// Разобрать правила из текста YAML. Вынесено отдельно, чтобы формат проверялся
/// тестами без файловой системы.
pub fn parse_rules(text: &str, origin: &Path) -> Result<Vec<System1Rule>, String> {
    let document: RuleFile = serde_yaml::from_str(text)
        .map_err(|error| format!("правила System-1 {} не разобраны: {}", origin.display(), error))?;

    if document.elements.len() == 0 {
        return Err(format!("в {} нет ни одного правила", origin.display()));
    }

    let mut rules = Vec::with_capacity(document.elements.len());
    for (id, mapping) in enumerate(&document.elements)? {
        let raw: RawRule = serde_yaml::from_value(serde_yaml::Value::Mapping(mapping)).map_err(
            |error| {
                format!(
                    "правило {} в {} не разобран: {}",
                    id,
                    origin.display(),
                    error
                )
            },
        )?;

        if raw.question.trim().is_empty() || raw.true_criteria.trim().is_empty() {
            // Молча пропущенное правило дало бы вердикт «false» без причины —
            // то есть тихую ложь в отчёте. Поэтому это ошибка.
            return Err(format!(
                "правило {} в {}: пустой question или true_criteria",
                id,
                origin.display()
            ));
        }

        rules.push(System1Rule {
            id,
            name: raw.name,
            question: raw.question,
            true_criteria: raw.true_criteria,
            false_criteria: raw.false_criteria,
            threshold: raw.threshold,
        });
    }
    Ok(rules)
}

/// Перечислить пары (идентификатор, тело) для обоих форматов.
///
/// Идентификатор берётся из поля `id` в теле правила, а не из позиции и не из
/// префикса «e»: набор и имена правил — данные графа, а не константа движка.
/// Строковый ключ используется как запасной вариант (формат `elements: {soma: …}`),
/// а числовой ключ без `id` — ошибка: молчаливый id «1» дал бы в отчёте ключи,
/// которых нет в эталоне, то есть тихую ложь вместо внятного отказа.
fn enumerate(root: &RulesRoot) -> Result<Vec<(String, serde_yaml::Mapping)>, String> {
    match root {
        RulesRoot::Mapping(map) => map
            .iter()
            .map(|(key, value)| {
                let mapping = value
                    .as_mapping()
                    .cloned()
                    .ok_or_else(|| format!("элемент {:?} — не отображение полей", key))?;
                let id = match key.as_str() {
                    Some(text) if !text.trim().is_empty() => text.to_string(),
                    _ => match mapping.get(serde_yaml::Value::String("id".into())) {
                        Some(serde_yaml::Value::String(text)) if !text.trim().is_empty() => {
                            text.clone()
                        }
                        _ => {
                            return Err(format!(
                                "элемент {:?} без поля id: укажите `id: <имя>` в теле правила. \
                                 Идентификатор приходит из данных, а не выводится из позиции.",
                                key
                            ))
                        }
                    },
                };
                Ok((id, mapping))
            })
            .collect(),
        RulesRoot::Sequence(list) => list
            .iter()
            .map(|mapping| {
                let id = mapping
                    .get(serde_yaml::Value::String("id".into()))
                    .and_then(serde_yaml::Value::as_str)
                    .map(str::to_string)
                    .filter(|text| !text.trim().is_empty())
                    .ok_or_else(|| "правило в списке без поля id".to_string())?;
                Ok((id, mapping.clone()))
            })
            .collect(),
    }
}

/// Разрешить `rules_file` узла в путь на диске.
///
/// Относительный путь считается от каталога агентов — того самого, что уже
/// лежит на руках у рантайма и который попадает в инсталлер
/// (`tauri.conf.json` → `bundle.resources`). Относительный путь от `cwd`
/// ломается в инсталле точно так же, как уже ломался путь к модели.
pub fn resolve_rules_path(agents_dir: &Path, rules_file: &str) -> PathBuf {
    let candidate = Path::new(rules_file);
    if candidate.is_absolute() {
        return candidate.to_path_buf();
    }
    agents_dir.join(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> PathBuf {
        PathBuf::from("rules.yaml")
    }

    /// Боевой формат: `elements` как отображение `1 -> {...}`, id берётся из тела.
    #[test]
    fn mapping_format_is_supported() {
        let text = "elements:\n  1:\n    id: \"e1\"\n    question: \"Есть?\"\n    true_criteria: \"да\"\n    false_criteria: \"нет\"\n    threshold: 0.36\n";
        let rules = parse_rules(text, &origin()).expect("разобрать");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "e1");
        assert_eq!(rules[0].threshold_or_default(), 0.36);
    }

    /// Строковый ключ — тоже законный идентификатор, `id` в теле не нужен.
    #[test]
    fn string_key_is_used_as_id() {
        let text = "elements:\n  somatika:\n    question: \"Есть?\"\n    true_criteria: \"да\"\n";
        let rules = parse_rules(text, &origin()).expect("разобрать");
        assert_eq!(rules[0].id, "somatika");
    }

    /// Числовой ключ без `id` — ошибка, а не молчаливый id «1»: такого ключа нет
    /// в эталоне, и отчёт тихо разошёлся бы с ожиданием.
    #[test]
    fn numeric_key_without_id_is_an_error() {
        let text = "elements:\n  1:\n    question: \"Есть?\"\n    true_criteria: \"да\"\n";
        let error = parse_rules(text, &origin()).expect_err("без id — ошибка");
        assert!(error.contains("id"), "в ошибке нет совета про id: {}", error);
    }

    /// Явные идентификаторы: количество и имена — данные графа, а не движка.
    #[test]
    fn sequence_format_allows_any_ids_and_count() {
        let text = "elements:\n  - id: e1\n    question: \"Есть?\"\n    true_criteria: \"да\"\n  - id: somatika\n    question: \"Есть?\"\n    true_criteria: \"да\"\n  - id: card\n    question: \"Есть?\"\n    true_criteria: \"да\"\n";
        let rules = parse_rules(text, &origin()).expect("разобрать");
        assert_eq!(rules.len(), 3, "движок не должен требовать девять правил");
        let ids: Vec<&str> = rules.iter().map(|rule| rule.id.as_str()).collect();
        assert_eq!(ids, ["e1", "somatika", "card"]);
    }

    /// Пустой вопрос или положительный критерий — ошибка, а не «false по всем».
    #[test]
    fn empty_criteria_are_rejected_not_defaulted() {
        let text = "elements:\n  e1:\n    question: \"Есть?\"\n";
        let error = parse_rules(text, &origin()).expect_err("пустые критерии обязаны быть ошибкой");
        assert!(
            error.contains("e1"),
            "в ошибке нет идентификатора правила: {}",
            error
        );
    }

    #[test]
    fn empty_file_is_an_error() {
        let error = parse_rules("elements: {}\n", &origin()).expect_err("пусто — ошибка");
        assert!(error.contains("нет ни одного правила"), "{}", error);
    }

    /// Порядок вариантов — инвариант, на котором держится чтение слота 1.
    #[test]
    fn options_are_negative_then_positive() {
        let rule = System1Rule {
            id: "e1".into(),
            name: "Сопротивление".into(),
            question: "Описан?".into(),
            true_criteria: "прямо сказано".into(),
            false_criteria: "жалоба на самочувствие".into(),
            threshold: Some(0.36),
        };
        let question = rule.to_typed_question();
        assert_eq!(question.options[0], "жалоба на самочувствие");
        assert_eq!(question.options[1], "прямо сказано");
    }

    #[test]
    fn missing_threshold_defaults_to_half() {
        let rule = System1Rule {
            id: "e1".into(),
            name: String::new(),
            question: "Описан?".into(),
            true_criteria: "да".into(),
            false_criteria: "нет".into(),
            threshold: None,
        };
        assert_eq!(rule.threshold_or_default(), DEFAULT_THRESHOLD);
    }

    /// Относительный `rules_file` считается от каталога агентов, а не от cwd.
    #[test]
    fn relative_rules_path_resolves_from_agents_dir() {
        let agents = Path::new("/app/agents");
        let resolved = resolve_rules_path(agents, "psychotherapist/database/rules.yaml");
        assert_eq!(
            resolved,
            PathBuf::from("/app/agents/psychotherapist/database/rules.yaml")
        );
    }

    #[test]
    fn absolute_rules_path_is_kept_as_is() {
        let absolute = PathBuf::from("/data/rules.yaml");
        let resolved = resolve_rules_path(Path::new("/app/agents"), absolute.to_str().unwrap());
        assert_eq!(resolved, absolute);
    }

    /// Гейт на РЕАЛЬНОЙ модели: плагин + боевые критерии + тексты кейсов обязаны
    /// дать те же вероятности, что и боевой стенд `tools/laya_probe.py`.
    ///
    /// Тексты берутся из `cases.yaml` — тот же источник, что у стенда. Раньше тест
    /// брал `fixtures/*.md`, а это другой текст, и расхождение в 0.01 p_true
    /// означало бы не порт, а разницу входов.
    ///
    /// Ожидание зафиксировано стендом: 189/234, у task1 промах `e4`, у task2 —
    /// `e6`, то есть 8/9 в каждом. Проверяется число промахов, а не «какой
    /// именно»: выдумывать ожидания нельзя, эталон — `cases.yaml`.
    ///
    /// `#[ignore]`: нужна модель 646 МБ и ONNX Runtime в `KingOrchData`.
    #[test]
    #[ignore = "нужна модель 646 МБ; запускать явно"]
    fn real_model_matches_probe_verdicts() {
        let config = crate::infra::config::load_config_early();
        match config.data_dir {
            Some(dir) if !dir.trim().is_empty() => {
                tauri_plugin_system1::set_data_dir(std::path::PathBuf::from(dir));
            }
            _ => panic!("в app_config нет data_dir — System-1 некуда класть модель"),
        }

        let project_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("у src-tauri есть родитель")
            .to_path_buf();

        let rules = load_rules(
            &project_root
                .join("agents/psychotherapist/database/element_validation_rules_system1.yaml"),
        )
        .expect("прочитать критерии System-1");
        let questions: Vec<TypedQuestion> =
            rules.iter().map(|rule| rule.to_typed_question()).collect();

        let cases_path = project_root.join("test_cases/new_tests_for_validator/cases.yaml");
        let cases_text = std::fs::read_to_string(&cases_path)
            .unwrap_or_else(|error| panic!("прочитать {}: {}", cases_path.display(), error));
        let cases: serde_yaml::Value =
            serde_yaml::from_str(&cases_text).expect("cases.yaml разобран");

        for case_id in ["task1", "task2"] {
            let case = cases
                .get("cases")
                .and_then(serde_yaml::Value::as_sequence)
                .and_then(|list| {
                    list.iter().find(|entry| {
                        entry.get("id").and_then(serde_yaml::Value::as_str) == Some(case_id)
                    })
                })
                .unwrap_or_else(|| panic!("кейс {} не найден", case_id));
            let text = case
                .get("prompt")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or_else(|| panic!("у кейса {} нет prompt", case_id));
            let expected = case
                .get("expected")
                .and_then(serde_yaml::Value::as_mapping)
                .unwrap_or_else(|| panic!("у кейса {} нет expected", case_id));

            let outcome = crate::infra::system1::decide(text, &questions).expect("инференс System-1");

            let mut wrong = Vec::new();
            for rule in &rules {
                let p_true = outcome
                    .probabilities
                    .get(&rule.id)
                    .copied()
                    .unwrap_or_else(|| panic!("нет вероятности для {}", rule.id));
                let verdict = p_true >= rule.threshold_or_default();
                let want = expected
                    .get(serde_yaml::Value::String(rule.id.clone()))
                    .and_then(serde_yaml::Value::as_bool)
                    .unwrap_or_else(|| panic!("нет expected для {}", rule.id));
                if verdict != want {
                    wrong.push(format!("{} (p={:.4})", rule.id, p_true));
                }
            }

            // Стэнд даёт 8/9 на task1 и task2. Если перенос испортил вероятности,
            // промахов станет больше — и это поймает assert.
            assert_eq!(
                wrong.len(),
                1,
                "{}: стенд даёт 8/9, плагин дал {} из {}, расхождения: {:?}",
                case_id,
                rules.len() - wrong.len(),
                rules.len(),
                wrong
            );

            let line = rules
                .iter()
                .map(|rule| {
                    format!(
                        "{}={:.4}",
                        rule.id,
                        outcome.probabilities.get(&rule.id).copied().unwrap_or(-1.0)
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            println!(
                "{}: 8/9 как у стенда, промах {:?}, {} мс (загрузка {} мс) | {}",
                case_id, wrong, outcome.elapsed_ms, outcome.load_ms, line
            );
        }
    }
}