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
  "gateway": "gateway:",
};

export function isCloudModelPath(path?: string | null): boolean {
  if (!path) return false;
  return Object.values(CLOUD_ROUTER_PREFIXES).some(prefix => path.startsWith(prefix));
}

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
/// из общих каталогов Store. ЕДИНСТВЕННЫЙ владелец выбора: захватывает его
/// ДО уничтожения опций и восстанавливает после сборки списка — перерисовка
/// каталога (model-catalog-changed) не подменяет выбранное юзером.
export function fillModelSelect(select: HTMLSelectElement | null) {
  if (!select) return;
  // Захват ДО innerHTML="": позже select.value — уже первый опшен,
  // выбор юзера уничтожен. Регресс 09.11.26: current читался после
  // пересоздания опций, и явный выбор молча подменялся первой локальной
  // моделью (репорт: обновил модели провайдера в дашборде шлюза —
  // выбранные в чате модели поменялись сами).
  const current = select.value;
  const explicit = select.dataset.modelExplicit === "true";
  select.innerHTML = "";
  for (const m of store.models) {
    const o = document.createElement("option");
    o.value = m;
    const fileName = m.split(/[/\\]/).pop() || m;
    const badges = m ? capabilityIcons(store.capabilities[m]) : "";
    o.text = fileName + (badges ? `  ${badges}` : "");
    o.title = fileName;
    select.appendChild(o);
  }
  renderCloudRouterOptions(select);
  // Восстановление — только здесь (SSOT): явный выбор юзера важнее lastModel.
  // applyModelValue валиден по наличию опции: модель, исчезнувшая из каталога
  // (например, после refresh моделей провайдера), честно падает на первый пункт.
  if (explicit && current) applyModelValue(select, current);
  else if (store.lastModel) applyModelValue(select, store.lastModel);
  syncModelSelectTitle(select);
}

/// Хинт селекта = полное имя выбранной модели (title выбранной опции);
/// пока ничего не выбрано — нейтральный «Выбор модели».
export function syncModelSelectTitle(select: HTMLSelectElement | null) {
  if (!select) return;
  const selected = Array.from(select.options).find(o => o.value === select.value);
  select.title = selected?.title || "Выбор модели";
}

/// Рисует optgroup «Облачные роутеры» поверх локальных моделей. Чистая
/// дорисовка списка: select.value не трогает — политика выбора живёт только
/// в fillModelSelect (SSOT). Прежнее восстановление здесь читало current
/// уже ПОСЛЕ пересоздания опций fillModelSelect и подменяло выбор первой
/// локальной моделью (баг потери выбора 09.11.26) — потому изъято.
export function renderCloudRouterOptions(select: HTMLSelectElement | null) {
  if (!select) return;
  select.querySelector('optgroup[label="Облачные роутеры"]')?.remove();
  
  const allCombos = Object.entries(store.cloudRouterCombos).flatMap(([router, combos]) =>
    combos.map(c => ({ ...c, router }))
  );
  if (!allCombos.length) {
    // Группа уже удалена выше — выбор мог молча упасть на локальную модель.
    syncModelSelectTitle(select);
    return;
  }
  
  const group = document.createElement("optgroup");
  group.label = "Облачные роутеры";
  for (const c of allCombos) {
    const o = document.createElement("option");
    o.value = `${CLOUD_ROUTER_PREFIXES[c.router]}${c.name}`;
    o.text = `☁️ [${c.router}] ${c.name}`;
    o.title = c.name;
    group.appendChild(o);
  }
  select.appendChild(group);
  syncModelSelectTitle(select);
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
    // Показывать нечего: нет агента — нет и его пути (иначе title давно
    // выбранного другого остался бы висеть поверх пустого состояния).
    select.title = "";
    return;
  }

  for (const e of entries) {
    const o = document.createElement("option");
    o.value = e.id;
    o.text = agentOptionLabel(e);
    // Подсказка «где физически лежит файл агента»: рисует ФАКТИЧЕСКИЙ путь,
    // а не сваренный из иконки отображаемый заголовок пункта. `source_path`
    // приходит от бэкенда (он знает agents_dir); в ручных литералах/старых
    // моках поля нет — тогда показываем относительный, но не молчим.
    o.title = e.source_path || e.rel_path;
    select.appendChild(o);
  }
  if (prev && Array.from(select.options).some(o => o.value === prev)) select.value = prev;
  else if (store.lastAgent && Array.from(select.options).some(o => o.value === store.lastAgent)) select.value = store.lastAgent;
  else if (prev) {
    // Выбранный агент исчез из списка (пользователь убрал его в настройках).
    console.warn(`[fillAgentSelect] Агент «${prev}» больше не выбран — переключение на первый доступный`);
  }
  // Тултип на ЗАКРЫТОМ селекте — путь выбранного агента (наведение на свёрнутый
  // список тоже показывает, какой файл редактируешь; option.title исключает
  // лишь сам момент, когда список раскрыт).
  const selected = Array.from(select.options).find(o => o.value === select.value);
  select.title = selected?.value ? (selected.title || "") : "";
}