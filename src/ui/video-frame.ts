import { convertFileSrc } from "@tauri-apps/api/core";

/**
 * Извлечение первого кадра видео в JPEG data-url (кадр-миниатюра).
 *
 * Зачем: клип в messages[] лежит только по file_path (десятки МБ, base64 не
 * хранится), поэтому лёгкое превью добывается стримингом через Tauri
 * asset-протокол (<video preload="metadata"> читает только нужные байты),
 * а не полным get_video_data_url (десятки МБ base64 по IPC на каждый hover).
 *
 * Контракт: функция НИКОГДА не бросает — при любой неудаче resolve(null) с
 * console.warn-причиной; наверху null означает fallback/placeholder.
 */

const FRAME_TIMEOUT_MS = 8000;
const MAX_FRAME_SIDE = 480;
const JPEG_QUALITY = 0.85;

/** Кадр через asset-протокол: путь на диске → потокобайтовый URL. */
export async function extractVideoFirstFrameFromPath(path: string): Promise<string | null> {
  return extractVideoFirstFrame(convertFileSrc(path));
}

/** Кадр из полного data-url (fallback, когда asset-протокол недоступен). */
export async function extractVideoFirstFrameFromDataUrl(dataUrl: string): Promise<string | null> {
  return extractVideoFirstFrame(dataUrl);
}

/** Ядро: скрытый <video src=srcUrl> → loadeddata → seek на ~0.05с → canvas → JPEG. */
export async function extractVideoFirstFrame(srcUrl: string): Promise<string | null> {
  const video = document.createElement("video");
  video.muted = true;
  video.preload = "metadata";
  video.src = srcUrl;
  video.style.display = "none";
  video.setAttribute("aria-hidden", "true");
  document.body.appendChild(video);
  try {
    return await withTimeout(captureFirstFrame(video));
  } catch (e) {
    console.warn("[video-frame] кадр не получен:", e);
    return null;
  } finally {
    // Cleanup всегда, даже при ошибке/таймауте.
    video.removeAttribute("src");
    video.load();
    video.remove();
  }
}

function withTimeout(p: Promise<string | null>): Promise<string | null> {
  return new Promise((resolve) => {
    const timer = window.setTimeout(() => resolve(null), FRAME_TIMEOUT_MS);
    p.then(
      (v) => {
        window.clearTimeout(timer);
        resolve(v);
      },
      (e) => {
        window.clearTimeout(timer);
        console.warn("[video-frame] кадр не получен:", e);
        resolve(null);
      },
    );
  });
}

/** Ждёт первый decodable кадр (loadeddata + seek, webm рисует чёрный кадр на 0.0). */
async function captureFirstFrame(video: HTMLVideoElement): Promise<string | null> {
  await waitEvent(video, "loadeddata");
  video.currentTime = 0.05;
  await waitEvent(video, "seeked");
  const w = video.videoWidth;
  const h = video.videoHeight;
  if (!w || !h) throw new Error("видео без размеров кадра (videoWidth/videoHeight = 0)");
  const scale = Math.min(1, MAX_FRAME_SIDE / Math.max(w, h));
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(w * scale);
  canvas.height = Math.round(h * scale);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("canvas 2d-контекст недоступен");
  ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL("image/jpeg", JPEG_QUALITY);
}

function waitEvent(video: HTMLVideoElement, event: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const onDone = () => {
      cleanup();
      resolve();
    };
    const onFail = () => {
      cleanup();
      reject(new Error(`видео не смогло отдать кадр (событие ${event})`));
    };
    const cleanup = () => {
      video.removeEventListener(event, onDone);
      video.removeEventListener("error", onFail);
    };
    video.addEventListener(event, onDone, { once: true });
    video.addEventListener("error", onFail, { once: true });
  });
}
