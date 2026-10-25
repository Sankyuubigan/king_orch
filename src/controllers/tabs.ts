import { invoke } from "@tauri-apps/api/core";
import { store } from "../store";
import { bus } from "../events";
import type { AppTab, TabSection } from "../types";
import { ChatController } from "./chat";
import { buildChatPage, buildWebviewPage, copyMarkdownToClipboard, fillAgentSelect, fillModelSelect } from "../utils";
import type { ChatPageElements, SharedChatControls } from "../utils";
import { showToast, confirmDialog, createWorkspaceMenu } from "../ui";
import type { WorkspaceMenuController } from "../ui";
import { deleteSession, renameSession, openSessionFolder, fetchSessions } from "../services";
import { trackError } from "../telemetry";
import { logFront } from "@my-tauri-plugins/plugin-logs";
import { computeTargetIndex, computeShifts } from "../utils/drag";
import type { AgentStudioTab } from "./studio-page";

/**
 * Статичные разделы, живущие в `index.html`.
 *
 * `agent-studio` здесь НЕТ намеренно: студия — не статичная разметка, а экземпляр из
 * `<template id="template-agent-studio">`, который создаётся в рантайме (`buildAgentStudioPage`).
 * Хранить её адрес как CSS-селектор нельзя: узлов может быть несколько, и селектор
 * `#view-agent-studio` попадёт в первый попавшийся.
 */
const SECTION_VIEWS: Partial<Record<TabSection, string>> = {
  sessions: "#view-sessions",
  settings: "#view-settings",
  logs: "#view-logs",
  engines: "#view-engines",
};

const SECTION_ICONS: Record<TabSection, string> = {
  sessions: "🕘",
  "agent-studio": "🧪",
  settings: "⚙️",
  logs: "📝",
  engines: "🚂",
};

/**
 * Разделы, которые можно открыть НЕСКОЛЬКИМИ вкладками.
 *
 * `agent-studio` здесь потому, что студия создаётся фабрикой экземпляров: у каждой вкладки
 * свой холст графа, свой открытый файл и свой undo. `settings`/`engines` — дубли одного
 * живого DOM-узла (состояние общее).
 */
const DUPLICABLE_SECTIONS: TabSection[] = ["settings", "engines", "agent-studio"];

const SECTION_LAZY_TEMPLATES: Partial<Record<TabSection, string>> = {
  settings: "template-settings-lazy-panels",
  logs: "template-logs-content",
  engines: "template-engines-content",
};

const TAB_ICONS: Partial<Record<AppTab["type"], string>> = {
  main: "🏠",
  chat: "💬",
  webview: "🌐",
};

/** Заголовок вкладки-веб-страницы. */
const WEBVIEW_TITLE = "Web UI облачных роутеров";

export interface TabControllerHooks {
  /**
   * Создать вкладку «Студии агентов».
   *
   * Фабрика приходит снаружи (`main.ts`), потому что она тянет динамический импорт
   * drawflow-редактора: `tabs.ts` не должен знать про ленивую загрузку графа, иначе
   * получится цикл дверей `controllers/index -> tabs -> studio-page -> index`.
   * Если хук не задан — секция студии просто не открывается.
   */
  createStudioTab?: (instanceKey: string) => AgentStudioTab;
  /// Раздел-вкладка активирована: графический контроллер и пр. пересоздают
  /// размеры канвы (граф строится по getBoundingClientRect).
  onSectionActivated?: (tab: AppTab, section: TabSection) => void;
  /// Вкладка активирована: для фокусируемых селектов/пересоздания.
  onTabActivated?: (tab: AppTab) => void;
}

interface TabEntry {
  id: string;
  type: AppTab["type"];
  sessionId: string | null;
  section: TabSection | null;
  url: string | null;
  /// Элемент-«страница» вкладки: pageRoot для main/chat/webview,
  /// собственный wrapper .section-host для section (узел раздела один
  /// на приложение и мигрирует в wrapper активной вкладки).
  viewEl: HTMLElement;
  /// Кнопка в полосе вкладок.
  stripEl: HTMLElement;
  labelEl: HTMLElement;
  ctrl: ChatController | null;
  /**
   * Экземпляр студии агентов — только у вкладок раздела `agent-studio`.
   *
   * Вкладка ВЛАДЕЕТ этим экземпляром: его DOM, его три контроллера, его активная
   * суб-вкладка. `dispose()` вызывается ровно там, где вкладка умирает (closeTab /
   * resetEntries / смена раздела in-place), — иначе закрытая вкладка оставляла бы
   * живой холст, слушатели шины и узел в DOM.
   */
  studio: AgentStudioTab | null;
  title: string;
  sessionLoadStarted: boolean;
}

/// Текущая активная chat-вкладка (фокус), не требующая обязательной обработки.
let activeChatController: ChatController | null = null;

/** Контроллер АКТИВНОЙ чат/главной вкладки (для настроек: селект модели и пр.). */
export function getActiveChatController(): ChatController | null {
  return activeChatController;
}

export function getChatControllerForSession(sessionId: string): ChatController | null {
  return TAB_REGISTRY.get(sessionId) ?? null;
}

export function getTabNavigationMenu(tabCtrl: TabController): { setBadge: (id: string, visible: boolean) => void } | null {
  return (tabCtrl as any).navigationMenu ?? null;
}

// Сессия → контроллер (для «Копировать в буфер» живым стейтом).
const TAB_REGISTRY = new Map<string, ChatController>();

let uidSeq = 0;
function genTabId(): string {
  return `tab_${Date.now().toString(36)}_${(uidSeq++).toString(36)}`;
}

export class TabController {
  private stripEl: HTMLElement;
  private actionsEl: HTMLElement;
  private slotEl: HTMLElement;
  private shared: SharedChatControls;
  private hooks: TabControllerHooks;
  private navigationMenu: WorkspaceMenuController | null = null;
  private entries: TabEntry[] = [];
  private persistTimer: number | null = null;

  private onDocumentClick = (event: MouseEvent) => {
    const target = event.target as Node;
    if (!this.stripEl.contains(target) && !this.actionsEl.contains(target)) this.closeNavigationMenu();
  };

  private onDocumentKeydown = (event: KeyboardEvent) => {
    if (event.key === "Escape") this.closeNavigationMenu();
  };

  constructor(
    stripEl: HTMLElement,
    actionsEl: HTMLElement,
    slotEl: HTMLElement,
    shared: SharedChatControls,
    hooks?: TabControllerHooks,
  ) {
    this.stripEl = stripEl;
    this.actionsEl = actionsEl;
    this.slotEl = slotEl;
    this.shared = shared;
    this.hooks = hooks || {};
    this.setupNavigationMenu();
    document.addEventListener("click", this.onDocumentClick);
    document.addEventListener("keydown", this.onDocumentKeydown);
    this.bindStripEvents();
    this.bindBusEvents();
  }

  // ── Публичное API ──

  get activeTab(): AppTab | null {
    return store.tabs.find(t => t.id === store.activeTabId) ?? null;
  }

  private resetEntries() {
    if (this.persistTimer !== null) {
      clearTimeout(this.persistTimer);
      this.persistTimer = null;
    }
    for (const entry of this.entries) {
      if (entry.ctrl) TAB_REGISTRY.delete(entry.ctrl.state.sessionId ?? "");
      // Статичный раздел возвращаем на парковку, экземпляр студии освобождаем:
      // его DOM ничей, парковать нечего (см. closeTab).
      if (entry.studio) this.releaseStudio(entry);
      else if (entry.type === "section") this.parkSectionNode(entry.viewEl);
      else entry.viewEl.remove();
      entry.stripEl.remove();
    }
    this.entries = [];
    store.tabs = [];
    store.activeTabId = null;
    activeChatController = null;
    this.clearWorkspaceActive();
  }

  /** Инициализация после загрузки конфига: восстанавливает вкладки.
   * Битую сохранённую вкладку пропускаем с логом — одна гнилая запись
   * не должна ронять весь старт (иначе пустое окно без ошибок). */
  init(config: any, persist = true) {
    this.resetEntries();
    const saved: AppTab[] = Array.isArray(config?.tabs) ? config.tabs : [];
    for (const tab of saved) {
      try {
        this.materializeTab(tab);
      } catch (e) {
        logFront(`[tabs] init: пропуск битой вкладки ${tab?.id} (${tab?.type}:${tab?.section ?? tab?.sessionId ?? ""}): ${e}`);
        void trackError("tabs.init-materialize", e);
      }
    }
    if (this.entries.length === 0) {
      this.newMainTab(persist);
    } else {
      const savedActive = (config as any)?.active_tab ?? (config as any)?.activeTab ?? null;
      const wanted = typeof savedActive === "string" && this.entries.some(t => t.id === savedActive) ? savedActive : null;
      this.activate(wanted ?? this.entries[0].id, persist);
    }
    this.renderStrip();
    this.updateStudioVisibility(!!config?.show_advanced_features);
    if (persist) this.persistTabsImmediate();
    void this.refreshLabels();
  }

  /** Скрывает/показывает кнопку «Студия агентов» в Настройках. */
  updateStudioVisibility(visible: boolean) {
    document.querySelectorAll<HTMLElement>('.settings-nav-btn[data-section="agent-studio"]')
      .forEach(b => { b.style.display = visible ? "" : "none"; });
  }

  /** Новая пустая (главная) вкладка — экран «Новая вкладка» с logo.svg. */
  newMainTab(persist = true): AppTab {
    const tab: AppTab = { id: genTabId(), type: "main", sessionId: null, section: null, url: null };
    this.materializeTab(tab);
    this.renderStrip();
    this.activate(tab.id, persist);
    if (persist) this.persistTabs();
    return tab;
  }

  private materializeSectionContent(section: TabSection) {
    const templateId = SECTION_LAZY_TEMPLATES[section];
    if (!templateId) return;
    const viewSel = SECTION_VIEWS[section];
    if (!viewSel) return;
    const target = document.querySelector<HTMLElement>(`${viewSel} [data-lazy-content="${section}"]`);
    if (!target || target.childElementCount > 0) return;
    const template = document.getElementById(templateId) as HTMLTemplateElement | null;
    if (!template) {
      const error = `Не найден шаблон ленивого раздела ${section} (${templateId})`;
      logFront(`[tabs] ${error}`);
      void trackError("tabs.lazy-template", error);
      return;
    }
    target.appendChild(template.content.cloneNode(true));
    bus.emit("updates:render");
  }

  /** Оригинальный DOM-узел СТАТИЧНОГО раздела. Узел один на приложение, клонов нет:
   * дубли settings/engines делят один живой узел (контроллеры биндятся по id
   * один раз при старте, клоны были бы мертвыми). Узел мигрирует
   * appendChild в wrapper активной вкладки, на парковке — без .active.
   *
   * Студии здесь нет намеренно: у неё нет узла в `index.html` — экземпляр создаёт
   * фабрика (`createStudio`), поэтому искать студию селектором нельзя (узлов может
   * быть несколько, селектор попал бы в первый попавшийся). */
  private sectionEl(tabSection: TabSection): HTMLElement | null {
    const sel = SECTION_VIEWS[tabSection] ?? "#view-settings";
    const el = document.querySelector<HTMLElement>(sel);
    if (!el) {
      const err = `Не найден static-раздел ${tabSection} (${sel})`;
      logFront(`[tabs] ${err}`);
      void trackError("tabs.section-missing", err);
      showToast(err, "error");
    }
    return el;
  }

  /** Место статичных разделов: они лежат прямыми детьми рабочей области. */
  private sectionHost(): HTMLElement {
    return document.getElementById("workspace-content") ?? document.body;
  }

  /**
   * Создать экземпляр студии агентов для вкладки `instanceKey`.
   *
   * Фабрика приходит хуком (`createStudioTab`), потому что тянет динамический импорт
   * drawflow-редактора: `tabs.ts` не должен зависеть от двери `controllers/index`
   * (получился бы цикл `index -> tabs -> studio-page -> index`).
   *
   * Корень вставляем в рабочую область сразу, как статичные разделы: `.view`
   * показывается только с классом `active`, который ставит `activate()`.
   */
  private createStudio(instanceKey: string): AgentStudioTab {
    const create = this.hooks.createStudioTab;
    if (!create) {
      throw new Error("Студия агентов недоступна: TabController создан без хука createStudioTab");
    }
    const studio = create(instanceKey);
    this.sectionHost().appendChild(studio.root);
    return studio;
  }

  /** Освободить экземпляр студии вкладки (если он есть) и забыть ссылку. */
  private releaseStudio(entry: TabEntry): void {
    entry.studio?.dispose();
    entry.studio = null;
  }

  /** Открыть/сфокусировать вкладку раздела. sessions/logs —
   * синглтон; settings/engines/agent-studio — дубли разрешены, у студии
   * каждый дубль владеет СВОИМ экземпляром (свой граф, свой файл, свой undo). */
  openSection(tabSection: TabSection): AppTab | null {
    if (!DUPLICABLE_SECTIONS.includes(tabSection)) {
      const existing = this.entries.find(
        t => t.type === "section" && t.section === tabSection,
      );
      if (existing) {
        logFront(`[tabs] openSection ${tabSection} -> focus ${existing.id}`);
        this.activate(existing.id);
        return this.tabToApp(existing);
      }
    }
    if (tabSection === "agent-studio") {
      // Студия не имеет узла в index.html: проверять нечего, кроме наличия фабрики.
      // Честно сообщаем, если хук не передан (иначе клик молча ничего не сделает).
      if (!this.hooks.createStudioTab) {
        const err = "Студия агентов недоступна: контроллер вкладок создан без фабрики студии";
        logFront(`[tabs] ${err}`);
        void trackError("tabs.studio-hook-missing", err);
        showToast(err, "error");
        return null;
      }
    } else {
      const el = this.sectionEl(tabSection);
      if (!el) return null;
    }
    const tab: AppTab = { id: genTabId(), type: "section", sessionId: null, section: tabSection, url: null };
    this.materializeTab(tab);
    const created = this.entries[this.entries.length - 1];
    if (!created || created.type !== "section" || created.section !== tabSection) {
      logFront(`[tabs] openSection ${tabSection}: materialize пропустил (дедуп)`);
      return null;
    }
    logFront(`[tabs] openSection ${tabSection} -> new ${created.id}`);
    this.renderStrip();
    this.activate(created.id);
    this.persistTabs();
    return this.tabToApp(created);
  }

  /** Открыть раздел В ТОЙ ЖЕ вкладке (клик из settings-nav): активная
   * section-вкладка перепривязывается на новый раздел без создания вкладки.
   * DOM не клонируется — только смена указателя на оригинальный узел.
   * Студия — исключение: у неё нет общего узла, поэтому экземпляр создаётся
   * заново, а прежний освобождается (`dispose`), иначе закрытая вкладка
   * оставляла бы в рабочей области живой чужой холст. */
  openSectionInPlace(tabSection: TabSection): AppTab | null {
    const active = this.entries.find(t => t.id === store.activeTabId);
    if (active && active.type === "section") {
      if (active.section === tabSection) {
        logFront(`[tabs] in-place ${tabSection}: уже открыт в ${active.id}`);
        this.activate(active.id);
        return this.tabToApp(active);
      }
      let nextViewEl: HTMLElement;
      let nextStudio: AgentStudioTab | null = null;
      if (tabSection === "agent-studio") {
        if (!this.hooks.createStudioTab) {
          const err = "Студия агентов недоступна: контроллер вкладок создан без фабрики студии";
          logFront(`[tabs] ${err}`);
          void trackError("tabs.studio-hook-missing", err);
          showToast(err, "error");
          return null;
        }
        nextStudio = this.createStudio(active.id);
        nextViewEl = nextStudio.root;
      } else {
        const src = this.sectionEl(tabSection);
        if (!src) return null;
        nextViewEl = src;
      }
      // Синглтон-раздел уже открыт в другой вкладке — закрываем её,
      // чтобы не плодить два таба на один static-view.
      if (!DUPLICABLE_SECTIONS.includes(tabSection)) {
        const other = this.entries.find(e => e.id !== active.id && e.type === "section" && e.section === tabSection);
        if (other) this.closeTab(other.id);
      }
      const from = active.section ?? "?";
      const prevStudio = active.studio;
      active.section = tabSection;
      active.studio = nextStudio;
      active.viewEl = nextViewEl;
      active.title = this.sectionTitle(tabSection);
      // Прежний экземпляр этой вкладки больше никому не принадлежит.
      if (prevStudio && prevStudio !== nextStudio) prevStudio.dispose();
      store.tabs = this.entries.map(t => this.tabToApp(t));
      logFront(`[tabs] in-place ${from} -> ${tabSection} в ${active.id}`);
      this.renderStrip();
      this.activate(active.id);
      this.persistTabs();
      return this.tabToApp(active);
    }
    return this.openSection(tabSection);
  }

  /** Открыть/сфокусировать чат-вкладку сессии `id` (bus "session:open"). */
  openSessionTab(id: string): AppTab | null {
    const existing = this.entries.find(t => t.type === "chat" && t.sessionId === id);
    if (existing) {
      this.activate(existing.id);
      return this.tabToApp(existing);
    }
    const tab: AppTab = { id: genTabId(), type: "chat", sessionId: id, section: null, url: null };
    this.materializeTab(tab);
    this.renderStrip();
    this.activate(tab.id);
    this.persistTabs();
    return tab;
  }

  /**
   * Открыть дашборд роутера вкладкой внутри приложения.
   *
   * Вызывается по событию плагина (`onOpenDashboard`): к этому моменту роутер
   * уже поднят и адрес проверен, поэтому хост ничего не стартует — дублировать
   * логику «поднять роутер» в двух местах нельзя, они разъедутся. Идентификатор
   * роутера приходит в payload и уходит в заголовок вкладки: дашборд есть у
   * всех роутеров, и без подписи нельзя понять, чей открыт.
   */
  openWebview(payload: { router: string; base_url: string }): AppTab {
    const url = `${payload.base_url.replace(/\/+$/, "")}/dashboard`;
    // Дедуп по URL, а не «первая попавшаяся webview-вкладка»: вкладок может
    // быть несколько (по одной на роутер), и повторный клик должен возвращать
    // пользователя в его же вкладку, а не переключать на чужую.
    const existing = this.entries.find(t => t.type === "webview" && t.url === url);
    if (existing) {
      this.activate(existing.id);
      // Повторный клик «Web UI» перезагружает iframe: плагин шлёт событие
      // только после живой health-пробы, поэтому перезагрузка безопасна и
      // заодно чинит вкладку, застрявшую на странице ошибки (инцидент
      // 25.10.26 — iframe не мог оживить сам себя: JS на странице ошибки
      // браузера не исполняется).
      this.reloadWebviewEntry(existing);
      return this.tabToApp(existing);
    }
    const tab: AppTab = {
      id: genTabId(),
      type: "webview",
      sessionId: null,
      section: null,
      url,
      title: `Web UI · ${payload.router}`,
    };
    this.materializeTab(tab);
    this.renderStrip();
    this.activate(tab.id);
    this.persistTabs();
    return tab;
  }

  /**
   * Перезагрузить webview-вкладки дашборда роутера с адресом `base_url`.
   *
   * Вызывается по событию жизни от супервизора плагина (`running:true`):
   * плагин шлёт его только при отвечающем порте, поэтому перезагрузка
   * безопасна и оживляет вкладку, застрявшую на странице ошибки браузера.
   */
  reloadWebviewsByBaseUrl(baseUrl: string): void {
    const base = baseUrl.replace(/\/+$/, "");
    for (const entry of this.entries) {
      if (entry.type !== "webview" || !entry.url) continue;
      if (entry.url !== `${base}/dashboard`) continue;
      this.reloadWebviewEntry(entry);
    }
  }

  /** Перезагрузить iframe одной webview-вкладки (повторное присвоение src —
   * всегда навигация, даже на тот же адрес). */
  private reloadWebviewEntry(entry: TabEntry): void {
    if (entry.type !== "webview" || !entry.url) return;
    const iframe = entry.viewEl.querySelector<HTMLIFrameElement>("iframe.webview-frame");
    if (!iframe) {
      logFront(`[tabs] reloadWebviewEntry: в ${entry.id} нет iframe — пропущено`);
      return;
    }
    iframe.src = entry.url;
    logFront(`[tabs] reload webview ${entry.id} (${entry.url})`);
  }

  closeTab(id: string) {
    const idx = this.entries.findIndex(t => t.id === id);
    if (idx === -1) {
      logFront(`[tabs] closeTab: нет вкладки ${id}`);
      return;
    }
    const entry = this.entries[idx];
    if (entry.ctrl) TAB_REGISTRY.delete(entry.ctrl.state.sessionId ?? "");
    if (entry.studio) {
      // Экземпляр студии принадлежит только этой вкладке: узел удаляем, ресурсы
      // освобождаем. Парковка (как для статичных разделов) здесь не нужна и вредна —
      // холст закрытой вкладки не должен оставаться в рабочей области.
      this.releaseStudio(entry);
      logFront(`[tabs] closeTab студия ${entry.section} ${id} (экземпляр освобождён)`);
    } else if (entry.type === "section") {
      // Оригинальный узел один на всех и из DOM не удаляется никогда.
      // Последний наследник — паркуем узел (снять .active, вернуть в
      // workspace-content); общий — класс не трогаем, его поднимет activate.
      const shared = this.entries.some(e => e.id !== id && e.viewEl === entry.viewEl);
      if (!shared) this.parkSectionNode(entry.viewEl);
      logFront(`[tabs] closeTab section ${entry.section} ${id}${shared ? " (узел общий, оставлен)" : " (парковка)"}`);
    } else {
      entry.viewEl.remove();
      logFront(`[tabs] closeTab ${entry.type} ${id}`);
    }
    entry.stripEl.remove();
    this.entries.splice(idx, 1);
    store.tabs = this.entries.map(t => this.tabToApp(t));
    if (store.activeTabId === id) {
      store.activeTabId = null;
    }
    if (this.entries.length === 0) {
      // Закрыта последняя вкладка — создаём новую главную.
      this.newMainTab();
      return;
    }
    if (!store.activeTabId) {
      const next = this.entries[Math.min(idx, this.entries.length - 1)];
      this.activate(next.id);
    } else {
      // Переактивация текущей: её activate поднимет узел заново
      // (closeTab мог снять класс с общего узла до splice).
      this.activate(store.activeTabId);
    }
    this.renderStrip();
    this.persistTabs();
  }

  // ── Materialization (view + контроллер) ──

  private materializeTab(tab: AppTab) {
    // Дедуп только для синглтон-разделов; settings/engines — дубли разрешены.
    if (tab.type === "section" && tab.section && !DUPLICABLE_SECTIONS.includes(tab.section)
      && this.entries.some(e => e.type === "section" && e.section === tab.section)) {
      return;
    }
    const sessionId = (tab as any).sessionId ?? (tab as any).session_id ?? null;
    if (tab.type === "chat" && !sessionId) {
      logFront(`[tabs] init: вкладка ${tab.id} помечена как чат, но диалог не указан — открыта как новая`);
      tab = { ...tab, type: "main" };
    }
    const entry: TabEntry = {
      id: tab.id,
      type: tab.type,
      sessionId,
      section: tab.section ?? null,
      url: tab.url ?? null,
      viewEl: document.createElement("div"),
      stripEl: document.createElement("div"),
      labelEl: document.createElement("span"),
      ctrl: null,
      studio: null,
      title: this.defaultTitle(tab),
      sessionLoadStarted: false,
    };
    switch (tab.type) {
      case "main": {
        const page = buildChatPage(true, this.shared);
        entry.viewEl = page.pageRoot;
        this.bindChatPage(page, entry, true);
        entry.title = "Новая сессия";
        break;
      }
      case "chat": {
        const page = buildChatPage(false, this.shared);
        entry.viewEl = page.pageRoot;
        this.bindChatPage(page, entry, false);
        entry.title = this.defaultTitle(tab);
        break;
      }
      case "section": {
        if (tab.section === "agent-studio") {
          // Прямой lookup узла здесь был бы ошибкой: студия не статична, её экземпляр
          // создаётся фабрикой (свой DOM, свой граф, свой undo на каждую вкладку).
          entry.studio = this.createStudio(tab.id);
          entry.viewEl = entry.studio.root;
          entry.title = this.sectionTitle("agent-studio");
          break;
        }
        // Прямой lookup без тоста: user-facing пути (openSection/InPlace)
        // уже проверили узел через sectionEl; throw здесь ловит init().
        const viewEl = document.querySelector<HTMLElement>(SECTION_VIEWS[tab.section as TabSection] ?? "#view-settings");
        if (!viewEl) throw new Error(`Не найден static-раздел ${tab.section}`);
        entry.viewEl = viewEl;
        entry.title = this.sectionTitle(tab.section as TabSection);
        break;
      }
      case "webview": {
        entry.viewEl = buildWebviewPage(tab.url || "about:blank");
        entry.title = this.defaultTitle(tab);
        break;
      }
    }
    this.entries.push(entry);
    if (entry.viewEl !== this.slotEl && !(entry.type === "section")) {
      // Страницы чат/веб вставляем в слот; разделы остаются на месте —
      // статические лежат в разметке, корень студии вставила фабрика (`createStudio`).
      this.slotEl.appendChild(entry.viewEl);
    }
    store.tabs = this.entries.map(t => this.tabToApp(t));
  }

  private bindChatPage(page: ChatPageElements, entry: TabEntry, isMain: boolean) {
    const ctrl = new ChatController(page, {
      onBecameChat: (sessionId: string) => {
        const wasMain = entry.type !== "chat";
        entry.type = "chat";
        entry.sessionId = sessionId;
        entry.sessionLoadStarted = true;
        if (entry.ctrl) TAB_REGISTRY.set(sessionId, entry.ctrl);
        if (wasMain) {
          entry.viewEl.classList.remove("main-mode");
          page.viewChat.classList.remove("main-mode");
        }
        store.tabs = this.entries.map(t => this.tabToApp(t));
        this.renderStrip();
        this.persistTabs();
        bus.emit("session:changed");
      },
      onDraftSession: (sessionId: string) => {
        // Мain-вкладка накопила черновик: привязываем sessionId без конвертации.
        if (entry.type !== "main") return;
        entry.sessionId = sessionId;
        entry.sessionLoadStarted = true;
        if (entry.ctrl) TAB_REGISTRY.set(sessionId, entry.ctrl);
        store.tabs = this.entries.map(t => this.tabToApp(t));
        this.persistTabs();
      },
    });
    entry.ctrl = ctrl;
    if (entry.sessionId) TAB_REGISTRY.set(entry.sessionId, ctrl);
    // Селекты моделей/агентов заполняем сразу: config:loaded мог уже пройти
    // до восстановления вкладок (init() идёт ПОСЛЕ loadConfig).
    fillModelSelect(page.modelSelect);
    fillAgentSelect(page.agentSelect);
    if (isMain) page.viewChat.classList.add("main-mode");
  }

  private defaultTitle(tab: AppTab): string {
    if (tab.type === "chat") return (tab as any).title || "Новая сессия";
    if (tab.type === "webview") return tab.title || WEBVIEW_TITLE;
    if (tab.type === "section") return this.sectionTitle(tab.section as TabSection);
    return "Новая сессия";
  }

  private sectionTitle(s: TabSection): string {
    switch (s) {
      case "sessions": return "История сессий";
      case "agent-studio": return "Студия агентов";
      case "settings": return "Настройки";
      case "logs": return "Логи";
      case "engines": return "Движки";
    }
  }

  private tabToApp(t: TabEntry): AppTab {
    return {
      id: t.id,
      type: t.type,
      sessionId: t.sessionId,
      section: t.section,
      url: t.url,
      title: t.title,
    };
  }

  // ── Активация ──

  /** Инвариант видимости: гасим ВСЕ .active внутри рабочей области,
   * включая stray-узлы на любом уровне вложенности. Внутренний mainView
   * чата поднимает refreshActivePage сразу после (subchat-стейт при смене
   * вкладки и так сбрасывался в main, поведение не меняется). Сабтаб студии
   * поднимает onSectionActivated. */
  private clearWorkspaceActive() {
    const root = document.getElementById("workspace-content") ?? document.body;
    root.querySelectorAll(".view.active, .tab-page.active").forEach(n => n.classList.remove("active"));
  }

  /** Возврат раздел-узла на парковку, когда его вкладка закрыта и наследников нет. */
  private parkSectionNode(node: HTMLElement) {
    node.classList.remove("active");
    const park = document.getElementById("workspace-content") ?? document.body;
    if (node.parentElement !== park) park.appendChild(node);
  }

  private activateSession(entry: TabEntry) {
    if (entry.sessionLoadStarted || !entry.sessionId || !entry.ctrl) return;
    entry.sessionLoadStarted = true;
    const sessionId = entry.sessionId;
    const load = entry.type === "main"
      ? entry.ctrl.openSessionDraft(sessionId)
      : entry.ctrl.openSession(sessionId);
    void Promise.resolve(load).catch((error) => {
      entry.sessionLoadStarted = false;
      logFront(`[tabs] не удалось загрузить сессию ${sessionId}: ${error}`);
      void trackError("tabs.load-restored-session", error);
    });
  }

  activate(id: string, persist = true) {
    store.activeTabId = id;
    let nowFocus: ChatController | null = null;
    this.clearWorkspaceActive();
    for (const t of this.entries) {
      t.stripEl.classList.toggle("active", t.id === id);
    }
    const active = this.entries.find(t => t.id === id);
    if (active) {
      if (active.studio) {
        // Вкладка студии поднимает СВОЮ суб-вкладку (в одной открыт граф, в другой —
        // тесты) и, если это граф, свой холст. Переключение соседней вкладки её не трогает.
        active.studio.activate();
      } else if (active.type === "section" && active.section) {
        this.materializeSectionContent(active.section);
      } else {
        this.activateSession(active);
      }
      active.viewEl.classList.add("active");
      if (active.type === "chat" || active.type === "main") {
        nowFocus = active.ctrl;
      }
      logFront(`[tabs] activate ${active.type}${active.section ? `:${active.section}` : ""} ${id}`);
    } else {
      logFront(`[tabs] activate: нет вкладки ${id}`);
    }
    // Самопроверка инварианта «ровно одна видимая страница» — сразу в лог,
    // чтобы наслоение было видно без отладчика.
    const sectionVisible = document.querySelectorAll("#workspace-content > .view.active").length;
    const pageVisible = Array.from(this.slotEl.children).filter(
      ch => (ch as HTMLElement).classList?.contains("tab-page") && (ch as HTMLElement).classList.contains("active"),
    ).length;
    if (sectionVisible + pageVisible !== 1) {
      logFront(`[tabs] ВНИМАНИЕ: видимых страниц ${sectionVisible + pageVisible} (разделы:${sectionVisible} чат/веб:${pageVisible}), ожидалась 1`);
    }
    activeChatController = nowFocus;
    this.refreshActivePage();
    this.renderStrip();
    if (persist) this.persistTabs();
    const tab = this.entries.find(t => t.id === id);
    if (tab && tab.type === "section") {
      this.hooks.onSectionActivated?.(this.tabToApp(tab), tab.section as TabSection);
    } else if (tab) {
      this.hooks.onTabActivated?.(this.tabToApp(tab));
    }
    bus.emit("updates:render");
  }

  /** При смене активации: сверить активный .view внутри страниц чата. */
  private refreshActivePage() {
    const active = this.entries.find(t => t.id === store.activeTabId);
    if (!active) return;
    if (active.type === "main" || active.type === "chat") {
      active.ctrl?.mainView.classList.add("active");
      active.ctrl?.subView.classList.remove("active");
    }
    if (active.ctrl) activeChatController = active.ctrl;
  }

  // ── Полоса вкладок ──

  private setupNavigationMenu() {
    this.actionsEl.innerHTML = "";
    const button = document.createElement("button");
    button.className = "workspace-tab-menu-toggle";
    button.type = "button";
    button.title = "Главное меню";
    button.setAttribute("aria-label", "Главное меню");
    button.setAttribute("aria-haspopup", "menu");
    button.setAttribute("aria-controls", "workspace-main-menu");
    button.setAttribute("aria-expanded", "false");
    button.textContent = "☰";
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      if (button.getAttribute("aria-expanded") === "true") this.closeNavigationMenu();
      else this.navigationMenu?.open();
    });
    button.addEventListener("keydown", (event) => {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        this.navigationMenu?.open(true);
      }
    });
    this.actionsEl.appendChild(button);
    // «Новая вкладка» из бургера удалена: единственный способ создать вкладку —
    // кнопка «＋» на панели вкладок. Пользователи сначала открывали бургер из
    // привычки (люди ищут старые точки входа где попало), и пункт рядом с
    // навигационными разделами обещал поведение, которое он не выполняет:
    // по воздуху висел как переключатель раздела, значит и мёртвый вес.
    this.navigationMenu = createWorkspaceMenu(button, [
      { id: "sessions", label: "История сессий", icon: "🕘", action: () => { this.openSection("sessions"); } },
      { id: "agent-studio", label: "Студия агентов", icon: "🧪", action: () => { this.openSection("agent-studio"); } },
      { id: "engines", label: "Движки", icon: "🚂", badge: true, action: () => { this.openSection("engines"); } },
      { id: "settings", label: "Настройки", icon: "⚙️", badge: true, action: () => { this.openSection("settings"); } },
      { id: "logs", label: "Логи", icon: "📝", action: () => { this.openSection("logs"); } },
    ]);
  }

  private closeNavigationMenu() {
    this.navigationMenu?.close();
  }

  private renderStrip() {
    // Полоса перерисовывается целиком (innerHTML = "" сбрасывает scrollLeft
    // в 0), поэтому позицию прокрутки снимаем до и возвращаем после —
    // иначе активная вкладка уезжает за край при каждом переключении.
    const savedScroll = this.stripEl.scrollLeft;
    this.stripEl.innerHTML = "";
    // Номер вкладки студии вычисляем на каждой перерисовке: заголовок — производное
    // от порядка вкладок, а не скопированное значение (иначе после закрытия первой
    // студии вторая навсегда осталась бы «Студия агентов 2»). Первой номер не пишем.
    const studioBaseTitle = this.sectionTitle("agent-studio");
    let studioNo = 0;
    for (const t of this.entries) {
      if (!t.studio) continue;
      studioNo += 1;
      t.title = studioNo === 1 ? studioBaseTitle : `${studioBaseTitle} ${studioNo}`;
    }
    for (const t of this.entries) {
      const isActive = t.id === store.activeTabId;
      const el = document.createElement("div");
      el.className = `workspace-tab${isActive ? " active" : ""}`;
      el.dataset.tabId = t.id;
      el.title = t.title;
      const icon = document.createElement("span");
      icon.className = "workspace-tab-icon";
      icon.textContent = t.type === "section" ? SECTION_ICONS[t.section as TabSection] : (TAB_ICONS[t.type] ?? "❔");
      const label = document.createElement("span");
      label.className = "workspace-tab-label";
      label.textContent = t.title;
      const close = document.createElement("button");
      close.className = "workspace-tab-close";
      close.title = "Закрыть вкладку";
      close.textContent = "✕";
      close.addEventListener("click", (e) => { e.stopPropagation(); this.closeTab(t.id); });
      el.append(icon, label, close);
      el.addEventListener("click", () => this.activate(t.id));
      el.addEventListener("contextmenu", (e) => { e.preventDefault(); e.stopPropagation(); this.showContextMenu(t, e.clientX, e.clientY); });
      this.stripEl.appendChild(el);
      t.stripEl = el;
      t.labelEl = label;
    }
    const addBtn = document.createElement("button");
    addBtn.className = "workspace-tab-add";
    addBtn.title = "Новая вкладка";
    addBtn.textContent = "＋";
    addBtn.addEventListener("click", () => this.newMainTab());
    this.stripEl.appendChild(addBtn);
    this.stripEl.scrollLeft = savedScroll;
    this.scrollActiveIntoView();
  }

  /** Дотянуть активную вкладку в видимую зону полосы (inline: nearest — без
   * лишнего движения, если вкладка уже целиком видна). Опциональный вызов:
   * в jsdom scrollIntoView отсутствует. */
  private scrollActiveIntoView() {
    const active = this.entries.find(t => t.id === store.activeTabId);
    active?.stripEl?.scrollIntoView?.({ inline: "nearest", block: "nearest" });
  }

  private showContextMenu(entry: TabEntry, x: number, y: number) {
    document.querySelectorAll(".tab-menu-dropdown").forEach(d => d.remove());
    const menu = document.createElement("div");
    menu.className = "tab-menu-dropdown";
    menu.style.left = `${x}px`;
    menu.style.top = `${y}px`;
    if (entry.type === "chat" && entry.sessionId && entry.ctrl) {
      menu.appendChild(this.menuItem("✏️", "Переименовать", async () => {
        const cur = fetchSessionTitlesCache.get(entry.sessionId as string) || entry.title;
        const newT = prompt("Новое название:", cur);
        if (!newT || newT.trim() === "" || newT.trim() === cur) return;
        try {
          await renameSession(entry.sessionId as string, newT.trim());
          bus.emit("session:changed");
        } catch (e) { showToast(`Ошибка: ${e}`, "error"); void trackError("tabs.rename", e); }
      }));
      menu.appendChild(this.menuItem("📋", "Копировать в буфер", () => {
        void copyMarkdownToClipboard(entry.title, entry.ctrl!.state.history);
      }));
      menu.appendChild(this.menuItem("📁", "Проводник", () => {
        void openSessionFolder(entry.sessionId as string).catch(err => void trackError("tabs.explorer", err));
      }));
      menu.appendChild(this.menuItem("🗑️", "Удалить сессию", async () => {
        const yes = await confirmDialog("Удаление", "Удалить сессию с диска? Вкладка закроется.");
        if (!yes) return;
        try {
          await deleteSession(entry.sessionId as string);
          bus.emit("session:deleted", entry.sessionId as string);
        } catch (e) { showToast(`Ошибка: ${e}`, "error"); void trackError("tabs.delete", e); }
      }));
      menu.appendChild(this.menuItem("✕", "Закрыть вкладку", () => this.closeTab(entry.id)));
    } else if (entry.type === "main") {
      menu.appendChild(this.menuItem("🧹", "Открыть новую вкладку", () => this.newMainTab()));
    } else {
      menu.appendChild(this.menuItem("✕", "Закрыть вкладку", () => this.closeTab(entry.id)));
    }
    menu.classList.add("show");
    document.body.appendChild(menu);
    setTimeout(() => {
      const closer = (e: MouseEvent) => {
        if (!menu.contains(e.target as Node)) { menu.remove(); document.removeEventListener("click", closer); }
      };
      document.addEventListener("click", closer);
    }, 0);
  }

  private menuItem(icon: string, label: string, onAction: () => void): HTMLElement {
    const btn = document.createElement("button");
    btn.className = "session-menu-item";
    btn.innerHTML = `<span class="menu-ico">${icon}</span>${label}`;
    btn.addEventListener("click", () => { document.querySelectorAll(".tab-menu-dropdown").forEach(d => d.remove()); onAction(); });
    return btn;
  }

  // ── Перетаскивание по оси X (Pointer Events + transform, без перерисовки DOM) ──

  private bindStripEvents() {
    // Полоса скроллится только по горизонтали (overflow-y: hidden), а Chromium
    // вертикальное колесо в горизонтальный контейнер не транслирует — делаем
    // это явно. preventDefault только если сдвинулись: у края и при отсутствии
    // переполнения событие должно уйти дальше обычным порядком.
    this.stripEl.addEventListener("wheel", (e: WheelEvent) => {
      const strip = this.stripEl;
      if (strip.scrollWidth <= strip.clientWidth) return;
      const raw = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
      if (raw === 0) return;
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? strip.clientWidth : 1;
      const before = strip.scrollLeft;
      strip.scrollLeft = before + raw * unit;
      if (strip.scrollLeft !== before) e.preventDefault();
    }, { passive: false });

    this.stripEl.addEventListener("pointerdown", (e) => {
      const target = (e.target as HTMLElement).closest(".workspace-tab") as HTMLElement | null;
      if (!target || (e.target as HTMLElement).closest(".workspace-tab-close")) return;
      const id = target.dataset.tabId!;
      const idx = this.entries.findIndex(t => t.id === id);
      if (idx === -1) return;

      const startX = e.clientX;
      const startY = e.clientY;
      let dragging = false;
      let startIdx = idx;
      let targetIdx = idx;
      let centers: number[] = [];
      let step = 0;
      let tabEls: HTMLElement[] = [];

      const onMove = (ev: PointerEvent) => {
        if (!dragging) {
          const dx = ev.clientX - startX;
          const dy = ev.clientY - startY;
          if (Math.abs(dx) < 5 && Math.abs(dy) < 5) return;
          dragging = true;

          tabEls = Array.from(this.stripEl.querySelectorAll<HTMLElement>(".workspace-tab"));
          const rects = tabEls.map(el => el.getBoundingClientRect());
          centers = rects.map(r => r.left + r.width / 2);
          step = tabEls.length > 1 ? rects[1].left - rects[0].left : rects[0].width + 4;

          target.setPointerCapture(ev.pointerId);
          target.classList.add("dragging");
          target.style.zIndex = "100";
          target.style.transition = "none";
          for (const el of tabEls) {
            if (el !== target) el.style.transition = "transform 0.18s cubic-bezier(0.2, 0, 0, 1)";
          }
        }

        const dx = ev.clientX - startX;
        target.style.transform = `translateX(${dx}px)`;

        const currentCenter = centers[startIdx] + dx;
        const newTarget = computeTargetIndex(centers, currentCenter);

        if (newTarget !== targetIdx) {
          targetIdx = newTarget;
          const shifts = computeShifts(tabEls.length, startIdx, targetIdx, step);
          for (let i = 0; i < tabEls.length; i++) {
            if (i === startIdx) continue;
            tabEls[i].style.transform = `translateX(${shifts[i]}px)`;
          }
        }
      };

      const onUp = () => {
        this.stripEl.removeEventListener("pointermove", onMove);
        this.stripEl.removeEventListener("pointerup", onUp);
        this.stripEl.removeEventListener("pointercancel", onUp);

        if (dragging) {
          for (const el of tabEls) {
            el.classList.remove("dragging");
            el.style.transform = "";
            el.style.transition = "";
            el.style.zIndex = "";
          }
          if (targetIdx !== startIdx) {
            const [moved] = this.entries.splice(startIdx, 1);
            this.entries.splice(targetIdx, 0, moved);
            store.tabs = this.entries.map(t => this.tabToApp(t));
          }
          this.renderStrip();
          this.persistTabs();
        }
      };

      this.stripEl.addEventListener("pointermove", onMove);
      this.stripEl.addEventListener("pointerup", onUp);
      this.stripEl.addEventListener("pointercancel", onUp);
    });
  }

  // ── Персистентность ──

  private persistTabs() {
    if (this.persistTimer !== null) clearTimeout(this.persistTimer);
    this.persistTimer = window.setTimeout(() => this.persistTabsImmediate(), 300);
  }

  private persistTabsImmediate() {
    this.persistTimer = null;
    bus.emit("tabs:changed");
    void invoke("set_tabs", {
      tabs: store.tabs,
      activeTab: store.activeTabId,
    }).catch((e) => void trackError("tabs.persist", e));
  }

  // ── Дребезг лейблов ──

  private refreshSessionTitle(entry: TabEntry) {
    const base = fetchSessionTitlesCache.get(entry.sessionId as string);
    entry.title = base || entry.title || "Новая сессия";
  }

  private async refreshLabels() {
    let map: Map<string, string> = new Map();
    try { map = await fetchSessionTitles(); } catch (_) { map = new Map(); }
    fetchSessionTitlesCache = map;
    let changed = false;
    for (const t of this.entries) {
      if (t.type !== "chat") continue;
      const before = t.title;
      this.refreshSessionTitle(t);
      if (t.title !== before) changed = true;
    }
    this.renderStrip();
    // persistTabs/set_tabs читают store.tabs, а не this.entries: без синхронизации
    // в app_config.json уедут устаревшие заголовки и вкладка после перезапуска
    // мигнёт старым именем до ответа get_sessions.
    if (changed) {
      store.tabs = this.entries.map(t => this.tabToApp(t));
      this.persistTabs();
    }
  }

  // ── Шина ──

  private bindBusEvents() {
    bus.on("session:open", (id: string) => this.openSessionTab(id));
    bus.on("session:deleted", (id: string) => {
      const victims = this.entries.filter(t => t.sessionId === id);
      for (const v of victims) this.closeTab(v.id);
    });
    bus.on("session:changed", () => void this.refreshLabels());
    bus.on("advanced:visibility", (visible: boolean) => this.updateStudioVisibility(visible));
  }
}

// Кэш названий сессий (подписка session:changed).
let fetchSessionTitlesCache: Map<string, string> = new Map();

async function fetchSessionTitles(): Promise<Map<string, string>> {
  const sessions = (await fetchSessions()) ?? [];
  const map = new Map<string, string>();
  for (const s of sessions) map.set(s.id, s.title);
  return map;
}