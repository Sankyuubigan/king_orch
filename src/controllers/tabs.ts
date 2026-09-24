import { invoke } from "@tauri-apps/api/core";
import { store } from "../store";
import { bus } from "../events";
import type { AppTab, TabSection } from "../types";
import { ChatController } from "./chat";
import { buildChatPage, buildWebviewPage, copyMarkdownToClipboard, fillAgentSelect, fillModelSelect } from "../utils";
import type { ChatPageElements, SharedChatControls } from "../utils";
import { showToast, confirmDialog } from "../ui";
import { deleteSession, openSessionFolder, fetchSessions } from "../services";
import { trackError } from "../telemetry";
import { logFront } from "@my-tauri-plugins/plugin-logs";
import { ensureStarted } from "@my-tauri-plugins/plugin-9router";

const SECTION_VIEWS: Record<TabSection, string> = {
  sessions: "#view-sessions",
  "agent-studio": "#view-agent-studio",
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

const DUPLICABLE_SECTIONS: TabSection[] = ["settings", "engines"];

const TAB_ICONS: Partial<Record<AppTab["type"], string>> = {
  main: "🏠",
  chat: "💬",
  webview: "🌐",
};

export interface TabControllerHooks {
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
  customTitle: string | null;
  url: string | null;
  /// Элемент-«страница» вкладки: pageRoot для main/chat/webview,
  /// собственный wrapper .section-host для section (узел раздела один
  /// на приложение и мигрирует в wrapper активной вкладки).
  viewEl: HTMLElement;
  /// Кнопка в полосе вкладок.
  stripEl: HTMLElement;
  labelEl: HTMLElement;
  ctrl: ChatController | null;
  title: string;
}

/// Текущая активная chat-вкладка (фокус), не требующая обязательной обработки.
let activeChatController: ChatController | null = null;

/** Контроллер АКТИВНОЙ чат/главной вкладки (для настроек: селект модели и пр.). */
export function getActiveChatController(): ChatController | null {
  return activeChatController;
}

/** Контроллер чат-вкладки, в которой открыта сессия `sessionId` (если открыта). */
export function getChatControllerForSession(sessionId: string): ChatController | null {
  return TAB_REGISTRY.get(sessionId) ?? null;
}

// Сессия → контроллер (для «Копировать в буфер» живым стейтом).
const TAB_REGISTRY = new Map<string, ChatController>();

let uidSeq = 0;
function genTabId(): string {
  return `tab_${Date.now().toString(36)}_${(uidSeq++).toString(36)}`;
}

export class TabController {
  private stripEl: HTMLElement;
  private slotEl: HTMLElement;
  private shared: SharedChatControls;
  private hooks: TabControllerHooks;
  private entries: TabEntry[] = [];
  private persistTimer: number | null = null;
  private dragState: { id: string; index: number } | null = null;

  constructor(stripEl: HTMLElement, slotEl: HTMLElement, shared: SharedChatControls, hooks?: TabControllerHooks) {
    this.stripEl = stripEl;
    this.slotEl = slotEl;
    this.shared = shared;
    this.hooks = hooks || {};
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
      if (entry.type === "section") {
        this.parkSectionNode(entry.viewEl);
      } else {
        entry.viewEl.remove();
      }
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
      // Активная вкладка восстановления: если ID пропал — берём первую.
      const wanted = saved.some(t => t.id === store.activeTabId) ? store.activeTabId : null;
      this.activate(wanted ?? this.entries[0].id, persist);
    }
    this.renderStrip();
    this.updateStudioVisibility(!!config?.show_advanced_features);
    if (persist) this.persistTabsImmediate();
  }

  /** Скрывает/показывает кнопку «Студия агентов» в Настройках. */
  updateStudioVisibility(visible: boolean) {
    document.querySelectorAll<HTMLElement>('.settings-nav-btn[data-section="agent-studio"]')
      .forEach(b => { b.style.display = visible ? "" : "none"; });
  }

  /** Новая пустая (главная) вкладка — экран «Новая вкладка» с logo.svg. */
  newMainTab(persist = true): AppTab {
    const tab: AppTab = { id: genTabId(), type: "main", sessionId: null, section: null, customTitle: null, url: null };
    this.materializeTab(tab);
    this.renderStrip();
    this.activate(tab.id, persist);
    if (persist) this.persistTabs();
    return tab;
  }

  /** Оригинальный DOM-узел раздела. Узел один на приложение, клонов нет:
   * дубли settings/engines делят один живой узел (контроллеры биндятся по id
   * один раз при старте, клоны были бы мертвыми). Узел мигрирует
   * appendChild в wrapper активной вкладки, на парковке — без .active. */
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

  /** Открыть/сфокусировать вкладку раздела. sessions/agent-studio/logs —
   * синглтон; settings/engines — дубли разрешены, все дубли указывают на один
   * живой DOM-узел (состояние общее, привязки контроллеров целы). */
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
    const el = this.sectionEl(tabSection);
    if (!el) return null;
    const tab: AppTab = { id: genTabId(), type: "section", sessionId: null, section: tabSection, customTitle: null, url: null };
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
   * DOM не клонируется — только смена указателя на оригинальный узел. */
  openSectionInPlace(tabSection: TabSection): AppTab | null {
    const active = this.entries.find(t => t.id === store.activeTabId);
    if (active && active.type === "section") {
      if (active.section === tabSection) {
        logFront(`[tabs] in-place ${tabSection}: уже открыт в ${active.id}`);
        this.activate(active.id);
        return this.tabToApp(active);
      }
      const src = this.sectionEl(tabSection);
      if (!src) return null;
      // Синглтон-раздел уже открыт в другой вкладке — закрываем её,
      // чтобы не плодить два таба на один static-view.
      if (!DUPLICABLE_SECTIONS.includes(tabSection)) {
        const other = this.entries.find(e => e.id !== active.id && e.type === "section" && e.section === tabSection);
        if (other) this.closeTab(other.id);
      }
      const from = active.section ?? "?";
      active.section = tabSection;
      active.viewEl = src;
      active.title = this.sectionTitle(tabSection);
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
    const tab: AppTab = { id: genTabId(), type: "chat", sessionId: id, section: null, customTitle: null, url: null };
    this.materializeTab(tab);
    this.renderStrip();
    this.activate(tab.id);
    this.persistTabs();
    return tab;
  }

  /** Открыть вкладку веб-страницы (iframe 9Router Web UI). */
  async openWebview(): Promise<AppTab | null> {
    try {
      const status = await ensureStarted();
      const base = status?.base_url;
      if (!base) throw new Error("9Router сервер не поднят");
      const existing = this.entries.find(t => t.type === "webview");
      if (existing) { this.activate(existing.id); return this.tabToApp(existing); }
      const tab: AppTab = { id: genTabId(), type: "webview", sessionId: null, section: null, customTitle: null, url: `${base}/dashboard` };
      this.materializeTab(tab);
      this.renderStrip();
      this.activate(tab.id);
      this.persistTabs();
      return tab;
    } catch (e) {
      showToast(`Не удалось открыть Web UI 9Router: ${e}`, "error");
      void trackError("tabs.openWebview", e);
      // 9Router не установлен — ведём на честную панель в Настройках.
      this.openSection("settings");
      return null;
    }
  }

  closeTab(id: string) {
    const idx = this.entries.findIndex(t => t.id === id);
    if (idx === -1) {
      logFront(`[tabs] closeTab: нет вкладки ${id}`);
      return;
    }
    const entry = this.entries[idx];
    if (entry.ctrl) TAB_REGISTRY.delete(entry.ctrl.state.sessionId ?? "");
    if (entry.type === "section") {
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
    const entry: TabEntry = {
      id: tab.id,
      type: tab.type,
      sessionId: tab.sessionId ?? null,
      section: tab.section ?? null,
      customTitle: tab.customTitle ?? null,
      url: tab.url ?? null,
      viewEl: document.createElement("div"),
      stripEl: document.createElement("div"),
      labelEl: document.createElement("span"),
      ctrl: null,
      title: this.defaultTitle(tab),
    };
    switch (tab.type) {
      case "main": {
        const page = buildChatPage(true, this.shared);
        entry.viewEl = page.pageRoot;
        this.bindChatPage(page, entry, true);
        // Экран новой вкладки: клики по плашке навигации разделов.
        page.sectionRail?.addEventListener("click", (e) => {
          const btn = (e.target as HTMLElement).closest(".main-screen-section-btn") as HTMLElement | null;
          if (!btn?.dataset.section) return;
          this.openSection(btn.dataset.section as TabSection);
        });
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
        // Прямой lookup без тоста: user-facing пути (openSection/InPlace)
        // уже проверили узел через sectionEl; throw здесь ловит init().
        const sel = SECTION_VIEWS[tab.section as TabSection] ?? "#view-settings";
        const viewEl = document.querySelector<HTMLElement>(sel);
        if (!viewEl) throw new Error(`Не найден static-раздел ${tab.section}`);
        entry.viewEl = viewEl;
        entry.title = this.sectionTitle(tab.section as TabSection);
        break;
      }
      case "webview": {
        entry.viewEl = buildWebviewPage(tab.url || "about:blank");
        entry.title = "Web UI 9Router";
        break;
      }
    }
    this.entries.push(entry);
    if (entry.viewEl !== this.slotEl && !(entry.type === "section")) {
      // Страницы чат/веб вставляем в слот; статические разделы остаются на месте.
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
    // Кнопка «+» (добавить модель) открывает Настройки.
    page.pageRoot.querySelector?.(".btn-add-model")?.addEventListener("click", () => this.openSection("settings"));
    if (isMain) {
      page.viewChat.classList.add("main-mode");
      if (entry.sessionId) {
        // Мain-вкладка с ранее сохранённым черновиком: поднимаем текст БЕЗ
        // конвертации в чат (welcome + прижатая плашка остаются на месте).
        void ctrl.openSessionDraft(entry.sessionId);
      }
    } else if (entry.sessionId) {
      // Ленивая загрузка сессии (восстановленные вкладки).
      void ctrl.openSession(entry.sessionId);
    }
  }

  private defaultTitle(tab: AppTab): string {
    if (tab.type === "chat") return tab.customTitle || "Курсор…";
    if (tab.type === "webview") return "Web UI 9Router";
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
      customTitle: t.customTitle,
      url: t.url,
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

  activate(id: string, persist = true) {
    store.activeTabId = id;
    let nowFocus: ChatController | null = null;
    this.clearWorkspaceActive();
    for (const t of this.entries) {
      t.stripEl.classList.toggle("active", t.id === id);
    }
    const active = this.entries.find(t => t.id === id);
    if (active) {
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

  private renderStrip() {
    this.stripEl.innerHTML = "";
    for (const t of this.entries) {
      const isActive = t.id === store.activeTabId;
      const el = document.createElement("div");
      el.className = `workspace-tab${isActive ? " active" : ""}`;
      el.dataset.tabId = t.id;
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
  }

  private showContextMenu(entry: TabEntry, x: number, y: number) {
    document.querySelectorAll(".tab-menu-dropdown").forEach(d => d.remove());
    const menu = document.createElement("div");
    menu.className = "tab-menu-dropdown";
    menu.style.left = `${x}px`;
    menu.style.top = `${y}px`;
    if (entry.type === "chat" && entry.sessionId && entry.ctrl) {
      menu.appendChild(this.menuItem("✏️", "Переименовать", async () => {
        const cur = entry.customTitle || entry.title;
        const newT = prompt("Новое название вкладки:", cur);
        if (newT && newT.trim() !== "" && newT !== cur) {
          entry.customTitle = newT.trim();
          this.refreshSessionTitle(entry);
          this.renderStrip();
          this.persistTabs();
        }
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

  // ── Перетаскивание по оси X (mousedown/mousemove) ──

  private bindStripEvents() {
    this.stripEl.addEventListener("mousedown", (e) => {
      const target = (e.target as HTMLElement).closest(".workspace-tab") as HTMLElement | null;
      if (!target || (e.target as HTMLElement).closest(".workspace-tab-close")) return;
      const id = target.dataset.tabId!;
      const idx = this.entries.findIndex(t => t.id === id);
      if (idx === -1) return;
      this.dragState = { id, index: idx };
      let moved = false;
      const move = (ev: MouseEvent) => {
        if (!this.dragState) return;
        const el = this.entries.find(t => t.id === this.dragState!.id);
        if (!el) return;
        if (!moved) {
          moved = true;
          el.stripEl.classList.add("dragging");
        }
        // Горизонтальный обход полосы: смещаем вкладку к месту курсора.
        const tabs = Array.from(this.stripEl.querySelectorAll<HTMLElement>(".workspace-tab"));
        let over = -1;
        for (let i = 0; i < tabs.length; i++) {
          const r = tabs[i].getBoundingClientRect();
          if (ev.clientX < r.left + r.width / 2) { over = i; break; }
        }
        if (over === -1) over = tabs.length;
        const curIdx = this.entries.findIndex(t => t.id === this.dragState!.id);
        if (over !== curIdx && over !== curIdx + 1) {
          const [movedEntry] = this.entries.splice(curIdx, 1);
          this.entries.splice(Math.max(0, over - (over > curIdx ? 1 : 0)), 0, movedEntry);
          store.tabs = this.entries.map(t => this.tabToApp(t));
          this.renderStrip();
        }
      };
      const up = () => {
        const st = this.dragState;
        this.dragState = null;
        this.entries.forEach(t => t.stripEl.classList.remove("dragging"));
        document.removeEventListener("mousemove", move);
        document.removeEventListener("mouseup", up);
        if (st) { this.persistTabs(); }
      };
      document.addEventListener("mousemove", move);
      document.addEventListener("mouseup", up);
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
    entry.title = entry.customTitle || base || "Новая сессия";
  }

  private async refreshLabels() {
    let map: Map<string, string> = new Map();
    try { map = await fetchSessionTitles(); } catch (_) { map = new Map(); }
    fetchSessionTitlesCache = map;
    for (const t of this.entries) {
      if (t.type === "chat") this.refreshSessionTitle(t);
    }
    this.renderStrip();
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