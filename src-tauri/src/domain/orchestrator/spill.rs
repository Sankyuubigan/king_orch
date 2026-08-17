use super::*;
use std::path::Path;
use std::fs;
use std::io::Write;
use serde_json::Value;
use crate::infra::{ChatMessage, LlmMessage, SubCall, ToolCallInfo, ModelParams, ChatAttachment, LlamaEngine, GrammarSpec, extract_model_filename, push_report};
use crate::domain::agent_manager::AgentProfile;

/// Если вывод инструмента большой — пишет полный текст в spill-файл и
/// возвращает выжимку (head 2000 + tail 1000) с локатором для встроенного
/// инструмента `read_spill`. Иначе возвращает текст как есть, без spill.
pub(crate) fn spill_if_large(output: &str, agent_id: &str, idx: u32) -> (String, Option<std::path::PathBuf>) {
    if output.len() <= SPILL_THRESHOLD {
        return (output.to_string(), None);
    }
    let root = spill_root_dir();
    let _ = std::fs::create_dir_all(&root);
    let fname = format!("spill_{}_{}.txt", sanitize_name(agent_id), idx);
    let fpath = root.join(&fname);
    if std::fs::write(&fpath, output).is_err() {
        return (output.to_string(), None);
    }
    let head: String = output.chars().take(2000).collect();
    let mut tail_chars: Vec<char> = output.chars().rev().take(1000).collect();
    tail_chars.reverse();
    let tail: String = tail_chars.into_iter().collect();
    let display = format!(
        "[РЕЗУЛЬТАТ ИНСТРУМЕНТА сохранён в файл spills]\n{}\n\n... [полный результат {} символов: {}] ...\n\n{}\n\nЧтобы дочитать полностью, вызови инструмент read_spill с аргументом {{\"path\": \"{}\"}}.",
        head, output.len(), fpath.display(), tail, fpath.display()
    );
    (display, Some(fpath))
}

/// Встроенный инструмент `read_spill`: читает spill-файл (только внутри
/// директории spills) и возвращает содержимое, обрезанное до 16К символов.
pub(crate) fn read_spill_file(path: &str) -> Result<String, String> {
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
    if content.len() > 16000 {
        Ok(format!(
            "{}...\n[обрезано до 16000 символов]",
            &content[..16000]
        ))
    } else {
        Ok(content)
    }
}

