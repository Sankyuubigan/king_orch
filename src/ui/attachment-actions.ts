import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { openImageViewer } from "./image-viewer";
import { showToast } from "./toast";
import { trackError } from "../telemetry";
import type { Attachment } from "../types";
import type { ImageAttachmentCallbacks } from "./image-attachment";

/**
 * SSOT операций над вложениями (просмотр / сохранение / превью-загрузка /
 * проводник). Живёт в ui-модуле, но это не визуальный компонент, а фабрика
 * колбэков: invoke-вызовы сосредоточены здесь, UI-компоненты получают операции
 * через callbacks (правило «никаких invoke внутри UI-компонентов»).
 *
 * Используется контроллерами chat.ts (сообщения чата) и sessions.ts
 * (предпросмотр истории сессий) — одна копия логики, whence-строки трекинга
 * сохранены прежними (история Aptabase не должна ломаться).
 */

const IMAGE_EXTENSIONS: Record<string, string[]> = {
  "image/jpeg": ["jpg", "jpeg"],
  "image/png": ["png"],
  "image/webp": ["webp"],
  "image/gif": ["gif"],
  "image/bmp": ["bmp"],
  "image/avif": ["avif"],
  "image/svg+xml": ["svg"],
  "image/tiff": ["tif", "tiff"],
  "image/x-icon": ["ico"],
};

function imageExtensions(mimeType: string, fileName: string): string[] {
  if (mimeType.toLowerCase().startsWith("video/")) {
    return ["webm", "avi", "mp4", "mkv"];
  }
  const known = IMAGE_EXTENSIONS[mimeType.toLowerCase()];
  if (known) return known;
  const fromName = fileName.match(/\.([a-z0-9]+)$/i)?.[1]?.toLowerCase();
  return fromName ? [fromName] : ["png"];
}

export function createAttachmentCallbacks(): ImageAttachmentCallbacks {
  return {
    // Промис возвращается сознательно (сигнатура интерфейса — void): вызывающий
    // может проигнорировать его, а тесты — дождаться завершения операции.
    onPreview: (attachment) => onPreviewImage(attachment),
    onSave: (attachment) => onSaveImage(attachment),
    loadPreview: (attachment, media) => loadAttachmentPreview(attachment, media),
    onOpenPath: (attachment) => onOpenAttachmentPath(attachment),
  };
}

async function onPreviewImage(attachment: Attachment) {
  try {
    const dataUrl = attachment.data_base64
      ? `data:${attachment.mime_type};base64,${attachment.data_base64}`
      : await invoke<string>("get_image_data_url", { path: attachment.file_path });
    openImageViewer(dataUrl, attachment.file_name);
  } catch (error) {
    showToast(`Не удалось открыть изображение: ${error}`, "error");
    void trackError("chat.previewImage", error);
  }
}

async function onSaveImage(img: Attachment) {
  try {
    const extensions = imageExtensions(img.mime_type, img.file_name);
    const defName = (img.file_name || `image.${extensions[0]}`).replace(/\.[a-z0-9]+$/i, "") + `.${extensions[0]}`;
    const isVideo = img.mime_type?.startsWith("video/") || false;
    const savePath = await saveDialog({ defaultPath: defName, filters: [{ name: isVideo ? "Видео" : "Изображения", extensions }] });
    if (!savePath) return;
    if (img.file_path) {
      await invoke("save_image_from_path", { sourcePath: img.file_path, destinationPath: savePath });
    } else if (img.data_base64) {
      await invoke("save_image_file", { path: savePath, dataBase64: img.data_base64 });
    } else {
      throw new Error("У изображения нет ни пути, ни данных");
    }
    showToast(`${isVideo ? "Видео" : "Изображение"} сохранено: ${savePath}`, "success");
  } catch (err) {
    showToast(`Ошибка сохранения: ${err}`, "error");
    void trackError("chat.saveImage", err);
  }
}

async function loadAttachmentPreview(attachment: Attachment, media: HTMLMediaElement) {
  if (!attachment.file_path) return;
  try {
    const command = attachment.mime_type?.startsWith("video/")
      ? "get_video_data_url"
      : "get_image_data_url";
    media.src = await invoke<string>(command, { path: attachment.file_path });
  } catch (error) {
    showToast(`Не удалось загрузить превью: ${error}`, "error");
    void trackError("chat.loadAttachmentPreview", error);
  }
}

async function onOpenAttachmentPath(attachment: Attachment) {
  if (!attachment.file_path) return;
  try {
    await invoke("reveal_path", { path: attachment.file_path });
  } catch (error) {
    showToast(`Не удалось открыть путь: ${error}`, "error");
    void trackError("chat.openAttachmentPath", error);
  }
}
