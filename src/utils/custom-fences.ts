/**
 * custom-fences — generic реестр custom JSON-фенсов чата (агент-agnostic).
 *
 * UI-механизм карточек/просмотров схем (src/ui/scheme-card.ts,
 * src/ui/scheme-viewer.ts) и извлечение блоков (src/utils/markdown.ts) НЕ знают
 * имён агентов: конкретные fence-типы (имя фенс-типа конкретного агента)
 * регистрирует точка склейки src/ui/scheme-registry.ts,
 * наполнение — из модулей агентов (agents/<агент>/views/*.ts). Markdown
 * одинаково извлекает ЛЮБОЙ зарегистрированный тип, не зная, что внутри.
 *
 * Сюда же вынесен контракт SchemeRenderer: единственная форма рендера схемы
 * агента (PRP 09.11.26, раздел 3). Контракт — деталь utils-слоя, чтобы модуль
 * агента не импортировал ничего из ui-слоя.
 */

export type SchemeMode = "svg" | "scheme";

/** Контракт рендера одной схемы агента (форма, а не механика DOM). */
export interface SchemeRenderer {
  /** Разбор сырого текста фенса; null → generic карточка-ошибка (честный отказ, §2.2). */
  parse(raw: string): unknown | null;
  /** Строка SVG для инлайнового превью и модалки (экранирование недоверенных
   * текстов — ответственность билдера агента). */
  buildSvg(parsed: unknown): string;
  /** Опц: mermaid-исходник альтернативного режима (тумблер «Схема»/«SVG»). */
  buildMermaid?(parsed: unknown): string;
  /** Подпись модалки (у кейса — target). */
  caption(parsed: unknown): string;
}

const fenceTypes = new Set<string>();

/** Регистрация fence-типа для generic-извлечения в markdown.ts (идемпотентно). */
export function registerCustomJsonFence(type: string): void {
  if (type) fenceTypes.add(type);
}

export function customJsonFenceTypes(): string[] {
  return Array.from(fenceTypes);
}
