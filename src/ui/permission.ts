import { invoke } from "@tauri-apps/api/core";

type PermissionPayload = {
  request_id: string;
  agent: string;
  tool: string;
  path: string;
  /** Чат, из которого пришёл запрос: ходы нескольких вкладок параллельны. */
  sessionId?: string;
};

/// FIFO-очередь плашек: Rust-approver держит много pending по request_id,
/// фронт показывает их СТРОГО по одному — первый пришёл, первый показан;
/// ответ скрыл текущую — показан следующий. Без очереди вторая плашка
/// затирала бы первую, и ответ на неё молча терял запрос.
const queue: PermissionPayload[] = [];
let current: PermissionPayload | null = null;

/** Инициализация плашки разрешений (кнопки + обработчики). Вызывается на старте. */
export function initPermissionDialog() {
  const overlay = document.getElementById("permission-overlay");
  const btnDeny = document.getElementById("permission-btn-deny");
  const btnOnce = document.getElementById("permission-btn-once");
  const btnSession = document.getElementById("permission-btn-session");

  const resolveAndHide = async (decision: string) => {
    if (!current) return;
    const req = current;
    current = null;
    try {
      await invoke("respond_permission", { requestId: req.request_id, decision });
    } catch (e) {
      console.error("respond_permission:", e);
    }
    // Следующий из очереди — или скрытие оверлея, если очередь пуста.
    showNext();
  };

  btnDeny?.addEventListener("click", () => resolveAndHide("deny"));
  btnOnce?.addEventListener("click", () => resolveAndHide("allow_once"));
  btnSession?.addEventListener("click", () => resolveAndHide("allow_session"));

  if (overlay) {
    overlay.addEventListener("click", (e) => {
      // Клик по фону = запрет (консервативный дефолт).
      if (e.target === overlay && current) resolveAndHide("deny");
    });
  }
}

/** Показать плашку по событию из бэкенда. Запрос встаёт в очередь; если
 *  ничего не показано — очередь отрисовывается немедленно. */
export function showPermissionRequest(payload: PermissionPayload) {
  queue.push(payload);
  if (!current) showNext();
}

/** Достать первый запрос из очереди и показать (или скрыть оверлей). */
function showNext() {
  current = queue.shift() ?? null;
  const overlay = document.getElementById("permission-overlay");
  if (!current) {
    overlay?.classList.remove("show");
    return;
  }
  const titleEl = document.getElementById("permission-title");
  const messageEl = document.getElementById("permission-message");
  if (!overlay || !titleEl || !messageEl) return;

  titleEl.textContent = `Запись вне проекта (${current.tool})`;
  const from = current.sessionId ? `\n\nИз чата: ${current.sessionId}` : "";
  messageEl.textContent =
    `Агент «${current.agent}» запросил доступ к записи по пути:\n${current.path}\n\n` +
    `«Разрешить в этом чате» запомнит доступ до конца текущей сессии.${from}`;
  overlay.classList.add("show");
  const btnDeny = document.getElementById("permission-btn-deny");
  setTimeout(() => btnDeny?.focus(), 100);
}
