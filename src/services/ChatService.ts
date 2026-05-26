import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage, ModelParams, ChatResponse } from "../types";

/**
 * Сервис управления стейтом чата и коммуникацией с LLM.
 * Выделен из main.ts по принципу SRP (Single Responsibility).
 */

export let globalChatHistory: ChatMessage[] = [];
export let globalDossier: Record<string, string> = {};
export let currentSessionId: string | null = null;
export let isProcessing = false;

let draftTimeout: number | undefined;

export function setProcessingState(state: boolean): boolean {
    isProcessing = state;
    return isProcessing;
}

export function setCurrentSessionId(id: string | null) {
    currentSessionId = id;
}

export function clearDraftTimeout() {
    clearTimeout(draftTimeout);
}

export function pushHistoryMessage(msg: ChatMessage) {
    globalChatHistory.push(msg);
}

export function setDossier(dossier: Record<string, string>) {
    globalDossier = dossier;
}

/**
 * Автосохранение черновика
 */
export function triggerDraftSave(draftText: string) {
    if (isProcessing) return;
    
    if (!currentSessionId && draftText.trim() !== "") {
        currentSessionId = Date.now().toString();
        globalChatHistory = [];
        globalDossier = {};
        invoke("save_session", { id: currentSessionId, messages: globalChatHistory, dossier: globalDossier, draft: draftText });
    } else if (currentSessionId) {
        clearTimeout(draftTimeout);
        draftTimeout = window.setTimeout(() => {
            invoke("save_session", { id: currentSessionId, messages: globalChatHistory, dossier: globalDossier, draft: draftText });
        }, 500);
    }
}

/**
 * Основной метод отправки сообщения LLM
 */
export async function sendChatRequest(
    modelPath: string,
    agentId: string,
    message: string,
    history: ChatMessage[],
    contextSize: number,
    kvQuantization: boolean,
    modelParams: ModelParams
): Promise<ChatResponse> {
    return await invoke("chat_request", {
        modelPath,
        agentId,
        message,
        history,
        contextSize,
        kvQuantization,
        dossier: globalDossier,
        modelParams
    });
}