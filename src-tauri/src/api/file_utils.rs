use std::fs;
use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentMetadata {
    pub file_name: String,
    pub mime_type: String,
    pub file_path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[tauri::command]
pub fn write_text_file(path: String, content: String) -> Result<(), String> {
    fs::write(&path, content).map_err(|e| format!("Ошибка записи файла: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn read_text_file(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| format!("Ошибка чтения файла: {}", e))
}

#[tauri::command]
pub fn get_attachment_metadata(paths: Vec<String>) -> Result<Vec<AttachmentMetadata>, String> {
    paths
        .into_iter()
        .map(|path| {
            let file_path = Path::new(&path);
            let metadata = fs::metadata(file_path).map_err(|error| {
                log::error!("Не удалось проверить путь {:?}: {}", path, error);
                format!("Путь недоступен: {}", path)
            })?;
            let file_name = file_path
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .ok_or_else(|| format!("Не удалось определить имя: {}", path))?
                .to_string();
            let is_dir = metadata.is_dir();
            let mime_type = if is_dir {
                "application/x-directory".to_string()
            } else {
                mime_from_path(file_path)
            };
            Ok(AttachmentMetadata {
                file_name,
                mime_type,
                file_path: path,
                is_dir,
                size: metadata.len(),
            })
        })
        .collect()
}

#[tauri::command]
pub fn get_image_data_url(path: String) -> Result<String, String> {
    let file_path = Path::new(&path);
    let metadata = fs::metadata(file_path).map_err(|error| {
        log::error!("Не удалось проверить изображение {:?}: {}", path, error);
        format!("Файл изображения недоступен: {}", path)
    })?;
    if metadata.is_dir() {
        return Err(format!("Путь является папкой, а не изображением: {}", path));
    }
    let mime_type = mime_from_path(file_path);
    if !mime_type.starts_with("image/") {
        return Err(format!("Файл не является поддерживаемым изображением: {}", path));
    }
    let bytes = fs::read(file_path).map_err(|error| {
        log::error!("Не удалось прочитать изображение {:?}: {}", path, error);
        format!("Не удалось прочитать изображение: {}", path)
    })?;
    if bytes.is_empty() {
        return Err(format!("Файл изображения пуст: {}", path));
    }
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{};base64,{}", mime_type, encoded))
}

#[tauri::command]
pub fn save_image_from_path(source_path: String, destination_path: String) -> Result<(), String> {
    let source = Path::new(&source_path);
    let metadata = fs::metadata(source).map_err(|error| {
        log::error!("Не удалось проверить исходное изображение {:?}: {}", source_path, error);
        format!("Исходное изображение недоступно: {}", source_path)
    })?;
    if metadata.is_dir() {
        return Err("Нельзя сохранить папку как изображение".to_string());
    }
    if let Some(parent) = Path::new(&destination_path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| {
                log::error!("Не удалось создать папку {:?}: {}", parent, error);
                format!("Не удалось создать папку: {}", error)
            })?;
        }
    }
    fs::copy(source, &destination_path).map_err(|error| {
        log::error!("Не удалось сохранить изображение {:?}: {}", destination_path, error);
        format!("Ошибка записи файла: {}", error)
    })?;
    Ok(())
}

#[tauri::command]
pub fn reveal_path(path: String) -> Result<(), String> {
    crate::infra::reveal_path(&path)
}

/// Сохранить base64-картинку из чата по выбранному пути (пункт меню
/// «Сохранить изображение»). Принимает и чистый base64, и data-URL.
#[tauri::command]
pub fn save_image_file(path: String, data_base64: String) -> Result<(), String> {
    let raw = data_base64.split_once(',').map(|(_, b)| b).unwrap_or(&data_base64);
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .map_err(|e| format!("Ошибка декодирования base64: {}", e))?;
    if bytes.is_empty() {
        return Err("Пустые данные изображения".to_string());
    }
    if let Some(parent) = std::path::Path::new(&path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("Не удалось создать папку: {}", e))?;
        }
    }
    fs::write(&path, &bytes).map_err(|e| format!("Ошибка записи файла: {}", e))?;
    Ok(())
}

fn mime_from_path(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "ico" => "image/x-icon",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "m4a" => "audio/mp4",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "json" => "application/json",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
    .to_string()
}
