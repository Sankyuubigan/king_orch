use crate::domain::{load_subtitles, load_chapters, load_video_title, parse_time_to_secs, SubtitleSegment};
use crate::llm::LlamaEngine;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use std::time::Instant;
use regex::Regex;

pub fn emit_log(app: &AppHandle, msg: &str) {
    let _ = app.emit("log", msg);
}

pub fn emit_status(app: &AppHandle, msg: &str, progress: u8) {
    let _ = app.emit("status", msg);
    let _ = app.emit("progress", progress);
}

fn load_prompt(app: &AppHandle, filename: &str) -> String {
    let exe_dir = app.path().executable_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    
    let paths = vec![
        exe_dir.join("prompts").join(filename),
        std::path::PathBuf::from("prompts").join(filename),
    ];
    
    for p in paths {
        if let Ok(content) = fs::read_to_string(&p) {
            return content.trim().to_string();
        }
    }
    
    format!("ОШИБКА: Файл промпта {} не найден в папке prompts!", filename)
}

fn is_ad_chapter(title: &str) -> bool {
    let lower = title.to_lowercase();
    lower.contains("реклам") || lower.contains("спонсор") || lower.contains("интеграци") || 
    lower.contains("boosty") || lower.contains("патреон") || lower.contains("patreon") ||
    lower.contains("подпиш") || lower.contains("телеграм")
}

pub fn run_smart_summary(
    app: AppHandle,
    model_path: String,
    sub_path: String,
    info_path: String,
    include_timestamps: bool,
    temperature: f32,
    context_size: u32,
    kv_quantization: bool,
    sponsor_segments: Vec<(f64, f64)>,
    cancel_flag: Arc<AtomicBool>,
) -> Result<String, String> {
    let start_time = Instant::now();

    emit_status(&app, "Чтение файла субтитров...", 5);
    let raw_segments = load_subtitles(&sub_path)?;
    if raw_segments.is_empty() { return Err("Субтитры пусты или не найдены.".to_string()); }

    let chapters = load_chapters(&info_path);
    let video_title = load_video_title(&info_path);
    
    let mut segments = Vec::new();
    let mut _ads_skipped = 0;

    for seg in &raw_segments {
        let secs = parse_time_to_secs(&seg.start_time);
        let mut is_ad = false;
        
        for chap in &chapters {
            if secs >= chap.start_time && secs <= chap.end_time {
                if is_ad_chapter(&chap.title) { is_ad = true; }
                break;
            }
        }

        if !is_ad {
            for (start, end) in &sponsor_segments {
                if secs >= *start && secs <= *end { is_ad = true; break; }
            }
        }
        
        if is_ad { _ads_skipped += 1; } else { segments.push(seg.clone()); }
    }

    if segments.is_empty() { return Err("После фильтрации рекламы не осталось субтитров.".to_string()); }

    let mut full_text = String::new();
    for s in &segments {
        full_text.push_str(&format!("[{}] {}\n", s.get_clean_start(), s.text));
    }

    emit_status(&app, "Загрузка модели в VRAM...", 10);
    let engine = LlamaEngine::new(&model_path, context_size, kv_quantization)?;

    if cancel_flag.load(Ordering::SeqCst) { return Err("Отменено".to_string()); }

    let exact_tokens = engine.count_tokens(&full_text)?;
    let limit_tokens = (context_size as f64 * 0.85) as usize; 
    
    let mut sys_map = load_prompt(&app, "system_map.txt");
    let mut sys_final = load_prompt(&app, "system_final.txt");

    let time_instruction = if include_timestamps {
        "\nОбязательно сохраняй таймкоды [ММ:СС] для каждой новости/темы."
    } else {
        "\nВАЖНО: Пиши текст без таймкодов."
    };

    sys_map.push_str(time_instruction);
    sys_final.push_str(time_instruction);

    let final_result: String;

    if exact_tokens < limit_tokens {
        emit_status(&app, "Обработка всего видео за один раз...", 20);
        let user_prompt = format!("НАЗВАНИЕ ВИДЕО: {}\nВХОДНЫЕ ДАННЫЕ (Текст видео):\n{}", video_title, full_text);
        
        let app_clone = app.clone();
        final_result = engine.generate(&sys_final, &user_prompt, 2500, temperature, cancel_flag.clone(), move |local_p, msg| {
            let global_p = 20.0 + (local_p as f64 / 100.0 * 70.0);
            emit_status(&app_clone, msg, global_p as u8);
        })?;
    } else {
        emit_status(&app, "Умное разбиение текста...", 20);
        let chunks = smart_chunking_with_overlap(&engine, &segments, limit_tokens);
        let total_chunks = chunks.len();
        let mut chunk_summaries = Vec::new();

        for (i, chunk_text) in chunks.iter().enumerate() {
            if cancel_flag.load(Ordering::SeqCst) { return Err("Отменено".to_string()); }
            
            let base_percent = 20.0 + ((i as f64 / total_chunks as f64) * 60.0);
            let chunk_step = 60.0 / total_chunks as f64;
            let user_prompt = format!("НАЗВАНИЕ ВИДЕО: {}\nКУСОК ТРАНСКРИПТА:\n{}", video_title, chunk_text);
            
            let app_clone = app.clone();
            let out = engine.generate(&sys_map, &user_prompt, 1000, temperature, cancel_flag.clone(), move |local_p, msg| {
                let global_p = base_percent + (local_p as f64 / 100.0 * chunk_step);
                emit_status(&app_clone, &format!("[Часть {}/{}] {}", i + 1, total_chunks, msg), global_p as u8);
            })?;
            
            let clean_out = out.trim().to_uppercase();
            if !clean_out.contains("РЕКЛАМА") && !clean_out.contains("НЕТ ВАЖНОЙ ИНФОРМАЦИИ") {
                chunk_summaries.push(out);
            }
        }

        if cancel_flag.load(Ordering::SeqCst) { return Err("Отменено".to_string()); }
        
        emit_status(&app, "Финальная сборка отчета...", 80);
        let combined = chunk_summaries.join("\n\n=== Следующая часть ===\n\n");
        let user_final = format!("НАЗВАНИЕ ВИДЕО: {}\nВХОДНЫЕ ДАННЫЕ (Черновики):\n{}", video_title, combined);
        
        let app_clone = app.clone();
        final_result = engine.generate(&sys_final, &user_final, 3000, temperature, cancel_flag, move |local_p, msg| {
            let global_p = 80.0 + (local_p as f64 / 100.0 * 15.0);
            emit_status(&app_clone, &format!("[Финал] {}", msg), global_p as u8);
        })?;
    }

    let re_think = Regex::new(r"(?is)<think>.*?</think>").unwrap();
    let clean_result = re_think.replace_all(&final_result, "").trim().to_string();

    let elapsed = start_time.elapsed();
    emit_log(&app, &format!("✅ Готово! Общее время: {:.1} сек.", elapsed.as_secs_f32()));
    emit_status(&app, "Готово!", 100);
    
    Ok(clean_result)
}

fn smart_chunking_with_overlap(engine: &LlamaEngine, segments: &[SubtitleSegment], limit_tokens: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current_chunk_lines: Vec<String> = Vec::new();
    let mut current_tokens: usize = 0;
    let overlap_count = 15;

    for seg in segments {
        let line = format!("[{}] {}\n", seg.get_clean_start(), seg.text);
        let line_tokens = engine.count_tokens(&line).unwrap_or_else(|_| line.chars().count() / 2);

        if current_tokens + line_tokens > limit_tokens && !current_chunk_lines.is_empty() {
            chunks.push(current_chunk_lines.join(""));
            let start_idx = current_chunk_lines.len().saturating_sub(overlap_count);
            current_chunk_lines = current_chunk_lines[start_idx..].to_vec();
            current_tokens = current_chunk_lines.iter().map(|l| engine.count_tokens(l.as_str()).unwrap_or_else(|_| l.chars().count() / 2)).sum();
        }

        current_chunk_lines.push(line);
        current_tokens += line_tokens;
    }

    if !current_chunk_lines.is_empty() { chunks.push(current_chunk_lines.join("")); }
    chunks
}