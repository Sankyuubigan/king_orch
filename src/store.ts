import type { AgentEntry, AppTab, ModelParams } from "./types";

/**
 * Центральное хранилище состояния приложения.
 * Единый источник истины — никакого дублирования переменных.
 * Каждый контроллер читает/пишет сюда, а не в свои локальные переменные.
 *
 * ⚠️ Чат-стейт (история, сессия, стриминг, обработка) вынесен в per-tab
 * `ChatTabState` (см. controllers/chat.ts) — вкладок может быть несколько.
 * В Store остаются только общеприкладные поля и флаг «идёт обработка где-либо».
 */
class Store {
  // Рабочая область: вкладки + активная
  tabs: AppTab[] = [];
  activeTabId: string | null = null;

  // Глобальный флаг «в приложении идёт обработка» (только один запрос за раз).
  // Per-tab «обрабатывается ли моя вкладка» живёт в ChatTabState.
  isProcessing = false;

  // Продвинутые функции
  showAdvancedFeatures = false;

  // Какие entry points показывать в списке агентов чата.
  // ЕДИНСТВЕННЫЙ источник правды о видимости (в фронтматтере `visible` удалён).
  // Порядок массива = порядок в выпадающем списке, поэтому не сортировать.
  agentVisibility: string[] = ["basic"];

  // Тестирование агентов
  testFileContent: any[] | null = null;

  // Параметры текущей модели (включая DRY/XTC)
  currentModelParams: ModelParams | null = null;

  // Переводчик сообщений: модель и целевой язык ("ru" | "en")
  translatorModel: string | null = null;
  translatorLang: string = "ru";

  // Лимит контекста (из конфига, обновляется при loadConfig)
  contextSize = 0;

  // Рабочая директория для кодера (bash tool current_dir)
  workdir: string | null = null;

  // ── Общие каталоги для дропдаунов всех чат-вкладок (SSOT) ──
  // Заполняются SettingsController.loadConfig и перерисовываются каждой вкладкой.
  models: string[] = [];
  lastModel: string | null = null;
  lastAgent: string | null = null;
  agents: AgentEntry[] = [];
  capabilities: Record<string, { uncen: boolean; vision: boolean; audio: boolean }> = {};
  agentsReady = false;
  cloudRouterCombos: Record<string, { name: string; models: string[] }[]> = {};
}

/** Глобальный синглтон стора. Импортируется контроллерами. */
export const store = new Store();