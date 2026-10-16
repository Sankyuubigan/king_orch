use tauri::{AppHandle, State};
use crate::api::AppState;
use crate::infra;

#[tauri::command]
pub fn get_sessions(app: AppHandle) -> Vec<infra::SessionMeta> {
    infra::get_sessions(&app)
}

#[tauri::command]
pub fn load_session(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<infra::ChatSession, String> {
    let mut session = infra::get_session(&app, &id)?;
    // Единственное место подъёма счётчика облачных запросов: при старте
    // приложения память пуста, а на диске лежит уже накопленный расход. Отдаём
    // наружу именно max(память, файл) — сырое значение из файла откатило бы
    // цифру назад у сессии, которая ещё не успела дописаться.
    session.cloud_requests = state.prime_cloud_total(&id, session.cloud_requests);
    Ok(session)
}

#[tauri::command]
pub fn save_session(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    messages: Vec<infra::ChatMessage>,
    draft: String,
    model: Option<String>,
    agent: Option<String>,
) -> Result<(), String> {
    let title = messages
        .iter()
        .find(|m| m.msg_type == "message" && m.author.as_deref() == Some("user") && !m.content.trim().is_empty())
        .map(|m| {
            let text = m.content.replace('\n', " ");
            if text.chars().count() > 35 {
                format!("{}...", text.chars().take(32).collect::<String>())
            } else {
                text
            }
        })
        .unwrap_or_else(|| "Новая сессия".to_string());
    let cloud_requests = state.cloud_total(&id);
    // Отказ сохранения обязан быть виден в логе: фронтенд этот Err может
    // проглотить (core rules §2.2 — запрет молчаливых ошибок).
    let result = infra::save_session(&app, &id, &title, messages, draft, model, agent, cloud_requests);
    if let Err(reason) = &result {
        let path = infra::session_manager::sessions_dir(&app).join(format!("{}.json", id));
        log::error!("save_session: НЕ СОХРАНЕНО — {} ({})", path.display(), reason);
    }
    result
}

#[tauri::command]
pub fn delete_session(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.forget_cloud_counter(&id);
    infra::delete_session(&app, &id)
}

#[tauri::command]
pub fn rename_session(app: AppHandle, id: String, new_title: String) -> Result<(), String> {
    infra::rename_session(&app, &id, &new_title)
}

#[tauri::command]
pub fn open_session_folder(app: AppHandle, id: String) -> Result<(), String> {
    infra::open_session_folder(&app, &id)
}