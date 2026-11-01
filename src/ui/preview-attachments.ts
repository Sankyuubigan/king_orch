import { trackError } from "../telemetry";
import type { Attachment } from "../types";
import type { ImageAttachmentCallbacks } from "./image-attachment";
import { extractVideoFirstFrameFromDataUrl, extractVideoFirstFrameFromPath } from "./video-frame";

/**
 * Компактный ряд мини-вложений сообщения в предпросмотре истории сессий
 * (правая панель при наведении). Визуальная гранула и клик-поведение —
 * не как в чате: миниатюра-кнопка, у видео — кадр 1 + бейдж «🎬 видео»,
 * у файлов — карточка с именем.
 *
 * Контракт модуля: НИКАКИХ импортов @tauri-apps/api/core — все операции
 * (просмотрщик, проводник, сохранение, ленивые превью) приходят через
 * callbacks; кадр видео добывается внутри video-frame.ts.
 */

/** Кеш кадров видео на уровне контроллера: key — file_path вложения. */
export type VideoFrameCache = Map<string, Promise<string | null>>;

export function createPreviewAttachmentRow(
  attachments: Attachment[],
  callbacks: ImageAttachmentCallbacks,
  frames: VideoFrameCache,
): HTMLDivElement {
  const row = document.createElement("div");
  row.className = "preview-attachments";
  for (const att of attachments) {
    if (att.mime_type?.startsWith("image/")) {
      row.appendChild(createImageThumb(att, callbacks));
    } else if (att.mime_type?.startsWith("video/")) {
      row.appendChild(createVideoThumb(att, callbacks, frames));
    } else {
      row.appendChild(createFileCard(att, callbacks));
    }
  }
  return row;
}

/** Картинка: миниатюра (data_base64 сразу, иначе лениво get_image_data_url); клик → просмотрщик. */
function createImageThumb(att: Attachment, callbacks: ImageAttachmentCallbacks): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "preview-att-thumb";
  btn.title = att.file_name || "изображение";

  const img = document.createElement("img");
  img.className = "preview-att-img";
  img.alt = att.file_name || "изображение";
  img.draggable = false;
  if (att.data_base64) {
    img.src = `data:${att.mime_type};base64,${att.data_base64}`;
  } else if (att.file_path) {
    callbacks.loadPreview?.(att, img);
  }
  btn.appendChild(img);

  btn.addEventListener("click", () => callbacks.onPreview(att));
  return btn;
}

/** Видео: кадр 1 + бейдж «🎬 видео» + имя; клик → проводник (плейбека в предпросмотре нет). */
function createVideoThumb(
  att: Attachment,
  callbacks: ImageAttachmentCallbacks,
  frames: VideoFrameCache,
): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "preview-att-thumb preview-att-video";
  btn.title = att.file_name || "видео";

  const img = document.createElement("img");
  img.className = "preview-att-img";
  img.alt = att.file_name || "видео";
  img.draggable = false;
  btn.appendChild(img);

  const badge = document.createElement("span");
  badge.className = "preview-att-badge";
  badge.textContent = "🎬 видео";
  btn.appendChild(badge);

  const name = document.createElement("span");
  name.className = "preview-att-name";
  name.textContent = att.file_name || "видео";
  btn.appendChild(name);

  btn.addEventListener("click", () => callbacks.onOpenPath?.(att));

  void requestVideoFrame(att, callbacks, frames).then((frame) => {
    if (frame) {
      img.src = frame;
    } else {
      // Явный fallback-placeholder: видео есть, но превью недоступно.
      img.remove();
      const fallback = document.createElement("div");
      fallback.className = "preview-att-video-fallback";
      fallback.textContent = "🎬";
      btn.insertBefore(fallback, badge);
    }
  }).catch(() => undefined); // отказ уже залогирован внутри; наверху — placeholder

  return btn;
}

/**
 * Полная цепочка добычи кадра, закешированная ЦЕЛИКОМ (включая fallback и
 * отказ): повторный hover не перекачивает видео ни через asset-протокол,
 * ни тем более через get_video_data_url.
 */
function requestVideoFrame(
  att: Attachment,
  callbacks: ImageAttachmentCallbacks,
  frames: VideoFrameCache,
): Promise<string | null> {
  const key = att.file_path ?? `${att.file_name}|${att.mime_type}`;
  const cached = frames.get(key);
  if (cached) return cached;
  const promise = acquireVideoFrame(att, callbacks);
  frames.set(key, promise);
  return promise;
}

async function acquireVideoFrame(att: Attachment, callbacks: ImageAttachmentCallbacks): Promise<string | null> {
  // 1) Основной путь: asset-протокол — стримим только метаданные/кадр.
  if (att.file_path) {
    const frame = await extractVideoFirstFrameFromPath(att.file_path);
    if (frame) return frame;
  }
  // 2) Fallback: полный data-url через общий callbacks (get_video_data_url
  //    для файла на диске; data_base64 — если вложение вдруг с данными),
  //    затем кадр из data-url тем же canvas-механизмом.
  let frame: string | null = null;
  if (att.data_base64) {
    frame = await extractVideoFirstFrameFromDataUrl(`data:${att.mime_type};base64,${att.data_base64}`);
  } else if (att.file_path) {
    const video = document.createElement("video");
    video.preload = "metadata";
    await Promise.resolve(callbacks.loadPreview?.(att, video));
    if (video.src) frame = await extractVideoFirstFrameFromDataUrl(video.src);
  }
  if (frame) return frame;
  // 3) Кадра нет (файл удалён/не декодируется) — явный placeholder, ошибка
  //    не «проглатывается»: trackError единожды на вложение (кеш обещания).
  console.warn(`[sessions-preview] кадр видео недоступен: ${att.file_name}`);
  void trackError("sessions.preview.videoFrame", new Error(`кадр видео недоступен: ${att.file_path ?? att.file_name}`));
  return null;
}

/** Файл/папка: карточка с именем; клик — проводник (файл на диске) или сохранение. */
function createFileCard(att: Attachment, callbacks: ImageAttachmentCallbacks): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "preview-att-file";
  btn.textContent = `${att.is_dir ? "📁" : "📄"} ${att.file_name || "файл"}`;
  if (att.file_path) btn.title = att.file_path;
  btn.addEventListener("click", () => {
    if (att.file_path) callbacks.onOpenPath?.(att);
    else callbacks.onSave(att);
  });
  return btn;
}
