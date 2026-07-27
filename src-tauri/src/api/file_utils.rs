use std::fs;

#[tauri::command]
pub fn write_text_file(path: String, content: String) -> Result<(), String> {
    fs::write(&path, content).map_err(|e| format!("Ошибка записи файла: {}", e))?;
    Ok(())
}
