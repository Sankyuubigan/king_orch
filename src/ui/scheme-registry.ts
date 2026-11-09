/**
 * scheme-registry — точка склейки generic-механики чата и рендеров агентов.
 *
 * Единственное место, где имена фенс-типов встречаются в коде приложения:
 * каждый агент привозит свой renderer из `agents/<агент>/views/*.ts`, реестр
 * связывает fenceType → SchemeRenderer и наполняет generic-список извлечения
 * (`src/utils/custom-fences.ts`). UI-слои (scheme-card, scheme-viewer, chat,
 * markdown) имён агентов не знают — берут renderer по data-fence-type.
 *
 * Lazy-инициализация (`ensureSchemesRegistered`) вызывается из
 * `materializeSchemeCards` и `schemeRendererFor`: регистрация гарантированно
 * успевает до первого рендера карточки; идемпотентна.
 */

import type { SchemeRenderer } from "../utils";
import { registerCustomJsonFence } from "../utils";
import {
  SCHEME_FENCE_TYPE,
  neurosisCaseRenderer,
} from "../../agents/psychotherapist/views/neurosis-case";

const renderers = new Map<string, SchemeRenderer>();

/** Реестр честен: пустой fenceType — форма контракта нарушена, шум проспал бы
 * блоки без renderer'а (§2.2). */
export function registerScheme(fenceType: string, renderer: SchemeRenderer): void {
  if (!fenceType) throw new Error("scheme-registry: fenceType обязателен");
  renderers.set(fenceType, renderer);
  registerCustomJsonFence(fenceType);
}

let ensured = false;

/** Регистрация всех привезённых схем агентов. */
export function ensureSchemesRegistered(): void {
  if (ensured) return;
  ensured = true;
  registerScheme(SCHEME_FENCE_TYPE, neurosisCaseRenderer);
}

/** Renderer по fence-типу чата (data-fence-type pre.scheme-fence). */
export function schemeRendererFor(fenceType: string): SchemeRenderer | undefined {
  ensureSchemesRegistered();
  return renderers.get(fenceType);
}

/**
 * Точка инициализации из main.ts (паттерн initXxx): регистрация фенс-типов
 * ОБЯЗАНА пройти до первого renderMarkdown (до создания контроллеров чата).
 * renderMarkdown извлекает фенс только по зарегистрированным типам — если
 * реестр пуст в момент рендера, pre.scheme-fence вообще не появляется в HTML
 * (регресс 09.11.26: материалайзер не находил блоков, схема не отображалась).
 */
export function initSchemeRegistry(): void {
  ensureSchemesRegistered();
}
