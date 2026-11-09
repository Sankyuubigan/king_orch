/**
 * scheme-card — generic карточка custom JSON-фенса в чате (для ВСЕХ агентов).
 *
 * Заменяет агент-специфичную карточку (src/ui-файл, удалён 09.11.2026;
 * бизнес-логика рендера — `agents/<агент>/views/*.ts`). Материализует `<pre class="scheme-fence" data-fence-type="...">` (продукт
 * renderMarkdown) в карточку: inline SVG-превью, тумблер «Меркмайнд»/«SVG» при
 * наличии buildMermaid у renderer'а, честная карточка-ошибка (§2.2) при
 * null-parse. Клик по view открывает полноэкранную модалку схем
 * (src/ui/scheme-viewer.ts, Шаг 2 PRP).
 */

import type { SchemeMode, SchemeRenderer } from "../utils";
import { ensureSchemesRegistered, schemeRendererFor } from "./scheme-registry";
import { openSchemeViewer } from "./scheme-viewer";

export interface MaterializeSchemeCardsOptions {
  /** Рендер mermaid чата (передаётся из chat.ts рядом с renderMermaid). */
  renderMermaid?: () => Promise<void>;
}

/** SSOT машины режимов для карточки и модалки: кладёт контент режима в view.
 * В режиме схемы — промис renderMermaid (или undefined); у модалки — базовый
 * промис для до-настройки после рендера. */
export function applySchemeModeInto(
  view: HTMLElement,
  renderer: SchemeRenderer,
  parsed: unknown,
  mode: SchemeMode,
  renderMermaid?: () => Promise<void>,
): Promise<void> | undefined {
  if (mode === "scheme") {
    // Экранируем разметку, чтобы textContent pre.mermaid содержал исходник
    // (mermaid.run читает textContent; htmlLabels отрисует <b>/<br/>).
    const scheme = (renderer.buildMermaid?.(parsed) ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;");
    view.innerHTML = `<pre class="mermaid">${scheme}</pre>`;
    return renderMermaid ? renderMermaid() : undefined;
  }
  view.innerHTML = renderer.buildSvg(parsed);
  return undefined;
}

export function materializeSchemeCards(
  container: HTMLElement,
  opts: MaterializeSchemeCardsOptions = {},
): void {
  ensureSchemesRegistered();
  const blocks = container.querySelectorAll<HTMLElement>("pre.scheme-fence");
  blocks.forEach((pre) => materializeOne(pre, opts));
}

function materializeOne(pre: HTMLElement, opts: MaterializeSchemeCardsOptions): void {
  if (pre.dataset.materialized === "1") return; // идемпотентность
  pre.dataset.materialized = "1";

  const fenceType = pre.dataset.fenceType || "";
  const renderer = fenceType ? schemeRendererFor(fenceType) : undefined;
  const raw = (pre.textContent || "").trim();
  const parsed = renderer ? renderer.parse(raw) : null;

  const card = document.createElement("div");
  card.className = "scheme-card";
  card.style.cssText =
    "border:1px solid #2d3748;border-radius:12px;padding:10px;margin:6px 0;background:#0d1117;";

  if (!renderer || !parsed) {
    // Битый JSON / незарегистрированный fence-тип — честная ошибка вместо
    // тихого пропуска (§00.3 / §2.2).
    const err = document.createElement("div");
    err.className = "scheme-card-error";
    err.textContent = "⚠️ Блок схемы повреждён: не удалось разобрать данные визуального отчёта";
    err.style.cssText =
      "padding:8px 12px;color:#fca5a5;background:#2a1215;border:1px solid #7f1d1d;border-radius:8px;font-size:13px;";
    card.appendChild(err);
    pre.replaceWith(card);
    return;
  }

  const modeText = (mode: SchemeMode): string =>
    `Режим отображения: ${mode === "svg" ? "SVG" : "Меркмайнд"}`;

  const modeLabel = document.createElement("div");
  modeLabel.className = "scheme-card-mode-label";
  modeLabel.style.cssText =
    "font-size:12px;font-weight:600;color:#94a3b8;letter-spacing:0.3px;";

  const view = document.createElement("div");
  view.className = "scheme-card-view";

  const btn = document.createElement("button");
  btn.className = "scheme-card-toggle";
  btn.type = "button";
  btn.textContent = "Меркмайнд";
  btn.style.cssText =
    "display:block;margin-left:auto;margin-bottom:6px;background:#283593;color:#ffffff;border:none;border-radius:6px;padding:4px 12px;font-size:12px;cursor:pointer;";
  // Тумблер только когда у renderer'а есть альтернативная схема (SSOT честных
  // данных: кнопка-мёртвый-уголок в отсутствие buildMermaid недопустима).
  let mode: SchemeMode = "svg";
  if (typeof renderer.buildMermaid === "function") {
    const applyMode = (m: SchemeMode): void => {
      mode = m;
      applySchemeModeInto(view, renderer, parsed, mode, opts.renderMermaid);
      btn.textContent = mode === "svg" ? "Меркмайнд" : "SVG";
      modeLabel.textContent = modeText(mode);
    };
    applyMode("svg");
    btn.addEventListener("click", () => applyMode(mode === "svg" ? "scheme" : "svg"));
  } else {
    btn.hidden = true;
    modeLabel.textContent = modeText("svg"); // режим один — подпись статична
    applySchemeModeInto(view, renderer, parsed, "svg");
  }

  // Клик по схеме → полноэкранный просмотр (модалка открывается в том режиме,
  // в котором карточка была в момент клика).
  view.addEventListener("click", () => {
    openSchemeViewer({ renderer, parsed, mode, renderMermaid: opts.renderMermaid });
  });

  card.appendChild(modeLabel);
  card.appendChild(btn);
  card.appendChild(view);
  pre.replaceWith(card);
}
