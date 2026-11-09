/**
 * scheme-viewer — полноэкранный просмотр схем чата (09.11.2026, PRP «Схемы
 * чата», Шаги 2-3). Generic для ВСЕХ агентов: открывается и по клику на
 * карточку схемы (src/ui/scheme-card.ts), и по клику на любую mermaid-схему
 * чата (attachMermaidSchemeOpen из chat.ts::renderMermaid).
 *
 * Режимы: renderer-путь (тумблер «Схема»/«SVG» внутри модалки, открытие в
 * режиме карточки в момент клика) и staticNode-путь (клон готового svg из
 * mermaid, без тумблера — переключать нечего, combo «Схема» уже на экране в
 * натуральном размере). Механика оверлея — общий core
 * (src/ui/viewer-overlay-core.ts), mirror image-viewer.ts.
 *
 * Размер: SVG показывается в нативном масштабе (viewBox → px), stage прокручи-
 * вается в обе стороны — «крупно и можно рассмотреть» без зум-контролов (YAGNI).
 * Mermaid рисует svg с max-width (useMaxWidth); в модалке потолок снимается
 * (naturalizeMermaidSize) — схема идёт в натуральном размере.
 */

import type { SchemeMode, SchemeRenderer } from "../utils";
import { applySchemeModeInto } from "./scheme-card";
import { bindOverlayViewer } from "./viewer-overlay-core";

const OVERLAY_ID = "scheme-viewer-overlay";
const CLOSE_ID = "scheme-viewer-close";
const TOGGLE_ID = "scheme-viewer-toggle";
const VIEW_ID = "scheme-viewer-view";
const CAPTION_ID = "scheme-viewer-caption";
const MODE_LABEL_ID = "scheme-viewer-mode-label";

/** Активный показ: renderer-путь (данные + режим) или staticNode-путь. */
interface ActiveScheme {
  mode: SchemeMode;
  renderer?: SchemeRenderer;
  parsed?: unknown;
  staticNode?: Node;
  renderMermaid?: () => Promise<void>;
}

let viewerApi: ReturnType<typeof bindOverlayViewer> | undefined;
let active: ActiveScheme | null = null;
const boundToggles = new WeakSet<HTMLElement>();

export interface SchemeViewerRequest {
  /** Вариант A: renderer из реестра схем (тумблер SVG/Схема доступен при
   * наличии buildMermaid в renderer'е). */
  renderer?: SchemeRenderer;
  parsed?: unknown;
  /** Вариант B: готовый DOM-узел без тумблера (клон mermaid svg). */
  staticNode?: Node;
  /** Подпись; по умолчанию — renderer.caption(parsed), в staticNode — «Схема». */
  caption?: string;
  /** Исходный режим (модалка открывается в режиме карточки из клика). */
  mode?: SchemeMode;
  renderMermaid?: () => Promise<void>;
}

function lookup(): {
  overlay: HTMLElement | null;
  close: HTMLButtonElement | null;
  toggle: HTMLButtonElement | null;
  view: HTMLElement | null;
  caption: HTMLElement | null;
  modeLabel: HTMLElement | null;
} {
  return {
    overlay: document.getElementById(OVERLAY_ID),
    close: document.getElementById(CLOSE_ID) as HTMLButtonElement | null,
    toggle: document.getElementById(TOGGLE_ID) as HTMLButtonElement | null,
    view: document.getElementById(VIEW_ID),
    caption: document.getElementById(CAPTION_ID),
    modeLabel: document.getElementById(MODE_LABEL_ID),
  };
}

/** Ставит подпись режима сверху модалки: пользователь всегда видит, что
 * показано (решение владельца 09.11.26). staticNode — всегда «Схема». */
function setModeLabel(mode: SchemeMode): void {
  const { modeLabel } = lookup();
  if (!modeLabel) return;
  modeLabel.textContent = `Режим отображения: ${mode === "svg" ? "SVG" : "Схема"}`;
}

function applyContent(): void {
  const { view } = lookup();
  if (!view || !active) return;
  view.innerHTML = "";
  if (active.staticNode) {
    // Клон может приехать с шириной чата (width-атрибут/потолок mermaid) —
    // в модалке он обязан вписаться по ширине ЦЕЛИКОМ (тот же инвариант).
    view.appendChild(active.staticNode.cloneNode(true));
    naturalizeMermaidSize(view);
    return;
  }
  if (!active.renderer) {
    // Защитный guard: форму модалки нарушают только код-багом (toggle скрыт
    // при отсутствующем renderer) — честный рантайм-журнал, не тихое молчание.
    console.error("scheme-viewer: нет renderer'а для показа схемы");
    return;
  }
  const done = applySchemeModeInto(
    view,
    active.renderer,
    active.parsed!,
    active.mode,
    active.renderMermaid,
  );
  if (done && active.mode === "scheme") {
    void done.then(() => naturalizeMermaidSize(view));
  }
}

/** Мер попарный потолок (useMaxWidth) и width-атрибут: SVG обязан помещаться
 * по ширине модалки ЦЕЛИКОМ (регресс 09.11.26: width=naturalWidth давал
 * горизонтальный ползунок). Отказ «схема шире окна» структурно невозможен:
 * max-width:100% при height:auto держит пропорции viewBox. */
function naturalizeMermaidSize(view: HTMLElement): void {
  const svg = view.querySelector("svg");
  if (!svg) return; // renderMermaid упал → fallback-код внутри view: потолка нет
  svg.removeAttribute("width");
  svg.style.removeProperty("max-width"); // толчок mermaid useMaxWidth в px
  svg.style.maxWidth = "100%";
  svg.style.height = "auto";
}

function ensureBound(): void {
  viewerApi ??= bindOverlayViewer(OVERLAY_ID, {
    closeId: CLOSE_ID,
    onClosed: () => {
      // Контент модалки не переживает закрытие: без утечек ссылок/анимаций.
      const view = document.getElementById(VIEW_ID);
      if (view) view.innerHTML = "";
      active = null;
    },
  });
  const { toggle } = lookup();
  if (!toggle || boundToggles.has(toggle)) return;
  boundToggles.add(toggle);
  toggle.addEventListener("click", () => {
    if (!active?.renderer) return; // тумблер живёт только при renderer-пути
    const nextMode: SchemeMode = active.mode === "svg" ? "scheme" : "svg";
    active.mode = nextMode;
    toggle.textContent = nextMode === "svg" ? "Схема" : "SVG";
    setModeLabel(nextMode);
    applyContent();
  });
}

export function openSchemeViewer(req: SchemeViewerRequest): void {
  ensureBound();
  const { overlay, view, toggle, caption } = lookup();
  if (!overlay || !view || !toggle || !caption) return;

  active = {
    mode: req.mode ?? "svg",
    renderer: req.renderer,
    parsed: req.parsed,
    staticNode: req.staticNode,
    renderMermaid: req.renderMermaid,
  };

  const hasModes = !req.staticNode && typeof req.renderer?.buildMermaid === "function";
  toggle.style.display = hasModes ? "" : "none";
  toggle.textContent = active.mode === "svg" ? "Схема" : "SVG";
  // Подпись режима сверху: staticNode — клон mermaid-схемы, режим всегда «Схема»
  setModeLabel(req.staticNode ? "scheme" : active.mode);

  const captionText =
    req.caption ??
    (req.renderer && req.parsed != null ? req.renderer.caption(req.parsed) : "Схема");
  caption.textContent = captionText;
  caption.hidden = !captionText;

  applyContent();
  viewerApi!.open();
}

/** Клик-просмотр готовых mermaid-схем чата (любой агент): вешается после
 * renderMermaid (chat.ts). Идемпотентно — dataset-флаг против дублей при
 * повторных прогонах renderMermaid. Пустой svg (fallback-код-блок) — честный
 * пропуск: визуала-то нет. Внутри самой модалки схемы не навешиваются. */
export function attachMermaidSchemeOpen(root: ParentNode = document): void {
  root.querySelectorAll("pre.mermaid").forEach((block) => {
    const el = block instanceof HTMLElement ? block : null;
    if (!el) return;
    if (el.dataset.schemeOpenAttached === "1") return;
    el.dataset.schemeOpenAttached = "1";
    if (el.closest(`#${OVERLAY_ID}`)) return;
    const svg = el.querySelector("svg");
    if (!svg) return;
    el.classList.add("mermaid-openable");
    el.addEventListener("click", () =>
      openSchemeViewer({ staticNode: svg, caption: "Схема" }),
    );
  });
}
