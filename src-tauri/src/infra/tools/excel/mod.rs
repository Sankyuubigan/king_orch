//! 📊 Инструменты Excel (`excel_inspect/read/apply/audit/render/convert`).
//!
//! Тонкие схемы + исполнение поверх SSOT-ядра `excel-tools` (umya-операции,
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

/// Есть ли в пути поднимающийся компонент (`..`)? Проверка по компонентам,
/// а не подстрокой: имя файла с двумя точками подряд («бюджет..2024.xlsx»)
/// легально, а `..\evil.xlsx` — нет.
fn path_has_parent_ref(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
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
    if path_has_parent_ref(Path::new(trimmed)) {
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
        excel_tools::pipeline::sanitize_stem(stem),
        EXCEL_OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

/// soffice для пересчёта/PDF: конфиг-ключ libreoffice_path → авто-детект
/// (PATH → стандартные пути). None — честные статусы без пересчёта.
fn soffice_for_tools() -> Option<PathBuf> {
    let custom = crate::infra::load_config_early().libreoffice_path;
    excel_tools::soffice::detect_soffice(custom.as_deref())
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
    excel_tools::inspect::structure(&path).map_err(tool_error).map(|v| v.to_string())
}

fn run_read(args: &Value, ctx: &ToolCtx, allowed: &[String]) -> Result<String, ToolError> {
    let path = arg_path(args)?;
    let path = ensure_allowed_source(&path, ctx, allowed)?;
    let sheet = args.get("sheet").and_then(|v| v.as_str());
    let range = args.get("range").and_then(|v| v.as_str());
    excel_tools::inspect::read_range(&path, sheet, range).map_err(tool_error).map(|v| v.to_string())
}

/// Механический потолок payload'а set_rows в одном excel_apply (в ячейках).
/// Инвариант: один вызов агента должен влезать в бюджет генерации
/// (max_gen_tokens=2048, поднимать запрещено — AGENTS.md §0.5):
/// ~6-8 ток/ячейка + резерв на конверт ~350 ток → (2048-350)/8 ≈ 212 → 200.
/// Превышение — мгновенная структурированная ошибка с инструкцией, а не
/// тихая смерть в MAX_TOKENS и докачках (кейс excel_operator 03-04.11.26:
/// 3 докачки подряд → отказ workflow).
const MAX_SET_ROWS_CELLS_PER_APPLY: usize = 200;

fn gate_set_rows_payload(ops: &Value) -> Result<(), ToolError> {
    let mut total_cells = 0usize;
    if let Some(arr) = ops.as_array() {
        for op in arr {
            if op.get("op").and_then(|v| v.as_str()) != Some("set_rows") {
                continue;
            }
            if let Some(rows) = op.get("rows").and_then(|v| v.as_array()) {
                for row in rows {
                    let cells = row.as_array().map(|r| r.len()).unwrap_or(1);
                    total_cells = total_cells.saturating_add(cells);
                }
            }
        }
    }
    if total_cells > MAX_SET_ROWS_CELLS_PER_APPLY {
        return Err(ToolError::Usage(format!(
            "Блок данных слишком велик: {} ячеек в set_rows при потолке {} на один excel_apply (бюджет генерации модели). Дроби запись: несколько excel_apply с set_rows по ≤15 строк; каждый следующий вызов — с path из output_path предыдущего результата.",
            total_cells, MAX_SET_ROWS_CELLS_PER_APPLY
        )));
    }
    Ok(())
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
    // Гейт ДО пайплайна: большой блок всё равно не влезает в бюджет генерации
    // — отклоняем сразу с инструкцией по дроблению.
    gate_set_rows_payload(ops)?;
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
    let request = excel_tools::pipeline::ApplyRequest {
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
        excel_tools::pipeline::apply(&request, soffice_for_tools().as_deref(), excel_tools::soffice::DEFAULT_TIMEOUT)
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
    // Cleanup ДО `?`: Err не должен покидать функцию с живой аудиторской
    // копией на диске (раньше ранний выход оставлял файл в audit_tmp).
    let result = excel_tools::pipeline::audit(
        &path,
        &temp,
        soffice_for_tools().as_deref(),
        excel_tools::soffice::DEFAULT_TIMEOUT,
    );
    let _ = std::fs::remove_dir_all(&temp);
    result.map_err(tool_error).map(|v| v.to_string())
}

fn run_render(args: &Value, ctx: &ToolCtx, allowed: &[String]) -> Result<String, ToolError> {
    let path = arg_path(args)?;
    let path = ensure_allowed_source(&path, ctx, allowed)?;
    let soffice = soffice_for_tools().ok_or_else(|| ToolError::NotFound(excel_tools::ExcelError::SofficeNotDetected.to_string()))?;
    let out_dir = output_dir(ctx)?;
    let stem = sequenced_stem(
        &path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "таблица".to_string()),
    );
    let pdf = excel_tools::soffice::render_pdf(&soffice, &path, &out_dir, excel_tools::soffice::DEFAULT_TIMEOUT)
        .map_err(tool_error)?;
    // render_pdf возвращает <stem из исходника>.pdf — переименование в
    // уникальное имя SSOT-нейминга lib (seq + эпоха): эпоха закрывает
    // перезапись PDF после рестарта приложения (seq process-global сбрасывается,
    // а rename на Windows молча заменяет существующий файл).
    let unique = excel_tools::pipeline::unique_pdf_path(&out_dir, &stem);
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
            excel_tools::convert::csv_to_xlsx(&path, &out, sheet, delimiter).map_err(tool_error)?
        }
        other => {
            let out = out_dir.join(format!("{stem}.{other}"));
            let delimiter = delimiter.unwrap_or(if other == "tsv" { '\t' } else { ',' });
            excel_tools::convert::xlsx_to_csv(&path, &out, sheet, delimiter).map_err(tool_error)?
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
fn tool_error(e: excel_tools::ExcelError) -> ToolError {
    match e {
        excel_tools::ExcelError::FileNotFound(_) => ToolError::NotFound(e.to_string()),
        excel_tools::ExcelError::SofficeNotDetected => ToolError::NotFound(e.to_string()),
        excel_tools::ExcelError::Usage(_) => ToolError::Usage(e.to_string()),
        excel_tools::ExcelError::UnsupportedFormat(..) => ToolError::Usage(e.to_string()),
        // Детерминированный отказ пер-операции (лист не найден, не последний
        // лист, «БЕЗ ведущего '='»…): Display готов к показу — агент обязан
        // видеть причину (core §2.2). Кейс 04.11.26: OpFailed проваливался в
        // Io-маску → модели уходила «внутренняя ошибка excel-обработки» →
        // слепой аборт по лимиту неудач.
        excel_tools::ExcelError::OpFailed { .. } => ToolError::Usage(e.to_string()),
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
                "ops": {"type": "array", "items": {"type": "object"}, "description": "Список операций. Доступные op: add_sheet{name}; delete_sheet{name} (не последний); rename_sheet{from,to}; set_cell{sheet,addr,value|formula,style?,number_format?} — формула БЕЗ ведущего '=' (\"=B2*2\" неверно, \"B2*2\" верно), value сохраняет JSON-тип (число — числом, строка — текстом); set_rows{sheet,addr,rows} — БЛОК строк одной операцией (rows: массив строк-массивов от addr; ячейка — значение, null (пропуск) или {value?,formula?,number_format?,style?}) — данные таблиц ПИШИ ИМ, а не десятками set_cell; ≤15 строк за вызов (блоки больше тул отклонит — дроби на несколько excel_apply); format_range{sheet,range,number_format?,style?} — формат/стиль на весь диапазон; add_table{sheet,range,name?} — умная таблица Excel (первая строка диапазона — заголовки); set_column_width{sheet,col,width}; autofit_columns{sheet?,min?,max?}; merge_cells{sheet,range}; auto_filter{sheet,range}; freeze_panes{sheet,cell}; add_chart{sheet,chart_type:bar|column|line|pie|area|scatter,anchor,series:[\"Лист!$B$2:$B$10\"],categories_range?,title?,width_cells?,height_cells?}. Неизвестная op — отказ. Форма операции — ТАГГИРОВАННАЯ: имя op в поле \"op\", не ключом объекта. Для НОВОЙ книги (без path) создавать лист заранее НЕ нужно: листы из операций создаются автоматически; можно писать и в стартовый лист \"Sheet1\". Пример ops: [{\"op\":\"set_rows\",\"sheet\":\"Продажи\",\"addr\":\"A1\",\"rows\":[[\"Дата\",\"Товар\",\"Цена\"],[\"2026-10-01\",\"Ноутбук\",45000]]}]"},
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
        let request = excel_tools::pipeline::ApplyRequest {
            source: None,
            output_dir: &tmp,
            output_stem: Some("sample"),
            ops: &ops,
            allow_issues: false,
        };
        let result = excel_tools::pipeline::apply(&request, None, std::time::Duration::from_secs(30))
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
        // Блок-операции обязательны в контракте для модели (SSOT с op-словарём
        // ядра): без них модель пишет данные по set_cell и обрывается по лимиту
        // токенов (кейс 03.11.2026).
        let ops_desc = apply.2["inputSchema"]["properties"]["ops"]["description"]
            .as_str()
            .expect("описание ops в схеме");
        for op in ["set_rows", "format_range", "add_table"] {
            assert!(ops_desc.contains(op), "в схеме ops нет '{op}'");
        }
        // Кейс 04.11.26: модель учила tagged-форму на ошибках (missing `op`,
        // missing `addr`) и пропускала создание листа. Пример и правило новой
        // книги обязательны в контракте.
        assert!(ops_desc.contains("Пример ops"), "нет JSON-примера в описании ops");
        assert!(ops_desc.contains("Sheet1"), "нет правила стартового листа");
        assert!(ops_desc.contains("\"op\":\"set_rows\""), "пример без tagged-формы");
    }

    #[test]
    fn op_failed_error_reaches_agent_honestly() {
        // Кейс 04.11.26: OpFailed маскировался в ToolError::Io → агент получал
        // «внутренняя ошибка excel-обработки» и не мог исправить ops → аборт
        // по лимиту неудач. Контракт: детерминированный отказ пер-операции —
        // Usage-класс (is_honest_tool_error в vram.rs пропускает Usage как есть).
        let mapped = tool_error(excel_tools::ExcelError::OpFailed {
            op: "set_rows (#0)".to_string(),
            msg: "Некорректные аргументы: Лист 'Продажи' не найден".to_string(),
        });
        assert!(matches!(mapped, ToolError::Usage(_)), "{mapped}");
        assert!(mapped.to_string().contains("Лист 'Продажи' не найден"), "{mapped}");
    }

    #[test]
    fn apply_new_book_auto_creates_sheets_from_ops() {
        // §00.2: для новой книги отказ «Лист 'X' не найден» невозможен —
        // пре-пасс создаёт листы из ops (модель не обязана звать add_sheet).
        let (ctx, _registry) = ctx_with_registry("apply_autosheet");
        let out = excel_session_outputs_dir(ctx.session_id);
        let _ = std::fs::remove_dir_all(&out);
        let args = serde_json::json!({
            "ops": [
                {"op": "set_rows", "sheet": "Продажи", "addr": "A1",
                 "rows": [["Дата", "Товар", "Цена"], ["2026-10-01", "Ноутбук", 45000]]}
            ]
        });
        let result = execute_excel_tool("excel_apply", &args, &ctx, &[])
            .expect("инструмент диспетчеризован")
            .expect("новая книга: лист из ops создан пре-пассом");
        assert!(result.contains("\"ok_"), "{result}");
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn apply_existing_book_missing_sheet_is_honest_usage() {
        // Для СУЩЕСТВУЮЩЕЙ книги режим строгий: опечатка в имени листа —
        // честный отказ с именем операции и причиной (не Io-маска).
        let (ctx, _registry) = ctx_with_registry("apply_strict");
        // Гейт источников читает существующие книги только из session outputs —
        // source-книга создаётся в разрешённой зоне, не в sandbox.
        let dir = excel_session_outputs_dir(ctx.session_id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("source.xlsx");
        write_sample_book(&file);
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "ops": [
                {"op": "set_rows", "sheet": "Нет_такого", "addr": "A1",
                 "rows": [["a"], ["b"]]}
            ]
        });
        let err = execute_excel_tool("excel_apply", &args, &ctx, &[])
            .expect("инструмент диспетчеризован")
            .expect_err("опечатка в листе существующей книги — отказ");
        assert!(matches!(err, ToolError::Usage(_)), "{err}");
        assert!(err.to_string().contains("Нет_такого"), "{err}");
        assert!(err.to_string().contains("set_rows"), "в ошибке нет имени операции: {err}");
        let _ = std::fs::remove_dir_all(&dir);
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
    fn source_gate_allows_double_dot_in_filename() {
        // T-B4: «бюджет..2024.xlsx» — легальное имя (точки внутри имени файла,
        // а не компонент «..»); подстрочный contains("..") давал ложный отказ.
        let (ctx, _registry) = ctx_with_registry("gate_dd");
        let dir = sandbox("gate_dd");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("бюджет..2024.xlsx");
        std::fs::write(&file, b"stub").unwrap();
        let allowed = vec![file.to_string_lossy().to_string()];
        let ok = ensure_allowed_source(file.to_string_lossy().as_ref(), &ctx, &allowed)
            .expect("имя с двумя точками подряд — легальный файл");
        assert_eq!(ok, file);
    }

    #[test]
    fn source_gate_rejects_parent_traversal() {
        // T-B4: поднимающийся компонент «..» остаётся запрещён.
        let (ctx, _registry) = ctx_with_registry("gate_trav");
        let dir = sandbox("gate_trav");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("real.xlsx");
        std::fs::write(&file, b"stub").unwrap();
        let allowed = vec![file.to_string_lossy().to_string()];
        let evil = format!("{}\\..\\real.xlsx", dir.display());
        let err = ensure_allowed_source(evil.as_str(), &ctx, &allowed).unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)), "{err}");
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
        let read = excel_tools::inspect::read_range(Path::new(&path), Some("Sheet1"), Some("A1:B3"))
            .expect("чтение созданной книги");
        assert!(read.to_string().contains("B2*2"), "формула живая в файле: {read}");
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn apply_rejects_oversized_set_rows_block_before_pipeline() {
        // Механический потолок payload'а (инвариант бюджета генерации):
        // слишком большой блок отклоняется мгновенно, без запуска пайплайна —
        // иначе тихий MAX_TOKENS и смерть в докачках (кейс 03-04.11.26).
        let (ctx, _registry) = ctx_with_registry("apply_gate");
        // 26 строк × 8 ячеек = 208 ячеек > потолка 200.
        let big_rows = vec![vec![serde_json::Value::from(0); 8]; 26];
        let args = serde_json::json!({"ops": [
            {"op": "set_rows", "sheet": "Sheet1", "addr": "A1", "rows": big_rows}
        ]});
        let err = execute_excel_tool("excel_apply", &args, &ctx, &[])
            .unwrap()
            .unwrap_err();
        assert!(err.to_string().contains("слишком велик"), "{err}");
        // Пограничный блок ровно в потолок (200 ячеек = 25×8) гейт проходит;
        // дальше возможна только валидация самой книги — важно, что это НЕ
        // отказ гейта.
        let ok_rows = vec![vec![serde_json::Value::from(0); 8]; 25];
        let args_ok = serde_json::json!({"ops": [
            {"op": "set_rows", "sheet": "Sheet1", "addr": "A1", "rows": ok_rows}
        ]});
        match execute_excel_tool("excel_apply", &args_ok, &ctx, &[]) {
            None => panic!("excel_apply должен диспетчеризоваться"),
            Some(Ok(text)) => assert!(!text.contains("слишком велик"), "{text}"),
            Some(Err(e)) => assert!(!e.to_string().contains("слишком велик"), "{e}"),
        }
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
