/**
 * Тонкий бутстраппер — только создаёт контроллеры и связывает их.
 * Вся логика изолирована в модулях-контроллерах.
 * Импорты идут через двери (index.ts) — модули не лезут в кишки друг друга.
 */
import { initConfirmDialog } from "./ui";
import { ChatController, SessionController, SettingsController, GraphController } from "./controllers";
import { bus } from "./events";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

function initApp() {
  initConfirmDialog();

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
    chkKvQuant: $<HTMLInputElement>("chk-kv-quant"),
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
    chkKvQuant: $<HTMLInputElement>("chk-kv-quant"),
    tempSlider: $<HTMLInputElement>("temp-slider"),
    topkSlider: $<HTMLInputElement>("topk-slider"),
    toppSlider: $<HTMLInputElement>("topp-slider"),
    minpSlider: $<HTMLInputElement>("minp-slider"),
    reppenSlider: $<HTMLInputElement>("reppen-slider"),
    prespenSlider: $<HTMLInputElement>("prespen-slider"),
    viewChat: $<HTMLDivElement>("view-chat"),
    viewSubchat: $<HTMLDivElement>("view-subchat"),
  });

  // ─── Контроллер графа (вкладка 🔀) ───
  const graphCtrl = new GraphController({
    graphContainer: $<HTMLDivElement>("graph-container"),
    graphSidebar: $<HTMLDivElement>("graph-sidebar"),
    graphTeamSelect: $<HTMLSelectElement>("graph-team-select"),
    graphDetailTitle: $<HTMLSpanElement>("graph-detail-title"),
    graphDetailContent: $<HTMLDivElement>("graph-detail-content"),
    graphSidebarClose: $<HTMLButtonElement>("graph-sidebar-close"),
  });

  // ─── Переключение вкладок (общий UI, не принадлежит одному контроллеру) ───
  const tabChat = $<HTMLButtonElement>("tab-chat");
  const tabGraph = $<HTMLButtonElement>("tab-graph");
  const tabSettings = $<HTMLButtonElement>("tab-settings");
  const tabLogs = $<HTMLButtonElement>("tab-logs");
  const viewChat = $<HTMLDivElement>("view-chat");
  const viewGraph = $<HTMLDivElement>("view-graph");
  const viewSettings = $<HTMLDivElement>("view-settings");
  const viewLogs = $<HTMLDivElement>("view-logs");
  const viewSubchat = $<HTMLDivElement>("view-subchat");
  const btnClearLogs = $<HTMLButtonElement>("btn-clear-logs");
  const logView = $<HTMLTextAreaElement>("log-view");

  function switchTab(tab: 'chat' | 'graph' | 'settings' | 'logs') {
    tabChat.classList.toggle('active', tab === 'chat');
    tabGraph.classList.toggle('active', tab === 'graph');
    tabSettings.classList.toggle('active', tab === 'settings');
    tabLogs.classList.toggle('active', tab === 'logs');
    viewChat.classList.toggle('active', tab === 'chat');
    viewGraph.classList.toggle('active', tab === 'graph');
    viewSubchat.classList.remove('active');
    viewSettings.classList.toggle('active', tab === 'settings');
    viewLogs.classList.toggle('active', tab === 'logs');
    if (tab === 'graph') requestAnimationFrame(() => graphCtrl.onTabActivated());
  }

  tabChat?.addEventListener("click", () => switchTab('chat'));
  tabGraph?.addEventListener("click", () => switchTab('graph'));
  tabSettings?.addEventListener("click", () => switchTab('settings'));
  tabLogs?.addEventListener("click", () => switchTab('logs'));
  btnClearLogs?.addEventListener("click", () => { logView.value = ""; });

  // ─── Мосты шины событий ───
  bus.on("tab:switch", (tab: string) => switchTab(tab as 'chat' | 'graph' | 'settings' | 'logs'));
  bus.on("log", (msg: string) => chatCtrl.logToGUI(msg));

  // ─── Старт ───
  settingsCtrl.loadConfig();
}

document.addEventListener("DOMContentLoaded", initApp);