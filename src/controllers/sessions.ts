import { store } from "../store";
import { bus } from "../events";
import { showToast, confirmDialog } from "../ui";
import { fetchSessions, deleteSession, renameSession, openSessionFolder, loadSession } from "../services";
import { copyMarkdownToClipboard } from "../utils";
import { trackError } from "../telemetry";
import { getChatControllerForSession } from "./tabs";

export interface SessionElements {
  sessionList: HTMLDivElement;
}

/// Раздел «История сессий» (кнопка на экране новой вкладки). Клик по сессии
/// открывает её в чат-вкладке (bus "session:open" → TabController).
export class SessionController {
  private el: SessionElements;

  constructor(el: SessionElements) {
    this.el = el;
    this.bindDomEvents();
    this.bindBusEvents();
    this.loadSessionsListUI();
  }

  async loadSessionsListUI() {
    try {
      const sessions = await fetchSessions();
      this.el.sessionList.innerHTML = "";
      for (const s of sessions) {
        const div = document.createElement("div");
        const isOpen = store.tabs.some(t => t.type === "chat" && t.sessionId === s.id);
        div.className = `session-item ${isOpen ? 'active' : ''}`;
        if (isOpen) div.title = "Открыта во вкладке";
        const titleSpan = document.createElement("span");
        titleSpan.className = "session-title";
        titleSpan.title = s.title;
        titleSpan.textContent = s.title;
        const actions = document.createElement("div");
        actions.className = "session-item-actions";
        actions.innerHTML = `<button class="btn-session-menu">⋮</button><div class="session-menu-dropdown"><button class="session-menu-item btn-rename"><span class="menu-ico">✏️</span>Переименовать</button><button class="session-menu-item btn-copy"><span class="menu-ico">📋</span>Копировать в буфер</button><button class="session-menu-item btn-explore"><span class="menu-ico">📁</span>Проводник</button><button class="session-menu-item danger btn-delete"><span class="menu-ico">🗑️</span>Удалить</button></div>`;
        const renameBtn = actions.querySelector('.btn-rename') as HTMLElement;
        renameBtn.dataset.id = s.id;
        renameBtn.dataset.title = s.title;
        const exploreBtn = actions.querySelector('.btn-explore') as HTMLElement;
        exploreBtn.dataset.id = s.id;
        const copyBtn = actions.querySelector('.btn-copy') as HTMLElement;
        copyBtn.dataset.id = s.id;
        const deleteBtn = actions.querySelector('.btn-delete') as HTMLElement;
        deleteBtn.dataset.id = s.id;
        div.appendChild(titleSpan);
        if (isOpen) {
          const badge = document.createElement("span");
          badge.className = "session-open-badge";
          badge.textContent = "открыта";
          div.appendChild(badge);
        }
        div.appendChild(actions);
        div.addEventListener("click", (e) => { if (!(e.target as HTMLElement).closest('.session-item-actions')) bus.emit("session:open", s.id); });
        const menuBtn = div.querySelector('.btn-session-menu');
        const dropdown = div.querySelector('.session-menu-dropdown');
        menuBtn?.addEventListener("click", (e) => { e.stopPropagation(); document.querySelectorAll('.session-menu-dropdown.show').forEach(dd => { if (dd !== dropdown) dd.classList.remove('show'); }); dropdown?.classList.toggle('show'); });
        renameBtn.addEventListener("click", async (e) => { e.stopPropagation(); dropdown?.classList.remove('show'); const cur = renameBtn.dataset.title || ''; const newT = prompt("Новое название:", cur); if (newT && newT.trim() !== "" && newT !== cur) { try { await renameSession(s.id, newT.trim()); this.loadSessionsListUI(); } catch(err) { showToast(`Ошибка: ${err}`, "error"); void trackError("sessions.rename", err); } } });
        exploreBtn.addEventListener("click", async (e) => { e.stopPropagation(); dropdown?.classList.remove('show'); try { await openSessionFolder(s.id); } catch(err) { void trackError("sessions.openFolder", err); } });
        copyBtn.addEventListener("click", async (e) => { e.stopPropagation(); dropdown?.classList.remove('show'); try { await this.copySessionToClipboard(s.id, s.title); } catch(err) { showToast(`Ошибка: ${err}`, "error"); void trackError("sessions.copy", err); } });
        deleteBtn.addEventListener("click", async (e) => { e.stopPropagation(); dropdown?.classList.remove('show'); await this.deleteSessionUI(s.id); });
        this.el.sessionList.appendChild(div);
      }
    } catch(e) {
      void trackError("sessions.loadList", e);
    }
  }

  private async copySessionToClipboard(id: string, title: string) {
    // Открытая вкладка: копируем ЖИВОЙ список сообщений (без перезагрузки).
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
      // TabController закроет чат-вкладку этой сессии (если открыта).
      bus.emit("session:deleted", id);
      this.loadSessionsListUI();
      showToast("Удалено.", "success");
    } catch(e) { showToast(`Ошибка: ${e}`, "error"); void trackError("sessions.delete", e); }
  }

  private bindDomEvents() {
    document.addEventListener("click", (e) => { document.querySelectorAll('.msg-menu-dropdown.show, .session-menu-dropdown.show, .tab-menu-dropdown.show').forEach(dd => { if (!dd.parentElement?.contains(e.target as Node)) dd.classList.remove('show'); }); });
  }

  private bindBusEvents() {
    bus.on("session:changed", () => this.loadSessionsListUI());
    bus.on("session:deleted", () => this.loadSessionsListUI());
    bus.on("config:loaded", () => this.loadSessionsListUI());
    bus.on("tabs:changed", () => this.loadSessionsListUI());
  }
}