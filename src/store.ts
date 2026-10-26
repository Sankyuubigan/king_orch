import type { AgentEntry, AppTab, ModelParams } from "./types";

/**
 * Центральное хранилище состояния приложения.
 * Единый источник истины — никакого дублирования переменных.
 * Каждый контроллер читает/пишет сюда, а не в свои локальные переменные.
 *
 * ⚠️ Чат-стейт (история, сессия, стриминг, обработка) вынесен в per-tab
 * `ChatTabState` (см. controllers/chat.ts) — вкладок может быть несколько,
 * и обработка в них ПАРАЛЛЕЛЬНАЯ: облачные ходы идут одновременно, локальные
 * встают в очередь на стороне бэкенда. Глобального флага «идёт обработка»
 * нет — единственный флаг: per-tab `ChatTabState.isProcessing`.
 */
class Store {
  // Рабочая область: вкладки + активная
  tabs: AppTab[] = [];
  activeTabId: string | null = null;

  // Продвинутые функции
  showAdvancedFeatures = false;

  // Какие entry points показывать в списке агентов чата.
  // ЕДИНСТВЕННЫЙ источник правды о пользовательском выборе: порядок массива =
  // порядок в выпадающем списке, поэтому не сортировать.
  // Второй, независимый сигнал — `visible` у самого агента (решение автора
  // «предлагать ли в каталоге»), см. utils/agent-visibility.ts.
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