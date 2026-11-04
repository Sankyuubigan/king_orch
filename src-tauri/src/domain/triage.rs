//! 🩺 Триаж моделей: 7 пробников известных классов отказов малых моделей.
//!
//! Классы доказаны 14-модельным прогоном 04.11.2026 и интернет-разведкой
//! (tasks/261104_1822-cdabe7fa/): попугайство, think-жор, yes-bias/штамп,
//! рестарт-вместо-продолжения, петли повтора, JSON-надёжность, instruction following.
//!
//! Чистая доменная логика: только `LlmEngine::generate_chat` + анализ текста —
//! БЕЗ агентов, графов и MCP. Промпты пробников GENERIC: никаких кейсов
//! пользователей (правило запрета хардкода юзер-кейсов в бизнес-логике).
//!
//! Хелперы `normalized`/`is_continuation_restart` — SSOT: их же использует
//! orchestrator в детекте рестарта докачки (не дублировать).

use crate::infra::config::ModelParams;
use crate::infra::llm_types::{GenerationResult, LlmMessage};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Статус одного пробника.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TriageStatus {
    Pass,
    Warn,
    Fail,
    /// Пробник не выполнен (отмена юзера или ошибка движка) — в вердикт не идёт.
    Skip,
}

/// Вердикт одного пробника.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TriageVerdict {
    pub probe_id: String,
    pub title: String,
    pub status: TriageStatus,
    pub detail: String,
    #[serde(default)]
    pub metrics: serde_json::Value,
}

/// Отчёт по одной модели (одна запись триаж-батча).
#[derive(Serialize, Deserialize, Clone)]
pub struct TriageModelReport {
    pub model: String,
    pub verdict_label: String,
    /// Ошибка запуска (движок не поднялся) — пробников нет.
    pub error: Option<String>,
    pub probes: Vec<TriageVerdict>,
    pub duration_ms: u64,
    #[serde(default)]
    pub ram_peak_mb: Option<f64>,
    #[serde(default)]
    pub vram_peak_mb: Option<f64>,
}

/// Структурные пробники: их fail/warn определяет итоговый вердикт.
/// Пробники 5-7 — качество-детали (петли, JSON, инструкции).
const STRUCTURAL_PROBES: [&str; 4] = ["parroting", "think_share", "yes_bias", "continuation"];

pub const PROBE_TITLES: [(&str, &str); 7] = [
    ("parroting", "Попугайство (эхо контекста)"),
    ("think_share", "Think-жор бюджета"),
    ("yes_bias", "Yes-bias / штамп «всё true»"),
    ("continuation", "Продолжение с обрыва"),
    ("loops", "Петли повтора"),
    ("json_reliability", "JSON-надёжность (без грамматики)"),
    ("ifeval_lite", "Instruction following (lite)"),
];

/// Итоговый вердикт по списку пробников: fail на структурных → «МУСОР»,
/// warn на структурных → «Рискованная», иначе «Годна». Все Skip → «Не удалось проверить».
pub fn verdict_label(probes: &[TriageVerdict]) -> String {
    let structural = |id: &str| {
        probes
            .iter()
            .find(|p| p.probe_id == id)
            .map(|p| p.status)
    };
    let structural_all_skip = STRUCTURAL_PROBES
        .iter()
        .all(|id| structural(id) == Some(TriageStatus::Skip));
    if structural_all_skip {
        return "Не удалось проверить (ошибка движка или отмена)".to_string();
    }
    let has = |st: TriageStatus| {
        STRUCTURAL_PROBES
            .iter()
            .any(|id| structural(id) == Some(st))
    };
    if has(TriageStatus::Fail) {
        "МУСОР для System-2 (структурный отказ)".to_string()
    } else if has(TriageStatus::Warn) {
        "Рискованная для System-2".to_string()
    } else {
        "Годна для System-2".to_string()
    }
}

// ─── SSOT-хелперы (переиспользуются orchestrator'ом) ───

/// Нормализация текста: collapse всех пробельных последовательностей в один пробел
/// (регистр сохраняется — сравнение префиксов чувствительно к регистру, это ок:
/// рестарт повторяет текст дословно).
pub fn normalized(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Длина общего префикса двух строк посимвольно (по char, не по байтам).
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// Детект «рестарт-класса»: piece повторяет начало original вместо продолжения.
///
/// Критерий: общий нормализованный префикс ≥60 символов. Короткий original
/// (<80 норм-символов) не показателен — совпадение может быть случайным.
pub fn is_continuation_restart(original: &str, piece: &str) -> bool {
    let o = normalized(original);
    if o.chars().count() < 80 {
        return false;
    }
    let p = normalized(piece);
    common_prefix_len(&o, &p) >= 60
}

/// Разделить текст на (тело think, видимый ответ): пары `<think>…</think>` +
/// незакрытый `<think>` (весь хвост — think: обрыв по бюджету думателя).
pub fn split_think(text: &str) -> (String, String) {
    let mut think = String::new();
    let mut visible = String::new();
    let mut rest = text;
    loop {
        match rest.find("<think>") {
            None => {
                visible.push_str(rest);
                break;
            }
            Some(i) => {
                visible.push_str(&rest[..i]);
                let after = &rest[i + "<think>".len()..];
                match after.find("</think>") {
                    Some(j) => {
                        think.push_str(&after[..j]);
                        rest = &after[j + "</think>".len()..];
                    }
                    None => {
                        // Незакрытый think — весь хвост считается размышлением.
                        think.push_str(after);
                        break;
                    }
                }
            }
        }
    }
    (think, visible)
}

// ─── Токенизация и n-граммы (попугайство) ───

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

/// Доля 4-грамм ответа, встречающихся в документе (попугайство: >30% fail, >12% warn).
fn gram_overlap(answer: &str, doc: &str, n: usize) -> f64 {
    let aw = words(answer);
    if aw.len() < n {
        return 0.0;
    }
    let doc_set: std::collections::HashSet<Vec<String>> =
        words(doc).windows(n).map(|w| w.to_vec()).collect();
    let grams: Vec<&[String]> = aw.windows(n).collect();
    let hits = grams.iter().filter(|g| doc_set.contains(&g.to_vec())).count();
    hits as f64 / grams.len() as f64
}

// ─── Парсеры вердиктов (yes_bias, json_reliability) ───

/// Первый сбалансированный `{...}` в тексте (учёт кавычек/скобок в строках — грубо).
fn first_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    for (i, c) in text[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=start + i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Вытащить булев вердикты s1..s5: сначала JSON, фолбэк — скан «"sN" … true/false».
fn parse_bool_verdicts(text: &str) -> Option<[bool; 5]> {
    let mut out = [false; 5];
    let mut found = 0usize;
    if let Some(obj) = first_json_object(text) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(obj) {
            for i in 1..=5 {
                if let Some(b) = v.get(format!("s{}", i)).and_then(|x| x.as_bool()) {
                    out[i - 1] = b;
                    found += 1;
                }
            }
            if found == 5 {
                return Some(out);
            }
        }
    }
    // Фолбэк: последовательный скан ключей s1..s5 → ближайшие true/false после ключа.
    found = 0;
    for i in 1..=5 {
        let key = format!("\"s{}\"", i);
        let Some(kpos) = text.find(&key) else { break };
        let tail = &text[kpos + key.len()..];
        let t_true = tail.find("true");
        let t_false = tail.find("false");
        let (val, is_true) = match (t_true, t_false) {
            (Some(a), Some(b)) => {
                if a < b {
                    (a, true)
                } else {
                    (b, false)
                }
            }
            (Some(_), None) => (t_true.unwrap(), true),
            (None, Some(_)) => (t_false.unwrap(), false),
            (None, None) => break,
        };
        // Верникт должен стоять до следующего ключа (иначе ключ пропущен моделью).
        if let Some(next) = (i + 1..=5).find_map(|j| tail.find(&format!("\"s{}\"", j))) {
            if val > next {
                break;
            }
        }
        out[i - 1] = is_true;
        found += 1;
    }
    if found == 5 {
        Some(out)
    } else {
        None
    }
}

/// Первый сбалансированный JSON-объект, распарсенный как Value.
fn parse_first_json(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str(first_json_object(text)?).ok()
}

// ─── Прогон ───

/// Один вызов генерации через движок (sync-блокирующий — как весь тестовый контур).
#[allow(clippy::too_many_arguments)]
fn gen(
    engine: &crate::infra::LlmEngine,
    params: &ModelParams,
    messages: &[LlmMessage],
    max_tokens: usize,
    disable_reasoning: bool,
    cancel_flag: &Arc<AtomicBool>,
    label: &str,
    log_cb: &dyn Fn(String),
) -> Result<GenerationResult, String> {
    engine.generate_chat(
        messages,
        max_tokens,
        params,
        "Auto",
        disable_reasoning,
        cancel_flag.clone(),
        label,
        None,
        |_, _: &str| {},
        |m: String| log_cb(m),
    )
}

fn user_msg(content: &str) -> LlmMessage {
    LlmMessage {
        role: "user".to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

fn assistant_msg(content: &str) -> LlmMessage {
    LlmMessage {
        role: "assistant".to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

const STOP_MAX_TOKENS: &str = "max_tokens";

fn is_max_tokens(stop_reason: &str) -> bool {
    stop_reason.eq_ignore_ascii_case(STOP_MAX_TOKENS)
}

/// Прогнать все 7 пробников на одной модели. Каждый пробник изолирован: его
/// ошибка (отмена/упавший движок) даёт Skip, а не роняет весь триаж.
pub fn run_triage(
    engine: &crate::infra::LlmEngine,
    model_params: &ModelParams,
    cancel_flag: Arc<AtomicBool>,
    log_cb: &dyn Fn(String),
    progress_cb: &dyn Fn(usize, &str),
) -> Vec<TriageVerdict> {
    let cancelled = || cancel_flag.load(Ordering::Relaxed);
    let mut out: Vec<TriageVerdict> = Vec::with_capacity(PROBE_TITLES.len());
    // Текст пробника 4 — на нём работает и пробник 5 (петли).
    let mut continuation_text = String::new();

    for (idx, (probe_id, title)) in PROBE_TITLES.iter().enumerate() {
        let idx = idx + 1;
        progress_cb(idx, title);
        if cancelled() {
            out.push(TriageVerdict {
                probe_id: probe_id.to_string(),
                title: title.to_string(),
                status: TriageStatus::Skip,
                detail: "Отменено пользователем".to_string(),
                metrics: serde_json::Value::Null,
            });
            continue;
        }
        let verdict = match *probe_id {
            "parroting" => probe_parroting(engine, model_params, cancel_flag.clone(), log_cb),
            "think_share" => probe_think_share(engine, model_params, cancel_flag.clone(), log_cb),
            "yes_bias" => probe_yes_bias(engine, model_params, cancel_flag.clone(), log_cb),
            "continuation" => {
                let (v, text) =
                    probe_continuation(engine, model_params, cancel_flag.clone(), log_cb);
                continuation_text = text;
                v
            }
            "loops" => probe_loops(&continuation_text),
            "json_reliability" => {
                probe_json_reliability(engine, model_params, cancel_flag.clone(), log_cb)
            }
            "ifeval_lite" => probe_ifeval_lite(engine, model_params, cancel_flag.clone(), log_cb),
            other => {
                let _ = other;
                continue;
            }
        };
        log_cb(format!(
            "[triage] {}/{} {} → {:?}: {}",
            idx,
            PROBE_TITLES.len(),
            title,
            verdict.status,
            verdict.detail
        ));
        out.push(verdict);
    }
    out
}

fn skip(probe_id: &str, title: &str, detail: String) -> TriageVerdict {
    TriageVerdict {
        probe_id: probe_id.to_string(),
        title: title.to_string(),
        status: TriageStatus::Skip,
        detail,
        metrics: serde_json::Value::Null,
    }
}

fn verdict_of(
    probe_id: &str,
    title: &str,
    status: TriageStatus,
    detail: String,
    metrics: serde_json::Value,
) -> TriageVerdict {
    TriageVerdict {
        probe_id: probe_id.to_string(),
        title: title.to_string(),
        status,
        detail,
        metrics,
    }
}

/// 1. Попугайство: bait-документ-легенда; ответ должен быть СВОИМИ словами.
/// Механизм glm-oss-класса: вербатим-эхо длинного контекста (OWASP LLM07).
fn probe_parroting(
    engine: &crate::infra::LlmEngine,
    params: &ModelParams,
    cancel: Arc<AtomicBool>,
    log_cb: &dyn Fn(String),
) -> TriageVerdict {
    const ID: &str = "parroting";
    const TITLE: &str = "Попугайство (эхо контекста)";
    let doc = "История Поющего моста. В городе Каменке через реку Тихую стоял старый \
деревянный мост. Его построили в 1832 году мастера артели Свиридова, и он имел \
необычную особенность: в ветреную погоду доски моста издавали протяжные звуки, \
похожие на низкое пение. Жители называли его Поющим мостом. Мост пережил три \
большых паводка, но в 1971 году был разобран при строительстве бетонной плотины. \
Доски старого моста частично сохранились: из них собрали настил смотровой площадки \
на берегу, где и сегодня в сильный ветер можно услышать то самое низкое гудение, \
которое когда-то пело над рекой Тихой.";
    let prompt = format!(
        "{}\n\nВопрос: почему мост в Каменке называли Поющим? Ответь ОДНИМ предложением \
СВОИМИ словами. Не цитируй документ и не копируй его формулировки.",
        doc
    );
    let messages = vec![user_msg(&prompt)];
    let gen = match gen(
        engine,
        params,
        &messages,
        300,
        true,
        &cancel,
        "triage:parroting",
        log_cb,
    ) {
        Ok(g) => g,
        Err(e) => return skip(ID, TITLE, format!("Ошибка движка: {}", e)),
    };
    let (think, visible) = split_think(&gen.text);
    let answer = visible.trim().to_string();
    if answer.is_empty() {
        if !think.trim().is_empty() {
            return verdict_of(
                ID, TITLE,
                TriageStatus::Fail,
                "Пустой ответ: весь вывод ушёл в размышления (think-жор)".to_string(),
                serde_json::json!({ "answer_chars": 0, "think_chars": think.chars().count() }),
            );
        }
        return verdict_of(ID, TITLE, TriageStatus::Fail, "Пустой ответ".to_string(),
            serde_json::json!({ "answer_chars": 0 }));
    }
    let overlap = gram_overlap(&answer, doc, 4);
    let pct = (overlap * 100.0).round() as u64;
    let (status, mark) = if overlap > 0.30 {
        (TriageStatus::Fail, "💥")
    } else if overlap > 0.12 {
        (TriageStatus::Warn, "⚠️")
    } else {
        (TriageStatus::Pass, "✅")
    };
    verdict_of(
        ID, TITLE, status,
        format!(
            "{} Пересечение 4-грамм ответа с документом: {}% (пороги: >12% warn, >30% fail)",
            mark, pct
        ),
        serde_json::json!({ "overlap": overlap, "answer_chars": answer.chars().count() }),
    )
}

/// 2. Think-жор: тривиальный вопрос с разумением; меряем долю think от вывода.
/// Класс granite/Spark: думатель съедает бюджет, видимого ответа нет.
fn probe_think_share(
    engine: &crate::infra::LlmEngine,
    params: &ModelParams,
    cancel: Arc<AtomicBool>,
    log_cb: &dyn Fn(String),
) -> TriageVerdict {
    const ID: &str = "think_share";
    const TITLE: &str = "Think-жор бюджета";
    let messages = vec![user_msg("Сколько будет 2+3? Ответь только числом.")];
    let gen = match gen(
        engine,
        params,
        &messages,
        512,
        false,
        &cancel,
        "triage:think_share",
        log_cb,
    ) {
        Ok(g) => g,
        Err(e) => return skip(ID, TITLE, format!("Ошибка движка: {}", e)),
    };
    let (think_text, visible) = split_think(&gen.text);
    let think_chars = gen.reasoning.chars().count() + think_text.chars().count();
    let visible_chars = visible.trim().chars().count();
    if visible_chars == 0 {
        if think_chars > 0 {
            return verdict_of(
                ID, TITLE, TriageStatus::Fail,
                "Думатель без ответа: весь вывод — размышления".to_string(),
                serde_json::json!({ "think_chars": think_chars, "visible_chars": 0, "share": 1.0 }),
            );
        }
        return verdict_of(ID, TITLE, TriageStatus::Fail, "Пустой ответ".to_string(),
            serde_json::json!({ "think_chars": 0, "visible_chars": 0 }));
    }
    let total = think_chars + visible_chars;
    let share = if total > 0 { think_chars as f64 / total as f64 } else { 0.0 };
    let (status, mark) = if share > 0.8 {
        (TriageStatus::Fail, "💥")
    } else if share > 0.5 {
        (TriageStatus::Warn, "⚠️")
    } else {
        (TriageStatus::Pass, "✅")
    };
    verdict_of(
        ID, TITLE, status,
        format!(
            "{} Доля размышлений: {}% (пороги: >50% warn, >80% fail); видимый ответ: {} симв.",
            mark,
            (share * 100.0).round() as u64,
            visible_chars
        ),
        serde_json::json!({ "think_chars": think_chars, "visible_chars": visible_chars, "share": share }),
    )
}

/// 3. Yes-bias: 5 простых утверждений (3 ложных, 2 истинных) с эталоном.
/// Класс четвёрки «всё-true»: априорный true без оценки критерия.
fn probe_yes_bias(
    engine: &crate::infra::LlmEngine,
    params: &ModelParams,
    cancel: Arc<AtomicBool>,
    log_cb: &dyn Fn(String),
) -> TriageVerdict {
    const ID: &str = "yes_bias";
    const TITLE: &str = "Yes-bias / штамп «всё true»";
    // Эталон: [F, T, F, F, T]
    const EXPECTED: [bool; 5] = [false, true, false, false, true];
    let prompt = "Оцени 5 утверждений и верни вердикт по каждому в формате JSON: \
{\"s1\": true, \"s2\": true, \"s3\": true, \"s4\": true, \"s5\": true} — где true означает \
«утверждение верно», false — «неверно». Только JSON, без пояснений.\n\
s1: Солнце вращается вокруг Земли.\n\
s2: Вода кипит при 100 градусах Цельсия (при нормальном атмосферном давлении).\n\
s3: Москва — столица Франции.\n\
s4: Число 7 меньше числа 5.\n\
s5: В году 12 месяцев.";
    let messages = vec![user_msg(prompt)];
    let gen = match gen(
        engine,
        params,
        &messages,
        256,
        true,
        &cancel,
        "triage:yes_bias",
        log_cb,
    ) {
        Ok(g) => g,
        Err(e) => return skip(ID, TITLE, format!("Ошибка движка: {}", e)),
    };
    let (_think, visible) = split_think(&gen.text);
    let Some(verdicts) = parse_bool_verdicts(&visible) else {
        return verdict_of(
            ID, TITLE, TriageStatus::Fail,
            "JSON с вердиктами не распознан (пересекается с JSON-надёжностью)".to_string(),
            serde_json::json!({ "raw": safe_trunc(&gen.text, 200) }),
        );
    };
    let score = verdicts
        .iter()
        .zip(EXPECTED.iter())
        .filter(|(a, e)| a == e)
        .count();
    let all_true = verdicts.iter().all(|v| *v);
    let (status, mark) = if score >= 4 {
        (TriageStatus::Pass, "✅")
    } else if score == 3 {
        (TriageStatus::Warn, "⚠️")
    } else {
        (TriageStatus::Fail, "💥")
    };
    let uniform_note = if all_true { "; ШТАМП: все true" } else { "" };
    verdict_of(
        ID, TITLE, status,
        format!(
            "{} Совпадений с эталоном: {}/5 (эталон [F,T,F,F,T]){}",
            mark, score, uniform_note
        ),
        serde_json::json!({ "score": score, "verdicts": verdicts, "expected": EXPECTED }),
    )
}

/// 4. Продолжение с обрыва: форсируем обрыв малым n_predict, докачиваем
/// assistant-prefill (БЕЗ юзер-хода), детектим рестарт по общему префиксу.
/// Класс granite/Spark/LFM: модель перезапускает ответ вместо продолжения.
/// Возвращает (вердикт, суммарный текст для пробника 5 «петли»).
fn probe_continuation(
    engine: &crate::infra::LlmEngine,
    params: &ModelParams,
    cancel: Arc<AtomicBool>,
    log_cb: &dyn Fn(String),
) -> (TriageVerdict, String) {
    const ID: &str = "continuation";
    const TITLE: &str = "Продолжение с обрыва";
    let prompt = "Напиши короткий рассказ из 10 предложений о маяке на берегу моря.";
    let messages = vec![user_msg(prompt)];
    let first = match gen(
        engine,
        params,
        &messages,
        140,
        true,
        &cancel,
        "triage:continuation",
        log_cb,
    ) {
        Ok(g) => g,
        Err(e) => return (skip(ID, TITLE, format!("Ошибка движка: {}", e)), String::new()),
    };
    if !is_max_tokens(&first.stop_reason) {
        return (
            verdict_of(
                ID, TITLE, TriageStatus::Pass,
                format!("Не обрывается (stop_reason: {})", first.stop_reason),
                serde_json::json!({ "stop_reason": first.stop_reason }),
            ),
            first.text.clone(),
        );
    }
    let (_t1, partial) = split_think(&first.text);
    let partial = partial.trim().to_string();
    if partial.is_empty() {
        return (
            verdict_of(
                ID, TITLE, TriageStatus::Fail,
                "Обрыв внутри размышлений: видимого ответа нет (think-жор)".to_string(),
                serde_json::json!({ "stop_reason": first.stop_reason }),
            ),
            String::new(),
        );
    }
    // Prefill-докачка: последнее сообщение — assistant, БЕЗ юзер-хода «продолжи».
    let messages2 = vec![user_msg(prompt), assistant_msg(&partial)];
    let second = match gen(
        engine,
        params,
        &messages2,
        256,
        true,
        &cancel,
        "triage:continuation2",
        log_cb,
    ) {
        Ok(g) => g,
        Err(e) => return (skip(ID, TITLE, format!("Ошибка движка при докачке: {}", e)), String::new()),
    };
    let (_t2, piece) = split_think(&second.text);
    let piece = piece.trim().to_string();
    if piece.is_empty() {
        return (
            verdict_of(
                ID, TITLE, TriageStatus::Fail,
                "Докачка вернула пустой текст (рестарт внутри размышлений или think-жор)".to_string(),
                serde_json::json!({ "stop2": second.stop_reason, "partial_chars": partial.chars().count() }),
            ),
            String::new(),
        );
    }
    let prefix = common_prefix_len(&normalized(&partial), &normalized(&piece));
    let combined = format!("{} {}", partial, piece);
    if prefix >= 40 {
        return (
            verdict_of(
                ID, TITLE, TriageStatus::Fail,
                format!(
                    "РЕСТАРТ-КЛАСС: докачка повторяет начало ответа (общий префикс {} симв., порог 40)",
                    prefix
                ),
                serde_json::json!({ "prefix_len": prefix, "stop2": second.stop_reason }),
            ),
            combined,
        );
    }
    if is_max_tokens(&second.stop_reason) {
        return (
            verdict_of(
                ID, TITLE, TriageStatus::Warn,
                "Обрывается снова: не завершает рассказ за одну докачку".to_string(),
                serde_json::json!({ "prefix_len": prefix, "stop2": second.stop_reason }),
            ),
            combined,
        );
    }
    (
        verdict_of(
            ID, TITLE, TriageStatus::Pass,
            format!(
                "Продолжила без рестарта (префикс {} симв. < порога 40)",
                prefix
            ),
            serde_json::json!({ "prefix_len": prefix, "stop2": second.stop_reason }),
        ),
        combined,
    )
}

/// 5. Петли повтора: анализ суммарного текста пробника 4 (предложение ≥6 слов,
/// повторившееся ≥3 раз = 💥, 2 = ⚠️). Деградация декодирования (класс Ternary).
fn probe_loops(text: &str) -> TriageVerdict {
    const ID: &str = "loops";
    const TITLE: &str = "Петли повтора";
    if text.trim().is_empty() {
        return skip(ID, TITLE, "Нет текста для анализа (пробник 4 не выполнен)".to_string());
    }
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for raw in text.split(['.', '!', '?', '\n']) {
        let s = raw.trim();
        if words(s).len() >= 6 {
            *counts.entry(normalized(s).to_lowercase()).or_insert(0) += 1;
        }
    }
    let Some((sentence, rep)) = counts.iter().max_by_key(|(_, c)| **c) else {
        return verdict_of(ID, TITLE, TriageStatus::Pass,
            "Предложений достаточной длины нет — петли не ищутся".to_string(),
            serde_json::json!({ "max_repeat": 0 }));
    };
    let rep = *rep;
    let (status, mark) = if rep >= 3 {
        (TriageStatus::Fail, "💥")
    } else if rep == 2 {
        (TriageStatus::Warn, "⚠️")
    } else {
        (TriageStatus::Pass, "✅")
    };
    verdict_of(
        ID, TITLE, status,
        format!(
            "{} Максимум повторов одного предложения (≥6 слов): {} (пороги: 2 warn, ≥3 fail)",
            mark, rep
        ),
        serde_json::json!({ "max_repeat": rep, "sentence": safe_trunc(sentence, 120) }),
    )
}

/// 6. JSON-надёжность: 3 прогона без грамматики — доля валидных JSON.
fn probe_json_reliability(
    engine: &crate::infra::LlmEngine,
    params: &ModelParams,
    cancel: Arc<AtomicBool>,
    log_cb: &dyn Fn(String),
) -> TriageVerdict {
    const ID: &str = "json_reliability";
    const TITLE: &str = "JSON-надёжность (без грамматики)";
    let prompt = "Сгенерируй JSON-объект с полями \"name\" (строка), \"age\" (число) и \
\"city\" (строка) для вымышленного человека. Ответ — только JSON, без пояснений.";
    let messages = vec![user_msg(prompt)];
    let mut valid = 0usize;
    let mut fails: Vec<String> = Vec::new();
    for i in 1..=3 {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let gen = match gen(
            engine,
            params,
            &messages,
            256,
            true,
            &cancel,
            &format!("triage:json{}", i),
            log_cb,
        ) {
            Ok(g) => g,
            Err(e) => return skip(ID, TITLE, format!("Ошибка движка: {}", e)),
        };
        let (_t, visible) = split_think(&gen.text);
        match parse_first_json(&visible) {
            Some(v) if v.get("name").is_some() && v.get("age").is_some() && v.get("city").is_some() => {
                valid += 1;
            }
            _ => fails.push(safe_trunc(&visible, 150)),
        }
    }
    let (status, mark) = if valid == 3 {
        (TriageStatus::Pass, "✅")
    } else if valid >= 1 {
        (TriageStatus::Warn, "⚠️")
    } else {
        (TriageStatus::Fail, "💥")
    };
    let mut detail = format!(
        "{} Валидных JSON с полями name/age/city: {}/3 (без грамматики)",
        mark, valid
    );
    if let Some(first_fail) = fails.first() {
        detail.push_str(&format!("; пример отказа: «{}»", first_fail));
    }
    verdict_of(
        ID, TITLE, status, detail,
        serde_json::json!({ "valid": valid, "total": 3, "fails": fails }),
    )
}

/// 7. Instruction following (lite): 2 машинно-верифицируемых задания IFEval-стиля.
fn probe_ifeval_lite(
    engine: &crate::infra::LlmEngine,
    params: &ModelParams,
    cancel: Arc<AtomicBool>,
    log_cb: &dyn Fn(String),
) -> TriageVerdict {
    const ID: &str = "ifeval_lite";
    const TITLE: &str = "Instruction following (lite)";
    let call = |prompt: &str, n: usize, label: &str| -> Result<String, String> {
        let messages = vec![user_msg(prompt)];
        let g = gen(engine, params, &messages, n, true, &cancel, label, log_cb)?;
        let (_t, visible) = split_think(&g.text);
        Ok(visible)
    };
    // (a) Ровно 3 пункта списка "- ", в каждом ≤5 слов.
    let a_ok = match call(
        "Ответь ровно тремя пунктами списка. Каждый пункт — отдельная строка, \
начинающаяся с \"- \". Тема пунктов: зима. В каждом пункте не больше 5 слов. \
Никакого текста до или после списка.",
        256,
        "triage:ifeval_a",
    ) {
        Ok(text) => {
            let lines: Vec<&str> = text
                .lines()
                .map(|l| l.trim())
                .filter(|l| l.starts_with("- "))
                .collect();
            lines.len() == 3
                && lines.iter().all(|l| l[2..].split_whitespace().count() <= 5)
        }
        Err(e) => return skip(ID, TITLE, format!("Ошибка движка: {}", e)),
    };
    // (b) Ровно 4 слова о кошках.
    let b_ok = match call("Напиши ровно 4 слова о кошках. Только 4 слова.", 128, "triage:ifeval_b") {
        Ok(text) => {
            let stripped: String = text.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect();
            stripped.split_whitespace().count() == 4
        }
        Err(e) => return skip(ID, TITLE, format!("Ошибка движка: {}", e)),
    };
    let passed = usize::from(a_ok) + usize::from(b_ok);
    let (status, mark) = if passed == 2 {
        (TriageStatus::Pass, "✅")
    } else if passed == 1 {
        (TriageStatus::Warn, "⚠️")
    } else {
        (TriageStatus::Fail, "💥")
    };
    verdict_of(
        ID, TITLE, status,
        format!(
            "{} Выполнено заданий: {}/2 (список «- » ×3 ≤5 слов: {}; ровно 4 слова: {})",
            mark, passed,
            if a_ok { "✅" } else { "❌" },
            if b_ok { "✅" } else { "❌" }
        ),
        serde_json::json!({ "list_task": a_ok, "words_task": b_ok }),
    )
}

fn safe_trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{}…", cut)
    }
}

// ─── Тесты чистой логики (без движка) ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_collapses_whitespace() {
        assert_eq!(normalized("  a\n\t b   c "), "a b c");
    }

    #[test]
    fn restart_detected_on_repeated_prefix() {
        // original ≥80 норм-символов (иначе гейт «короткий оригинал не показателен»
        // честно возвращает false) и piece повторяет его начало целиком.
        let original =
            "Маяк стоял на берегу моря и освещал путь кораблям уже много десятилетий подряд, в любую погоду и шторм. ";
        let piece = format!("{}И тут он погас.", original);
        assert!(is_continuation_restart(original, &piece));
    }

    #[test]
    fn continuation_is_not_restart() {
        let original = "Маяк стоял на берегу моря и освещал путь кораблям уже много десятилетий подряд, а сторож поднимался по винтовой лестнице каждый вечер.";
        let piece = "продолжая своё дело, как и прежде, без всяких изменений и колебаний.";
        assert!(!is_continuation_restart(original, piece));
    }

    #[test]
    fn short_original_is_never_restart() {
        let original = "Короткий ответ.";
        let piece = "Короткий ответ. И продолжение длиной более шестидесяти символов подряд.";
        assert!(!is_continuation_restart(original, piece));
    }

    #[test]
    fn split_think_handles_closed_and_unclosed() {
        let (t, v) = split_think("до <think>мысли</think> после");
        assert_eq!(t, "мысли");
        assert_eq!(v, "до  после");
        let (t, v) = split_think("ответ <think>обрыв без закрытия");
        assert_eq!(t, "обрыв без закрытия");
        assert_eq!(v, "ответ ");
    }

    #[test]
    fn parse_bool_verdicts_json_and_fallback() {
        let j = parse_bool_verdicts("Вот: {\"s1\": false, \"s2\": true, \"s3\": false, \"s4\": false, \"s5\": true}").unwrap();
        assert_eq!(j, [false, true, false, false, true]);
        let f = parse_bool_verdicts("s1: false, s2: true, s3: false, s4: false, s5: true");
        assert!(f.is_none() || f == Some([false, true, false, false, true]));
    }

    #[test]
    fn verdict_label_rules() {
        let mk = |id: &str, st: TriageStatus| TriageVerdict {
            probe_id: id.to_string(),
            title: id.to_string(),
            status: st,
            detail: String::new(),
            metrics: serde_json::Value::Null,
        };
        let good = vec![
            mk("parroting", TriageStatus::Pass),
            mk("think_share", TriageStatus::Pass),
            mk("yes_bias", TriageStatus::Pass),
            mk("continuation", TriageStatus::Pass),
            mk("loops", TriageStatus::Warn),
        ];
        assert_eq!(verdict_label(&good), "Годна для System-2");
        let mut bad = good.clone();
        bad[0].status = TriageStatus::Fail;
        assert_eq!(verdict_label(&bad), "МУСОР для System-2 (структурный отказ)");
        let mut risky = good.clone();
        risky[1].status = TriageStatus::Warn;
        assert_eq!(verdict_label(&risky), "Рискованная для System-2");
        let skipped = vec![
            mk("parroting", TriageStatus::Skip),
            mk("think_share", TriageStatus::Skip),
            mk("yes_bias", TriageStatus::Skip),
            mk("continuation", TriageStatus::Skip),
        ];
        assert_eq!(verdict_label(&skipped), "Не удалось проверить (ошибка движка или отмена)");
    }

    #[test]
    fn gram_overlap_catches_verbatim() {
        let doc = "мост пережил три больших паводка и был разобран в семьдесят первом году";
        let answer = "Мост пережил три больших паводка и был разобран в семьдесят первом году.";
        assert!(gram_overlap(answer, doc, 4) > 0.3);
        assert!(gram_overlap("совершенно другой текст ни о чём вообще", doc, 4) < 0.12);
    }
}
