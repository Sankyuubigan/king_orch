import type { Attachment } from "../types";

export interface ImageAttachmentCallbacks {
  onPreview: (attachment: Attachment) => void;
  onSave: (attachment: Attachment) => void;
  /** Ленивая загрузка: для изображения — get_image_data_url, для видео — get_video_data_url. */
  loadPreview?: (attachment: Attachment, media: HTMLMediaElement) => void;
  onOpenPath?: (attachment: Attachment) => void;
}

export function createImageAttachmentElement(
  attachment: Attachment,
  callbacks?: ImageAttachmentCallbacks,
): HTMLDivElement {
  const wrapper = document.createElement("div");
  wrapper.className = "msg-attachment-image-wrap";

  const dataUrl = attachment.data_base64
    ? `data:${attachment.mime_type};base64,${attachment.data_base64}`
    : null;
  const preview = document.createElement("button");
  preview.type = "button";
  preview.className = "msg-attachment-preview";
  preview.title = "Открыть изображение";
  preview.setAttribute("aria-label", "Открыть изображение");

  const img = document.createElement("img");
  img.className = "msg-attachment-img";
  if (dataUrl) img.src = dataUrl;
  img.alt = attachment.file_name || "изображение";
  img.title = attachment.file_name || "изображение";
  img.draggable = false;
  preview.appendChild(img);

  if (callbacks) {
    preview.addEventListener("click", () => callbacks.onPreview(attachment));
    if (!dataUrl && attachment.file_path) {
      callbacks.loadPreview?.(attachment, img);
    }
  }

  if (callbacks) {
    // Файл на диске → «Показать в проводнике» (повторное сохранение не нужно).
    // Только base64 (вложение юзера без пути) → остаётся сохранение на диск.
    const action = document.createElement("button");
    action.type = "button";
    action.className = "msg-attachment-action";
    if (attachment.file_path) {
      action.title = "Показать в проводнике";
      action.setAttribute("aria-label", "Показать в проводнике");
      action.textContent = "📂";
      action.addEventListener("click", (event) => {
        event.stopPropagation();
        callbacks.onOpenPath?.(attachment);
      });
    } else {
      action.title = "Сохранить изображение";
      action.setAttribute("aria-label", "Сохранить изображение");
      action.textContent = "⬇";
      action.addEventListener("click", (event) => {
        event.stopPropagation();
        callbacks.onSave(attachment);
      });
    }
    wrapper.appendChild(action);
  }

  wrapper.appendChild(preview);
  return wrapper;
}
