use tauri::AppHandle;

use crate::domain;
use crate::infra;

/// Полный каталог entry points: `.md` агенты + YAML-графы, БЕЗ фильтрации по
/// видимости. Что показывать пользователю — решает `agent_visibility` в конфиге
/// (выбирается в Настройках → «Агенты в чате»).
#[tauri::command]
pub fn get_agents(app: AppHandle) -> Vec<domain::AgentEntry> {
    let agents_dir = infra::find_agents_dir(&app);
    log::info!("[agents] Поиск entry points в: {}", agents_dir.display());
    let entries = domain::load_entry_points(&agents_dir);
    let graphs = entries.iter().filter(|e| e.entry_type == "workflow").count();
    log::info!(
        "[agents] Загружено entry points: {} (графов: {}, агентов: {})",
        entries.len(),
        graphs,
        entries.len() - graphs
    );
    entries
}
