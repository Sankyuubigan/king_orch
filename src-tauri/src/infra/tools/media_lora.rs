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

/// База LoRA активного ВИДЕО-бандла: выбор юзера (`selected_video_bundle`),
/// иначе дефолт каталога. Закачанность файлов здесь не проверяется — это
/// делает `resolve_video_bundle_for_tools` при генерации; здесь берётся
/// только имя базы (`"minimax_h3"` у бандлов семейства H3).
fn active_video_lora_base() -> Option<String> {
    let cfg = tauri_plugin_image_engine::engine::load_image_config_early();
    let selected = cfg.selected_video_bundle.clone().unwrap_or_default();
    let entry = if selected.is_empty() {
        tauri_plugin_image_engine::engine::default_video_bundle_entry()
    } else {
        tauri_plugin_image_engine::engine::video_bundle_by_name(&selected)
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

/// Резолв в папке ВИДЕО-лор (`video_lora_dir`, дефолт
/// `<KingOrchData>/video_loras`) на базу активного ВИДЕО-бандла
/// (`"minimax_h3"`). Зеркало `resolve_specs` для тулза generate_video.
pub fn resolve_video_specs(req: &LoraRequest) -> Result<Vec<LoraSpec>, ToolError> {
    resolve_specs_in_base(
        &tauri_plugin_image_engine::engine::video_lora_dir_early(),
        req,
        active_video_lora_base().as_deref(),
    )
}

// ─────────── Каталог лор юзера для контекста агента ───────────
//
// Юзер сам качает лоры в `<video_loras>/<база>/`; агент обязан видеть, ЧТО
// доступно, и выбирать по описаниям, а не по захардкоженным стемам. Описания
// живут в sidecar-файле `<стем>.md` РЯДОМ с safetensors (YAML frontmatter:
// description / triggers / default_strength / category) — данные переезжают
// вместе с файлом лоры, код при новых лорах не меняется. Нет sidecar —
// в каталог попадает голый стем (агент честно видит, что описания нет).

/// Расширения файлов-лор (зеркало приоритета плагина lora.rs).
const LORA_FILE_EXTS: [&str; 7] = [
    "safetensors", "sft", "gguf", "ckpt", "pt", "pth", "bin",
];

/// Одна запись каталога видео-лор юзера.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoLoraCatalogEntry {
    /// Стем файла (без расширения) — значение для элемента `lora`.
    pub stem: String,
    /// Имя файла с расширением.
    pub file: String,
    /// Короткое описание из sidecar `<стем>.md` (пусто — sidecar нет).
    pub description: String,
    /// Триггер-слова из sidecar (включаются агентом в prompt_en).
    pub triggers: Vec<String>,
    /// Дефолт силы из sidecar (None — не указан, агент подбирает).
    pub default_strength: Option<f32>,
    /// Категория из sidecar (nsfw_universal / nsfw_concept / realism / effect…).
    pub category: String,
}

/// Scan каталога: файлы-лоры + опциональные sidecar-описания. Отсортировано
/// по стему (стабильный порядок в промпте). Нет папки → пустой каталог.
pub fn scan_video_lora_catalog_in(dir: &std::path::Path) -> Vec<VideoLoraCatalogEntry> {
    let mut out: Vec<VideoLoraCatalogEntry> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !LORA_FILE_EXTS.contains(&ext.as_str()) {
            continue;
        }
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if file.is_empty() || stem.is_empty() {
            continue;
        }
        let (description, triggers, default_strength, category) =
            parse_lora_sidecar(&dir.join(format!("{}.md", stem)));
        out.push(VideoLoraCatalogEntry {
            stem,
            file,
            description,
            triggers,
            default_strength,
            category,
        });
    }
    out.sort_by(|a, b| a.stem.to_lowercase().cmp(&b.stem.to_lowercase()));
    out
}

/// Разобрать sidecar `<стем>.md`: YAML frontmatter между `---` строками,
/// ключи description / triggers / default_strength / category. Минимальный
/// парсер без YAML-зависимости: строка `key: value`, списки в `[a, b]`
/// или через запятую, кавычки значений срезаются. Битый/чужой файл — пустые
/// поля (описание необязательно, каталог не должен падать от мусора).
fn parse_lora_sidecar(path: &std::path::Path) -> (String, Vec<String>, Option<f32>, String) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (String::new(), Vec::new(), None, String::new());
    };
    let mut lines = text.lines().map(str::trim);
    let mut description = String::new();
    let mut triggers = Vec::new();
    let mut strength = None;
    let mut category = String::new();
    // frontmatter только если файл НАЧИНАЕТСЯ с "---".
    if lines.next() != Some("---") {
        return (description, triggers, strength, category);
    }
    for line in lines {
        if line == "---" {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key.trim() {
            "description" => description = value.to_string(),
            "triggers" => {
                let raw = value.trim_start_matches('[').trim_end_matches(']');
                triggers = raw
                    .split(',')
                    .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
                    .filter(|t| !t.is_empty())
                    .collect();
            }
            "default_strength" => strength = value.parse::<f32>().ok(),
            "category" => category = value.to_string(),
            _ => {}
        }
    }
    (description, triggers, strength, category)
}

/// Готовый блок «Доступные LoRA» для задачи агента: по строке на лору
/// (стем — описание — триггеры — сила — категория). Пустой каталог →
/// честная строка-заглушка, а не тишина (агент обязан знать, что лор нет).
/// Боевая точка: папка базы активного видео-бандла.
pub fn video_loras_prompt() -> String {
    let dir = tauri_plugin_image_engine::engine::video_lora_dir_early();
    let target = match active_video_lora_base().as_deref() {
        Some(base) if !base.trim().is_empty() => dir.join(base.trim()),
        _ => dir,
    };
    video_loras_prompt_in(&target)
}

pub fn video_loras_prompt_in(dir: &std::path::Path) -> String {
    let entries = scan_video_lora_catalog_in(dir);
    if entries.is_empty() {
        return "(в папке LoRA юзера пусто — аргумент lora не передавай)".to_string();
    }
    let mut out = String::new();
    for e in &entries {
        out.push_str(&format!("- {}", e.stem));
        if !e.description.is_empty() {
            out.push_str(&format!(" — {}", e.description));
        }
        if !e.triggers.is_empty() {
            out.push_str(&format!(" (триггеры: {})", e.triggers.join(", ")));
        }
        if let Some(s) = e.default_strength {
            out.push_str(&format!(" (strength: {:.2})", s));
        }
        if !e.category.is_empty() {
            out.push_str(&format!(" [{}]", e.category));
        }
        out.push('\n');
    }
    out
}

/// Лог-строка статуса прокидки: имена запроса, общий дефолт силы и
/// фактические файлы С ПО-ЭЛЕМЕНТНЫМИ силами (D2: слайдер-лоры и
/// пер-итем-подбор проверяются по этой строке в логе).
pub fn status_line(req: &LoraRequest, specs: &[LoraSpec]) -> String {
    status_line_tagged("[IMG-TOOL]", req, specs)
}

/// То же для видео-тула generate_video (тег VID-TOOL).
pub fn video_status_line(req: &LoraRequest, specs: &[LoraSpec]) -> String {
    status_line_tagged("[VID-TOOL]", req, specs)
}

fn status_line_tagged(tag: &str, req: &LoraRequest, specs: &[LoraSpec]) -> String {
    let resolved: Vec<String> = specs
        .iter()
        .map(|s| format!("{}×{:.2}", s.path, s.multiplier))
        .collect();
    format!(
        "{} LoRA requested=[{}] default_strength={:.2} resolved=[{}]",
        tag,
        req.names().join(", "),
        req.strength,
        resolved.join(" + ")
    )
}

#[cfg(test)]
#[path = "media_lora_tests.rs"]
mod media_lora_tests;
