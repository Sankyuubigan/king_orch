use tauri::AppHandle;

pub fn get_available_tools(_app: &AppHandle) -> Vec<String> {
    // Вся логика перенесена в стандартизированные MCP-серверы
    vec![]
}

pub fn execute_tool(_app: &AppHandle, _tool_name: &str, _arg: &str) -> Result<String, String> {
    Err("Инструменты перенесены на MCP".to_string())
}