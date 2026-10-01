import type { AgentEntry } from "../types";
import { store } from "../store";

/**
 * ЕДИНСТВЕННЫЙ источник правды о том, какие entry points показывать.
 *
 * Раньше предикат жил продублированно в двух местах
 * (`model-options.ts` и `controllers/agent-test.ts`) и опирался на два
 * независимых сигнала — `visible` во frontmatter и флаг `show_folder_agents`
 * в настройках. Теперь сигнал один: `store.agentVisibility`.
 */

/** Запись каталога по id, если она ещё существует на диске. */
export function findAgentEntry(id: string): AgentEntry | undefined {
  return store.agents.find(a => a.id === id);
}

/**
 * Entry points, которые пользователь выбрал для показа, в его порядке.
 * Записи, которых больше нет в каталоге (агент удалён), отбрасываются —
 * но факт расхождения логируется, чтобы «молча пропавший» агент не был ложью.
 */
export function visibleAgentEntries(): AgentEntry[] {
  const out: AgentEntry[] = [];
  const missing: string[] = [];
  for (const id of store.agentVisibility) {
    const entry = findAgentEntry(id);
    if (entry) out.push(entry);
    else missing.push(id);
  }
  // Пока каталог не загружен, «нет в каталоге» — это норма, а не расхождение:
  // селекты перерисовываются повторно после `get_agents` и предупреждать
  // на каждой вкладке при старте было бы шумом, скрывающим настоящие баги.
  if (missing.length && store.agentsReady) {
    console.warn(
      `[agent-visibility] В конфиге есть id, которых нет в каталоге (удалены?): ${missing.join(", ")}`
    );
  }
  return out;
}

/** Текст пункта выпадающего списка: `📁 Команда — Название графа (id)`. */
export function agentOptionLabel(entry: AgentEntry): string {
  const icon = entry.entry_type === "workflow" ? "📁" : "📊";
  const team = entry.folder ? `${entry.folder} — ` : "";
  return `${icon} ${team}${entry.name} (${entry.id})`;
}

/** Показывать ли запись каталога в UI. Единственный предикат видимости. */
export function isAgentVisible(id: string): boolean {
  return store.agentVisibility.includes(id);
}

/**
 * Запускается ли агент как workflow-граф. Сверяется с каталогом, а не с
 * префиксом иконки в тексте `<option>` — разбор отображаемой строки был
 * хрупкой привязкой к форматированию UI.
 */
export function isGraphAgent(id: string | null | undefined): boolean {
  if (!id) return false;
  return findAgentEntry(id)?.entry_type === "workflow";
}

/**
 * Отображаемое имя выбранного агента для заголовка ответа.
 * Берётся из каталога (frontmatter `name`), а не вырезается из текста
 * `<option>` регуляркой по иконке.
 */
export function getSelectedAgentName(select: HTMLSelectElement | null): string {
  const id = select?.value;
  if (!id) return "";
  return findAgentEntry(id)?.name || id;
}
