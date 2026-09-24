use std::fs;

#[tauri::command]
pub fn write_text_file(path: String, content: String) -> Result<(), String> {
    fs::write(&path, content).map_err(|e| format!("Ошибка записи файла: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn read_text_file(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| format!("Ошибка чтения файла: {}", e))
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
