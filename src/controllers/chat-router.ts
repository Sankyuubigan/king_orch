import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { onChunk as onNineRouterChunk } from "@my-tauri-plugins/plugin-9router";
import { formatSpeed } from "../utils";
import { logFront } from "@my-tauri-plugins/plugin-logs";
import type { DragDropPayload } from "../types";
import { getActiveProcessingChat } from "./chat";
import { getActiveChatController } from "./tabs";

/**
 * Роутер глобальных Tauri-событий движка и агентов.
 *
 * Регистрируется ОДИН раз (main.ts, initChatEventRouter). Все события
 * перенаправляются в контроллер вкладки, обрабатывающей текущий запрос
 * (обработка строго последовательная). Per-tab `listen(...)` не используется,
 * чтобы не плодить дублирующиеся обработчики на каждую чат-вкладку.
 */
export function initChatEventRouter(): void {
  const active = () => getActiveProcessingChat();

  listen("progress", (e) => active()?.onProgress(e.payload as number));
  listen("status", (e) => active()?.onStatus(e.payload as string));

  listen("stream_chunk", (e) => active()?.onStreamChunk(e.payload as any));

  // Стриминг облачных комбо 9Router: плагин шлёт { text, author, kind } —
  // приводим к формату stream_chunk и переиспользуем общий рендер.
  void onNineRouterChunk((c) => active()?.onStreamChunk({ kind: c.kind, author: c.author, text: c.text }));

  void getCurrentWebview().onDragDropEvent((event) => {
    getActiveChatController()?.onFileDragEvent(event.payload as DragDropPayload);
  });

  listen("subcall_done", (e) => active()?.onSubcallDone(e.payload as any));
  listen("agent_thought", (e) => active()?.onAgentThought(e.payload as any));
  listen("agent_tool_call", (e) => active()?.onAgentToolCall(e.payload as any));
  listen("engine_mode", (e) => active()?.onEngineMode(e.payload as any));

  // Модальные диалоги разрешений — показываем при активной обработке.
  listen("tool_permission_request", (e) => active()?.onToolPermission(e.payload as any));
  listen("vram_notice", (e) => active()?.onVramNotice(e.payload as any));

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