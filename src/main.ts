/**
 * Тонкий бутстраппер — только создаёт контроллеры и связывает их.
 * Вся логика изолирована в модулях-контроллерах.
 * Импорты идут через двери (index.ts) — модули не лезут в кишки друг друга.
 */
import { initConfirmDialog, initImageViewer, initPermissionDialog, initVramDialog, showToast } from "./ui";
// Регистрирует Web Component <about-updates-panel>, <logs-panel> и <llama-*-panel> из переиспользуемых плагинов.
import "@my-tauri-plugins/plugin-about-updates";
import "@my-tauri-plugins/plugin-logs";
import "@my-tauri-plugins/plugin-llama-engine";
// Регистрирует <image-engine-panel> / <image-bundle-panel> (движок изображений + бандл Qwen Image 2.1).
import "@my-tauri-plugins/plugin-image-engine";
// Регистрирует <cloud-routers-panel> (облачный шлюз: 9Router, ExtremeRouter, OmniRoute).
import "@my-tauri-plugins/plugin-cloud-routers";
// Регистрирует <downloader-widget> / <downloader-progress> (единый прогресс загрузок).
import "@my-tauri-plugins/plugin-downloader";
import { initUpdateWatcher as initLlamaUpdateWatcher } from "@my-tauri-plugins/plugin-llama-engine";
import { initUpdateWatcher as initImageUpdateWatcher } from "@my-tauri-plugins/plugin-image-engine";
import { initUpdateWatcher as initRoutersUpdateWatcher, onOpenDashboard } from "@my-tauri-plugins/plugin-cloud-routers";
import {
  SessionController, SettingsController, AgentPickerController,
  UpdatePopupController, TabController, initChatEventRouter,
  loadGraphController, initUpdateBadgeTargets, initUpdateWatchers, getTabNavigationMenu,
  initCardLayout, createAgentStudioTab,
} from "./controllers";
import type { SharedChatControls } from "./utils";
import type { TabSection } from "./types";
import { logFront } from "@my-tauri-plugins/plugin-logs";
import { initTelemetry, trackError } from "./telemetry";
import { bus } from "./events";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const bootStartedAt = performance.now();

function revealBootScreen() {
  requestAnimationFrame(() => {
    document.getElementById("boot-screen")?.classList.add("is-hidden");
    logFront(`[BOOT] first UI frame ${Math.round(performance.now() - bootStartedAt)}ms`);
  });
}

async function initApp() {
  logFront(`[BOOT] initApp start ${Math.round(performance.now() - bootStartedAt)}ms`);
  // ─── Телеметрия: вешаем ловушки ошибок UI раньше всего остального ───
  // (уважает настройку «Отправлять анонимные отчёты об ошибках»)
  await initTelemetry();
  initConfirmDialog();
  initImageViewer();
  initPermissionDialog();
  initVramDialog();
  // Плотная упаковка карточек в разделах (Настройки/Движки/тесты).
  // До старта сетки работают в запасном режиме — разметка не зависит от JS.
  initCardLayout();

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
  const sessionCtrl = new SessionController({
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
    themeSelect: $<HTMLSelectElement>("theme-select"),
    cardLayoutSelect: $<HTMLSelectElement>("card-layout-select"),
    tempSlider: $<HTMLInputElement>("temp-slider"), tempValue: $<HTMLElement>("temp-value"),
    topkSlider: $<HTMLInputElement>("topk-slider"), topkValue: $<HTMLElement>("topk-value"),
    toppSlider: $<HTMLInputElement>("topp-slider"), toppValue: $<HTMLElement>("topp-value"),
    minpSlider: $<HTMLInputElement>("minp-slider"), minpValue: $<HTMLElement>("minp-value"),
    reppenSlider: $<HTMLInputElement>("reppen-slider"), reppenValue: $<HTMLElement>("reppen-value"),
    prespenSlider: $<HTMLInputElement>("prespen-slider"), prespenValue: $<HTMLElement>("prespen-value"),
    btnResetParams: $<HTMLButtonElement>("btn-reset-params"),
    chkShowAdvanced: $<HTMLInputElement>("chk-show-advanced"),
    chkErrorReports: $<HTMLInputElement>("chk-error-reports"),
    translatorModelSelect: $<HTMLSelectElement>("translator-model-select"),
    translatorLangSelect: $<HTMLSelectElement>("translator-lang-select"),
    dataDirDisplay: $<HTMLElement>("data-dir-display"),
    btnChangeDataDir: $<HTMLButtonElement>("btn-change-data-dir"),
  });

  // ─── Панель «Агенты в чате» (две колонки в Настройках) ───
  // Контроллер сам подписан на `model-catalog-changed`, поэтому перерисовывается
  // и после загрузки конфига, и после правки выбора — держим только в шине.
  new AgentPickerController({
    root: $<HTMLElement>("group-box-chat-agents"),
    selectedTitle: $<HTMLElement>("agent-picker-selected-title"),
    selectedList: $<HTMLUListElement>("agent-picker-selected-list"),
    selectedEmpty: $<HTMLElement>("agent-picker-selected-empty"),
    clearBtn: $<HTMLButtonElement>("agent-picker-clear"),
    search: $<HTMLInputElement>("agent-picker-search"),
    addGraphsBtn: $<HTMLButtonElement>("agent-picker-add-graphs"),
    addAgentsBtn: $<HTMLButtonElement>("agent-picker-add-agents"),
    resetBtn: $<HTMLButtonElement>("agent-picker-reset"),
    catalogGroups: $<HTMLElement>("agent-picker-catalog-groups"),
    catalogEmpty: $<HTMLElement>("agent-picker-catalog-empty"),
  });

  // ─── Вкладки рабочей области (браузерные вкладки) ───
  // Студия агентов здесь НЕ создаётся: её экземпляр принадлежит вкладке и рождается
  // в момент открытия раздела. `TabController` получает только фабрику — знание
  // «как собрать студию» живёт в `controllers/studio-page`, а не в бутстраппере.
  // Иначе получилось бы ровно то, от чего уходили: один глобальный экземпляр на
  // приложение, который вторая вкладка делит с первой.
  const tabCtrl = new TabController(
    $<HTMLElement>("workspace-tabs"),
    $<HTMLElement>("workspace-tab-actions"),
    $<HTMLElement>("chat-tab-slot"),
    sharedSampling,
    {
      createStudioTab: (instanceKey) => createAgentStudioTab(instanceKey, loadGraphController),
      onSectionActivated: (_tab, section) => {
        if (section === "sessions") {
          void sessionCtrl.loadSessionsListUI();
        }
      },
    },
  );

  const config = await settingsCtrl.loadConfigCore();
  tabCtrl.init(config ?? {}, false);
  revealBootScreen();
  if (config) void settingsCtrl.hydrateCatalogs(config);
  logFront(`[BOOT] first UI ready ${Math.round(performance.now() - bootStartedAt)}ms`);

  // Кнопка ⚙ рядом со списком агентов в чате: открываем раздел Настроек и
  // подсвечиваем панель, чтобы юзер не искал её глазами.
  bus.on("agents:pick-requested", () => {
    tabCtrl.openSectionInPlace("settings");
    const box = $<HTMLElement>("group-box-chat-agents");
    if (!box) return;
    box.scrollIntoView({ behavior: "smooth", block: "center" });
    box.classList.add("is-highlighted");
    window.setTimeout(() => box.classList.remove("is-highlighted"), 1600);
  });

  // ——— Быстрая навигация по разделам (открытие в той же вкладке) ———
  // Делегирование на document: клоны settings/engines (дубли вкладок) id не
  // несут, слушатель на одном #settings-nav их бы не поймал.
  document.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest(".settings-nav-btn") as HTMLElement | null;
    if (!btn) return;
    const section = btn.dataset.section as TabSection | undefined;
    if (!section) return;
    tabCtrl.openSectionInPlace(section);
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

  // ─── Бейджи обновлений на кнопках меню и настройках ───
  const navMenu = getTabNavigationMenu(tabCtrl);
  initUpdateBadgeTargets({ menuEngines: navMenu });
  initLlamaUpdateWatcher();
  initImageUpdateWatcher();
  initRoutersUpdateWatcher();
  initUpdateWatchers();

  // ─── Дашборд роутера открываем вкладкой внутри приложения ───
  // Плагин поднимает роутер и присылает готовое событие с адресом; хост только
  // показывает вкладку. Отдельный пункт меню для этого не нужен — единственная
  // точка входа «Web UI» живёт в панели плагина.
  void onOpenDashboard((p) => {
    try {
      tabCtrl.openWebview(p);
    } catch (e) {
      showToast(`Не удалось открыть Web UI роутера: ${e}`, "error");
      void trackError("tabs.openWebview", e);
    }
  }).catch((e) => void trackError("tabs.openWebview-listen", e));
}

document.addEventListener("DOMContentLoaded", () => {
  initApp().catch((e) => {
    revealBootScreen();
    console.error("initApp failed:", e);
    showToast(`Ошибка инициализации: ${e}`, "error");
    void trackError("init", e);
  });
});