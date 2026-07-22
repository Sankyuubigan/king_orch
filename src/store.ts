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
  activeThoughtsBlock: HTMLDivElement | null = null;
  realtimeSubcallKeys = new Set<string>();

  // Трекинг сообщений
  uidCounter = 0;
  msgUidList: string[] = [];

  // Черновик
  draftTimeout: number | undefined;

  // Продвинутые функции
  showAdvancedFeatures = false;

  // Командные агенты (из подпапок)
  showFolderAgents = false;

  // Тестирование агентов
  testFileContent: any[] | null = null;

  // Каталог моделей
  modelsCatalog: any[] = [];
  
  // Для стриминга текста в реальном времени
  rtStreamUid: string | null = null;
  rtStreamBuffer: string = "";
  rtIsJson: boolean = false;

  // Для стриминга мыслей в блок «Мысли агентов» в реальном времени
  rtThoughtUid: string | null = null;
  rtThoughtBuffer: string = "";
  rtThoughtAuthor: string = "";

  nextUid(): string {
    return `msg_${this.uidCounter++}`;
  }

  resetForNewSession() {
    this.chatHistory = [];
    this.msgUidList = [];
    this.uidCounter = 0;
    this.realtimeSubcallKeys.clear();
    this.activeThoughtsBlock = null;
    this.rtStreamUid = null;
    this.rtStreamBuffer = "";
    this.rtIsJson = false;
    this.rtThoughtUid = null;
    this.rtThoughtBuffer = "";
    this.rtThoughtAuthor = "";
  }
}

/** Глобальный синглтон стора. Импортируется контроллерами. */
export const store = new Store();