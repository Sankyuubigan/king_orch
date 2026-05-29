import { store } from "../store";
import { bus } from "../events";
import { showToast, confirmDialog } from "../ui";
import { fetchSessions, deleteSession, renameSession, openSessionFolder } from "../services";

export interface SessionElements {
  sessionList: HTMLDivElement;
  btnNewSession: HTMLButtonElement;
}

export class SessionController {
  private el: SessionElements;

  constructor(el: SessionElements) {
    this.el = el;
    this.bindDomEvents();
    this.bindBusEvents();
  }

  async loadSessionsListUI() {
    try {
      const sessions = await fetchSessions();
      this.el.sessionList.innerHTML = "";
      for (const s of sessions) {
        const div = document.createElement("div");
        div.className = `session-item ${s.id === store.currentSessionId ? 'active' : ''}`;
        div.innerHTML = `<span class="session-title" title="${s.title}">${s.title}</span><div class="session-item-actions"><button class="btn-session-menu">⋮</button><div class="session-menu-dropdown"><button class="session-menu-item btn-rename" data-id="${s.id}" data-title="${s.title}">✏️ Переименовать</button><button class="session-menu-item btn-explore" data-id="${s.id}">📁 Проводник</button><button class="session-menu-item danger btn-delete" data-id="${s.id}">🗑️ Удалить</button></div></div>`;
        div.addEventListener("click", (e) => { if (!(e.target as HTMLElement).closest('.session-item-actions')) bus.emit("session:open", s.id); });
        const menuBtn = div.querySelector('.btn-session-menu');
        const dropdown = div.querySelector('.session-menu-dropdown');
        menuBtn?.addEventListener("click", (e) => { e.stopPropagation(); document.querySelectorAll('.session-menu-dropdown.show').forEach(dd => { if (dd !== dropdown) dd.classList.remove('show'); }); dropdown?.classList.toggle('show'); });
        div.querySelector('.btn-rename')?.addEventListener("click", async (e) => { e.stopPropagation(); dropdown?.classList.remove('show'); const cur = (e.target as HTMLElement).getAttribute('data-title') || ''; const newT = prompt("Новое название:", cur); if (newT && newT.trim() !== "" && newT !== cur) { try { await renameSession(s.id, newT.trim()); this.loadSessionsListUI(); } catch(err) { showToast(`Ошибка: ${err}`, "error"); } } });
        div.querySelector('.btn-explore')?.addEventListener("click", async (e) => { e.stopPropagation(); dropdown?.classList.remove('show'); try { await openSessionFolder(s.id); } catch(err) {} });
        div.querySelector('.btn-delete')?.addEventListener("click", async (e) => { e.stopPropagation(); dropdown?.classList.remove('show'); await this.deleteSessionUI(s.id); });
        this.el.sessionList.appendChild(div);
      }
    } catch(e) {}
  }

  private async deleteSessionUI(id: string) {
    const yes = await confirmDialog("Удаление", "Удалить сессию?");
    if (!yes) return;
    try {
      await deleteSession(id);
      if (store.currentSessionId === id) bus.emit("session:new");
      else this.loadSessionsListUI();
      showToast("Удалено.", "success");
    } catch(e) { showToast(`Ошибка: ${e}`, "error"); }
  }

  private bindDomEvents() {
    this.el.btnNewSession?.addEventListener("click", () => bus.emit("session:new"));
    document.addEventListener("click", (e) => { document.querySelectorAll('.msg-menu-dropdown.show, .session-menu-dropdown.show').forEach(dd => { if (!dd.parentElement?.contains(e.target as Node)) dd.classList.remove('show'); }); });
  }

  private bindBusEvents() {
    bus.on("session:changed", () => this.loadSessionsListUI());
    bus.on("config:loaded", () => this.loadSessionsListUI());
    bus.on("processing:changed", (isProcessing: boolean) => { this.el.btnNewSession.disabled = isProcessing; });
  }
}