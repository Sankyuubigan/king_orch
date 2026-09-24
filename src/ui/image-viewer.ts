const initializedOverlays = new WeakSet<HTMLElement>();
let opener: HTMLElement | null = null;
let focusTimer: number | undefined;

function getViewerElements() {
  return {
    overlay: document.getElementById("image-viewer-overlay"),
    image: document.getElementById("image-viewer-image") as HTMLImageElement | null,
    caption: document.getElementById("image-viewer-caption"),
    close: document.getElementById("image-viewer-close") as HTMLButtonElement | null,
  };
}

function closeViewer() {
  const { overlay, image } = getViewerElements();
  if (focusTimer !== undefined) {
    window.clearTimeout(focusTimer);
    focusTimer = undefined;
  }
  overlay?.classList.remove("show");
  overlay?.setAttribute("aria-hidden", "true");
  if (image) image.removeAttribute("src");
  if (opener?.isConnected) opener.focus();
  opener = null;
}

export function initImageViewer() {
  const { overlay, close } = getViewerElements();
  if (!(overlay instanceof HTMLElement) || initializedOverlays.has(overlay)) return;
  initializedOverlays.add(overlay);

  close?.addEventListener("click", closeViewer);
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) closeViewer();
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && overlay.classList.contains("show")) closeViewer();
  });
}

export function openImageViewer(dataUrl: string, fileName: string) {
  initImageViewer();
  const { overlay, image, caption, close } = getViewerElements();
  if (!overlay || !image || !caption) return;

  opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  image.src = dataUrl;
  image.alt = fileName || "изображение";
  caption.textContent = fileName;
  caption.hidden = !fileName;
  overlay.classList.add("show");
  overlay.setAttribute("aria-hidden", "false");
  if (focusTimer !== undefined) window.clearTimeout(focusTimer);
  focusTimer = window.setTimeout(() => {
    close?.focus();
    focusTimer = undefined;
  }, 0);
}
