use super::*;
use crate::infra::LlmMessage;

/// Единый pipeline компакции контекста перед генерацией. Работает ТОЛЬКО с
/// не-system сообщениями (system-промпт сохраняется целиком). Стратегии по
/// возрастанию агрессивности (по паттернам deepseek-harness `compaction.md`):
///   0) бюджет считается в РЕАЛЬНЫХ ТОКЕНАХ (через `token_count`), а не в символах;
///   1) сворачиваем крупные (ещё не spilled) результаты инструментов в head/tail
///      (голова+хвост), теряя только середину — как `ctx.toolResultPruner`;
///   2) сжимаем самые старые сообщения в ОДИН связный LLM-саммари (`summarize`),
///      заменяя диапазон узлом `[СЖАТАЯ ИСТОРИЯ]` (а не тупыми эксцерптами);
///   3) если суммаризовать нельзя/не вышло (диапазон мал или LLM упал) —
///      усекаем старые сообщения head/tail (truncate, не delete): сохраняем сигнал;
///   4) абсолютный last-resort — жёстко дропаем старейшие, пока не влезем
///      (только если ничего другого не помогло; узел саммари не трогаем).
/// Отчёт — чтобы вызывающий ОБЯЗАТЕЛЬНО записал в лог, что из контекста удалено
/// (правило: тихих операций с данными нет).
pub(crate) struct CompactionReport {
    pub tool_results_pruned: usize,
    /// История была изменена (саммари или усечение) — для лога/телеметрии.
    pub history_compressed: bool,
    /// История сжата через LLM-саммари (а не просто усечена).
    pub history_summarized: bool,
    /// Число исторических сообщений, усечённых head/tail (слой 3).
    pub history_truncated: usize,
    /// Сколько старых сообщений ушло из промпта (слой 2 replace-счётчик + слой 4 drop).
    pub old_messages_dropped: usize,
}

/// `token_count(msgs)` — реальный подсчёт токенов промпта (в проде — `engine.get_tokens_count`).
/// `summarize(text)` — LLM-саммаризация диапазона; `None` = не удалось/неприменимо.
/// Оба — замыкания, чтобы модуль оставался тестируемым без запущенного llama-server.
pub(crate) fn compact_llm_messages<L, F, G>(
    messages: &mut Vec<LlmMessage>,
    budget_tokens: usize,
    keep_recent: usize,
    token_count: F,
    summarize: G,
    log_cb: L,
) -> CompactionReport
where
    L: Fn(String),
    F: Fn(&[LlmMessage]) -> usize,
    G: Fn(&str) -> Option<String>,
{
    let mut report = CompactionReport {
        tool_results_pruned: 0,
        history_compressed: false,
        history_summarized: false,
        history_truncated: 0,
        old_messages_dropped: 0,
    };
    if messages.len() <= 1 {
        return report;
    }

    let total = |msgs: &[LlmMessage]| token_count(msgs);
    if total(messages) <= budget_tokens {
        return report;
    }

    // Слой 1: head/tail pruning крупных результатов инструментов.
    for m in messages.iter_mut().skip(1) {
        if m.content.contains("[РЕЗУЛЬТАТ ИНСТРУМЕНТА")
            && !m.content.contains("сохранён в файл spills")
            && !m.content.contains("свёрнуто")
            && m.content.chars().count() > 1500
        {
            let tool = m
                .content
                .lines()
                .next()
                .map(|l| {
                    l.trim_start_matches("[РЕЗУЛЬТАТ ИНСТРУМЕНТА ")
                        .trim_end_matches("]:")
                        .trim()
                        .to_string()
                })
                .unwrap_or_else(|| "инструмент".to_string());
            let (head, tail) = head_tail(m.content.as_str(), 400);
            let dropped = m.content.chars().count() - head.chars().count() - tail.chars().count();
            m.content = format!(
                "[РЕЗУЛЬТАТ ИНСТРУМЕНТА {}] …[свёрнуто {} симв: оставлены голова+хвост]…\n{}\n…\n{}",
                tool, dropped, head, tail
            );
            report.tool_results_pruned += 1;
        }
    }
    if total(messages) <= budget_tokens {
        return report;
    }

    // Слои 2/3: старые сообщения (кроме system + `keep_recent` последних).
    let n = messages.len();
    let keep_recent = keep_recent.max(2);
    if n > keep_recent + 2 {
        let mut compress_end = n - keep_recent; // эксклюзивно
        // Защита пары: если сохраняемый messages[compress_end] — результат
        // инструмента, а его вызов (assistant, в dropped) удалён — дропаем и его,
        // чтобы не оставлять висящий результат без пары.
        if compress_end < n
            && messages[compress_end].content.contains("[РЕЗУЛЬТАТ ИНСТРУМЕНТА")
            && messages[compress_end - 1].role == "assistant"
        {
            compress_end += 1;
        }
        let range_count = compress_end - 1; // сколько старых сообщений в диапазоне
        if range_count >= 6 {
            // Слой 2: LLM-саммари затенённого диапазона.
            let mut to_summarize = String::new();
            for m in messages.iter().take(compress_end).skip(1) {
                to_summarize.push_str(&format!("({}) {}\n\n", m.role, m.content));
            }
            match summarize(&to_summarize) {
                Some(summary) => {
                    messages.drain(1..compress_end);
                    messages.insert(
                        1,
                        LlmMessage {
                            role: "system".to_string(),
                            content: format!("[СЖАТАЯ ИСТОРИЯ]: {}", summary),
                        },
                    );
                    report.old_messages_dropped += range_count;
                    report.history_compressed = true;
                    report.history_summarized = true;
                }
                None => {
                    log_cb("⚠️ Саммаризация истории не удалась — переходим к усечению (head/tail).".into());
                    truncate_range(messages, compress_end, &mut report);
                }
            }
        } else {
            // Слой 3: диапазон мал — сразу усечение (head/tail), суммаризация не окупается.
            truncate_range(messages, compress_end, &mut report);
        }
    }
    if total(messages) <= budget_tokens {
        return report;
    }

    // Слой 4: абсолютный last-resort — жёстко дропаем старейшие, пока не влезем.
    // Не трогаем узел `[СЖАТАЯ ИСТОРИЯ]` (дропаем индекс 2, если он на месте 1).
    while messages.len() > 2 {
        if total(messages) <= budget_tokens {
            break;
        }
        let idx = if messages
            .get(1)
            .map_or(false, |m| m.content.contains("[СЖАТАЯ ИСТОРИЯ]"))
        {
            2
        } else {
            1
        };
        if idx >= messages.len() {
            break;
        }
        messages.remove(idx);
        report.old_messages_dropped += 1;
    }
    if report.old_messages_dropped > 0 || report.history_truncated > 0 {
        report.history_compressed = true;
    }
    report
}

/// Голова + хвост по codepoint-символам (Rust `char` = scalar value, surrogate-pair целы;
/// графемные кластеры могут разорваться — только косметика). Если текст короче 2·keep —
/// возвращает его целиком без хвоста.
fn head_tail(s: &str, keep: usize) -> (String, String) {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= keep * 2 {
        return (s.to_string(), String::new());
    }
    let head: String = chars.iter().take(keep).collect();
    let tail: String = chars.iter().rev().take(keep).rev().collect();
    (head, tail)
}

/// Усечь (head/tail) старые сообщения диапазона `[1..end)`, не удаляя их из промпта.
fn truncate_range(messages: &mut Vec<LlmMessage>, end: usize, report: &mut CompactionReport) {
    for m in messages.iter_mut().take(end).skip(1) {
        let len = m.content.chars().count();
        if len > 800 {
            let (head, tail) = head_tail(m.content.as_str(), 400);
            let dropped = len - head.chars().count() - tail.chars().count();
            m.content = format!("{} …[свёрнуто {} симв]… {}", head, dropped, tail);
            report.history_truncated += 1;
        }
    }
    report.history_compressed = true;
}

/// Усечь самое крупное не-system сообщение (head/tail), чтобы влезть в лимит при
/// переполнении одним гигантом. Возвращает true, если что-то усечено.
pub(crate) fn truncate_largest<L, F>(
    messages: &mut Vec<LlmMessage>,
    budget_tokens: usize,
    token_count: F,
    log_cb: L,
) -> bool
where
    L: Fn(String),
    F: Fn(&[LlmMessage]) -> usize,
{
    let mut best: Option<usize> = None;
    let mut best_len = 0usize;
    for (i, m) in messages.iter().enumerate().skip(1) {
        if m.content.chars().count() > best_len {
            best_len = m.content.chars().count();
            best = Some(i);
        }
    }
    let idx = match best {
        Some(i) if best_len > 800 => i,
        _ => return false,
    };
    let content = messages[idx].content.clone();
    let (head, tail) = head_tail(&content, 400);
    let dropped = content.chars().count() - head.chars().count() - tail.chars().count();
    if dropped == 0 {
        return false;
    }
    messages[idx].content = format!("{} …[свёрнуто {} симв]… {}", head, dropped, tail);
    // Если всё ещё не влезаем — рекурсивно усекаем ещё сильнее (половина).
    if token_count(messages) > budget_tokens {
        let half = (head.chars().count() / 2).max(1);
        let (h2, t2) = head_tail(&content, half);
        let d2 = content.chars().count() - h2.chars().count() - t2.chars().count();
        messages[idx].content = format!("{} …[свёрнуто {} симв]… {}", h2, d2, t2);
    }
    log_cb(format!("🗜️ Переполнение: усечено самое крупное сообщение на {} симв.", dropped));
    true
}

/// Детект переполнения контекста по тексту ошибки генерации (llama-server / HTTP).
pub(crate) fn is_context_overflow(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("context length")
        || e.contains("exceed")
        || e.contains("n_ctx")
        || e.contains("too long")
        || e.contains("http 400")
        || e.contains("kv cache")
        || e.contains("sliding window")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn char_tokens(msgs: &[LlmMessage]) -> usize {
        msgs.iter().map(|m| m.content.chars().count()).sum()
    }

    fn big_tool_result(chars: usize) -> String {
        let body: String = std::iter::repeat('x').take(chars).collect();
        format!("[РЕЗУЛЬТАТ ИНСТРУМЕНТА bash]:\n{}", body)
    }

    #[test]
    fn head_tail_keeps_both_ends() {
        let s: String = std::iter::repeat('a').take(200).collect()
            + &"MIDDLE".repeat(60)
            + &std::iter::repeat('z').take(200).collect::<String>();
        let (h, t) = head_tail(&s, 400);
        assert!(h.starts_with('a'));
        assert!(t.ends_with('z'));
        assert!(h.chars().count() <= 400);
        assert!(t.chars().count() <= 400);
    }

    #[test]
    fn prunes_big_results_and_fits_budget() {
        let mut msgs = vec![LlmMessage { role: "system".to_string(), content: "sys".into() }];
        msgs.push(LlmMessage { role: "user".to_string(), content: "привет".into() });
        msgs.push(LlmMessage { role: "assistant".to_string(), content: "ок".into() });
        msgs.push(LlmMessage { role: "user".to_string(), content: big_tool_result(2000) });
        let budget = char_tokens(&msgs) - 1000; // до pruning не влезаем
        let rep = compact_llm_messages(&mut msgs, budget, 4, char_tokens, |_| None, |_| {});
        assert_eq!(rep.tool_results_pruned, 1);
        assert!(msgs[3].content.contains("свёрнуто"));
        assert!(char_tokens(&msgs) <= budget, "должны влезть после pruning");
    }

    #[test]
    fn keeps_small_conversations_untouched() {
        let mut msgs = vec![LlmMessage { role: "system".to_string(), content: "sys".into() }];
        msgs.push(LlmMessage { role: "user".to_string(), content: "привет".into() });
        msgs.push(LlmMessage { role: "assistant".to_string(), content: "ок".into() });
        let rep = compact_llm_messages(&mut msgs, 100000, 4, char_tokens, |_| None, |_| {});
        assert_eq!(rep.tool_results_pruned, 0);
        assert!(!rep.history_compressed);
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn summarizes_when_over_budget() {
        let mut msgs = vec![LlmMessage { role: "system".to_string(), content: "sys".into() }];
        for i in 0..10 {
            msgs.push(LlmMessage { role: "user".to_string(), content: format!("старое сообщение номер {}", i) });
        }
        msgs.push(LlmMessage { role: "user".to_string(), content: "недавнее".into() });
        msgs.push(LlmMessage { role: "assistant".to_string(), content: "ответ".into() });
        // бюджет меньше реального, чтобы форсировать сжатие
        let budget = 50;
        let rep = compact_llm_messages(
            &mut msgs,
            budget,
            2,
            char_tokens,
            |_| Some("Краткое саммари старой переписки.".into()),
            |_| {},
        );
        assert!(rep.history_summarized, "ожидаем LLM-саммари");
        assert!(msgs.iter().any(|m| m.content.contains("[СЖАТАЯ ИСТОРИЯ]")));
        assert!(char_tokens(&msgs) <= budget + 1, "должны влезть после сжатия");
    }

    #[test]
    fn fallback_to_truncate_when_summarize_fails() {
        let mut msgs = vec![LlmMessage { role: "system".to_string(), content: "sys".into() }];
        for i in 0..10 {
            msgs.push(LlmMessage { role: "user".to_string(), content: format!("старое сообщение номер {}", i) });
        }
        msgs.push(LlmMessage { role: "user".to_string(), content: "недавнее".into() });
        msgs.push(LlmMessage { role: "assistant".to_string(), content: "ответ".into() });
        let budget = 50;
        let rep = compact_llm_messages(&mut msgs, budget, 2, char_tokens, |_| None, |_| {});
        assert!(!rep.history_summarized);
        assert!(rep.history_truncated > 0, "ожидаем усечение head/tail");
        assert!(char_tokens(&msgs) <= budget + 1);
    }
}
