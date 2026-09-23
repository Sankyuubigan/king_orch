/**
 * Тонкий бутстраппер — только создаёт контроллеры и связывает их.
 * Вся логика изолирована в модулях-контроллерах.
 * Импорты идут через двери (index.ts) — модули не лезут в кишки друг друга.
 */
import { initConfirmDialog, initPermissionDialog, initVramDialog, showToast } from "./ui";
// Регистрирует Web Component <about-updates-panel>, <logs-panel> и <llama-*-panel> из переиспользуемых плагинов.
import "@my-tauri-plugins/plugin-about-updates";
import "@my-tauri-plugins/plugin-logs";
import "@my-tauri-plugins/plugin-llama-engine";
// Регистрирует <nine-router-panel> (облачный шлюз 9Router: комбо + дашборд).
import "@my-tauri-plugins/plugin-9router";
// Регистрирует <downloader-widget> / <downloader-progress> (единый прогресс загрузок).
import "@my-tauri-plugins/plugin-downloader";
import {
  SessionController, SettingsController, GraphController, AgentTestController,
  CodingTestController, UpdatePopupController, TabController, initChatEventRouter,
} from "./controllers";
import type { SharedChatControls } from "./utils";
import type { TabSection } from "./types";
import { logFront } from "@my-tauri-plugins/plugin-logs";
import { initTelemetry, trackError } from "./telemetry";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

async function initApp() {
  // ─── Телеметрия: вешаем ловушки ошибок UI раньше всего остального ───
  // (уважает настройку «Отправлять анонимные отчёты об ошибках»)
  await initTelemetry();
  initConfirmDialog();
  initPermissionDialog();
  initVramDialog();

  // ─── Глобальные Tauri-события движка и агентов (регистрируются ОДИН раз) ───
  // (прогресс/статус/стриминг маршрутизируются в обрабатывающую чат-вкладку)
  initChatEventRouter();

  // ─── Сторожевик зависаний главного потока ───
  // Если поток UI заблокирован дольше ~1с (тяжёлый рендер/синх. работа),
  // пишем в локальный лог — помогает диагностировать «белые» окна и фризы.
  try {
    if ("PerformanceObserver" in window) {
      const po = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          const blocked = Math.round(entry.duration);
          if (blocked >= 1000) {
            logFront(`[FE-STALL] main thread blocked ${blocked}ms (${entry.name})`);
          }
        }
      });
      po.observe({ entryTypes: ["longtask"] });
    }
  } catch {
    /* PerformanceObserver недоступен — не критично */
  }

  // ─── Общие (статичные) контролы сэмплинга: живут в разделе «Настройки»,
  // используются ВСЕМИ чат-вкладками. ───
  const sharedSampling: SharedChatControls = {
    maxGenSlider: $<HTMLInputElement>("max-gen-slider"),
    chkKvQuantK: $<HTMLInputElement>("chk-kv-quant-k"),
    chkKvQuantV: $<HTMLInputElement>("chk-kv-quant-v"),
    tempSlider: $<HTMLInputElement>("temp-slider"),
    topkSlider: $<HTMLInputElement>("topk-slider"),
    toppSlider: $<HTMLInputElement>("topp-slider"),
    minpSlider: $<HTMLInputElement>("minp-slider"),
    reppenSlider: $<HTMLInputElement>("reppen-slider"),
    prespenSlider: $<HTMLInputElement>("prespen-slider"),
  };

  // ─── Контроллер раздела «История сессий» ───
  // Экземпляр живёт всё приложение: слушает bus (session:open/session:deleted),
  // по которым TabController открывает/закрывает вкладки.
  void new SessionController({
    sessionList: $<HTMLDivElement>("session-list"),
    sessionPreviewTitle: $<HTMLDivElement>("session-preview-title"),
    sessionPreviewDate: $<HTMLDivElement>("session-preview-date"),
    sessionPreviewOpen: $<HTMLButtonElement>("session-preview-open"),
    sessionPreviewBody: $<HTMLDivElement>("session-preview-body"),
    sessionSelectToggle: $<HTMLButtonElement>("session-select-toggle"),
    sessionSelectBar: $<HTMLDivElement>("session-select-bar"),
    sessionSelectCount: $<HTMLSpanElement>("session-select-count"),
    sessionSelectAll: $<HTMLButtonElement>("session-select-all"),
    sessionBulkDelete: $<HTMLButtonElement>("session-bulk-delete"),
    sessionSelectCancel: $<HTMLButtonElement>("session-select-cancel"),
  });

  // ─── Контроллер настроек (раздел ⚙️; селекты моделей/агентов — per-tab) ───
  const settingsCtrl = new SettingsController({
    maxGenSlider: $<HTMLInputElement>("max-gen-slider"), maxGenValue: $<HTMLElement>("max-gen-value"),
    chatFontSlider: $<HTMLInputElement>("chat-font-slider"), chatFontValue: $<HTMLElement>("chat-font-value"),
    chkKvQuantK: $<HTMLInputElement>("chk-kv-quant-k"),
    chkKvQuantV: $<HTMLInputElement>("chk-kv-quant-v"),
    themeSelect: $<HTMLSelectElement>("theme-select"),
    promptFormatSelect: $<HTMLSelectElement>("prompt-format-select"),
    tempSlider: $<HTMLInputElement>("temp-slider"), tempValue: $<HTMLElement>("temp-value"),
    topkSlider: $<HTMLInputElement>("topk-slider"), topkValue: $<HTMLElement>("topk-value"),
    toppSlider: $<HTMLInputElement>("topp-slider"), toppValue: $<HTMLElement>("topp-value"),
    minpSlider: $<HTMLInputElement>("minp-slider"), minpValue: $<HTMLElement>("minp-value"),
    reppenSlider: $<HTMLInputElement>("reppen-slider"), reppenValue: $<HTMLElement>("reppen-value"),
    prespenSlider: $<HTMLInputElement>("prespen-slider"), prespenValue: $<HTMLElement>("prespen-value"),
    btnResetParams: $<HTMLButtonElement>("btn-reset-params"),
    chkShowAdvanced: $<HTMLInputElement>("chk-show-advanced"),
    chkShowFolderAgents: $<HTMLInputElement>("chk-show-folder-agents"),
    chkErrorReports: $<HTMLInputElement>("chk-error-reports"),
    translatorModelSelect: $<HTMLSelectElement>("translator-model-select"),
    translatorLangSelect: $<HTMLSelectElement>("translator-lang-select"),
  });

  // ─── Контроллер графа (суб-вкладка 🔀 в студии агентов) ───
  const graphCtrl = new GraphController({
    graphContainer: $<HTMLDivElement>("graph-container"),
    graphSidebar: $<HTMLDivElement>("graph-sidebar"),
    graphDetailTitle: $<HTMLSpanElement>("graph-detail-title"),
    graphDetailContent: $<HTMLDivElement>("graph-detail-content"),
    graphSidebarClose: $<HTMLButtonElement>("graph-sidebar-close"),
    btnOpenWorkflow: $<HTMLButtonElement>("btn-open-workflow"),
    btnSaveWorkflow: $<HTMLButtonElement>("btn-save-workflow"),
    currentWorkflowName: $<HTMLSpanElement>("current-workflow-name"),
    btnUndo: $<HTMLButtonElement>("btn-undo"),
    btnRedo: $<HTMLButtonElement>("btn-redo"),
    dirtyIndicator: $<HTMLSpanElement>("dirty-indicator"),
  });

  // ─── Контроллер теста агентов (суб-вкладка в студии агентов) ───
  const agentTestCtrl = new AgentTestController({
    testFilePath: $<HTMLInputElement>("test-file-path"),
    btnSelectTestFile: $<HTMLButtonElement>("btn-select-test-file"),
    testAgentList: $<HTMLDivElement>("test-agent-list"),
    testModelList: $<HTMLDivElement>("test-model-list"),
    btnRunTest: $<HTMLButtonElement>("btn-run-test"),
    testProgress: $<HTMLDivElement>("test-progress"),
    testStatusLabel: $<HTMLDivElement>("test-status-label"),
    testProgressBar: $<HTMLDivElement>("test-progress-bar"),
    testResultsBox: $<HTMLDivElement>("test-results-box"),
    testResultsContent: $<HTMLDivElement>("test-results-content"),
    btnSaveTestResults: $<HTMLButtonElement>("btn-save-test-results"),
    testModeSelect: $<HTMLSelectElement>("test-mode-select"),
    yamlTestPanel: $<HTMLDivElement>("yaml-test-panel"),
    pipelineTestPanel: $<HTMLDivElement>("pipeline-test-panel"),
    pipelineTestList: $<HTMLDivElement>("pipeline-test-list"),
    pipelineModelList: $<HTMLDivElement>("pipeline-model-list"),
    btnRunPipelineTest: $<HTMLButtonElement>("btn-run-pipeline-test"),
    pipelineProgress: $<HTMLDivElement>("pipeline-progress"),
    pipelineStatusLabel: $<HTMLDivElement>("pipeline-status-label"),
    pipelineProgressBar: $<HTMLDivElement>("pipeline-progress-bar"),
    pipelineResultsBox: $<HTMLDivElement>("pipeline-results-box"),
    pipelineResultsContent: $<HTMLDivElement>("pipeline-results-content"),
  });

  // ─── Контроллер кодинг-теста LLM (суб-вкладка в студии агентов) ───
  const codingTestCtrl = new CodingTestController({
    codingModelList: $<HTMLDivElement>("coding-model-list"),
    codingSuiteList: $<HTMLDivElement>("coding-suite-list"),
    codingQuickCount: $<HTMLInputElement>("coding-quick-count"),
    codingVrBudget: $<HTMLInputElement>("coding-vr-budget"),
    btnRunCodingTest: $<HTMLButtonElement>("btn-run-coding-test"),
    btnStopCodingTest: $<HTMLButtonElement>("btn-stop-coding-test"),
    codingProgress: $<HTMLDivElement>("coding-progress"),
    codingStatusLabel: $<HTMLDivElement>("coding-status-label"),
    codingProgressBar: $<HTMLDivElement>("coding-progress-bar"),
    codingLog: $<HTMLDivElement>("coding-log"),
    codingResultsBox: $<HTMLDivElement>("coding-results-box"),
    codingResultsContent: $<HTMLDivElement>("coding-results-content"),
    codingReportLink: $<HTMLAnchorElement>("coding-report-link"),
  });

  // ─── Суб-вкладки студии агентов (singleton, активируются вкладкой-разделом) ───
  const subtabGraph = $<HTMLButtonElement>("subtab-graph");
  const subtabAiTest = $<HTMLButtonElement>("subtab-ai-test");
  const subtabCodingTest = $<HTMLButtonElement>("subtab-coding-test");
  const viewSubGraph = $<HTMLDivElement>("view-sub-graph");
  const viewSubAiTest = $<HTMLDivElement>("view-sub-ai-test");
  const viewSubCodingTest = $<HTMLDivElement>("view-sub-coding-test");

  let activeSubTab: 'graph' | 'ai-test' | 'coding-test' = 'graph';

  function switchSubTab(tab: 'graph' | 'ai-test' | 'coding-test') {
    activeSubTab = tab;
    subtabGraph.classList.toggle('active', tab === 'graph');
    subtabAiTest.classList.toggle('active', tab === 'ai-test');
    subtabCodingTest.classList.toggle('active', tab === 'coding-test');
    viewSubGraph.classList.toggle('active', tab === 'graph');
    viewSubAiTest.classList.toggle('active', tab === 'ai-test');
    viewSubCodingTest.classList.toggle('active', tab === 'coding-test');
    if (tab === 'graph') requestAnimationFrame(() => graphCtrl.onTabActivated());
  }

  subtabGraph?.addEventListener("click", () => switchSubTab('graph'));
  subtabAiTest?.addEventListener("click", () => { switchSubTab('ai-test'); agentTestCtrl.init(); });
  subtabCodingTest?.addEventListener("click", () => { switchSubTab('coding-test'); codingTestCtrl.init(); });

  // ─── Контроллер вкладок рабочей области (браузерные вкладки) ───
  const tabCtrl = new TabController(
    $<HTMLElement>("workspace-tabs"),
    $<HTMLElement>("chat-tab-slot"),
    sharedSampling,
    {
      onSectionActivated: (_tab, section) => {
        if (section === "agent-studio") {
          switchSubTab(activeSubTab);
          if (activeSubTab === 'graph') requestAnimationFrame(() => graphCtrl.onTabActivated());
        }
      },
    },
  );

  // ─── Старт: конфиг → восстановление вкладок ───
  const config = await settingsCtrl.loadConfig();
  tabCtrl.init(config ?? {});

  // ——— Быстрая навигация по разделам из «Настройки» (открытие в той же вкладке) ———
  $<HTMLElement>("settings-nav")?.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest(".settings-nav-btn") as HTMLElement | null;
    const section = btn?.dataset.section as TabSection | undefined;
    if (!section) return;
    tabCtrl.openSection(section);
  });

  // ─── Синхронизация списка моделей после изменений в веб-компонентах плагина ───
  // Модели добавляются/удаляются/скачиваются в <llama-*-panel>; обновляем
  // общие каталоги в Store (chat-вкладки перерисовывают селекты по config:loaded).
  document.addEventListener("llama:models-changed", () => {
    void settingsCtrl.loadConfig();
  });

  // ─── Проверка обновления при старте (всплывающее окно справа по центру) ───
  const updatePopupCtrl = new UpdatePopupController({
    container: $<HTMLElement>("update-popup"),
    btnUpdate: $<HTMLButtonElement>("btn-update-now"),
    btnLater: $<HTMLButtonElement>("btn-update-later"),
  });
  void updatePopupCtrl.checkOnStartup();
}

document.addEventListener("DOMContentLoaded", () => {
  initApp().catch((e) => {
    console.error("initApp failed:", e);
    showToast(`Ошибка инициализации: ${e}`, "error");
    void trackError("init", e);
  });
});