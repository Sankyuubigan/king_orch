import { store } from "../store";
import { bus } from "../events";
import { showToast, confirmDialog } from "../ui";
import { fetchSessions, deleteSession, renameSession, openSessionFolder, loadSession } from "../services";
import { copyMarkdownToClipboard, renderMarkdown, stripStreamArtifacts, getAgentDisplayName } from "../utils";
import { trackError } from "../telemetry";
import { getChatControllerForSession } from "./tabs";
import type { SessionMeta, ChatMessage } from "../types";

export interface SessionElements {
  sessionList: HTMLDivElement;
  sessionPreviewTitle: HTMLDivElement;
  sessionPreviewDate: HTMLDivElement;
  sessionPreviewOpen: HTMLButtonElement;
  sessionPreviewBody: HTMLDivElement;
  sessionSelectToggle: HTMLButtonElement;
  sessionSelectBar: HTMLDivElement;
  sessionSelectCount: HTMLSpanElement;
  sessionSelectAll: HTMLButtonElement;
  sessionBulkDelete: HTMLButtonElement;
  sessionSelectCancel: HTMLButtonElement;
}

type DayGroup = "today" | "yesterday" | "earlier";

const GROUP_LABELS: Record<DayGroup, string> = {
  today: "Сегодня",
  yesterday: "Вчера",
  earlier: "Ранее",
};

function dayGroupOf(updatedSec: number): DayGroup {
  const ts = updatedSec * 1000;
  const now = new Date();
  const startToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (ts >= startToday) return "today";
  if (ts >= startToday - 24 * 60 * 60 * 1000) return "yesterday";
  return "earlier";
}

function formatItemTime(s: SessionMeta): string {
  const d = new Date(s.updated_at * 1000);
  const group = dayGroupOf(s.updated_at);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  if (group === "earlier") {
    return `${String(d.getDate()).padStart(2, "0")}.${String(d.getMonth() + 1).padStart(2, "0")}.${String(d.getFullYear()).slice(-2)}`;
  }
  return `${hh}:${mm}`;
}

function formatFullDate(updatedSec: number): string {
  const d = new Date(updatedSec * 1000);
  return d.toLocaleString("ru-RU", {
    day: "2-digit",
    month: "2-digit",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function messageRole(msg: ChatMessage): "user" | "agent" | "system" {
  if (msg.author === "user") return "user";
  if (msg.author === "system") return "system";
  return "agent";
}

export class SessionController {
  private el: SessionElements;
  private sessions: SessionMeta[] = [];
  private previewId: string | null = null;
  private previewSeq = 0;
  private previewDebounce: number | null = null;
  private previewCache = new Map<string, ChatMessage[]>();
  private selectMode = false;
  private selectedIds = new Set<string>();
  private sessionsLoadPromise: Promise<void> | null = null;
  private sessionsReloadQueued = false;

  constructor(el: SessionElements) {
    this.el = el;
    this.bindDomEvents();
    this.bindBusEvents();
    this.el.sessionPreviewOpen.addEventListener("click", () => {
      if (this.previewId) bus.emit("session:open", this.previewId);
    });
    this.el.sessionSelectToggle.addEventListener("click", () => this.setSelectMode(!this.selectMode));
    this.el.sessionSelectCancel.addEventListener("click", () => this.setSelectMode(false));
    this.el.sessionSelectAll.addEventListener("click", () => this.selectAll());
    this.el.sessionBulkDelete.addEventListener("click", () => void this.bulkDeleteSelected());
  }

  private setSelectMode(on: boolean) {
    this.selectMode = on;
    if (!on) this.selectedIds.clear();
    const left = document.querySelector(".sessions-left");
    left?.classList.toggle("select-mode", on);
    this.el.sessionSelectBar.hidden = !on;
    this.el.sessionSelectToggle.textContent = on ? "Готово" : "Выбрать";
    this.el.sessionSelectToggle.classList.toggle("active", on);
    this.syncSelectionUI();
  }

  private selectAll() {
    if (this.selectedIds.size === this.sessions.length) {
      this.selectedIds.clear();
    } else {
      for (const s of this.sessions) this.selectedIds.add(s.id);
    }
    this.syncSelectionUI();
  }

  private toggleSelected(id: string) {
    if (this.selectedIds.has(id)) this.selectedIds.delete(id);
    else this.selectedIds.add(id);
    this.syncSelectionUI();
  }

  private syncSelectionUI() {
    const n = this.selectedIds.size;
    this.el.sessionSelectCount.textContent = `Выбрано: ${n}`;
    this.el.sessionBulkDelete.disabled = n === 0;
    this.el.sessionSelectAll.textContent =
      this.sessions.length > 0 && n === this.sessions.length ? "Снять" : "Все";
    for (const item of this.el.sessionList.querySelectorAll<HTMLElement>(".session-item")) {
      const sid = item.dataset.sid;
      if (!sid) continue;
      const selected = this.selectedIds.has(sid);
      item.classList.toggle("selected", selected);
      const cb = item.querySelector<HTMLInputElement>(".session-item-checkbox");
      if (cb) cb.checked = selected;
    }
  }

  private async bulkDeleteSelected() {
    const ids = [...this.selectedIds];
    if (ids.length === 0) return;
    const yes = await confirmDialog(
      "Удаление",
      ids.length === 1 ? "Удалить сессию?" : `Удалить ${ids.length} сессий?`
    );
    if (!yes) return;
    let failed = 0;
    for (const id of ids) {
      try {
        await deleteSession(id);
        this.previewCache.delete(id);
        if (this.previewId === id) this.clearPreview();
        bus.emit("session:deleted", id);
      } catch (e) {
        failed++;
        void trackError("sessions.bulkDelete", e);
      }
    }
    this.setSelectMode(false);
    await this.loadSessionsListUI();
    if (failed > 0) {
      showToast(`Удалено с ошибками: ${failed} из ${ids.length}`, "error");
    } else {
      showToast(ids.length === 1 ? "Удалено." : `Удалено: ${ids.length}`, "success");
    }
  }

  loadSessionsListUI(): Promise<void> {
    if (this.sessionsLoadPromise) {
      this.sessionsReloadQueued = true;
      return this.sessionsLoadPromise;
    }
    const promise = (async () => {
      do {
        this.sessionsReloadQueued = false;
        await this.loadSessionsListUIOnce();
      } while (this.sessionsReloadQueued);
    })().finally(() => {
      this.sessionsLoadPromise = null;
      if (this.sessionsReloadQueued) void this.loadSessionsListUI();
    });
    this.sessionsLoadPromise = promise;
    return promise;
  }

  private async loadSessionsListUIOnce() {
    try {
      const sessions = (await fetchSessions()) as SessionMeta[];
      sessions.sort((a, b) => b.updated_at - a.updated_at);
      this.sessions = sessions;
      // Убираем из выделения сессии, которых больше нет на диске.
      for (const id of [...this.selectedIds]) {
        if (!sessions.some(s => s.id === id)) this.selectedIds.delete(id);
      }
      this.el.sessionList.innerHTML = "";

      let lastGroup: DayGroup | null = null;
      for (const s of sessions) {
        const group = dayGroupOf(s.updated_at);
        if (group !== lastGroup) {
          lastGroup = group;
          const header = document.createElement("div");
          header.className = "session-group-title";
          header.textContent = GROUP_LABELS[group];
          this.el.sessionList.appendChild(header);
        }
        this.el.sessionList.appendChild(this.buildItem(s));
      }

      if (sessions.length === 0) {
        const empty = document.createElement("div");
        empty.className = "session-list-empty";
        empty.textContent = "Сессий пока нет";
        this.el.sessionList.appendChild(empty);
      }

      this.syncSelectionUI();
      this.syncPreviewAfterReload();
    } catch (e) {
      void trackError("sessions.loadList", e);
    }
  }

  private buildItem(s: SessionMeta): HTMLDivElement {
    const div = document.createElement("div");
    const isOpen = store.tabs.some(t => t.type === "chat" && t.sessionId === s.id);
    div.className = `session-item ${isOpen ? "active" : ""}`;
    div.dataset.sid = s.id;
    if (this.previewId === s.id) div.classList.add("previewing");
    if (this.selectedIds.has(s.id)) div.classList.add("selected");
    if (isOpen) div.title = "Открыта во вкладке";

    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.className = "session-item-checkbox";
    cb.checked = this.selectedIds.has(s.id);
    cb.tabIndex = -1;
    cb.addEventListener("click", (e) => {
      e.stopPropagation();
      this.toggleSelected(s.id);
    });

    const main = document.createElement("div");
    main.className = "session-item-main";
    const titleSpan = document.createElement("span");
    titleSpan.className = "session-title";
    titleSpan.title = s.title;
    titleSpan.textContent = s.title;
    const timeSpan = document.createElement("span");
    timeSpan.className = "session-item-time";
    timeSpan.textContent = formatItemTime(s);
    main.appendChild(titleSpan);
    main.appendChild(timeSpan);

    const actions = document.createElement("div");
    actions.className = "session-item-actions";
    actions.innerHTML = `<button class="btn-session-menu">⋮</button><div class="session-menu-dropdown"><button class="session-menu-item btn-rename"><span class="menu-ico">✏️</span>Переименовать</button><button class="session-menu-item btn-copy"><span class="menu-ico">📋</span>Копировать в буфер</button><button class="session-menu-item btn-explore"><span class="menu-ico">📁</span>Проводник</button><button class="session-menu-item danger btn-delete"><span class="menu-ico">🗑️</span>Удалить</button></div>`;
    const renameBtn = actions.querySelector(".btn-rename") as HTMLElement;
    renameBtn.dataset.id = s.id;
    renameBtn.dataset.title = s.title;
    const exploreBtn = actions.querySelector(".btn-explore") as HTMLElement;
    exploreBtn.dataset.id = s.id;
    const copyBtn = actions.querySelector(".btn-copy") as HTMLElement;
    copyBtn.dataset.id = s.id;
    const deleteBtn = actions.querySelector(".btn-delete") as HTMLElement;
    deleteBtn.dataset.id = s.id;

    div.appendChild(cb);
    div.appendChild(main);
    if (isOpen) {
      const badge = document.createElement("span");
      badge.className = "session-open-badge";
      badge.textContent = "открыта";
      div.appendChild(badge);
    }
    div.appendChild(actions);

    div.addEventListener("mouseenter", () => this.schedulePreview(s.id));
    div.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).closest(".session-item-actions")) return;
      if (this.selectMode) {
        this.toggleSelected(s.id);
        return;
      }
      bus.emit("session:open", s.id);
    });

    const menuBtn = div.querySelector(".btn-session-menu");
    const dropdown = div.querySelector(".session-menu-dropdown");
    menuBtn?.addEventListener("click", (e) => {
      e.stopPropagation();
      if (this.selectMode) return;
      document.querySelectorAll(".session-menu-dropdown.show").forEach(dd => {
        if (dd !== dropdown) dd.classList.remove("show");
      });
      dropdown?.classList.toggle("show");
    });
    renameBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      dropdown?.classList.remove("show");
      const cur = renameBtn.dataset.title || "";
      const newT = prompt("Новое название:", cur);
      if (newT && newT.trim() !== "" && newT !== cur) {
        try {
          await renameSession(s.id, newT.trim());
          this.previewCache.delete(s.id);
          this.loadSessionsListUI();
        } catch (err) {
          showToast(`Ошибка: ${err}`, "error");
          void trackError("sessions.rename", err);
        }
      }
    });
    exploreBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      dropdown?.classList.remove("show");
      try {
        await openSessionFolder(s.id);
      } catch (err) {
        void trackError("sessions.openFolder", err);
      }
    });
    copyBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      dropdown?.classList.remove("show");
      try {
        await this.copySessionToClipboard(s.id, s.title);
      } catch (err) {
        showToast(`Ошибка: ${err}`, "error");
        void trackError("sessions.copy", err);
      }
    });
    deleteBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      dropdown?.classList.remove("show");
      await this.deleteSessionUI(s.id);
    });
    return div;
  }

  private schedulePreview(id: string) {
    if (this.previewDebounce !== null) window.clearTimeout(this.previewDebounce);
    this.previewDebounce = window.setTimeout(() => {
      this.previewDebounce = null;
      void this.showPreview(id);
    }, 140);
  }

  private async showPreview(id: string, force = false) {
    if (!force && this.previewId === id && this.el.sessionPreviewBody.dataset.rid === id) return;
    const seq = ++this.previewSeq;
    const meta = this.sessions.find(s => s.id === id);
    this.previewId = id;
    this.markPreviewing(id);

    this.el.sessionPreviewTitle.textContent = meta?.title ?? "Сессия";
    this.el.sessionPreviewDate.textContent = meta ? formatFullDate(meta.updated_at) : "";
    this.el.sessionPreviewOpen.hidden = false;
    this.el.sessionPreviewOpen.dataset.sid = id;
    this.el.sessionPreviewBody.dataset.rid = "";
    this.el.sessionPreviewBody.innerHTML = `<div class="sessions-preview-empty">Загрузка…</div>`;

    try {
      const messages = await this.getPreviewMessages(id);
      if (seq !== this.previewSeq) return;
      this.renderPreviewMessages(id, messages);
    } catch (e) {
      if (seq !== this.previewSeq) return;
      this.el.sessionPreviewBody.innerHTML = `<div class="sessions-preview-empty">Не удалось загрузить сессию</div>`;
      void trackError("sessions.preview", e);
    }
  }

  private async getPreviewMessages(id: string): Promise<ChatMessage[]> {
    const live = getChatControllerForSession(id);
    if (live) return live.state.history.filter(m => m && m.type === "message");
    const cached = this.previewCache.get(id);
    if (cached) return cached;
    const session = await loadSession(id);
    const messages = ((session?.messages ?? []) as ChatMessage[]).filter(m => m && m.type === "message");
    this.previewCache.set(id, messages);
    return messages;
  }

  private renderPreviewMessages(id: string, messages: ChatMessage[]) {
    const body = this.el.sessionPreviewBody;
    body.innerHTML = "";
    body.dataset.rid = id;
    body.scrollTop = 0;

    if (messages.length === 0) {
      const empty = document.createElement("div");
      empty.className = "sessions-preview-empty";
      empty.textContent = "В этой сессии пока нет сообщений";
      body.appendChild(empty);
      return;
    }

    for (const msg of messages) {
      const raw = (msg.content ?? "").trim();
      if (!raw) continue;
      const role = messageRole(msg);
      const bubble = document.createElement("div");
      bubble.className = `preview-message preview-message-${role}`;
      if (role === "agent" && msg.author) {
        const name = document.createElement("span");
        name.className = "preview-agent-name";
        name.textContent = getAgentDisplayName(msg.author) ?? "";
        bubble.appendChild(name);
      }
      const content = document.createElement("div");
      content.className = "preview-msg-content markdown-body";
      const text = role === "agent" ? stripStreamArtifacts(raw) : raw;
      content.innerHTML = renderMarkdown(text);
      bubble.appendChild(content);
      body.appendChild(bubble);
    }
    body.scrollTop = 0;
  }

  private markPreviewing(id: string) {
    this.el.sessionList.querySelectorAll(".session-item.previewing").forEach(el => el.classList.remove("previewing"));
    const item = this.el.sessionList.querySelector(`.session-item[data-sid="${CSS.escape(id)}"]`);
    item?.classList.add("previewing");
  }

  private syncPreviewAfterReload() {
    if (!this.previewId) return;
    const meta = this.sessions.find(s => s.id === this.previewId);
    if (!meta) {
      this.clearPreview();
      return;
    }
    this.markPreviewing(meta.id);
    this.el.sessionPreviewTitle.textContent = meta.title;
    this.el.sessionPreviewDate.textContent = formatFullDate(meta.updated_at);
    this.el.sessionPreviewOpen.dataset.sid = meta.id;
    void this.showPreview(meta.id, true);
  }

  private clearPreview() {
    this.previewId = null;
    this.previewSeq++;
    this.el.sessionPreviewTitle.textContent = "Предпросмотр";
    this.el.sessionPreviewDate.textContent = "";
    this.el.sessionPreviewOpen.hidden = true;
    delete this.el.sessionPreviewOpen.dataset.sid;
    this.el.sessionPreviewBody.dataset.rid = "";
    this.el.sessionPreviewBody.innerHTML = `<div class="sessions-preview-empty">Наведите на сессию слева, чтобы увидеть переписку</div>`;
    this.el.sessionList.querySelectorAll(".session-item.previewing").forEach(el => el.classList.remove("previewing"));
  }

  private async copySessionToClipboard(id: string, title: string) {
    const liveCtl = getChatControllerForSession(id);
    const messages = liveCtl
      ? liveCtl.state.history
      : ((await loadSession(id))?.messages ?? []);
    await copyMarkdownToClipboard(title, messages);
  }

  private async deleteSessionUI(id: string) {
    const yes = await confirmDialog("Удаление", "Удалить сессию?");
    if (!yes) return;
    try {
      await deleteSession(id);
      this.previewCache.delete(id);
      if (this.previewId === id) this.clearPreview();
      bus.emit("session:deleted", id);
      this.loadSessionsListUI();
      showToast("Удалено.", "success");
    } catch (e) {
      showToast(`Ошибка: ${e}`, "error");
      void trackError("sessions.delete", e);
    }
  }

  private bindDomEvents() {
    document.addEventListener("click", (e) => {
      document.querySelectorAll(".msg-menu-dropdown.show, .session-menu-dropdown.show, .tab-menu-dropdown.show").forEach(dd => {
        if (!dd.parentElement?.contains(e.target as Node)) dd.classList.remove("show");
      });
    });
  }

  private bindBusEvents() {
    bus.on("session:changed", () => {
      this.previewCache.clear();
      this.loadSessionsListUI();
    });
    bus.on("session:deleted", () => {
      this.previewCache.clear();
      this.loadSessionsListUI();
    });
  }
}
