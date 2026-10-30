//! 📊 Инструменты Excel (`excel_inspect/read/apply/audit/render/convert`).
//!
//! Тонкие схемы + исполнение поверх SSOT-ядра `ko-excel-tools` (umya-операции,
//! QA-гейты, recalc/PDF через LibreOffice). По образцу `video.rs`, но: тулов
//! шесть, источник — путь файла (политика видимости 28.10.2026: пути вложений
//! видны модели), артефакты прикрепляются к ответу ПО ПУТИ (xlsx/pdf в base64
//! не энкодятся — карточка юзера работает по `file_path`).
//!
//! Гейт источника (§00.2): читать/редактировать можно ТОЛЬКО файлы, которые
//! (а) юзер явно отдал в чат (allow-read список вложений) или (б) создала
//! сама сессия в `<exe>/excel_outputs/<session_id>/`. Произвольные файлы
//! диска для excel-тулов структурно недоступны — модель не может «угадать»
//! путь мимо гейта.
//!
//! Файловая точка результатов: `<exe_dir>/excel_outputs/<session_id>/<stem>_<seq>_<ts>.ext`.
//! Счётчик `seq` в имени устраняет коллизию «два apply за одну секунду с одним
//! stem → тихая перезапись первого артефакта».

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::infra::tools::media::ImageArtifactRegistry;
use crate::infra::tools::{Tool, ToolCtx, ToolError};

static EXCEL_OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Все имена excel-тулов (фильтр пост-прохода attach и capability-гейта).
pub const EXCEL_TOOL_NAMES: [&str; 6] = [
    "excel_inspect",
    "excel_read",
    "excel_apply",
    "excel_audit",
    "excel_render",
    "excel_convert",
];

pub fn is_excel_tool(name: &str) -> bool {
    EXCEL_TOOL_NAMES.contains(&name)
}

/// Папка результатов сессии: рядом с exe — `excel_outputs/<session_id>/`
/// (правило cwd: current_exe().parent(), НЕ app.path()).
fn excel_session_outputs_dir(session_id: &str) -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    exe_dir.join("excel_outputs").join(session_id)
}

/// Mime по расширению результата (карточка вложения).
fn mime_for_path(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        _ => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    }
}

/// Нормализация пути для сравнения (Windows: сепаратор и регистр не значимы).
fn normalized_path(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

/// Гейт источника: путь обязан быть абсолютным, без `..`, и входить в
/// allow-read список (вложения юзера) ИЛИ в папку результатов сессии
/// (артефакты, созданные в этой сессии — сценарий «почини свой же файл»).
fn ensure_allowed_source(path: &str, ctx: &ToolCtx, allowed: &[String]) -> Result<PathBuf, ToolError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ToolError::Usage(
            "path пуст. Возьми абсолютный путь из строки [Вложение: … → путь] \
             или из output_path предыдущего тул-результата."
                .to_string(),
        ));
    }
    if trimmed.contains("..") {
        return Err(ToolError::Usage(
            "path с '..' запрещён: используй точный абсолютный путь".to_string(),
        ));
    }
    let candidate = Path::new(trimmed);
    if !candidate.is_absolute() {
        return Err(ToolError::Usage(format!(
            "path должен быть абсолютным (получено: '{trimmed}')"
        )));
    }
    let normalized = normalized_path(trimmed);
    let in_outputs = normalized_path(&excel_session_outputs_dir(ctx.session_id).to_string_lossy())
        + "\\";
    let allowed_hit = allowed.iter().any(|a| normalized_path(a) == normalized);
    if !allowed_hit && !normalized.starts_with(&in_outputs) {
        return Err(ToolError::Forbidden(format!(
            "файл '{trimmed}' не входит в allow-read: работай только с вложениями \
             пользователя (пути в [Вложение: …]) и файлами, созданными в этой сессии \
             (output_path тул-результатов)."
        )));
    }
    if !candidate.is_file() {
        return Err(ToolError::NotFound(format!(
            "файл не найден: {trimmed}"
        )));
    }
    Ok(candidate.to_path_buf())
}

/// Зарегистрировать результат в артефакт-реестре и вернуть строку-маркер
/// для пост-прохода attach.
fn register_artifact(path: &str, tool_name: &str, ctx: &ToolCtx) -> Result<String, ToolError> {
    let registry = ctx.image_artifacts.ok_or_else(|| {
        ToolError::Usage("внутренний media-реестр артефактов недоступен".to_string())
    })?;
    let id = registry
        .register(path.to_string(), tool_name)
        .map_err(ToolError::Usage)?;
    Ok(format!("[EXCEL_SAVED id={id}]"))
}

/// Уникальный стем результата: `<stem>-<seq>` (см. док модуля о коллизиях).
fn sequenced_stem(stem: &str) -> String {
    format!(
        "{}-{}",
        ko_excel_tools::pipeline::sanitize_stem(stem),
        EXCEL_OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

/// soffice для пересчёта/PDF: конфиг-ключ libreoffice_path → авто-детект
/// (PATH → стандартные пути). None — честные статусы без пересчёта.
fn soffice_for_tools() -> Option<PathBuf> {
    let custom = crate::infra::load_config_early().libreoffice_path;
    ko_excel_tools::soffice::detect_soffice(custom.as_deref())
}

fn output_dir(ctx: &ToolCtx) -> Result<PathBuf, ToolError> {
    let dir = excel_session_outputs_dir(ctx.session_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| ToolError::Io(format!("создание каталога результатов: {e}")))?;
    Ok(dir)
}

// ─────────────────────────── Исполнение тулов ───────────────────────────

fn run_inspect(args: &Value, ctx: &ToolCtx, allowed: &[String]) -> Result<String, ToolError> {
    let path = arg_path(args)?;
    let path = ensure_allowed_source(&path, ctx, allowed)?;
    ko_excel_tools::inspect::structure(&path).map_err(tool_error).map(|v| v.to_string())
}

fn run_read(args: &Value, ctx: &ToolCtx, allowed: &[String]) -> Result<String, ToolError> {
    let path = arg_path(args)?;
    let path = ensure_allowed_source(&path, ctx, allowed)?;
    let sheet = args.get("sheet").and_then(|v| v.as_str());
    let range = args.get("range").and_then(|v| v.as_str());
    ko_excel_tools::inspect::read_range(&path, sheet, range).map_err(tool_error).map(|v| v.to_string())
}

fn run_apply(args: &Value, ctx: &ToolCtx, allowed: &[String]) -> Result<String, ToolError> {
    let ops = args
        .get("ops")
        .ok_or_else(|| ToolError::Usage("ops обязателен (массив операций)".to_string()))?;
    if !ops.is_array() {
        return Err(ToolError::Usage(
            "ops должен быть массивом операций (см. схему тулa)".to_string(),
        ));
    }
    let source = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) if !p.trim().is_empty() => Some(ensure_allowed_source(p, ctx, allowed)?),
        _ => None,
    };
    let out_dir = output_dir(ctx)?;
    // Стем всегда уникален (seq): авто-имя пайплайна + `_HHMMSS` не спасает
    // от двух apply за одну секунду — перезапись молча ломала бы первый
    // артефакт (путь в реестре указывал бы уже на другую книгу).
    let auto_stem = match &source {
        Some(src) => format!(
            "{}_изменённый",
            src.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "файл".to_string())
        ),
        None => "новая_таблица".to_string(),
    };
    let stem = Some(match args.get("output_stem").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => sequenced_stem(s),
        _ => sequenced_stem(&auto_stem),
    });
    let request = ko_excel_tools::pipeline::ApplyRequest {
        source: source.as_deref(),
        output_dir: &out_dir,
        output_stem: stem.as_deref(),
        ops,
        allow_issues: args
            .get("allow_issues")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    };
    let result =
        ko_excel_tools::pipeline::apply(&request, soffice_for_tools().as_deref(), ko_excel_tools::soffice::DEFAULT_TIMEOUT)
            .map_err(tool_error)?;
    // Артефакт регистрируется ТОЛЬКО когда файл прошёл гейты (output_path
    // не null): файл с проваленной валидацией удалён и прикрепить его нельзя.
    if let Some(path) = result.get("output_path").and_then(|v| v.as_str()) {
        let marker = register_artifact(path, "excel_apply", ctx)?;
        log::info!("[EXCEL] apply → {} ({})", path, result.get("status").and_then(|v| v.as_str()).unwrap_or("?"));
        return Ok(format!("{marker}\n{result}"));
    }
    Ok(result.to_string())
}

fn run_audit(args: &Value, ctx: &ToolCtx, allowed: &[String]) -> Result<String, ToolError> {
    let path = arg_path(args)?;
    let path = ensure_allowed_source(&path, ctx, allowed)?;
    let out_dir = output_dir(ctx)?;
    let temp = out_dir.join("audit_tmp");
    let result = ko_excel_tools::pipeline::audit(
        &path,
        &temp,
        soffice_for_tools().as_deref(),
        ko_excel_tools::soffice::DEFAULT_TIMEOUT,
    )
    .map_err(tool_error)?;
    let _ = std::fs::remove_dir_all(&temp);
    Ok(result.to_string())
}

fn run_render(args: &Value, ctx: &ToolCtx, allowed: &[String]) -> Result<String, ToolError> {
    let path = arg_path(args)?;
    let path = ensure_allowed_source(&path, ctx, allowed)?;
    let soffice = soffice_for_tools().ok_or_else(|| ToolError::NotFound(ko_excel_tools::ExcelError::SofficeNotDetected.to_string()))?;
    let out_dir = output_dir(ctx)?;
    let stem = sequenced_stem(
        &path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "таблица".to_string()),
    );
    let pdf = ko_excel_tools::soffice::render_pdf(&soffice, &path, &out_dir, ko_excel_tools::soffice::DEFAULT_TIMEOUT)
        .map_err(tool_error)?;
    // render_pdf возвращает <stem из исходника>.pdf — переименование в
    // уникальное имя (seq) для защиты от коллизии за секунду.
    let unique = out_dir.join(format!("{stem}.pdf"));
    if pdf != unique {
        std::fs::rename(&pdf, &unique)
            .map_err(|e| ToolError::Io(format!("переименование PDF: {e}")))?;
    }
    let marker = register_artifact(unique.to_string_lossy().as_ref(), "excel_render", ctx)?;
    log::info!("[EXCEL] render → {}", unique.display());
    Ok(format!(
        "{marker}\n{}",
        serde_json::json!({
            "output_path": unique.display().to_string(),
            "note": "PDF приложен к ответу: пользователь видит таблицу без Excel.",
        })
    ))
}

fn run_convert(args: &Value, ctx: &ToolCtx, allowed: &[String]) -> Result<String, ToolError> {
    let path = arg_path(args)?;
    let path = ensure_allowed_source(&path, ctx, allowed)?;
    let to = args
        .get("to")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(to.as_str(), "xlsx" | "csv" | "tsv") {
        return Err(ToolError::Usage(format!(
            "to должен быть \"xlsx\", \"csv\" или \"tsv\" (получено: '{to}')"
        )));
    }
    let out_dir = output_dir(ctx)?;
    let stem_source = args
        .get("output_stem")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "конвертация".to_string())
        });
    let stem = sequenced_stem(&stem_source);
    let sheet = args.get("sheet").and_then(|v| v.as_str());
    let delimiter = args.get("delimiter").and_then(|v| v.as_str()).and_then(parse_delimiter);
    let summary = match to.as_str() {
        "xlsx" => {
            let out = out_dir.join(format!("{stem}.xlsx"));
            ko_excel_tools::convert::csv_to_xlsx(&path, &out, sheet, delimiter).map_err(tool_error)?
        }
        other => {
            let out = out_dir.join(format!("{stem}.{other}"));
            let delimiter = delimiter.unwrap_or(if other == "tsv" { '\t' } else { ',' });
            ko_excel_tools::convert::xlsx_to_csv(&path, &out, sheet, delimiter).map_err(tool_error)?
        }
    };
    let summary_json = summary.to_json();
    let out_str = summary.output_path;
    let marker = register_artifact(&out_str, "excel_convert", ctx)?;
    log::info!("[EXCEL] convert → {}", out_str);
    Ok(format!("{marker}\n{summary_json}"))
}

fn arg_path(args: &Value) -> Result<String, ToolError> {
    args.get("path")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| ToolError::Usage("path обязателен (абсолютный путь файла)".to_string()))
}

/// "tab"/"\\t"/";"/","/"|"/'/' → char-разделитель. None — дефолт формата.
fn parse_delimiter(raw: &str) -> Option<char> {
    match raw.trim() {
        "tab" | "\\t" | "\t" => Some('\t'),
        ";" => Some(';'),
        "," => Some(','),
        "|" => Some('|'),
        _ => None,
    }
}

/// Мост ExcelError → ToolError (Display-текст честный, ready-to-show).
fn tool_error(e: ko_excel_tools::ExcelError) -> ToolError {
    match e {
        ko_excel_tools::ExcelError::FileNotFound(_) => ToolError::NotFound(e.to_string()),
        ko_excel_tools::ExcelError::SofficeNotDetected => ToolError::NotFound(e.to_string()),
        ko_excel_tools::ExcelError::Usage(_) => ToolError::Usage(e.to_string()),
        ko_excel_tools::ExcelError::UnsupportedFormat(..) => ToolError::Usage(e.to_string()),
        _ => ToolError::Io(e.to_string()),
    }
}

/// Исполнить excel-тул по имени (диспетчер). `allowed_source_paths` —
/// allow-read список вложений юзера (request_media.current_attachment_paths()).
pub fn execute_excel_tool(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
    allowed_source_paths: &[String],
) -> Option<Result<String, ToolError>> {
    let outcome = match name {
        "excel_inspect" => run_inspect(args, ctx, allowed_source_paths),
        "excel_read" => run_read(args, ctx, allowed_source_paths),
        "excel_apply" => run_apply(args, ctx, allowed_source_paths),
        "excel_audit" => run_audit(args, ctx, allowed_source_paths),
        "excel_render" => run_render(args, ctx, allowed_source_paths),
        "excel_convert" => run_convert(args, ctx, allowed_source_paths),
        _ => return None,
    };
    Some(outcome)
}

// ─────────────────────── Пост-проход attach (артефакты) ───────────────────────

pub fn extract_saved_excel_id(tool_output: &str) -> Option<String> {
    let marker = "[EXCEL_SAVED id=";
    let start = tool_output.find(marker)? + marker.len();
    let end = tool_output[start..].find(']')?;
    Some(tool_output[start..start + end].to_string())
}

/// Пост-проход run_chat: маркер [EXCEL_SAVED] в результатах tool_calls →
/// цепляет файл к финальному сообщению агента по `file_path` (xlsx/pdf в
/// base64 не энкодятся — карточка юзера работает по пути). Возвращает число
/// прицепленных файлов (частичная ошибка одного артефакта не валит остальные).
pub fn attach_saved_excel_from_sub_calls(
    message: &mut crate::infra::ChatMessage,
    sub_calls: &[crate::infra::SubCall],
    registry: &ImageArtifactRegistry,
) -> Result<usize, String> {
    let mut found: Vec<(String, String)> = Vec::new(); // (id, tool_name)
    for sub_call in sub_calls {
        for tool_call in &sub_call.tool_calls {
            if !is_excel_tool(&tool_call.tool_name) {
                continue;
            }
            if let Some(id) = extract_saved_excel_id(&tool_call.result) {
                let key = (id, tool_call.tool_name.clone());
                if !found.contains(&key) {
                    found.push(key);
                }
            }
        }
    }
    let mut attached = 0;
    for (id, tool_name) in found {
        let path = match registry.resolve(&id, &tool_name) {
            Ok(path) => path,
            Err(error) => {
                log::error!("[EXCEL] артефакт {} ({}) не резолвнут: {}", id, tool_name, error);
                continue;
            }
        };
        match excel_path_to_attachment(&path) {
            Ok(attachment) => {
                match &mut message.attachments {
                    Some(attachments) => attachments.push(attachment),
                    None => message.attachments = Some(vec![attachment]),
                }
                attached += 1;
            }
            Err(error) => {
                log::error!("[EXCEL] артефакт {} не приложен: {}", id, error);
            }
        }
    }
    Ok(attached)
}

/// Прочитать параметры файла результата как ChatAttachment (только путь —
/// без base64).
pub fn excel_path_to_attachment(
    path: &str,
) -> Result<crate::infra::ChatAttachment, String> {
    let bytes_len = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|error| {
            log::error!("[EXCEL] Не удалось прочитать параметры файла {:?}: {}", path, error);
            "Созданный файл недоступен".to_string()
        })?;
    if bytes_len == 0 {
        return Err("Созданный файл пустой".to_string());
    }
    let file_name = Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "таблица.xlsx".to_string());
    Ok(crate::infra::ChatAttachment {
        file_name,
        mime_type: mime_for_path(path).to_string(),
        data_base64: String::new(),
        file_path: Some(path.to_string()),
        is_dir: Some(false),
    })
}

// ─────────────────────────── Tool-заглушки (схемы) ───────────────────────────

/// Общее поле path в схемах чтения/аудита.
fn path_property(note: &str) -> Value {
    serde_json::json!({
        "type": "string",
        "description": format!("Абсолютный путь .xlsx: из строки [Вложение: имя → путь] текущего запроса или из output_path предыдущего тул-результата. {note}")
    })
}

pub struct ExcelInspect;
impl Tool for ExcelInspect {
    fn name(&self) -> &str { "excel_inspect" }
    fn description(&self) -> &str {
        "Структура книги Excel: листы (имена, размеры, объединения, диаграммы, автофильтры), суммарное число занятых ячеек. ПЕРВЫЙ шаг работы с любым файлом — осмотрись, прежде чем читать данные или менять что-то."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {"path": path_property("Только .xlsx (xls/xlsm — отказ).")},
            "required": ["path"]
        })
    }
    fn is_readonly(&self) -> bool { true }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage("excel_inspect должен исполняться через диспетчер".to_string()))
    }
}

pub struct ExcelRead;
impl Tool for ExcelRead {
    fn name(&self) -> &str { "excel_read" }
    fn description(&self) -> &str {
        "Значения и формулы диапазона книги. Формульные ячейки возвращают и формулу (f), и кэш значения (v); кэша у свежезаписанных формул может не быть до пересчёта. Пустые ячейки пропускаются. Лимит 2000 ячеек за вызов — большие данные читай по частям."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": path_property(""),
                "sheet": {"type": "string", "description": "Имя листа (опционально: первый лист). Возьми из excel_inspect."},
                "range": {"type": "string", "description": "Диапазон \"A1:C10\" или \"Лист1!A1:C10\" (опционально: весь использованный прямоугольник)."}
            },
            "required": ["path"]
        })
    }
    fn is_readonly(&self) -> bool { true }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage("excel_read должен исполняться через диспетчер".to_string()))
    }
}

pub struct ExcelApply;
impl Tool for ExcelApply {
    fn name(&self) -> &str { "excel_apply" }
    fn description(&self) -> &str {
        "АТОМАРНО применить операции и получить НОВЫЙ файл .xlsx (исходник не меняется). Конвейер: ops → сохранение → статический аудит формул → пересчёт LibreOffice (если установлен) → скан ошибок значений. Файл с проваленными гейтами НЕ сохраняется: получишь список проблем с адресами — исправь ops и вызови снова. allow_issues:true — осознанный обход гейта, когда проблемы унаследованы от данных пользователя."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Опционально: абсолютный путь .xlsx-исходника для редактирования (новая книга, если не задан)."},
                "ops": {"type": "array", "items": {"type": "object"}, "description": "Список операций. Доступные op: add_sheet{name}; delete_sheet{name} (не последний); rename_sheet{from,to}; set_cell{sheet,addr,value|formula,style?,number_format?} — формула БЕЗ ведущего '=' (\"=B2*2\" неверно, \"B2*2\" верно), value сохраняет JSON-тип (число — числом, строка — текстом); set_column_width{sheet,col,width}; autofit_columns{sheet?,min?,max?}; merge_cells{sheet,range}; auto_filter{sheet,range}; freeze_panes{sheet,cell}; add_chart{sheet,chart_type:bar|column|line|pie|area|scatter,anchor,series:[\"Лист!$B$2:$B$10\"],categories_range?,title?,width_cells?,height_cells?}. Неизвестная op — отказ."},
                "output_stem": {"type": "string", "description": "Имя файла результата без расширения (опционально). Без него — автоген: <имя исходника>_изменённый / новая_таблица."},
                "allow_issues": {"type": "boolean", "description": "true — сохранить файл несмотря на найденные проблемы (только когда они унаследованы от данных пользователя и осознаны). По умолчанию false."}
            },
            "required": ["ops"]
        })
    }
    fn is_readonly(&self) -> bool { false }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage("excel_apply должен исполняться через диспетчер".to_string()))
    }
}

pub struct ExcelAudit;
impl Tool for ExcelAudit {
    fn name(&self) -> &str { "excel_audit" }
    fn description(&self) -> &str {
        "Аудит книги БЕЗ её изменения (сценарий «найди ошибку»): статические гейты формул (запрещённые динамические функции, внешние ссылки, #REF! в формулах) + пересчёт КОПИИ через LibreOffice (если установлен) и скан 7 ошибок значений (#DIV/0!, #REF!, #VALUE!…). Возвращает адреса и объяснения находок."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {"path": path_property("Только .xlsx.")},
            "required": ["path"]
        })
    }
    fn is_readonly(&self) -> bool { true }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage("excel_audit должен исполняться через диспетчер".to_string()))
    }
}

pub struct ExcelRender;
impl Tool for ExcelRender {
    fn name(&self) -> &str { "excel_render" }
    fn description(&self) -> &str {
        "PDF-рендер книги через LibreOffice: пользователь видит таблицу без Excel (визуальная проверка результата). Требует установленный LibreOffice — без него честный отказ. PDF прикрепляется к ответу автоматически."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {"path": path_property("Только .xlsx.")},
            "required": ["path"]
        })
    }
    fn is_readonly(&self) -> bool { true }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage("excel_render должен исполняться через диспетчер".to_string()))
    }
}

pub struct ExcelConvert;
impl Tool for ExcelConvert {
    fn name(&self) -> &str { "excel_convert" }
    fn description(&self) -> &str {
        "Конвертация: CSV/TSV → XLSX (числа распознаются, ведущие нули и \"+\"-префиксы сохраняются текстом, кодировки UTF-8/Windows-1251) или XLSX → CSV/TSV (значения; формула без кэша уходит текстом \"=формула\"). Разделитель определяется автоматически или задай delimiter."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Абсолютный путь исходника: .csv/.tsv для to=\"xlsx\", .xlsx для to=\"csv\"/\"tsv\"."},
                "to": {"type": "string", "enum": ["xlsx", "csv", "tsv"], "description": "Целевой формат."},
                "sheet": {"type": "string", "description": "Имя листа (опционально)."},
                "delimiter": {"type": "string", "description": "Разделитель: \",\", \";\", \"|\" или \"tab\" (опционально: авто-детект для CSV→XLSX; для XLSX→CSV по умолчанию \",\", для TSV — tab)."},
                "output_stem": {"type": "string", "description": "Имя файла результата без расширения (опционально)."}
            },
            "required": ["path", "to"]
        })
    }
    fn is_readonly(&self) -> bool { false }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage("excel_convert должен исполняться через диспетчер".to_string()))
    }
}

/// Схемы excel-тулов для промпта агента (мета-имя "excel").
pub fn excel_tool_schemas(agent_tools: &[String]) -> Vec<(String, String, Value)> {
    let tools: [&dyn Tool; 6] = [
        &ExcelInspect,
        &ExcelRead,
        &ExcelApply,
        &ExcelAudit,
        &ExcelRender,
        &ExcelConvert,
    ];
    let mut out = Vec::new();
    for tool in tools {
        if agent_tools.iter().any(|t| t == tool.name()) {
            out.push((
                "excel".to_string(),
                tool.name().to_string(),
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "inputSchema": tool.parameters(),
                }),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_registry(session: &str) -> (ToolCtx<'static>, &'static ImageArtifactRegistry) {
        // lease-паттерн из tools/mod.rs: static-строки живут дольше ctx.
        // Реестр тоже leak-им: ctx держит ссылку, тесту нужен доступ для resolve.
        let registry: &'static ImageArtifactRegistry =
            Box::leak(Box::new(ImageArtifactRegistry::new()));
        let ctx = ToolCtx {
            workspace_root: std::path::Path::new("."),
            write_root: std::path::Path::new("."),
            write_outside: crate::infra::WriteOutside::Prompt,
            session_id: Box::leak(session.to_string().into_boxed_str()),
            approver: crate::infra::test_approver(),
            agent_id: "excel_operator",
            bins_dir: std::path::Path::new("."),
            image_artifacts: Some(registry),
        };
        (ctx, registry)
    }

    fn sandbox(tag: &str) -> PathBuf {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test_tmp")
            .join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Пример книги через публичный API ядра (umya хосту не зависимость):
    /// apply создаёт файл в tmp-подпапке, копируем по нужному пути.
    fn write_sample_book(path: &Path) {
        let tmp = path.parent().unwrap().join("_sample_tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let ops = serde_json::json!([
            {"op": "set_cell", "sheet": "Sheet1", "addr": "A1", "value": "имя"},
            {"op": "set_cell", "sheet": "Sheet1", "addr": "B1", "value": "сумма"},
            {"op": "set_cell", "sheet": "Sheet1", "addr": "A2", "value": "запись"},
            {"op": "set_cell", "sheet": "Sheet1", "addr": "B2", "value": 12.5}
        ]);
        let request = ko_excel_tools::pipeline::ApplyRequest {
            source: None,
            output_dir: &tmp,
            output_stem: Some("sample"),
            ops: &ops,
            allow_issues: false,
        };
        let result = ko_excel_tools::pipeline::apply(&request, None, std::time::Duration::from_secs(30))
            .expect("создание примера книги");
        let produced = result["output_path"].as_str().expect("output_path");
        std::fs::copy(produced, path).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn schemas_only_for_granted_tools() {
        let none = excel_tool_schemas(&[]);
        assert!(none.is_empty());
        let all = excel_tool_schemas(&[
            "excel_inspect".into(), "excel_read".into(), "excel_apply".into(),
            "excel_audit".into(), "excel_render".into(), "excel_convert".into(),
        ]);
        let names: Vec<&str> = all.iter().map(|(_, name, _)| name.as_str()).collect();
        assert_eq!(names, EXCEL_TOOL_NAMES.to_vec(), "все 6 тулов в порядке объявления");
        assert!(all.iter().all(|(meta, _, _)| meta == "excel"));
        // Схема apply требует ops; формула описана без ведущего '='.
        let apply = all.iter().find(|(_, n, _)| n == "excel_apply").unwrap();
        assert_eq!(apply.2["inputSchema"]["required"], serde_json::json!(["ops"]));
    }

    #[test]
    fn source_gate_rejects_unknown_paths() {
        let (ctx, _registry) = ctx_with_registry("gate_test");
        let dir = sandbox("gate");
        let file = dir.join("private.xlsx");
        write_sample_book(&file);
        let other = dir.join("отчёт.xlsx");
        std::fs::copy(&file, &other).unwrap();
        // allow-read: «отчёт.xlsx» (юзер отдал в чат) + отсутствующий файл
        // «нет.xlsx» (тоже в списке, но на диске его нет — ветка NotFound).
        let missing = dir.join("нет.xlsx");
        let allowed = vec![
            other.to_string_lossy().to_string(),
            missing.to_string_lossy().to_string(),
        ];
        // Файл не из allow-read — Forbidden даже если существует.
        let err = ensure_allowed_source(file.to_string_lossy().as_ref(), &ctx, &allowed).unwrap_err();
        assert!(matches!(err, ToolError::Forbidden(_)), "{err}");
        // allow-read путь принимается.
        ensure_allowed_source(other.to_string_lossy().as_ref(), &ctx, &allowed).unwrap();
        // Относительный путь и ".." — отказ.
        assert!(ensure_allowed_source("data.xlsx", &ctx, &allowed).is_err());
        assert!(ensure_allowed_source("C:\\a\\..\\b.xlsx", &ctx, &allowed).is_err());
        // Отсутствующий файл ИЗ allow-read — NotFound (не Forbidden).
        let err = ensure_allowed_source(missing.to_string_lossy().as_ref(), &ctx, &allowed).unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)), "{err}");
    }

    #[test]
    fn source_gate_allows_session_outputs() {
        let (ctx, _registry) = ctx_with_registry("outputs_gate");
        let out = excel_session_outputs_dir(ctx.session_id);
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let file = out.join("новая_таблица-1_123.xlsx");
        write_sample_book(&file);
        ensure_allowed_source(file.to_string_lossy().as_ref(), &ctx, &[]).unwrap();
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn apply_creates_book_and_registers_artifact() {
        let (ctx, registry) = ctx_with_registry("apply_ok");
        let out = excel_session_outputs_dir(ctx.session_id);
        let _ = std::fs::remove_dir_all(&out);
        let ops = serde_json::json!([
            {"op": "set_cell", "sheet": "Sheet1", "addr": "A1", "value": "Товар"},
            {"op": "set_cell", "sheet": "Sheet1", "addr": "B1", "value": "Цена"},
            {"op": "set_cell", "sheet": "Sheet1", "addr": "A2", "value": "Хлеб"},
            {"op": "set_cell", "sheet": "Sheet1", "addr": "B2", "value": 45},
            {"op": "set_cell", "sheet": "Sheet1", "addr": "B3", "formula": "B2*2"}
        ]);
        let args = serde_json::json!({"ops": ops, "output_stem": "тестовая"});
        let result = execute_excel_tool("excel_apply", &args, &ctx, &[]).unwrap().unwrap();
        let id = extract_saved_excel_id(&result).expect("маркер EXCEL_SAVED в результате");
        let path = registry.resolve(&id, "excel_apply").unwrap();
        assert!(Path::new(&path).is_file(), "{path}");
        // Статус зависит от машины: есть LibreOffice — ok_recalced, нет —
        // ok_without_recalc; оба означают «файл сохранён и приложен».
        assert!(
            result.contains("ok_recalced") || result.contains("ok_without_recalc"),
            "честный статус доставки: {result}"
        );
        // Формула реально в файле: читаем через публичный API ядра.
        let read = ko_excel_tools::inspect::read_range(Path::new(&path), Some("Sheet1"), Some("A1:B3"))
            .expect("чтение созданной книги");
        assert!(read.to_string().contains("B2*2"), "формула живая в файле: {read}");
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn apply_validation_failure_has_no_marker() {
        let (ctx, _registry) = ctx_with_registry("apply_bad");
        let ops = serde_json::json!([
            {"op": "set_cell", "sheet": "Sheet1", "addr": "A1", "formula": "XLOOKUP(1,A:A,A:A)"}
        ]);
        let args = serde_json::json!({"ops": ops});
        let result = execute_excel_tool("excel_apply", &args, &ctx, &[]).unwrap().unwrap();
        assert!(extract_saved_excel_id(&result).is_none(), "битый файл не должен регистрироваться");
        assert!(result.contains("validation_failed"), "{result}");
    }

    #[test]
    fn unknown_excel_tool_returns_none() {
        let (ctx, _registry) = ctx_with_registry("unknown");
        assert!(execute_excel_tool("excel_nope", &serde_json::json!({}), &ctx, &[]).is_none());
        assert!(!is_excel_tool("generate_image"));
    }

    #[test]
    fn attach_links_excel_files_by_path_without_base64() {
        let (_ctx, registry) = ctx_with_registry("attach");
        let file = sandbox("attach").join("книга.xlsx");
        write_sample_book(&file);
        let path_str = file.to_string_lossy().to_string();
        let id = registry.register(path_str.clone(), "excel_apply").unwrap();
        let result = format!("[EXCEL_SAVED id={id}] ок");
        let mut message = crate::infra::ChatMessage {
            id: Some("msg_1".into()),
            msg_type: "message".into(),
            content: "готово".into(),
            sub_calls: None,
            author: None,
            model: None,
            time_sec: None,
            attachments: None,
            phase: None,
            author_name: None,
        };
        let sub_calls = vec![crate::infra::SubCall {
            agent_name: "excel_operator".to_string(),
            prompt: String::new(),
            response: String::new(),
            time_sec: 0.0,
            tool_calls: vec![crate::infra::ToolCallInfo {
                tool_name: "excel_apply".to_string(),
                arguments: "{}".to_string(),
                result: result.clone(),
            }],
            thinking: None,
        }];
        let attached = attach_saved_excel_from_sub_calls(&mut message, &sub_calls, &registry).unwrap();
        assert_eq!(attached, 1);
        let attachment = &message.attachments.as_ref().unwrap()[0];
        assert_eq!(attachment.file_name, "книга.xlsx");
        assert_eq!(attachment.file_path.as_deref(), Some(path_str.as_str()));
        assert!(attachment.data_base64.is_empty(), "xlsx не энкодится в base64");
        assert_eq!(attachment.mime_type, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet");
        // Чужой тул с тем же маркером не цепляется.
        let sub_calls_foreign = vec![crate::infra::SubCall {
            agent_name: "x".to_string(),
            prompt: String::new(),
            response: String::new(),
            time_sec: 0.0,
            tool_calls: vec![crate::infra::ToolCallInfo {
                tool_name: "generate_image".to_string(),
                arguments: "{}".to_string(),
                result,
            }],
            thinking: None,
        }];
        let mut message2 = crate::infra::ChatMessage {
            id: None,
            msg_type: "message".into(),
            content: String::new(),
            sub_calls: None,
            author: None,
            model: None,
            time_sec: None,
            attachments: None,
            phase: None,
            author_name: None,
        };
        let attached2 = attach_saved_excel_from_sub_calls(&mut message2, &sub_calls_foreign, &registry).unwrap();
        assert_eq!(attached2, 0);
    }

    #[test]
    fn convert_csv_to_xlsx_through_tool() {
        let (ctx, registry) = ctx_with_registry("convert");
        let dir = sandbox("convert");
        let csv = dir.join("данные.csv");
        std::fs::write(&csv, "a;b\n1;2\n").unwrap();
        let args = serde_json::json!({"path": csv.to_string_lossy(), "to": "xlsx"});
        // Юзер «отдал» csv в чат → путь в allow-read.
        let allowed = vec![csv.to_string_lossy().to_string()];
        let result = execute_excel_tool("excel_convert", &args, &ctx, &allowed).unwrap().unwrap();
        let id = extract_saved_excel_id(&result).expect("конверт регистрирует артефакт");
        let path = registry.resolve(&id, "excel_convert").unwrap();
        assert!(path.ends_with(".xlsx"), "{path}");
        // Гейт: конверт чужого csv (не из allow-read) — Forbidden.
        let foreign = dir.join("чужой.csv");
        std::fs::write(&foreign, "x\n1\n").unwrap();
        let args_foreign = serde_json::json!({"path": foreign.to_string_lossy(), "to": "xlsx"});
        let err = execute_excel_tool("excel_convert", &args_foreign, &ctx, &allowed).unwrap().unwrap_err();
        assert!(matches!(err, ToolError::Forbidden(_)), "{err}");
    }

    #[test]
    fn inspect_and_read_roundtrip_through_tool() {
        let (ctx, _registry) = ctx_with_registry("inspect");
        let file = sandbox("inspect").join("книга.xlsx");
        write_sample_book(&file);
        let allowed = vec![file.to_string_lossy().to_string()];
        let args = serde_json::json!({"path": file.to_string_lossy()});
        let result = execute_excel_tool("excel_inspect", &args, &ctx, &allowed).unwrap().unwrap();
        assert!(result.contains("Sheet1"), "{result}");
        let args = serde_json::json!({"path": file.to_string_lossy(), "range": "A1:B2"});
        let result = execute_excel_tool("excel_read", &args, &ctx, &allowed).unwrap().unwrap();
        assert!(result.contains("12.5"), "{result}");
    }
}
