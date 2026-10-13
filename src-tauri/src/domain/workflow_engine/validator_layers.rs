//! Состав и порядок слоёв иерархической проверки 9 элементов.
//!
//! Файл объявления — `agents/<команда>/database/validator_layers.yaml`.
//! Почему он, а не код: порядок слоёв и «кто видит чей результат» — это правила
//! предметной области, а не устройство движка (AGENTS.md §4: бизнес-логика в
//! папке агента, маршрутизация — в графе). Движок и стенд читают одно и то же
//! место, поэтому разойтись они не могут.
//!
//! Что файл НЕ хранит: форму ответа слоя. Она объявлена в
//! `agents/<команда>/signals/root.schema.json` (существующий контракт Method 3),
//! и грамматика строится из него. Состав слоёв здесь продублирован для
//! читаемости, поэтому [`validate_against_contracts`] сверяет его с контрактами:
//! расхождение — ошибка загрузки, а не тихая подмена.
//!
//! Формат:
//!
//! ```yaml
//! version: 1
//! layers:
//!   - agent: validator_surface
//!     elements: [e1, e2, e6, e7]
//!     context_from: []
//!   - agent: validator_facade
//!     elements: [e4, e5, e8, e9]
//!     context_from: [validator_surface]
//! ```
//!
//! `context_from` — агенты, чьи сигналы подставляются в промпт слоя
//! (`inject_response` в графе). Ссылка на слой НИЖЕ по списку запрещена:
//! воронка идёт сверху вниз, иначе слой получил бы результат, который ещё не
//! посчитан.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::domain::signals::SignalContract;

/// Имя файла объявления внутри папки команды.
pub const LAYERS_FILE: &str = "database/validator_layers.yaml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorLayer {
    /// id агента из `agents/<команда>/backend/<agent>.md`.
    pub agent: String,
    /// Идентификаторы элементов, которые слой выносит в свой сигнал.
    pub elements: Vec<String>,
    /// Агенты предыдущих слоёв, отчёты которых попадают в промпт слоя.
    pub context_from: Vec<String>,
}

/// Путь к файлу объявления для папки команды.
pub fn layers_path(team_dir: &Path) -> std::path::PathBuf {
    team_dir.join(LAYERS_FILE)
}

/// Разбор файла объявления + проверки структуры (без чтения контрактов).
pub fn load(team_dir: &Path) -> Result<Vec<ValidatorLayer>, String> {
    let path = layers_path(team_dir);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("не прочитать {}: {}", path.display(), error))?;
    let document: serde_yaml::Value = serde_yaml::from_str(&text)
        .map_err(|error| format!("{} не разобран: {}", path.display(), error))?;

    let raw_layers = document
        .get("layers")
        .and_then(serde_yaml::Value::as_sequence)
        .ok_or_else(|| format!("{}: нет списка layers", path.display()))?;

    let mut layers: Vec<ValidatorLayer> = Vec::with_capacity(raw_layers.len());
    for (index, raw) in raw_layers.iter().enumerate() {
        let where_ = format!("{}: layers[{}]", path.display(), index);
        let agent = raw
            .get("agent")
            .and_then(serde_yaml::Value::as_str)
            .map(str::trim)
            .filter(|agent| !agent.is_empty())
            .ok_or_else(|| format!("{}: слой без поля agent", where_))?
            .to_string();
        let elements = string_list(raw.get("elements"), &format!("{}: elements", where_))?;
        if elements.is_empty() {
            return Err(format!("{}: пустой elements — слой ничего не проверяет", where_));
        }
        let context_from = string_list(raw.get("context_from"), &format!("{}: context_from", where_))?;
        layers.push(ValidatorLayer {
            agent,
            elements,
            context_from,
        });
    }

    if layers.is_empty() {
        return Err(format!("{}: ни одного слоя", path.display()));
    }
    check_structure(&layers, &path.display().to_string())?;
    Ok(layers)
}

fn string_list(value: Option<&serde_yaml::Value>, where_: &str) -> Result<Vec<String>, String> {
    match value {
        None => Ok(Vec::new()),
        Some(serde_yaml::Value::Sequence(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| format!("{}: не строка", where_))
            })
            .collect(),
        Some(_) => Err(format!("{}: ожидался список", where_)),
    }
}

/// Инварианты воронки, не требующие контрактов сигналов.
fn check_structure(layers: &[ValidatorLayer], source: &str) -> Result<(), String> {
    let mut declared: BTreeSet<&str> = BTreeSet::new();
    let mut owner_of: BTreeMap<&str, &str> = BTreeMap::new();

    for (index, layer) in layers.iter().enumerate() {
        if !declared.insert(layer.agent.as_str()) {
            return Err(format!(
                "{}: слой {} объявлен дважды (layers[{}])",
                source, layer.agent, index
            ));
        }
        for element in &layer.elements {
            if let Some(previous) = owner_of.insert(element.as_str(), layer.agent.as_str()) {
                return Err(format!(
                    "{}: элемент {} попал в два слоя ({} и {})",
                    source, element, previous, layer.agent
                ));
            }
        }
    }

    for layer in layers {
        for source_agent in &layer.context_from {
            match declared.iter().find(|declared| **declared == source_agent.as_str()) {
                None => {
                    return Err(format!(
                        "{}: слой {} ссылается на неизвестный слой {}",
                        source, layer.agent, source_agent
                    ))
                }
                Some(_) => {
                    // Ссылка должна вести в уже пройденную часть воронки.
                    let source_index = layers
                        .iter()
                        .position(|other| other.agent == *source_agent)
                        .unwrap_or(usize::MAX);
                    let layer_index = layers
                        .iter()
                        .position(|other| other.agent == layer.agent)
                        .unwrap_or(usize::MAX);
                    if source_index >= layer_index {
                        return Err(format!(
                            "{}: слой {} берёт контекст слоя {}, который идёт не выше (воронка строго сверху вниз)",
                            source, layer.agent, source_agent
                        ));
                    }
                }
            }
        }
    }

    Ok(())
}

/// Сверка объявленного состава слоёв с контрактами сигналов.
///
/// Слой отдаёт ровно те поля, которые требует его контракт: иначе воронка
/// разошлась бы с формой ответа, и стенд мерил бы не то, что поедет в бою.
pub fn validate_against_contracts(
    layers: &[ValidatorLayer],
    contracts: &std::collections::HashMap<String, SignalContract>,
) -> Result<(), String> {
    for layer in layers {
        let contract = contracts.get(&layer.agent).ok_or_else(|| {
            format!(
                "у слоя {} нет контракта сигнала в agents/*/signals/root.schema.json",
                layer.agent
            )
        })?;
        let required = required_bool_keys(contract)?;
        let declared: BTreeSet<&str> = layer.elements.iter().map(String::as_str).collect();
        let from_contract: BTreeSet<&str> = required.iter().map(String::as_str).collect();
        if declared != from_contract {
            return Err(format!(
                "слой {} объявляет {:?}, а контракт сигнала требует {:?}",
                layer.agent, declared, from_contract
            ));
        }
    }
    Ok(())
}

/// Обязательные булевы ключи контракта: ровно те, что помечены `required`
/// и объявлены как `boolean`.
fn required_bool_keys(contract: &SignalContract) -> Result<BTreeSet<String>, String> {
    let schema = &contract.value_schema;
    let required: Vec<&str> = schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|items| items.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    if required.is_empty() {
        return Err(format!("в контракте сигнала '{}' нет required", contract.key));
    }
    let properties = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| format!("в контракте сигнала '{}' нет properties", contract.key))?;
    let mut keys = BTreeSet::new();
    for key in required {
        let property = properties
            .get(key)
            .ok_or_else(|| format!("в контракте '{}' нет поля {}", contract.key, key))?;
        let is_bool = property.get("type").and_then(serde_json::Value::as_str) == Some("boolean");
        if !is_bool {
            return Err(format!(
                "в контракте '{}' поле {} объявлено не boolean",
                contract.key, key
            ));
        }
        keys.insert(key.to_string());
    }
    Ok(keys)
}

/// Все элементы воронки — для сверки полноты с файлом критериев.
pub fn covered_elements(layers: &[ValidatorLayer]) -> BTreeSet<String> {
    layers
        .iter()
        .flat_map(|layer| layer.elements.iter().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::signals::{load_all_signal_contracts, load_signal_contract};

    /// Команда психолога — единственная, у которой есть воронка; путь и папка
    /// берутся из того же места, что и в стенде.
    fn team_dir() -> std::path::PathBuf {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .to_path_buf();
        root.join("agents").join("psychotherapist")
    }

    fn contracts() -> std::collections::HashMap<String, SignalContract> {
        load_all_signal_contracts(&team_dir().join("signals"))
    }

    #[test]
    fn funnel_file_is_readable_and_covers_every_element_once() {
        let layers = load(&team_dir()).expect("файл воронки прочитан");
        assert!(!layers.is_empty());

        let covered = covered_elements(&layers);
        let ids = crate::domain::workflow_engine::system1_rules::load_rules(
            &crate::domain::workflow_engine::system1_rules::resolve_rules_path(
                &team_dir().parent().expect("agents"),
                "psychotherapist/database/element_validation_rules_system1.yaml",
            ),
        )
        .expect("критерии System-1 прочитаны");
        let from_criteria: BTreeSet<String> = ids.iter().map(|rule| rule.id.clone()).collect();

        assert_eq!(
            covered, from_criteria,
            "воронка обязана закрывать ровно те же элементы, что и критерии System-1"
        );
    }

    #[test]
    fn every_layer_matches_its_signal_contract() {
        let layers = load(&team_dir()).expect("файл воронки прочитан");
        validate_against_contracts(&layers, &contracts())
            .expect("состав слоёв совпадает с контрактами сигналов");
    }

    #[test]
    fn layer_agents_exist_with_contracts() {
        let loaded = contracts();
        for layer in load(&team_dir()).expect("файл воронки прочитан") {
            assert!(
                load_signal_contract(&team_dir().join("signals"), &layer.agent).is_some(),
                "у агента {} нет контракта сигнала",
                layer.agent
            );
            assert!(
                loaded.contains_key(&layer.agent),
                "контракт агента {} не загружен",
                layer.agent
            );
        }
    }

    #[test]
    fn backward_context_reference_is_rejected() {
        let layers = vec![
            ValidatorLayer {
                agent: "second".to_string(),
                elements: vec!["e1".to_string()],
                context_from: vec!["first".to_string()],
            },
            ValidatorLayer {
                agent: "first".to_string(),
                elements: vec!["e2".to_string()],
                context_from: Vec::new(),
            },
        ];
        let error = check_structure(&layers, "test").expect_err("ссылка вниз запрещена");
        assert!(error.contains("строго сверху вниз"), "{}", error);
    }

    #[test]
    fn element_in_two_layers_is_rejected() {
        let layers = vec![
            ValidatorLayer {
                agent: "a".to_string(),
                elements: vec!["e1".to_string()],
                context_from: Vec::new(),
            },
            ValidatorLayer {
                agent: "b".to_string(),
                elements: vec!["e1".to_string(), "e2".to_string()],
                context_from: vec!["a".to_string()],
            },
        ];
        let error = check_structure(&layers, "test").expect_err("дубль элемента запрещён");
        assert!(error.contains("два слоя"), "{}", error);
    }

    #[test]
    fn layer_must_match_contract_keys() {
        let layers = vec![ValidatorLayer {
            agent: "validator_surface".to_string(),
            elements: vec!["e1".to_string(), "e3".to_string()],
            context_from: Vec::new(),
        }];
        let error = validate_against_contracts(&layers, &contracts())
            .expect_err("состав не совпадает с контрактом");
        assert!(error.contains("контракт сигнала требует"), "{}", error);
    }
}