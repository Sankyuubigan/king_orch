import type { Attachment } from "../types";
import type { ImageAttachmentCallbacks } from "./image-attachment";

/**
 * Карточка документа в чате (excel-артефакты: .xlsx/.csv/.pdf и прочие
 * не-image/не-video вложения). Файл лежит на диске по file_path (base64 в
 * messages[] не хранится) — клик по карточке и кнопка 📂 открывают папку с
 * выделенным файлом; вложение без пути (только base64) падает в сохранение
 * на диск (⬇).
 */
export function createFileAttachmentElement(
  attachment: Attachment,
  callbacks?: ImageAttachmentCallbacks,
): HTMLDivElement {
  const wrapper = document.createElement("div");
  wrapper.className = "msg-attachment-file-wrap";

  const card = document.createElement("button");
  card.type = "button";
  card.className = "msg-attachment-file";
  card.textContent = `${attachment.is_dir ? "📁" : "📄"} ${attachment.file_name || "файл"}`;
  if (attachment.file_path) card.title = attachment.file_path;
  wrapper.appendChild(card);

  if (callbacks) {
    // Файл на диске → клик по карточке открывает проводник; повторное
    // сохранение не нужно. Только base64 (вложение юзера без пути) → ⬇.
    if (attachment.file_path && callbacks.onOpenPath) {
      card.addEventListener("click", () => callbacks.onOpenPath?.(attachment));

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
    } else {
      card.title = "Сохранить файл";
      card.addEventListener("click", () => callbacks.onSave(attachment));

      const save = document.createElement("button");
      save.type = "button";
      save.className = "msg-attachment-action";
      save.title = "Сохранить файл";
      save.setAttribute("aria-label", "Сохранить файл");
      save.textContent = "⬇";
      save.addEventListener("click", (event) => {
        event.stopPropagation();
        callbacks.onSave(attachment);
      });
      wrapper.appendChild(save);
    }
  }

  return wrapper;
}
