use super::*;
use crate::domain::agent_manager::AgentProfile;
use crate::infra::{
    extract_model_filename, push_report, ChatAttachment, ChatMessage, GrammarSpec, LlamaEngine,
    LlmMessage, ModelParams, SubCall, ToolCallInfo,
};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Если вывод инструмента большой — пишет полный текст в spill-файл и
/// возвращает выжимку (head 2000 + tail 1000) с локатором для встроенного
/// инструмента `read_spill`. Иначе возвращает текст как есть, без spill.
pub(crate) fn spill_if_large(
    output: &str,
    agent_id: &str,
    idx: u32,
) -> (String, Option<std::path::PathBuf>) {
    if output.len() <= SPILL_THRESHOLD {
        return (output.to_string(), None);
    }
    let root = spill_root_dir();
    let _ = std::fs::create_dir_all(&root);
    // Имя уникально на КАЖДЫЙ вызов (наносекунды + счётчик процесса): путь spill
    // зашивается в текст ПЕРСИСТЯЩИХСЯ сообщений (read_spill читает по пути), и
    // перезапись файла новым прогоном подсунула бы старому сообщению чужой
    // контент. Старые файлы не трогаем — на них ссылаются старые сообщения;
    // файлы копятся, это цена честности ссылок.
    static SPILL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SPILL_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let fname = format!(
        "spill_{}_{}_{:x}{}.txt",
        sanitize_name(agent_id),
        idx,
        nanos,
        seq
    );
    let fpath = root.join(&fname);
    if std::fs::write(&fpath, output).is_err() {
        return (output.to_string(), None);
    }
    let head: String = output.chars().take(2000).collect();
    let mut tail_chars: Vec<char> = output.chars().rev().take(1000).collect();
    tail_chars.reverse();
    let tail: String = tail_chars.into_iter().collect();
    let display = format!(
        "[РЕЗУЛЬТАТ ИНСТРУМЕНТА сохранён в файл spills]\n{}\n\n... [полный результат {} символов: {}] ...\n\n{}\n\nЧтобы дочитать, вызови инструмент read_spill с path \"{}\" — читай ДИАПАЗОНАМИ (offset/limit), не копируй артефакт целиком.",
        head, output.len(), fpath.display(), tail, fpath.display()
    );
    (display, Some(fpath))
}

/// Встроенный инструмент `read_spill`: читает spill-файл (только внутри
/// директории spills) диапазонами символов. `offset` — 1-based позиция, с какой
/// начать (по умолчанию 1); `limit` — сколько символов прочитать (по умолчанию
/// 16000, максимум 16000). Срез по `char_indices` — безопасен для UTF-8.
pub(crate) fn read_spill_file(path: &str, offset: usize, limit: usize) -> Result<String, String> {
    let p = std::path::Path::new(path);
    // Канонизируем оба пути: на Windows canonicalize добавляет префикс \\?\,
    // поэтому сравнивать нужно канонизированные версии.
    let root_abs = spill_root_dir()
        .canonicalize()
        .unwrap_or_else(|_| spill_root_dir());
    let abs = p
        .canonicalize()
        .map_err(|e| format!("Невалидный путь spill: {}", e))?;
    if !abs.starts_with(&root_abs) {
        return Err("Чтение разрешено только внутри директории spills".to_string());
    }
    let content =
        std::fs::read_to_string(&abs).map_err(|e| format!("Ошибка чтения spill: {}", e))?;

    const MAX_PAGE: usize = 16000;
    let limit = limit.clamp(1, MAX_PAGE);
    let chars: Vec<char> = content.chars().collect();
    let total = chars.len();
    let start = offset.saturating_sub(1).min(total);
    let end = (start + limit).min(total);
    if end <= start {
        return Ok(format!(
            "[в диапазоне offset={}.. символов нет; всего {} символов]",
            offset, total
        ));
    }
    let slice: String = chars[start..end].iter().collect();
    let mut out = String::with_capacity(slice.len() + 64);
    out.push_str(&slice);
    if start > 0 && total > end {
        out.push_str(&format!(
            "\n… [показаны символы {}-{} из {}. Продолжай с offset={}]",
            start + 1,
            end,
            total,
            end + 1
        ));
    } else if total > end {
        out.push_str(&format!(
            "\n… [показаны символы 1-{} из {}. Продолжай с offset={}]",
            end, total, end + 1
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spill_paths_are_unique_for_same_agent_and_idx() {
        // Два прогона одного агента (параллельные чаты) не должны перезаписывать
        // файлы друг друга: путь зашит в персистящее сообщение старого прогона.
        let big = "A".repeat(SPILL_THRESHOLD + 1);
        let (_, first) = spill_if_large(&big, "dup_agent", 3);
        let (_, second) = spill_if_large(&big, "dup_agent", 3);
        let pa = first.expect("первый spill");
        let pb = second.expect("второй spill");
        assert_ne!(
            pa, pb,
            "пути двух spill-вызовов с одинаковыми agent/idx совпали — перезапись"
        );
        let _ = fs::remove_file(&pa);
        let _ = fs::remove_file(&pb);
    }
}
