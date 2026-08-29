/**
 * Тонкий бутстраппер — только создаёт контроллеры и связывает их.
 * Вся логика изолирована в модулях-контроллерах.
 * Импорты идут через двери (index.ts) — модули не лезут в кишки друг друга.
 */
import { initConfirmDialog, initPermissionDialog, showToast } from "./ui";
// Регистрирует Web Component <about-updates-panel> из переиспользуемого плагина.
import "@my-tauri-plugins/plugin-about-updates";
import { ChatController, SessionController, SettingsController, GraphController, AgentTestController, CodingTestController, UpdatePopupController } from "./controllers";
import { bus } from "./events";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { initTelemetry, trackError } from "./telemetry";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

async function initApp() {
  // ─── Телеметрия: вешаем ловушки ошибок UI раньше всего остального ───
  // (уважает настройку «Отправлять анонимные отчёты об ошибках»)
  await initTelemetry();
  initConfirmDialog();
  initPermissionDialog();

  // ─── Сторожевик зависаний главного потока ───
  // Если поток UI заблокирован дольше ~1с (тяжёлый рендер/синх. работа),
  // пишем в локальный лог — помогает диагностировать «белые» окна и фризы.
  try {
    if ("PerformanceObserver" in window) {
      const po = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          const blocked = Math.round(entry.duration);
          if (blocked >= 1000) {
            void invoke("log_frontend_event", {
              level: "FE-STALL",
              msg: `main thread blocked ${blocked}ms (${entry.name})`,
            });
          }
        }
      });
      po.observe({ entryTypes: ["longtask"] });
    }
  } catch {
    /* PerformanceObserver недоступен — не критично */
  }

  // ─── Контроллер сессий (боковая панель) ───
  const sessionCtrl = new SessionController({
    sessionList: $<HTMLDivElement>("session-list"),
    btnNewSession: $<HTMLButtonElement>("btn-new-session"),
  });

  // ─── Контроллер настроек (вкладка ⚙️) ───
  const settingsCtrl = new SettingsController({
    modelSelect: $<HTMLSelectElement>("model-select"),
    agentSelect: $<HTMLSelectElement>("agent-select"),
    contextSlider: $<HTMLInputElement>("context-slider"), contextValue: $<HTMLElement>("context-value"),
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
    downloadModelSelect: $<HTMLSelectElement>("download-model-select"),
    btnDownloadModel: $<HTMLButtonElement>("btn-download-model"),
    downloadProgressContainer: $<HTMLDivElement>("download-progress-container"),
    downloadProgressBar: $<HTMLDivElement>("download-progress-bar"),
    downloadStatusLabel: $<HTMLDivElement>("download-status-label"),
    btnAddModel: $<HTMLButtonElement>("btn-add-model"),
    chkShowAdvanced: $<HTMLInputElement>("chk-show-advanced"),
    chkShowFolderAgents: $<HTMLInputElement>("chk-show-folder-agents"),
    chkErrorReports: $<HTMLInputElement>("chk-error-reports"),
    modelsList: $<HTMLDivElement>("models-list"),
    btnAddModelLlm: $<HTMLButtonElement>("btn-add-model-llm"),
    translatorModelSelect: $<HTMLSelectElement>("translator-model-select"),
    translatorLangSelect: $<HTMLSelectElement>("translator-lang-select"),
    btnAutoDownload: $<HTMLButtonElement>("btn-auto-download"),
    autoDownloadModal: $<HTMLElement>("auto-download-modal"),
    modalModelName: $<HTMLElement>("modal-model-name"),
    modalSavePath: $<HTMLElement>("modal-save-path"),
    modalFreeSpace: $<HTMLElement>("modal-free-space"),
    btnModalCancel: $<HTMLButtonElement>("btn-modal-cancel"),
    btnModalConfirm: $<HTMLButtonElement>("btn-modal-confirm"),
    engineStatus: $<HTMLElement>("engine-status"),
    engineGpu: $<HTMLElement>("engine-gpu"),
    enginePath: $<HTMLElement>("engine-path"),
    engineVariantSelect: $<HTMLSelectElement>("engine-variant-select"),
    engineVariantHint: $<HTMLElement>("engine-variant-hint"),
    btnApplyEngineVariant: $<HTMLButtonElement>("btn-apply-engine-variant"),
    btnInstallEngine: $<HTMLButtonElement>("btn-install-engine"),
    btnCheckEngineUpdate: $<HTMLButtonElement>("btn-check-engine-update"),
    btnInstallEngineUpdate: $<HTMLButtonElement>("btn-install-engine-update"),
    btnRemoveEngine: $<HTMLButtonElement>("btn-remove-engine"),
    btnSetEngineDir: $<HTMLButtonElement>("btn-set-engine-dir"),
    engineProgressContainer: $<HTMLDivElement>("engine-progress-container"),
    engineProgressBar: $<HTMLDivElement>("engine-progress-bar"),
    engineStatusLabel: $<HTMLDivElement>("engine-status-label"),
    engineWarning: $<HTMLElement>("engine-warning"),
  });

  // ─── Контроллер чата (вкладка 💬) ───
  const chatCtrl = new ChatController({
    chatHistory: $<HTMLDivElement>("chat-history"),
    chatInput: $<HTMLTextAreaElement>("chat-input"),
    btnSend: $<HTMLButtonElement>("btn-send"),
    btnStop: $<HTMLButtonElement>("btn-stop"),
    chatFeedback: $<HTMLDivElement>("chat-feedback"),
    progressBar: $<HTMLDivElement>("progress-bar"),
    statusLabel: $<HTMLDivElement>("status-label"),
    agentSelect: $<HTMLSelectElement>("agent-select"),
    modelSelect: $<HTMLSelectElement>("model-select"),
    subchatHistory: $<HTMLDivElement>("subchat-history"),
    subchatTitle: $<HTMLSpanElement>("subchat-title"),
    btnBackChat: $<HTMLButtonElement>("btn-back-chat"),
    logView: $<HTMLTextAreaElement>("log-view"),
    contextSlider: $<HTMLInputElement>("context-slider"),
    maxGenSlider: $<HTMLInputElement>("max-gen-slider"),
    chkKvQuantK: $<HTMLInputElement>("chk-kv-quant-k"),
    chkKvQuantV: $<HTMLInputElement>("chk-kv-quant-v"),
    tempSlider: $<HTMLInputElement>("temp-slider"),
    topkSlider: $<HTMLInputElement>("topk-slider"),
    toppSlider: $<HTMLInputElement>("topp-slider"),
    minpSlider: $<HTMLInputElement>("minp-slider"),
    reppenSlider: $<HTMLInputElement>("reppen-slider"),
    prespenSlider: $<HTMLInputElement>("prespen-slider"),
    viewChat: $<HTMLDivElement>("view-chat"),
    viewSubchat: $<HTMLDivElement>("view-subchat"),
    btnAttach: $<HTMLButtonElement>("btn-attach"),
    fileInput: $<HTMLInputElement>("file-input"),
    filePreview: $<HTMLDivElement>("file-preview"),
    tokenCounter: $<HTMLDivElement>("token-counter"),
    engineBadge: $<HTMLDivElement>("engine-badge"),
  });

  // ─── Контроллер графа (вкладка 🔀 в студии агентов) ───
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

  // ─── Переключение вкладок ───
  const tabChat = $<HTMLButtonElement>("tab-chat");
  const tabAgentStudio = $<HTMLButtonElement>("tab-agent-studio");
  const tabSettings = $<HTMLButtonElement>("tab-settings");
  const tabLogs = $<HTMLButtonElement>("tab-logs");
  const viewChat = $<HTMLDivElement>("view-chat");
  const viewAgentStudio = $<HTMLDivElement>("view-agent-studio");
  const viewSettings = $<HTMLDivElement>("view-settings");
  const viewLogs = $<HTMLDivElement>("view-logs");
  const viewSubchat = $<HTMLDivElement>("view-subchat");
  const btnClearLogs = $<HTMLButtonElement>("btn-clear-logs");
  const btnSaveLogs = $<HTMLButtonElement>("btn-save-logs");
  const logView = $<HTMLTextAreaElement>("log-view");

  // Суб-вкладки студии агентов
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

  function switchTab(tab: 'chat' | 'agent-studio' | 'settings' | 'logs') {
    tabChat.classList.toggle('active', tab === 'chat');
    tabAgentStudio.classList.toggle('active', tab === 'agent-studio');
    tabSettings.classList.toggle('active', tab === 'settings');
    tabLogs.classList.toggle('active', tab === 'logs');
    viewChat.classList.toggle('active', tab === 'chat');
    viewAgentStudio.classList.toggle('active', tab === 'agent-studio');
    viewSubchat.classList.remove('active');
    viewSettings.classList.toggle('active', tab === 'settings');
    viewLogs.classList.toggle('active', tab === 'logs');
    if (tab === 'agent-studio') {
      switchSubTab(activeSubTab);
    }
    if (tab === 'agent-studio' && activeSubTab === 'graph') {
      requestAnimationFrame(() => graphCtrl.onTabActivated());
    }
  }

  // Управление видимостью студии агентов
  function updateAgentStudioVisibility(visible: boolean) {
    tabAgentStudio.classList.toggle('visible', visible);
    if (!visible && viewAgentStudio.classList.contains('active')) {
      switchTab('chat');
    }
  }

  tabChat?.addEventListener("click", () => switchTab('chat'));
  tabAgentStudio?.addEventListener("click", () => switchTab('agent-studio'));
  tabSettings?.addEventListener("click", () => switchTab('settings'));
  tabLogs?.addEventListener("click", () => switchTab('logs'));
  btnClearLogs?.addEventListener("click", () => { logView.value = ""; });
  btnSaveLogs?.addEventListener("click", async () => {
    const text = logView.value;
    if (!text.trim()) { showToast("Логи пусты", "info"); return; }
    try {
      const now = new Date();
      const ts = `${now.getFullYear()}-${String(now.getMonth()+1).padStart(2,'0')}-${String(now.getDate()).padStart(2,'0')}_${String(now.getHours()).padStart(2,'0')}${String(now.getMinutes()).padStart(2,'0')}${String(now.getSeconds()).padStart(2,'0')}`;
      const savePath = await save({
        defaultPath: `logs_${ts}.txt`,
        filters: [{ name: "Text", extensions: ["txt"] }],
      });
      if (!savePath) return;
      await invoke("write_text_file", { path: savePath, content: text });
      showToast(`Логи сохранены: ${savePath}`, "success");
    } catch (e) {
      showToast(`Ошибка сохранения: ${e}`, "error");
      void trackError("logs.save", e);
    }
  });

  subtabGraph?.addEventListener("click", () => switchSubTab('graph'));
  subtabAiTest?.addEventListener("click", () => {
    switchSubTab('ai-test');
    agentTestCtrl.init();
  });
  subtabCodingTest?.addEventListener("click", () => {
    switchSubTab('coding-test');
    codingTestCtrl.init();
  });

  // ─── Мосты шины событий ───
  bus.on("tab:switch", (tab: string) => switchTab(tab as 'chat' | 'agent-studio' | 'settings' | 'logs'));
  bus.on("log", (msg: string) => chatCtrl.logToGUI(msg));
  bus.on("advanced:visibility", (visible: boolean) => updateAgentStudioVisibility(visible));

  // ─── Старт ───
  settingsCtrl.loadConfig();

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