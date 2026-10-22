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

  if (callbacks?.onSave && attachment.file_path) {
    const save = document.createElement("button");
    save.type = "button";
    save.className = "msg-attachment-save";
    save.title = "Сохранить видео";
    save.setAttribute("aria-label", "Сохранить видео");
    save.textContent = "⬇";
    save.addEventListener("click", (event) => {
      event.stopPropagation();
      callbacks.onSave?.(attachment);
    });
    wrapper.appendChild(save);
  }

  wrapper.appendChild(video);
  wrapper.appendChild(play);
  return wrapper;
}
