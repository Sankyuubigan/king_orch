import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { formatSpeed } from "../utils";
import { logFront } from "@my-tauri-plugins/plugin-logs";
import type { DragDropPayload } from "../types";
import { showPermissionRequest } from "../ui";
import { getChatControllerForSession, getActiveChatController } from "./tabs";

/**
 * Роутер глобальных Tauri-событий движка и агентов.
 *
 * Регистрируется ОДИН раз (main.ts, initChatEventRouter). Бэкенд кладёт
 * `sessionId` в payload каждого события — роутер доставляет его в контроллер
 * вкладки ЭТОЙ сессии. Обработка параллельна (облачные ходы) и по очереди
 * (локальные, гейт в бэкенде): вкладки не мешают друг другу, событие чужой
 * сессии в эту вкладку не попадает. Per-tab `listen(...)` не используется,
 * чтобы не плодить дублирующиеся обработчики на каждую чат-вкладку.
 */
export function initChatEventRouter(): void {
  // Контроллер сессии из payload события (пустой/чужой sessionId → null,
  // событие игнорируется — например, для закрытой вкладки).
  const by = (e: any) => getChatControllerForSession(e?.payload?.sessionId ?? "");

  listen("progress", (e) => by(e)?.onProgress(e.payload.progress));
  // payload теперь объект { sessionId, text } (был голой строкой).
  listen("status", (e) => by(e)?.onStatus(e.payload.text));
  listen("stream_chunk", (e) => by(e)?.onStreamChunk(e.payload));
  // payload { sessionId } — содержимое роутеру не важно.
  listen("stream_reset", (e) => by(e)?.onStreamReset());
  // Вложенный объект разворачивается: { sessionId, subcall } → subcall.
  listen("subcall_done", (e) => by(e)?.onSubcallDone(e.payload.subcall));
  listen("agent_thought", (e) => by(e)?.onAgentThought(e.payload));
  listen("agent_tool_call", (e) => by(e)?.onAgentToolCall(e.payload));
  listen("engine_mode", (e) => by(e)?.onEngineMode(e.payload));

  // Стриминг облачных комбо (`cloud-routers-chunk` от плагина) в чат НЕ
  // маршрутизируется: команда `chat_completion` плагина хостом не вызывается —
  // облачный стрим чата идёт серверным путём CloudEngine::generate →
  // stream_chunk с sessionId (проверено grep'ом по src/, 26.10.2026).

  void getCurrentWebview().onDragDropEvent((event) => {
    getActiveChatController()?.onFileDragEvent(event.payload as DragDropPayload);
  });

  // Плашка разрешений — глобальный модал (очередь живёт в ui/permission.ts,
  // FIFO по request_id). Уведомление о памяти — машинно-уровневое, но модалка
  // убрана (tasks/31.10.26): доставляем В ЧАТ активной вкладки системным
  // сообщением (ChatController.onVramNotice); нет активной вкладки — в лог.
  listen("tool_permission_request", (e) => showPermissionRequest(e.payload));
  listen("vram_notice", (e) => {
    const ctrl = getActiveChatController();
    if (ctrl) ctrl.onVramNotice(e.payload);
    else logFront(`🎛️ ${e?.payload?.note ?? ""}`);
  });

  // Скачивание модели/движка — единый движок downloader (прогресс speed/ETA).
  listen("downloader:progress", (e) => {
    const p = e.payload as {
      label: string;
      downloaded: number;
      total: number;
      speed_bps?: number;
      status?: string;
      message?: string;
    };
    if (p.status && p.status !== "running") {
      logFront(
        p.status === "done"
          ? `✅ Завершено: ${p.label}`
          : `⚠️ ${p.label}: ${p.message || p.status}`,
      );
      return;
    }
    if (p.total > 0) {
      const pct = ((p.downloaded / p.total) * 100).toFixed(1);
      const mbD = (p.downloaded / 1024 / 1024).toFixed(1);
      const mbT = (p.total / 1024 / 1024).toFixed(1);
      const speed = formatSpeed(p.speed_bps);
      logFront(
        `📥 ${p.label}: ${mbD} MB / ${mbT} MB (${pct}%)${speed ? ` · ${speed}` : ""}`,
      );
    }
  });
}
