import { invoke } from "@tauri-apps/api/core";

/**
 * Сервис работы с сессиями (CRUD).
 * Выделен из main.ts для изоляции логики хранения.
 */

export async function fetchSessions(): Promise<any[]> {
    return await invoke("get_sessions");
}

export async function loadSession(id: string): Promise<any> {
    return await invoke("load_session", { id });
}

export async function deleteSession(id: string): Promise<void> {
    await invoke("delete_session", { id });
}

export async function renameSession(id: string, newTitle: string): Promise<void> {
    await invoke("rename_session", { id, newTitle });
}

export async function openSessionFolder(id: string): Promise<void> {
    await invoke("open_session_folder", { id });
}