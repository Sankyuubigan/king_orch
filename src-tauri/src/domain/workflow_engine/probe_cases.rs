//! Общий загрузчик эталонных кейсов для стендов `#[ignore]`.
//!
//! Кейсы лежат в YAML рядом с Python-стендами, и читать их оттуда — единственный
//! способ не размножить копии разметки (SSOT, AGENTS.md §2.1). Rust не исполняет
//! `tools/datasets.py`, поэтому список наборов отражён здесь константой; имена
//! совпадают с `datasets.py::CASE_SETS` / `METAPHOR_SENTENCES` / `REPORT_ORDER`.
//!
//! Ни одного кейса в коде движка: только чтение файлов (AGENTS.md §2.9).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// Имя набора → файл кейсов с разметкой 9 элементов.
pub const CASE_SET_FILES: &[(&str, &str)] = &[
    ("somatic_only", "cases_somatic.yaml"),
    ("neurosis_base", "cases.yaml"),
    ("mixed_body_mental", "cases_mixed.yaml"),
];

/// Предложения-метафоры: разметка 9 элементов обязательна (см. `load_metaphor_cases`).
pub const METAPHOR_FILE: &str = "sentences_soma_benchmark.yaml";

/// Порядок наборов в отчёте — как `datasets.py::REPORT_ORDER`.
pub const REPORT_ORDER: &[&str] = &[
    "somatic_only",
    "neurosis_base",
    "mixed_body_mental",
    "body_metaphor",
];

/// Один эталонный кейс.
#[derive(Clone)]
pub struct ProbeCase {
    pub dataset: &'static str,
    pub id: String,
    pub text: String,
    /// Эталон 9 элементов невроза. Разметка обязательна у всех наборов,
    /// включая метафоры.
    pub expected: Option<BTreeMap<String, bool>>,
}

pub fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("корень проекта")
        .to_path_buf()
}

pub fn cases_dir() -> PathBuf {
    project_root().join("test_cases/new_tests_for_validator")
}

/// Папка отчётов — та же, что у Python-стендов (`test/laya_probe/results`).
pub fn results_dir() -> PathBuf {
    project_root().join("test/laya_probe/results")
}

pub fn read_yaml(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("не прочитать {}: {}", path.display(), error))?;
    serde_yaml::from_str(&text).map_err(|error| format!("{} не разобран: {}", path.display(), error))
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Кейсы с разметкой 9 элементов.
pub fn load_marked_cases() -> Result<Vec<ProbeCase>, String> {
    let mut out = Vec::new();
    for (dataset, file) in CASE_SET_FILES {
        let path = cases_dir().join(file);
        let doc = read_yaml(&path)?;
        let cases = doc
            .get("cases")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{}: нет списка cases", path.display()))?;
        for case in cases {
            let id = str_field(case, "id")
                .ok_or_else(|| format!("{}: кейс без id", path.display()))?;
            let text = str_field(case, "prompt")
                .ok_or_else(|| format!("{}: у кейса {} нет prompt", path.display(), id))?;
let expected_object = case
            .get("expected")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{}: у кейса {} нет expected", path.display(), id))?;
        let expected = read_expected(&path, expected_object, &id)?;
        out.push(ProbeCase {
            dataset,
            id,
            text,
            expected: Some(expected),
        });
        }
    }
    Ok(out)
}

/// Предложения-метафор. Разметка 9 элементов обязательна: без неё набор
/// измерял только «нашлось хоть один элемент», что выполняется заливкой `true`
/// во все девять полей (разметка взята из ответа специалиста,
/// `tasks/answer 11.10.26.md` + самокоррекция по триггеру).
pub fn load_metaphor_cases() -> Result<Vec<ProbeCase>, String> {
    let path = cases_dir().join(METAPHOR_FILE);
    let doc = read_yaml(&path)?;
    let mut out = Vec::new();
    for sentence in doc
        .get("sentences")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{}: нет списка sentences", path.display()))?
    {
        if str_field(sentence, "category").as_deref() != Some("metaphor_mental") {
            continue;
        }
        let id = str_field(sentence, "id").unwrap_or_default();
        let text = str_field(sentence, "text")
            .ok_or_else(|| format!("{}: у предложения {} нет text", path.display(), id))?;
        let expected_object = sentence
            .get("expected")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                format!(
                    "{}: у предложения {} нет expected — набор без разметки снова \
                     начнёт измерять «нашлось хоть один элемент»",
                    path.display(),
                    id
                )
            })?;
        let expected = read_expected(&path, expected_object, &id)?;
        out.push(ProbeCase {
            dataset: "body_metaphor",
            id,
            text,
            expected: Some(expected),
        });
    }
    Ok(out)
}

/// Разбор блока `expected`: все поля обязательны и обязаны быть булевыми.
///
/// Пропущенное поле — ошибка, а не «значит false»: иначе кейс молча потерял бы
/// элементы и попал в счёт с заниженным числом полей.
fn read_expected(
    path: &Path,
    object: &Map<String, Value>,
    case_id: &str,
) -> Result<BTreeMap<String, bool>, String> {
    let mut expected = BTreeMap::new();
    for number in 1..=9 {
        let element = format!("e{}", number);
        let value = object.get(&element).ok_or_else(|| {
            format!(
                "{}: у кейса {} нет поля {} — разметка неполная",
                path.display(),
                case_id,
                element
            )
        })?;
        let flag = value.as_bool().ok_or_else(|| {
            format!(
                "{}: у кейса {} поле {} не boolean",
                path.display(),
                case_id,
                element
            )
        })?;
        expected.insert(element, flag);
    }
    Ok(expected)
}

/// Все 63 кейса в порядке отчёта.
pub fn load_all_cases() -> Result<Vec<ProbeCase>, String> {
    let mut cases = load_marked_cases()?;
    cases.extend(load_metaphor_cases()?);
    Ok(cases)
}

/// Папка движка: рядом с exe, иначе — из `app_config.json` (как в `run_pipeline_test_cli`).
pub fn resolve_engine_dir() -> PathBuf {
    let near_exe = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .map(|dir| crate::infra::llamacpp_installer::default_dir(&dir))
        .unwrap_or_default();
    if near_exe.join("backends").exists() {
        return near_exe;
    }
    std::env::var("APPDATA")
        .ok()
        .map(|appdata| Path::new(&appdata).join("com.kingorch.app").join("app_config.json"))
        .and_then(|cfg| std::fs::read_to_string(cfg).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| {
            value
                .get("llamacpp_dir")
                .and_then(Value::as_str)
                .map(PathBuf::from)
        })
        .filter(|path| path.join("backends").exists())
        .unwrap_or(near_exe)
}