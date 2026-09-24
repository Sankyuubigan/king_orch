import type { Attachment } from "../types";

export interface ImageAttachmentCallbacks {
  onPreview: (attachment: Attachment) => void;
  onSave: (attachment: Attachment) => void;
}

export function createImageAttachmentElement(
  attachment: Attachment,
  callbacks?: ImageAttachmentCallbacks,
): HTMLDivElement {
  const wrapper = document.createElement("div");
  wrapper.className = "msg-attachment-image-wrap";

  const dataUrl = `data:${attachment.mime_type};base64,${attachment.data_base64}`;
  const preview = document.createElement("button");
  preview.type = "button";
  preview.className = "msg-attachment-preview";
  preview.title = "Открыть изображение";
  preview.setAttribute("aria-label", "Открыть изображение");

  const img = document.createElement("img");
  img.className = "msg-attachment-img";
  img.src = dataUrl;
  img.alt = attachment.file_name || "изображение";
  img.title = attachment.file_name || "изображение";
  img.draggable = false;
  preview.appendChild(img);

  if (callbacks) {
    preview.addEventListener("click", () => callbacks.onPreview(attachment));
  }

  if (callbacks) {
    const save = document.createElement("button");
    save.type = "button";
    save.className = "msg-attachment-save";
    save.title = "Сохранить изображение";
    save.setAttribute("aria-label", "Сохранить изображение");
    save.textContent = "⬇";
    save.addEventListener("click", (event) => {
      event.stopPropagation();
      callbacks.onSave(attachment);
    });
    wrapper.appendChild(save);
  }

  wrapper.appendChild(preview);
  return wrapper;
}
