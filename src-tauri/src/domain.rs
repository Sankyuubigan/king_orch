use regex::Regex;
use serde_json::Value;
use std::fs;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SubtitleSegment {
    pub text: String,
    pub start_time: String,
    pub end_time: String,
}

impl SubtitleSegment {
    pub fn get_clean_start(&self) -> String {
        if self.start_time.is_empty() {
            return String::new();
        }
        self.start_time.split(',').next().unwrap_or("").to_string()
    }
}

// Новая структура для хранения Глав YouTube
#[derive(Debug, Clone)]
pub struct Chapter {
    pub start_time: f64,
    pub end_time: f64,
    pub title: String,
}

// Функция перевода таймкода ЧЧ:ММ:СС,МММ в секунды для сравнения с главами
pub fn parse_time_to_secs(time_str: &str) -> f64 {
    let parts: Vec<&str> = time_str.split(|c| c == ':' || c == ',' || c == '.').collect();
    if parts.len() >= 3 {
        let h: f64 = if parts.len() == 4 { parts[0].parse().unwrap_or(0.0) } else { 0.0 };
        let offset = if parts.len() == 4 { 1 } else { 0 };
        
        let m: f64 = parts[offset].parse().unwrap_or(0.0);
        let s: f64 = parts[offset + 1].parse().unwrap_or(0.0);
        let ms: f64 = if parts.len() > offset + 2 { parts[offset + 2].parse().unwrap_or(0.0) } else { 0.0 };
        
        return h * 3600.0 + m * 60.0 + s + ms / 1000.0;
    }
    0.0
}

// Загрузка названия видео из .info.json файла
pub fn load_video_title(file_path: &str) -> String {
    if let Ok(content) = fs::read_to_string(file_path) {
        if let Ok(json) = serde_json::from_str::<Value>(&content) {
            if let Some(title) = json.get("title").and_then(|v| v.as_str()) {
                return title.to_string();
            }
            if let Some(fulltitle) = json.get("fulltitle").and_then(|v| v.as_str()) {
                return fulltitle.to_string();
            }
        }
    }
    "Неизвестное видео".to_string()
}

// Загрузка глав из .info.json файла (скачивается через yt-dlp)
pub fn load_chapters(file_path: &str) -> Vec<Chapter> {
    let mut chapters = Vec::new();
    if let Ok(content) = fs::read_to_string(file_path) {
        if let Ok(json) = serde_json::from_str::<Value>(&content) {
            if let Some(chaps) = json.get("chapters").and_then(|c| c.as_array()) {
                for chap in chaps {
                    let start = chap.get("start_time").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let end = chap.get("end_time").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let title = chap.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    chapters.push(Chapter { start_time: start, end_time: end, title });
                }
            }
        }
    }
    chapters
}

pub fn load_subtitles(file_path: &str) -> Result<Vec<SubtitleSegment>, String> {
    let content = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
    let mut segments = Vec::new();

    let re = Regex::new(r"(?m)(\d{2}:\d{2}:\d{2}[,\.]\d{3})\s*-->\s*(\d{2}:\d{2}:\d{2}[,\.]\d{3})(?:.*?)\n([\s\S]*?)(?:\n\n|\z)").unwrap();
    let mut last_line = String::new();

    for cap in re.captures_iter(&content) {
        let start = cap[1].replace('.', ",");
        let end = cap[2].replace('.', ",");
        let raw_text = cap[3].to_string();
        
        let clean_re = Regex::new(r"<[^>]+>").unwrap();
        let text = clean_re.replace_all(&raw_text, "").to_string();
        
        let mut unique_lines = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }
            
            if trimmed != last_line {
                unique_lines.push(trimmed.to_string());
                last_line = trimmed.to_string();
            }
        }
        
        let final_text = unique_lines.join(" ");
        
        if !final_text.is_empty() {
            segments.push(SubtitleSegment {
                text: final_text,
                start_time: start,
                end_time: end,
            });
        }
    }

    if segments.is_empty() {
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with("WEBVTT") {
                segments.push(SubtitleSegment {
                    text: trimmed.to_string(),
                    start_time: String::new(),
                    end_time: String::new(),
                });
            }
        }
    }

    Ok(segments)
}