import { store } from "../store";

/// Маппит сырой id/author агента в отображаемое имя из каталога (frontmatter `name`).
/// Если агент не найден — возвращает входную строку как есть (совместимость).
export function getAgentDisplayName(authorOrId?: string | null): string | undefined {
  if (!authorOrId) return undefined;
  const agent = store.agents.find(a => a.id === authorOrId);
  return agent?.name || authorOrId;
}

/// Префикс значения комбо 9Router в списке моделей.
/// Выбор с этим префиксом идёт в облачный шлюз 9Router (плагин
/// tauri-plugin-9router), а НЕ в локальный llama-server.
export const NINE_ROUTER_MODEL_PREFIX = "9router:";

function capabilityIcons(meta?: { uncen?: boolean; vision?: boolean; audio?: boolean }): string {
  const out: string[] = [];
  if (meta?.uncen) out.push("😈");
  if (meta?.vision) out.push("👁️");
  if (meta?.audio) out.push("🎵");
  return out.join(" ");
}

function applyModelValue(select: HTMLSelectElement, value: string): boolean {
  if (!Array.from(select.options).some(option => option.value === value)) return false;
  select.value = value;
  return true;
}

/// Заполняет select моделей (локальные .gguf + optgroup «9Router (облако)»)
/// из общих каталогов Store. Сохраняет текущий выбор, если он ещё валиден.
export function fillModelSelect(select: HTMLSelectElement | null) {
  if (!select) return;
  select.innerHTML = "";
  for (const m of store.models) {
    const o = document.createElement("option");
    o.value = m;
    const fileName = m.split(/[/\\]/).pop() || m;
    const badges = m ? capabilityIcons(store.capabilities[m]) : "";
    o.text = fileName + (badges ? `  ${badges}` : "");
    select.appendChild(o);
  }
  renderNineRouterOptions(select);
  if (select.dataset.modelExplicit !== "true" && store.lastModel) {
    applyModelValue(select, store.lastModel);
  }
}

/// Рисует optgroup «9Router (облако)» поверх локальных моделей.
/// Сохраняет текущий выбор (может быть облачным комбо).
export function renderNineRouterOptions(select: HTMLSelectElement | null) {
  if (!select) return;
  const explicit = select.dataset.modelExplicit === "true";
  const current = select.value;
  select.querySelector('optgroup[label="9Router (облако)"]')?.remove();
  if (!store.nineRouterCombos.length) return;
  const group = document.createElement("optgroup");
  group.label = "9Router (облако)";
  for (const c of store.nineRouterCombos) {
    const o = document.createElement("option");
    o.value = `${NINE_ROUTER_MODEL_PREFIX}${c.name}`;
    o.text = `☁️ ${c.name}`;
    group.appendChild(o);
  }
  select.appendChild(group);
  if (explicit) applyModelValue(select, current);
  else if (store.lastModel) applyModelValue(select, store.lastModel);
}

/// Заполняет select агентов из каталога Store (уважает showFolderAgents).
export function fillAgentSelect(select: HTMLSelectElement | null) {
  if (!select) return;
  const prev = select.value;
  select.innerHTML = "";
  for (const e of store.agents) {
    if (!e.is_hidden && (e.folder === null || store.showFolderAgents)) {
      const o = document.createElement("option");
      o.value = e.id;
      const prefix = e.entry_type === 'workflow' ? '📁' : '📊';
      const folderPart = e.folder ? `${e.folder} - ` : '';
      o.text = `${prefix} ${folderPart}${e.name} (${e.id})`;
      select.appendChild(o);
    }
  }
  if (prev && Array.from(select.options).some(o => o.value === prev)) select.value = prev;
  else if (store.lastAgent && Array.from(select.options).some(o => o.value === store.lastAgent)) select.value = store.lastAgent;
}