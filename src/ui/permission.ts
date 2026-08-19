import { invoke } from "@tauri-apps/api/core";

type PermissionPayload = {
  request_id: string;
  agent: string;
  tool: string;
  path: string;
};

let pending: PermissionPayload | null = null;

/** Инициализация плашки разрешений (кнопки + обработчики). Вызывается на старте. */
export function initPermissionDialog() {
  const overlay = document.getElementById("permission-overlay");
  const btnDeny = document.getElementById("permission-btn-deny");
  const btnOnce = document.getElementById("permission-btn-once");
  const btnSession = document.getElementById("permission-btn-session");

  const resolveAndHide = async (decision: string) => {
    if (!pending) return;
    const req = pending;
    pending = null;
    try {
      await invoke("respond_permission", { requestId: req.request_id, decision });
    } catch (e) {
      console.error("respond_permission:", e);
    }
    overlay?.classList.remove("show");
  };

  btnDeny?.addEventListener("click", () => resolveAndHide("deny"));
  btnOnce?.addEventListener("click", () => resolveAndHide("allow_once"));
  btnSession?.addEventListener("click", () => resolveAndHide("allow_session"));

  if (overlay) {
    overlay.addEventListener("click", (e) => {
      // Клик по фону = запрет (консервативный дефолт).
      if (e.target === overlay && pending) resolveAndHide("deny");
    });
  }
}

/** Показать плашку по событию из бэкенда. Очередь не держим — показываем последний. */
export function showPermissionRequest(payload: PermissionPayload) {
  pending = payload;
  const overlay = document.getElementById("permission-overlay");
  const titleEl = document.getElementById("permission-title");
  const messageEl = document.getElementById("permission-message");
  if (!overlay || !titleEl || !messageEl) return;

  titleEl.textContent = `Запись вне проекта (${payload.tool})`;
  messageEl.textContent =
    `Агент «${payload.agent}» запросил доступ к записи по пути:\n${payload.path}\n\n` +
    `«Разрешить в этом чате» запомнит доступ до конца текущей сессии.`;
  overlay.classList.add("show");
  const btnDeny = document.getElementById("permission-btn-deny");
  setTimeout(() => btnDeny?.focus(), 100);
}