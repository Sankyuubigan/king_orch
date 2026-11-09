/**
 * viewer-overlay-core — единственная копия механики полноэкранных оверлеев
 * просмотра (09.11.2026, PRP «Схемы чата», Шаг 2).
 *
 * До этого модуля show/hide/Escape/фон/×/focus-restore дублировались в
 * image-viewer (вложениях чата) и породили бы дубль в scheme-viewer. Здесь
 * механика одна: показать overlay CSS-классом «show» + aria-hidden, закрыть по
 * ×/Esc/клику по самой подложке, вернуть фокус на элемент-открыватель; содержимое
 * (img.src / DOM-узлы) оверлей не знает — заполняет потребитель.
 *
 * Потребители: src/ui/image-viewer.ts (просмотр вложений) и
 * src/ui/scheme-viewer.ts (полноэкранные схемы карточек и mermaid).
 */

export interface ViewerOverlayApi {
  isOpen(): boolean;
  open(): void;
  close(): void;
}

export interface ViewerOverlayOptions {
  /** id кнопки × внутри overlay. */
  closeId: string;
  /** Очистка содержимого при закрытии (img.removeAttribute("src") / innerHTML=""). */
  onClosed?: () => void;
}

/** Один элемент накладки — одни обработчики (WeakSet переживает пересоздание
 * DOM тестов/или reload body: новый элемент привяжется заново). */
const boundOverlays = new WeakSet<HTMLElement>();

export function bindOverlayViewer(
  overlayId: string,
  opts: ViewerOverlayOptions,
): ViewerOverlayApi {
  const lookup = () => ({
    overlay: document.getElementById(overlayId),
    close: document.getElementById(opts.closeId) as HTMLButtonElement | null,
  });

  let opener: HTMLElement | null = null;
  let focusTimer: number | undefined;

  const isOpen = (): boolean => lookup().overlay?.classList.contains("show") ?? false;

  const close = (): void => {
    if (focusTimer !== undefined) {
      window.clearTimeout(focusTimer);
      focusTimer = undefined;
    }
    const { overlay } = lookup();
    if (!overlay) return;
    overlay.classList.remove("show");
    overlay.setAttribute("aria-hidden", "true");
    opts.onClosed?.();
    if (opener?.isConnected) opener.focus();
    opener = null;
  };

  const open = (): void => {
    ensureBound();
    const { overlay, close: closeBtn } = lookup();
    if (!overlay) return;
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    overlay.classList.add("show");
    overlay.setAttribute("aria-hidden", "false");
    if (focusTimer !== undefined) window.clearTimeout(focusTimer);
    focusTimer = window.setTimeout(() => {
      closeBtn?.focus();
      focusTimer = undefined;
    }, 0);
  };

  function ensureBound(): void {
    const { overlay, close: closeBtn } = lookup();
    if (!(overlay instanceof HTMLElement) || boundOverlays.has(overlay)) return;
    boundOverlays.add(overlay);
    closeBtn?.addEventListener("click", () => close());
    overlay.addEventListener("click", (event) => {
      if (event.target === overlay) close();
    });
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && isOpen()) close();
    });
  }

  const api: ViewerOverlayApi = { isOpen, open, close };
  ensureBound(); // привязка при инициализации, если overlay уже в DOM (main.ts)
  return api;
}
