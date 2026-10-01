import { invoke } from "@tauri-apps/api/core";
import { store } from "../store";
import { bus } from "../events";
import { showToast } from "../ui";
import { trackError } from "../telemetry";
import type { AgentEntry } from "../types";

export interface AgentPickerElements {
  root: HTMLElement;
  selectedTitle: HTMLElement;
  selectedList: HTMLUListElement;
  selectedEmpty: HTMLElement;
  clearBtn: HTMLButtonElement;
  search: HTMLInputElement;
  addGraphsBtn: HTMLButtonElement;
  addAgentsBtn: HTMLButtonElement;
  resetBtn: HTMLButtonElement;
  catalogGroups: HTMLElement;
  catalogEmpty: HTMLElement;
}

/// Единственный id, который показывается новому пользователю.
const DEFAULT_VISIBILITY = ["basic"];

/**
 * Панель «Агенты в чате» в Настройках: две колонки.
 *
 * Слева — что сейчас показывается в выпадающем списке агентов, в порядке
 * показа. Справа — весь каталог `agents/` без фильтров, разбитый на «графовые
 * команды» и «отдельные агенты». Правка любой ячейки сразу пишется в конфиг
 * (`agent_visibility`) и перерисовывает селекты во всех чат-вкладках.
 */
export class AgentPickerController {
  private el: AgentPickerElements;
  private searchQuery = "";

  constructor(el: AgentPickerElements) {
    this.el = el;
    this.bindEvents();
    this.render();
    bus.on("model-catalog-changed", () => this.render());
  }

  private bindEvents(): void {
    this.el.clearBtn.addEventListener("click", () => void this.setVisibility([]));
    this.el.resetBtn.addEventListener("click", () => void this.setVisibility([...DEFAULT_VISIBILITY]));
    this.el.addGraphsBtn.addEventListener("click", () => void this.addAll("workflow"));
    this.el.addAgentsBtn.addEventListener("click", () => void this.addAll("agent"));
    this.el.search.addEventListener("input", () => {
      this.searchQuery = this.el.search.value.trim().toLowerCase();
      this.renderCatalog();
    });
  }

  // ─── Мутации выбора ───

  private async setVisibility(next: string[]): Promise<void> {
    const deduped = Array.from(new Set(next));
    store.agentVisibility = deduped;
    await this.persist();
  }

  private async add(id: string): Promise<void> {
    if (store.agentVisibility.includes(id)) return;
    await this.setVisibility([...store.agentVisibility, id]);
  }

  private async remove(id: string): Promise<void> {
    await this.setVisibility(store.agentVisibility.filter(x => x !== id));
  }

  private async move(id: string, delta: -1 | 1): Promise<void> {
    const list = [...store.agentVisibility];
    const index = list.indexOf(id);
    const target = index + delta;
    if (index < 0 || target < 0 || target >= list.length) return;
    [list[index], list[target]] = [list[target], list[index]];
    await this.setVisibility(list);
  }

  private async addAll(entryType: AgentEntry["entry_type"]): Promise<void> {
    const missing = store.agents
      .filter(a => a.entry_type === entryType && !store.agentVisibility.includes(a.id))
      .map(a => a.id);
    if (!missing.length) {
      showToast("Все уже добавлены", "info");
      return;
    }
    await this.setVisibility([...store.agentVisibility, ...missing]);
  }

  /** Единственная точка записи: конфиг → шина → перерисовка всех селектов. */
  private async persist(): Promise<void> {
    try {
      await invoke("set_config_value", { key: "agent_visibility", value: store.agentVisibility });
      bus.emit("model-catalog-changed");
    } catch (e) {
      // Откатываем Store, если конфиг не записался — иначе UI показал бы
      // изменение, которого на самом деле нет (§2.2 core/rules).
      showToast(`Не удалось сохранить выбор агентов: ${e}`, "error");
      void trackError("agentPicker.persist", e);
      await this.reloadFromConfig();
    }
  }

  private async reloadFromConfig(): Promise<void> {
    try {
      const config = await invoke<{ agent_visibility?: string[] }>("get_config");
      store.agentVisibility = Array.isArray(config?.agent_visibility)
        ? config.agent_visibility
        : [...DEFAULT_VISIBILITY];
    } catch (e) {
      store.agentVisibility = [...DEFAULT_VISIBILITY];
      void trackError("agentPicker.reload", e);
    }
    this.render();
  }

  // ─── Отрисовка ───

  render(): void {
    this.renderSelected();
    this.renderCatalog();
  }

  private renderSelected(): void {
    const list = this.el.selectedList;
    list.replaceChildren();
    const entries = store.agentVisibility
      .map(id => store.agents.find(a => a.id === id))
      .filter((a): a is AgentEntry => Boolean(a));

    this.el.selectedTitle.textContent = `В списке чата (${store.agentVisibility.length})`;
    this.el.selectedEmpty.hidden = entries.length > 0;

    entries.forEach((entry, index) => {
      const item = document.createElement("li");
      item.className = "agent-picker-item";
      item.dataset.id = entry.id;

      const icon = document.createElement("span");
      icon.className = "agent-picker-icon";
      icon.textContent = entry.entry_type === "workflow" ? "📁" : "📊";

      const body = document.createElement("div");
      body.className = "agent-picker-item-body";
      const name = document.createElement("div");
      name.className = "agent-picker-item-name";
      name.textContent = entry.name;
      const meta = document.createElement("div");
      meta.className = "agent-picker-item-meta";
      meta.textContent = `${entry.id} · ${entry.role === "graph" ? "узел графа" : "отдельный агент"}`;
      body.append(name, meta);

      const actions = document.createElement("div");
      actions.className = "agent-picker-item-actions";
      actions.append(
        this.iconButton("↑", "Поднять выше в списке", () => void this.move(entry.id, -1), index === 0),
        this.iconButton(
          "↓",
          "Опустить ниже в списке",
          () => void this.move(entry.id, 1),
          index === entries.length - 1,
        ),
        this.iconButton("✕", "Убрать из списка", () => void this.remove(entry.id)),
      );

      item.append(icon, body, actions);
      list.appendChild(item);
    });
  }

  private iconButton(
    label: string,
    title: string,
    onClick: () => void,
    disabled = false,
  ): HTMLButtonElement {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "btn-icon agent-picker-btn";
    btn.textContent = label;
    btn.title = title;
    btn.disabled = disabled;
    btn.addEventListener("click", onClick);
    return btn;
  }

  private renderCatalog(): void {
    const container = this.el.catalogGroups;
    container.replaceChildren();

    const graphs = store.agents.filter(a => a.entry_type === "workflow");
    const agents = store.agents.filter(a => a.entry_type === "agent");
    const matchedGraphs = this.filterEntries(graphs);
    const matchedAgents = this.filterEntries(agents);
    this.el.catalogEmpty.hidden = matchedGraphs.length + matchedAgents.length > 0;

    if (graphs.length) {
      container.appendChild(this.buildGroup("Графовые команды", matchedGraphs, "📁"));
    }
    if (agents.length) {
      container.appendChild(this.buildGroup("Отдельные агенты", matchedAgents, "📊"));
    }
  }

  private filterEntries(entries: AgentEntry[]): AgentEntry[] {
    if (!this.searchQuery) return entries;
    const q = this.searchQuery;
    return entries.filter(a =>
      a.name.toLowerCase().includes(q) ||
      a.id.toLowerCase().includes(q) ||
      a.description.toLowerCase().includes(q) ||
      (a.folder ?? "").toLowerCase().includes(q),
    );
  }

  private buildGroup(title: string, entries: AgentEntry[], icon: string): HTMLElement {
    const details = document.createElement("details");
    details.className = "agent-picker-group";
    details.open = true;

    const summary = document.createElement("summary");
    summary.className = "agent-picker-group-title";
    summary.textContent = `${icon} ${title} (${entries.length})`;

    const list = document.createElement("ul");
    list.className = "agent-picker-list agent-picker-list-catalog";
    for (const entry of entries) {
      list.appendChild(this.buildCatalogItem(entry));
    }

    if (!entries.length) {
      const empty = document.createElement("li");
      empty.className = "agent-picker-empty";
      empty.textContent = "Нет совпадений";
      list.appendChild(empty);
    }

    details.append(summary, list);
    return details;
  }

  private buildCatalogItem(entry: AgentEntry): HTMLElement {
    const selected = store.agentVisibility.includes(entry.id);
    const item = document.createElement("li");
    item.className = "agent-picker-item";
    item.dataset.id = entry.id;
    if (selected) item.classList.add("is-selected");

    const icon = document.createElement("span");
    icon.className = "agent-picker-icon";
    icon.textContent = entry.entry_type === "workflow" ? "📁" : "📊";

    const body = document.createElement("div");
    body.className = "agent-picker-item-body";
    const name = document.createElement("div");
    name.className = "agent-picker-item-name";
    name.textContent = entry.name;
    const meta = document.createElement("div");
    meta.className = "agent-picker-item-meta";
    meta.textContent = entry.id;
    body.append(name, meta);
    if (entry.description) {
      const desc = document.createElement("div");
      desc.className = "agent-picker-item-desc";
      desc.textContent = entry.description;
      body.appendChild(desc);
    }

    const actions = document.createElement("div");
    actions.className = "agent-picker-item-actions";
    if (selected) {
      const badge = document.createElement("span");
      badge.className = "agent-picker-badge";
      badge.textContent = "✓ в списке";
      actions.appendChild(badge);
    } else {
      actions.appendChild(this.iconButton("+", "Добавить в список чата", () => void this.add(entry.id)));
    }

    item.append(icon, body, actions);
    return item;
  }
}
