//! 🩺 Диагностические тулы (`diag_snapshot`, `diag_recorder_start`,
//! `diag_recorder_stop`).
//!
//! Тонкий исполняющий слой поверх SSOT-ядра `ko-diag-tools` (аудит системы
//! через sysinfo/реестр/PowerShell, криминалистика дампов и журнала Windows,
//! рекордер «чёрного ящика» с принудительным сбросом каждой строки на диск).
//! По образцу excel-адаптера: схемы агентов — через `diag_tool_schemas`
//! (мета-имя "diag"), исполнение — через диспетчер (`execute_diag_tool`).
//!
//! Гейт записи НЕ нужен (excel-гейт про файлы юзера): diag-тулы не читают
//! произвольные файлы диска и пишут только в собственную папку сессии
//! `<exe>/diag_outputs/<session_id>/`.
//!
//! Реестр рекордеров — единственный модульный static фичи (прецедент
//! `EXCEL_OUTPUT_SEQUENCE`): `ToolCtx` не расширяется ради одной возможности,
//! ключ — `session_id`, значение — управление живущим фоновым потоком.
//! Рекордер умирает вместе с процессом King Orch; после перезапуска хоста
//! лог прошлой жизни находится на диске (`diag_recorder_stop` режим B).

use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::infra::tools::{Tool, ToolCtx, ToolError};

/// Все имена diag-тулов (фильтр capability-гейта диспетчера).
pub const DIAG_TOOL_NAMES: [&str; 3] =
    ["diag_snapshot", "diag_recorder_start", "diag_recorder_stop"];

pub fn is_diag_tool(name: &str) -> bool {
    DIAG_TOOL_NAMES.contains(&name)
}

/// Папка чёрного ящика сессии: рядом с exe — `diag_outputs/<session_id>/`
/// (правило cwd: current_exe().parent(), НЕ app.path()).
pub fn diag_session_outputs_dir(session_id: &str) -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe_dir.join("diag_outputs").join(session_id)
}

// ─────────── Реестр живущих рекордеров (единственный static фичи) ───────────

type RecorderRegistry = Mutex<HashMap<String, ko_diag_tools::recorder::RecorderControl>>;

/// См. док модуля: сознательный модульный static, ключ — session_id.
static RECORDER_REGISTRY: OnceLock<RecorderRegistry> = OnceLock::new();

fn recorder_registry() -> &'static RecorderRegistry {
    RECORDER_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

// ─────────────────────────── Исполнение тулов ───────────────────────────

/// `diag_snapshot`: полный аудит + криминалистика. Каталог рекордера
/// создаётся заранее — секция свежих логов отвечает «нет логов», а не
/// «каталог не читается» для ещё-никогда-не-регистрировавшей сессии.
fn run_snapshot(session_id: &str) -> Result<String, ToolError> {
    let dir = diag_session_outputs_dir(session_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| ToolError::Io(format!("создание каталога diag_outputs: {e}")))?;
    let diag = ko_diag_tools::snapshot::run_snapshot(Some(&dir));
    Ok(diag.render())
}

/// `diag_recorder_start`: запустить фоновый мониторинг. Повторный запуск
/// той же сессии — замена: живой рекордер останавливается (его итог честно
/// упоминается в ответе), стартует новый. Один рекордер на сессию.
fn run_recorder_start(args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
    let interval_default = ko_diag_tools::recorder::RecorderConfig::default().interval;
    let interval_ms = args
        .get("interval_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| interval_default.as_millis() as u64);
    if !(200..=5000).contains(&interval_ms) {
        return Err(ToolError::Usage(format!(
            "interval_ms допускается 200..5000 (минимум 200 — sysinfo::MINIMUM_CPU_UPDATE_INTERVAL, ниже статистика ЦП не обновляется; получено: {interval_ms})"
        )));
    }
    let cfg = ko_diag_tools::recorder::RecorderConfig {
        interval: Duration::from_millis(interval_ms),
        ..Default::default()
    };
    let dir = diag_session_outputs_dir(ctx.session_id);

    // Замена: живой прошлый рекордер этой сессии честно останавливается.
    let previous = recorder_registry()
        .lock()
        .unwrap()
        .remove(ctx.session_id);
    let replaced_note = previous.map(|control| {
        let summary = ko_diag_tools::recorder::stop_recording(
            control,
            &ko_diag_tools::recorder::RecorderConfig::default(),
        );
        format!(
            "Прежний рекордер сессии остановлен ({} тиков, файл {}); стартует новый.",
            summary.ticks,
            summary.path.display()
        )
    });

    let control = ko_diag_tools::recorder::start_recording(&dir, cfg)
        .map_err(|e| ToolError::Io(format!("рекордер не запущен: {e}")))?;
    // Путь читается ДО move в реестр (RecorderControl не Copy).
    let path = control.path().display().to_string();
    // Реестр — SSOT «кто жив»: без него stop-тул не нашёл бы рекордер и
    // ушёл бы в режим «лог прошлой жизни» уже при живой записи.
    recorder_registry()
        .lock()
        .unwrap()
        .insert(ctx.session_id.to_string(), control);
    log::info!(
        "[DIAG] recorder started (agent {}, session {}): {}",
        ctx.agent_id,
        ctx.session_id,
        path
    );

    let note = args
        .get("note")
        .and_then(|v| v.as_str())
        .unwrap_or("воспроизведи проблему; когда закончится — diag_recorder_stop");
    let mut lines = vec![
        "=== РЕКОРДЕР «ЧЁРНОГО ЯЩИКА» ЗАПУЩЕН ===".to_string(),
        format!("Файл лога: {path}"),
        format!("Интервал: {interval_ms} мс | лимиты: 60 мин / 10 МБ (авто-остановка)"),
        format!("Действие: {note}"),
    ];
    if let Some(prev) = replaced_note {
        lines.insert(1, prev);
    }
    lines.push(
        "Важно: рекордер пишется на диск после КАЖДОЙ строки (sync_all) — при намертво зависшей системе последняя строка выживет.".to_string(),
    );
    Ok(lines.join("\n"))
}

/// Хвост лога (кэп 200 строк) — общий для обоих режимов stop.
fn capped_tail_lines(args: &Value) -> usize {
    args.get("tail_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(40)
        .clamp(1, 200) as usize
}

/// Текстовый блок стоп-итога — формат для LLM-анализа и юзера.
fn format_stop_summary(
    summary: &ko_diag_tools::recorder::StopSummary,
    header: &str,
) -> String {
    let mut lines = vec![
        header.to_string(),
        format!("Файл лога: {}", summary.path.display()),
        format!("Начало записи: {}", summary.started_at.format("%Y-%m-%d %H:%M:%S")),
        format!("Записанных тиков: {}", summary.ticks),
    ];
    if let Some(err) = &summary.write_error {
        lines.push(format!("ОШИБКА ЗАПИСИ (см. причину; свежие данные могли не уйти): {err}"));
    }
    if let Some(auto) = &summary.auto_stopped {
        lines.push(format!("АВТО-ОСТАНОВКА: {auto} (лимет — не «виноват», просто потолок)"));
    }
    lines.push(format!("Хвост лога (последние {} строк):", summary.tail.len()));
    for line in &summary.tail {
        lines.push(format!("  {line}"));
    }
    lines.join("\n")
}

/// `diag_recorder_stop` — двухрежимный (перезагрузка машины НЕ теряет данные):
/// A) рекордер живой/учтённый → остановка + хвост;
/// B) реестр пуст (перезапуск хоста после зависания) → свежий лог прошлой
///    жизни через `latest_log` и его хвост;
/// C) и того и другого нет → честный NotFound (не паника, не тишина).
fn run_recorder_stop(args: &Value, ctx: &ToolCtx) -> Result<String, ToolError> {
    let tail_lines = capped_tail_lines(args);
    let dir = diag_session_outputs_dir(ctx.session_id);

    // Режим A: есть учтённый рекордер этой сессии.
    let control = recorder_registry()
        .lock()
        .unwrap()
        .remove(ctx.session_id);
    if let Some(control) = control {
        let cfg = ko_diag_tools::recorder::RecorderConfig {
            tail_lines,
            ..Default::default()
        };
        let summary = ko_diag_tools::recorder::stop_recording(control, &cfg);
        log::info!(
            "[DIAG] recorder stopped (agent {}, ticks {}): {}",
            ctx.agent_id,
            summary.ticks,
            summary.path.display()
        );
        return Ok(format_stop_summary(&summary, "=== РЕКОРДЕР ОСТАНОВЛЕН ==="));
    }

    // Режим B: реестр пуст — ищем лог прошлой жизни (зависание + перезапуск).
    match ko_diag_tools::recorder::latest_log(&dir) {
        Some(path) => {
            let tail = ko_diag_tools::recorder::read_log_tail(&path, tail_lines);
            log::info!(
                "[DIAG] recorder stop в пустом реестре: найден лог прошлой жизни {}",
                path.display()
            );
            let mut lines = vec![
                "=== ЛОГ РЕКОРДЕРА ПРОШЛОГО ПРОГОНА ===".to_string(),
                "Рекордер этой сессии НЕ был запущен (King Orch перезапускался —вероятно, машина зависла). Найден свежий лог на диске:".to_string(),
                format!("Файл лога: {}", path.display()),
            ];
            if tail.is_empty() {
                lines.push("Хвост: файл не читается или пуст (проверь права доступа).".to_string());
            } else {
                lines.push(format!("Хвост лога (последние {} строк):", tail.len()));
                for line in &tail {
                    lines.push(format!("  {line}"));
                }
            }
            Ok(lines.join("\n"))
        }
        None => {
            log::warn!("[DIAG] recorder stop без рекордера и без логов (agent {})", ctx.agent_id);
            Err(ToolError::NotFound(format!(
                "рекордер в этой сессии не запускался и свежих логов diag_recorder_*.log в {} нет. \
                 Сначала вызови diag_recorder_start, воспроизведи проблему, затем diag_recorder_stop.",
                dir.display()
            )))
        }
    }
}

/// Исполнить diag-тул по имени (диспетчер наподобие `execute_excel_tool`).
pub fn execute_diag_tool(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
) -> Option<Result<String, ToolError>> {
    let outcome = match name {
        "diag_snapshot" => run_snapshot(ctx.session_id),
        "diag_recorder_start" => run_recorder_start(args, ctx),
        "diag_recorder_stop" => run_recorder_stop(args, ctx),
        _ => return None,
    };
    Some(outcome)
}

// ─────────────────── Tool-обёртки (схемы для промптов) ───────────────────

pub struct DiagSnapshot;
impl Tool for DiagSnapshot {
    fn name(&self) -> &str { "diag_snapshot" }
    fn description(&self) -> &str {
        "Полный аудит системы ПРЯМО СЕЙЧАС + криминалистика прошлых сбоев: ОС/аптайм, ЦП (с замечанием по Intel 13/14), ОЗУ и файл подкачки, видеокарта/драйверы, изоляция ядра (HVCI), TDR-тайминги, живые дампы ядра и дампы BSOD за 5 дней, журнал Windows за 72 ч (WHEA/Kernel-Power/недостаток памяти/видеодрайвер/диски), краши приложений, топ-10 процессов по CPU/RAM, свежие логи «чёрного ящика». Статусы секций: OK / НАЙДЕНО / НЕ УДАЛОСЬ (с причиной) — секции с «НЕ УДАЛОСЬ» нельзя использовать для выводов."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {}, "required": [] })
    }
    fn is_readonly(&self) -> bool { true }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage("diag_snapshot должен исполняться через диспетчер".to_string()))
    }
}

pub struct DiagRecorderStart;
impl Tool for DiagRecorderStart {
    fn name(&self) -> &str { "diag_recorder_start" }
    fn description(&self) -> &str {
        "Запустить «чёрный ящик»: фоновая запись каждые 400 мс (интервал настраивается) — загрузка ЦП/ОЗУ/свопа, топ-процессы и КАЖДЫЙ новый процесс. Каждая строка сбрасывается на диск мимо кэша (sync_all) — последняя строка выживает при намертво зависшей системе. Повторный вызов останавливает прежний рекордер сессии и начинает новый. Лимиты: 60 мин / 10 МБ (авто-остановка). Порядок работы: запусти рекордер → попроси юзера воспроизвести проблему → останови и анализируй хвост."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "interval_ms": {"type": "integer", "description": "Интервал тиков в мс, 200..5000 (по умолчанию 400). Для игр/зависаний оставляй дефолт; для долгих «тормозов» можно 1000."},
                "note": {"type": "string", "description": "Что юзеру воспроизвести (приписано к ответу)."},
            },
            "required": []
        })
    }
    fn is_readonly(&self) -> bool { false }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage("diag_recorder_start должен исполняться через диспетчер".to_string()))
    }
}

pub struct DiagRecorderStop;
impl Tool for DiagRecorderStop {
    fn name(&self) -> &str { "diag_recorder_stop" }
    fn description(&self) -> &str {
        "Остановить «чёрный ящик» и получить хвост лога (последние строки — самые важные: непосредственно перед зависанием). Работает и ПОСЛЕ перезагрузки машины: если рекордер не дожил до остановки, находится свежий лог на диске и возвращается его хвост с указанием файла. Ищет новые процессы + пики нагрузки + что происходило перед смертью."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tail_lines": {"type": "integer", "description": "Сколько последних строк вернуть (по умолчанию 40, максимум 200)."}
            },
            "required": []
        })
    }
    fn is_readonly(&self) -> bool { false }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> Result<String, ToolError> {
        Err(ToolError::Usage("diag_recorder_stop должен исполняться через диспетчер".to_string()))
    }
}

/// Схемы diag-тулов для промпта агента (мета-имя "diag").
pub fn diag_tool_schemas(agent_tools: &[String]) -> Vec<(String, String, Value)> {
    let tools: [&dyn Tool; 3] = [&DiagSnapshot, &DiagRecorderStart, &DiagRecorderStop];
    let mut out = Vec::new();
    for tool in tools {
        if agent_tools.iter().any(|t| t == tool.name()) {
            out.push((
                "diag".to_string(),
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

    fn ctx(session: &str) -> ToolCtx<'static> {
        ToolCtx {
            workspace_root: Path::new("."),
            write_root: Path::new("."),
            write_outside: crate::infra::WriteOutside::Prompt,
            session_id: Box::leak(session.to_string().into_boxed_str()),
            approver: crate::infra::test_approver(),
            agent_id: "diag_specialist",
            bins_dir: Path::new("."),
            image_artifacts: None,
        }
    }

    #[test]
    fn schemas_only_for_granted_tools() {
        let none = diag_tool_schemas(&[]);
        assert!(none.is_empty(), "нет выданных diag-тулов → нет схем");
        let one = diag_tool_schemas(&["diag_snapshot".to_string()]);
        assert_eq!(one.len(), 1, "только granted-тул");
        assert_eq!(one[0].0, "diag", "мета-имя diag");
        assert_eq!(one[0].1, "diag_snapshot");
        let all = diag_tool_schemas(&[
            "diag_snapshot".to_string(),
            "diag_recorder_start".to_string(),
            "diag_recorder_stop".to_string(),
        ]);
        let names: Vec<&str> = all.iter().map(|(_, n, _)| n.as_str()).collect();
        assert_eq!(names, DIAG_TOOL_NAMES.to_vec());
    }

    #[test]
    fn is_diag_tool_names_and_exclusions() {
        assert!(is_diag_tool("diag_snapshot"));
        assert!(is_diag_tool("diag_recorder_start"));
        assert!(is_diag_tool("diag_recorder_stop"));
        // Не ловит чужие имена и префиксы.
        assert!(!is_diag_tool("excel_inspect"));
        assert!(!is_diag_tool("diagnostic"));
        assert!(!is_diag_tool("generate_image"));
    }

    #[test]
    fn unknown_tool_returns_none() {
        let args = serde_json::json!({});
        assert!(execute_diag_tool("diag_nope", &args, &ctx("unknown")).is_none());
        assert!(execute_diag_tool("excel_apply", &args, &ctx("unknown")).is_none());
    }

    #[test]
    fn snapshot_report_has_statuses_and_sections() {
        // Медленный тест (PowerShell ×3) — но это CORE-путь тулa.
        let result = execute_diag_tool(
            "diag_snapshot",
            &serde_json::json!({}),
            &ctx("snapshot_ok"),
        )
        .unwrap()
        .expect("snapshot исполняется без отказанных прав на core-путь");
        assert!(result.contains("ДИАГНОСТИКА ПК"), "шапка: {result}");
        assert!(
            result.contains("[OK]") && result.contains("[НАЙДЕНО]") && result.contains("[НЕ УДАЛОСЬ]")
            || result.contains("[OK]") && result.contains("[НАЙДЕНО]")
            || result.contains("[OK]") && result.contains("[НЕ УДАЛОСЬ]"),
            "минимум OK + любой из остальных статусов в рендере"
        );
        assert!(result.contains("### "), "секции существуют");
    }

    #[test]
    fn recorder_start_replaces_previous_and_stop_returns_summary() {
        let args = serde_json::json!({"interval_ms": 200});
        let started = execute_diag_tool("diag_recorder_start", &args, &ctx("replace_run"))
            .unwrap()
            .expect("старт рекордера");
        assert!(started.contains("Файл лога:"), "{started}");
        // Повторный старт той же сессии: замена (старый остановлен) → новый.
        let started2 = execute_diag_tool("diag_recorder_start", &args, &ctx("replace_run"))
            .unwrap()
            .expect("второй старт — замена, не отказ");
        assert!(started2.contains("Прежний рекордер сессии остановлен"), "{started2}");
        // Стоп: хвост в ответе, файл на диске.
        std::thread::sleep(std::time::Duration::from_millis(700));
        let stopped = execute_diag_tool("diag_recorder_stop", &args, &ctx("replace_run"))
            .unwrap()
            .expect("стоп живого рекордера");
        assert!(stopped.contains("=== РЕКОРДЕР ОСТАНОВЛЕН ==="), "{stopped}");
        assert!(stopped.contains("Хвост лога"), "{stopped}");
        // Текущий тик есть в хвосте (механика write_synced).
        assert!(stopped.contains("ЦП"), "тиковая строка с ЦП в хвосте: {stopped}");
    }

    #[test]
    fn stop_without_recorder_reads_last_life_log() {
        // Режим B: registry пуст, на диске есть лог (симуляция «перезапуск
        // после зависания»). Строим файл с именами рекордера в sandbox и
        // подкладываем его в outputs сессии.
        let session = "past_life";
        let dir = diag_session_outputs_dir(session);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join(format!("{}1.log", ko_diag_tools::recorder::FILE_PREFIX));
        std::fs::write(&log, "12:00:00.1 ЦП 41% | ОЗУ ...\n12:00:00.5 ЦП 42% | ОЗУ ...\n").unwrap();
        let args = serde_json::json!({"tail_lines": 2});
        let result = execute_diag_tool("diag_recorder_stop", &args, &ctx(session))
            .unwrap()
            .expect("режим B: чтение лога прошлой жизни");
        assert!(result.contains("ПРОШЛОГО ПРОГОНА"), "{result}");
        assert!(result.contains("ЦП 42%"), "хвост содержит последние строки: {result}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stop_with_nothing_is_honest_notfound() {
        // Ни рекордера, ни логов → честный отказ, а не тишина (§2.2).
        let session = "no_recorder_at_all";
        let dir = diag_session_outputs_dir(session);
        let _ = std::fs::remove_dir_all(&dir);
        let err = execute_diag_tool("diag_recorder_stop", &serde_json::json!({}), &ctx(session))
            .unwrap()
            .expect_err("должен быть NotFound");
        assert!(matches!(err, ToolError::NotFound(_)), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn start_interval_gate_is_honest() {
        let err = execute_diag_tool(
            "diag_recorder_start",
            &serde_json::json!({"interval_ms": 30}),
            &ctx("bad_interval"),
        )
        .unwrap()
        .expect_err("30 мс — Usage-отказ");
        assert!(matches!(err, ToolError::Usage(_)), "{err}");
        assert!(err.to_string().contains("200"), "причина называет минимум: {err}");
    }

    #[test]
    fn outputs_dir_is_session_scoped() {
        let dir = diag_session_outputs_dir("sess-1");
        assert!(dir.to_string_lossy().contains("diag_outputs\\sess-1"), "{dir:?}");
        let other = diag_session_outputs_dir("sess-2");
        assert_ne!(dir, other, "сессии не пересекаются");
    }
}
