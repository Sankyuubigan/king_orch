import { invoke } from "@tauri-apps/api/core";

type VramPayload = {
  request_id: string;
  note: string;
};

let pending: VramPayload | null = null;

/** Инициализация диалога pre-flight VRAM (кнопки + обработчики). Вызывается на старте. */
export function initVramDialog() {
  const overlay = document.getElementById("vram-overlay");
  const btnCancel = document.getElementById("vram-btn-cancel");
  const btnProceed = document.getElementById("vram-btn-proceed");

  const resolveAndHide = async (decision: string) => {
    if (!pending) return;
    const req = pending;
    pending = null;
    try {
      await invoke("respond_vram_choice", { requestId: req.request_id, decision });
    } catch (e) {
      console.error("respond_vram_choice:", e);
    }
    overlay?.classList.remove("show");
  };

  btnCancel?.addEventListener("click", () => resolveAndHide("cancel"));
  btnProceed?.addEventListener("click", () => resolveAndHide("proceed"));

  if (overlay) {
    overlay.addEventListener("click", (e) => {
      // Клик по фону = Отмена (консервативный дефолт).
      if (e.target === overlay && pending) resolveAndHide("cancel");
    });
  }
}

/** Показать диалог по событию из бэкенда. Очередь не держим — показываем последний. */
export function showVramRequest(payload: VramPayload) {
  pending = payload;
  const overlay = document.getElementById("vram-overlay");
  const noteEl = document.getElementById("vram-message");
  if (!overlay || !noteEl) return;

  noteEl.textContent = payload.note;
  overlay.classList.add("show");
  const btnCancel = document.getElementById("vram-btn-cancel");
  setTimeout(() => btnCancel?.focus(), 100);
}