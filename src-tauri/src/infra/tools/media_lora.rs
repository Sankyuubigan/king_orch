//! Опциональные LoRA-аргументы image-тулов (PRP 29.10.26, фаза 2).
//!
//! Извлечение из аргументов тула + резолв через SSOT плагина
//! (`tauri_plugin_image_engine::engine::resolve_lora_specs`). Схемные
//! описания полей живут в `media.rs` (`parameters()`); вся LoRA-логика —
//! здесь: никаких новых каналов прокидки и дублей резолва.
//!
//! Контракт фазы 2 (D2): `lora` — массив строк ИЛИ массив объектов
//! `{name, strength}` (микс форм допустим); строка = легаси-синоним без
//! собственной силы. Итоговая сила элемента: его `strength`, иначе общая
//! `lora_strength` (дефолт 1.0).

use serde_json::Value;

use tauri_plugin_image_engine::engine::LoraSpec;

use crate::infra::tools::ToolError;

/// Граница честной вилки strength (schema декларирует 0–2; дефолт 1.0).
/// Одна и та же вилка и для общей силы, и для per-item силы элемента.
pub const LORA_STRENGTH_MAX: f64 = 2.0;

/// Один элемент LoRA-запроса тула.
#[derive(Debug)]
pub struct LoraItem {
    /// Стем файла (без расширения), как пришёл — trim уже сделан.
    pub name: String,
    /// Индивидуальная сила; `None` = применить общий дефолт запроса.
    pub strength: Option<f32>,
}

/// LoRA-запрос тула: элементы + общий дефолт силы (`lora_strength`).
#[derive(Debug)]
pub struct LoraRequest {
    pub items: Vec<LoraItem>,
    pub strength: f32,
}

impl LoraRequest {
    /// Имена запроса по порядку (лог-строка, ошибка резолва).
    pub fn names(&self) -> Vec<String> {
        self.items.iter().map(|i| i.name.clone()).collect()
    }
}

/// Прочитать силу элемента из значения JSON с общей вилкой 0–2.
fn item_strength(map: &serde_json::Map<String, Value>) -> Result<Option<f32>, ToolError> {
    match map.get("strength") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let s = v.as_f64().ok_or_else(|| {
                ToolError::Usage(
                    "strength элемента lora должен быть числом 0–2".to_string(),
                )
            })?;
            if !s.is_finite() || s < 0.0 || s > LORA_STRENGTH_MAX {
                return Err(ToolError::Usage(format!(
                    "strength элемента lora вне диапазона 0–{}",
                    LORA_STRENGTH_MAX
                )));
            }
            Ok(Some(s as f32))
        }
    }
}

/// Разобрать один элемент массива `lora` (строка | объект `{name, strength}`).
fn parse_lora_item(item: &Value) -> Result<LoraItem, ToolError> {
    match item {
        // Легаси-форма: строка = имя, сила из общего дефолта.
        Value::String(raw) => {
            let name = raw.trim().to_string();
            if name.is_empty() {
                return Err(ToolError::Usage(
                    "lora: пустое имя в массиве — имена LoRA-файлов без расширения".to_string(),
                ));
            }
            Ok(LoraItem { name, strength: None })
        }
        // Форма фазы 2: {"name": стем, "strength": 0–2 (опционально)}
        Value::Object(map) => {
            let name_v = map.get("name").ok_or_else(|| {
                ToolError::Usage(
                    "элемент lora-объекта требует строковое поле name (стем без расширения)"
                        .to_string(),
                )
            })?;
            let name = name_v.as_str().map(str::trim).unwrap_or("").to_string();
            if name.is_empty() {
                return Err(ToolError::Usage(
                    "lora: поле name элемента пустое — имена LoRA-файлов без расширения".to_string(),
                ));
            }
            let strength = item_strength(map)?;
            Ok(LoraItem { name, strength })
        }
        _ => Err(ToolError::Usage(
            "элемент lora — строка (стем) или объект {name, strength}".to_string(),
        )),
    }
}

/// Извлечь LoRA-запрос из аргументов тула. `None` = поле не передано
/// (генерация без LoRA — прежний путь). Правила:
/// - `lora` — массив строк ИЛИ объектов `{name, strength}`; микс форм
///   в одном массиве допустим; повторы имён (регистронезависимо)
///   сворачиваются — первый элемент выигрывает вместе со своей силой;
/// - сила элемента переопределяет общий `lora_strength` (0–2, дефолт 1.0);
///   нечисловые/вне вилки — Usage ДО обращения к движку;
/// - `lora_strength` без `lora` — Usage (поле без имён бессмысленно).
pub fn extract_lora_request(args: &Value) -> Result<Option<LoraRequest>, ToolError> {
    let has_strength = matches!(args.get("lora_strength"), Some(v) if !v.is_null());
    let items: Option<Vec<LoraItem>> = match args.get("lora") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) => {
            if list.is_empty() {
                return Err(ToolError::Usage(
                    "lora: пустой массив имён — опустите поле вместо этого".to_string(),
                ));
            }
            let mut items: Vec<LoraItem> = Vec::with_capacity(list.len());
            for item in list {
                let parsed = parse_lora_item(item)?;
                if items
                    .iter()
                    .any(|known| known.name.eq_ignore_ascii_case(&parsed.name))
                {
                    continue;
                }
                items.push(parsed);
            }
            Some(items)
        }
        Some(_) => {
            return Err(ToolError::Usage(
                "lora должен быть массивом строк или объектов {name, strength}".to_string(),
            ))
        }
    };
    let strength = match args.get("lora_strength") {
        None | Some(Value::Null) => 1.0,
        Some(v) => {
            let s = v.as_f64().ok_or_else(|| {
                ToolError::Usage("lora_strength должен быть числом 0–2".to_string())
            })?;
            if !s.is_finite() || s < 0.0 || s > LORA_STRENGTH_MAX {
                return Err(ToolError::Usage(format!(
                    "lora_strength вне диапазона 0–{}",
                    LORA_STRENGTH_MAX
                )));
            }
            s as f32
        }
    };
    match items {
        None if has_strength => Err(ToolError::Usage(
            "lora_strength передан без lora — передайте массив имён lora".to_string(),
        )),
        None => Ok(None),
        Some(items) => Ok(Some(LoraRequest { items, strength })),
    }
}

/// Резолв через плагинный SSOT `resolve_lora_specs` (стем-матчинг,
/// приоритет `.safetensors`, честный текст ошибки: папка + доступные).
/// Сила каждого элемента — его `strength`, иначе общий дефолт запроса.
/// Легаси-плоский каталог (без базы) — используется только тестами; боевая
/// точка входа — `resolve_specs` (по базе активного бандла).
#[cfg(test)]
pub fn resolve_specs_in(
    dir: &std::path::Path,
    req: &LoraRequest,
) -> Result<Vec<LoraSpec>, ToolError> {
    let wanted: Vec<(String, f32)> = req
        .items
        .iter()
        .map(|i| (i.name.clone(), i.strength.unwrap_or(req.strength)))
        .collect();
    tauri_plugin_image_engine::engine::resolve_lora_specs(dir, &wanted)
        .map_err(ToolError::NotFound)
}

/// То же с указанием базы (F3, 29.10.26): поиск только в подпапке
/// `<dir>/<base>` — лоры чужой базы физически отсутствуют в каталоге
/// сервера, кросс-базное применение невозможно по design.
pub fn resolve_specs_in_base(
    dir: &std::path::Path,
    req: &LoraRequest,
    base: Option<&str>,
) -> Result<Vec<LoraSpec>, ToolError> {
    let wanted: Vec<(String, f32)> = req
        .items
        .iter()
        .map(|i| (i.name.clone(), i.strength.unwrap_or(req.strength)))
        .collect();
    tauri_plugin_image_engine::engine::resolve_lora_specs_for_base(dir, &wanted, base)
        .map_err(ToolError::NotFound)
}

/// База LoRA активного бандла (F3): выбор юзера (`selected_bundle` из
/// image-config), иначе дефолт каталога. Незакачанность файлов бандла здесь
/// НЕ проверяется — это делает `resolve_bundle_for_tools` при генерации;
/// здесь берётся только имя базы. Бандл отсутствует в каталоге → `None`
/// (легаси-плоский) — честный отказ произойдёт позже, при генерации.
fn active_lora_base() -> Option<String> {
    let cfg = tauri_plugin_image_engine::engine::load_image_config_early();
    let selected = cfg.selected_bundle.clone().unwrap_or_default();
    let entry = if selected.is_empty() {
        tauri_plugin_image_engine::engine::default_bundle_entry()
    } else {
        tauri_plugin_image_engine::engine::bundle_by_name(&selected)
    };
    entry.and_then(|e| e.lora_base)
}

/// Резолв в папке по действующему конфигу (`image_lora_dir`,
/// дефолт `<KingOrchData>/image_loras`) и на базу АКТИВНОГО бандла (F3).
pub fn resolve_specs(req: &LoraRequest) -> Result<Vec<LoraSpec>, ToolError> {
    resolve_specs_in_base(
        &tauri_plugin_image_engine::engine::image_lora_dir_early(),
        req,
        active_lora_base().as_deref(),
    )
}

/// Лог-строка статуса прокидки: имена запроса, общий дефолт силы и
/// фактические файлы С ПО-ЭЛЕМЕНТНЫМИ силами (D2: слайдер-лоры и
/// пер-итем-подбор проверяются по этой строке в логе).
pub fn status_line(req: &LoraRequest, specs: &[LoraSpec]) -> String {
    let resolved: Vec<String> = specs
        .iter()
        .map(|s| format!("{}×{:.2}", s.path, s.multiplier))
        .collect();
    format!(
        "[IMG-TOOL] LoRA requested=[{}] default_strength={:.2} resolved=[{}]",
        req.names().join(", "),
        req.strength,
        resolved.join(" + ")
    )
}

#[cfg(test)]
#[path = "media_lora_tests.rs"]
mod media_lora_tests;
