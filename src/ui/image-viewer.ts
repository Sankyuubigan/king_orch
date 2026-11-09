import { bindOverlayViewer } from "./viewer-overlay-core";

/**
 * Полноэкранный просмотр изображений-вложений чата (img.src + подпись).
 * Механика оверлея — общий core (src/ui/viewer-overlay-core.ts, 09.11.2026):
 * поведение В ТОЧНОСТИ прежнее — openImageViewer(dataUrl, fileName) ставит
 * картинку/подпись, закрытие по ×/Esc/фону, возврат фокуса, очистка src при
 * закрытии. Публичное API не менялось (main.ts + attachment-actions.ts).
 */

let viewerApi: ReturnType<typeof bindOverlayViewer> | undefined;

function getViewerApi(): ReturnType<typeof bindOverlayViewer> {
  viewerApi ??= bindOverlayViewer("image-viewer-overlay", {
    closeId: "image-viewer-close",
    onClosed: () => {
      const image = document.getElementById("image-viewer-image") as HTMLImageElement | null;
      image?.removeAttribute("src");
    },
  });
  return viewerApi;
}

export function initImageViewer() {
  getViewerApi();
}

export function openImageViewer(dataUrl: string, fileName: string) {
  const overlay = document.getElementById("image-viewer-overlay");
  const image = document.getElementById("image-viewer-image") as HTMLImageElement | null;
  const caption = document.getElementById("image-viewer-caption");
  if (!overlay || !image || !caption) return;

  getViewerApi().open();
  image.src = dataUrl;
  image.alt = fileName || "изображение";
  caption.textContent = fileName;
  caption.hidden = !fileName;
}
