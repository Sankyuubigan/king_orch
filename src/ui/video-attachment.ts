import type { Attachment } from "../types";
import type { ImageAttachmentCallbacks } from "./image-attachment";

/**
 * Видео-attachment в чате (рекурсия answer с клипом generate_video).
 * Клип лежит на диске по file_path (base64 в messages[] не хранится),
 * поэтому первый Play лениво тянет data-URL через invoke get_video_data_url.
 */
export function createVideoAttachmentElement(
  attachment: Attachment,
  callbacks?: ImageAttachmentCallbacks,
): HTMLDivElement {
  const wrapper = document.createElement("div");
  wrapper.className = "msg-attachment-video-wrap";

  const video = document.createElement("video");
  video.className = "msg-attachment-video";
  video.controls = true;
  video.preload = "metadata";
  video.style.display = "none";
  video.style.maxWidth = "100%";
  video.title = attachment.file_name || "видео";

  const play = document.createElement("button");
  play.type = "button";
  play.className = "msg-attachment-video-play";
  play.textContent = `▶ ${attachment.file_name || "видео"}`;
  if (attachment.file_path) play.title = attachment.file_path;

  play.addEventListener("click", async () => {
    if (video.src) {
      video.style.display = "";
      play.style.display = "none";
      void video.play().catch(() => undefined);
      return;
    }
    play.disabled = true;
    try {
      if (attachment.data_base64) {
        video.src = `data:${attachment.mime_type};base64,${attachment.data_base64}`;
      } else if (attachment.file_path && callbacks?.loadPreview) {
        await callbacks.loadPreview(attachment, video);
      }
      if (!video.src) throw new Error("не удалось получить данные видео");
      video.style.display = "";
      play.style.display = "none";
      void video.play().catch(() => undefined);
    } catch (error) {
      play.textContent = `⚠ ${attachment.file_name || "видео"}`;
      play.disabled = false;
      console.error("Видео не загрузилось:", error);
    }
  });

  // Клип всегда лежит на диске (base64 не хранится) — кнопка открывает
  // папку с выделенным файлом, повторное сохранение не нужно.
  if (callbacks?.onOpenPath && attachment.file_path) {
    const reveal = document.createElement("button");
    reveal.type = "button";
    reveal.className = "msg-attachment-action";
    reveal.title = "Показать в проводнике";
    reveal.setAttribute("aria-label", "Показать в проводнике");
    reveal.textContent = "📂";
    reveal.addEventListener("click", (event) => {
      event.stopPropagation();
      callbacks.onOpenPath?.(attachment);
    });
    wrapper.appendChild(reveal);
  }

  wrapper.appendChild(video);
  wrapper.appendChild(play);
  return wrapper;
}
