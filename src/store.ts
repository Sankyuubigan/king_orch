import type { ChatMessage } from "./types";

/**
 * Центральное хранилище состояния приложения.
 * Единый источник истины — никакого дублирования переменных.
 * Каждый контроллер читает/пишет сюда, а не в свои локальные переменные.
 */
class Store {
  // Чат
  isProcessing = false;
  chatHistory: ChatMessage[] = [];
  currentSessionId: string | null = null;
  dossier: Record<string, Record<string, string>> = {};
  activeThoughtsBlock: HTMLDivElement | null = null;
  realtimeSubcallKeys = new Set<string>();

  // Трекинг сообщений
  uidCounter = 0;
  msgUidList: string[] = [];

  // Черновик
  draftTimeout: number | undefined;

  // Каталог моделей
  modelsCatalog: any[] = [];

  nextUid(): string {
    return `msg_${this.uidCounter++}`;
  }

  resetForNewSession() {
    this.chatHistory = [];
    this.msgUidList = [];
    this.uidCounter = 0;
    this.dossier = {};
    this.realtimeSubcallKeys.clear();
    this.activeThoughtsBlock = null;
  }
}

/** Глобальный синглтон стора. Импортируется контроллерами. */
export const store = new Store();