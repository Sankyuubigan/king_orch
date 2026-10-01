import { store } from "../store";
import { agentOptionLabel, visibleAgentEntries } from "./agent-visibility";

/// Маппит сырой id/author агента в отображаемое имя из каталога (frontmatter `name`).
/// Если агент не найден — возвращает входную строку как есть (совместимость).
export function getAgentDisplayName(authorOrId?: string | null): string | undefined {
  if (!authorOrId) return undefined;
  const agent = store.agents.find(a => a.id === authorOrId);
  return agent?.name || authorOrId;
}

/// Префиксы значений комбо облачных роутеров в списке моделей.
export const CLOUD_ROUTER_PREFIXES: Record<string, string> = {
  "9router": "9router:",
  "extremerouter": "extremerouter:",
  "omniroute": "omniroute:",
};

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

/// Заполняет select моделей (локальные .gguf + optgroup «Облачные роутеры»)
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
  renderCloudRouterOptions(select);
  if (select.dataset.modelExplicit !== "true" && store.lastModel) {
    applyModelValue(select, store.lastModel);
  }
}

/// Рисует optgroup «Облачные роутеры» поверх локальных моделей.
export function renderCloudRouterOptions(select: HTMLSelectElement | null) {
  if (!select) return;
  const explicit = select.dataset.modelExplicit === "true";
  const current = select.value;
  select.querySelector('optgroup[label="Облачные роутеры"]')?.remove();
  
  const allCombos = Object.entries(store.cloudRouterCombos).flatMap(([router, combos]) =>
    combos.map(c => ({ ...c, router }))
  );
  if (!allCombos.length) return;
  
  const group = document.createElement("optgroup");
  group.label = "Облачные роутеры";
  for (const c of allCombos) {
    const o = document.createElement("option");
    o.value = `${CLOUD_ROUTER_PREFIXES[c.router]}${c.name}`;
    o.text = `☁️ [${c.router}] ${c.name}`;
    group.appendChild(o);
  }
  select.appendChild(group);
  if (explicit) applyModelValue(select, current);
  else if (store.lastModel) applyModelValue(select, store.lastModel);
}

/// Заполняет select агентов по выбору пользователя (`store.agentVisibility`).
/// Порядок пунктов = порядок в конфиге, поэтому НЕ сортируем (SSOT).
export function fillAgentSelect(select: HTMLSelectElement | null) {
  if (!select) return;
  const prev = select.value;
  select.innerHTML = "";

  const entries = visibleAgentEntries();
  if (entries.length === 0) {
    // Честное пустое состояние вместо молчаливо неработающего select.
    const hint = document.createElement("option");
    hint.value = "";
    hint.textContent = "⚠️ Агенты не выбраны — настройте в Настройках";
    hint.disabled = true;
    select.appendChild(hint);
    select.value = "";
    return;
  }

  for (const e of entries) {
    const o = document.createElement("option");
    o.value = e.id;
    o.text = agentOptionLabel(e);
    select.appendChild(o);
  }
  if (prev && Array.from(select.options).some(o => o.value === prev)) select.value = prev;
  else if (store.lastAgent && Array.from(select.options).some(o => o.value === store.lastAgent)) select.value = store.lastAgent;
  else if (prev) {
    // Выбранный агент исчез из списка (пользователь убрал его в настройках).
    console.warn(`[fillAgentSelect] Агент «${prev}» больше не выбран — переключение на первый доступный`);
  }
}