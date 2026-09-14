/** Окно-уведомление о VRAM (результат pre-flight; одна кнопка «ОК», non-blocking). */

/** Инициализация окна-уведомления VRAM (кнопка ОК + клик по фону). Вызывается на старте. */
export function initVramDialog() {
  const overlay = document.getElementById("vram-overlay");
  const btnOk = document.getElementById("vram-btn-ok");

  const hide = () => overlay?.classList.remove("show");
  btnOk?.addEventListener("click", hide);
  overlay?.addEventListener("click", (e) => {
    if (e.target === overlay) hide();
  });
}

/** Показать уведомление по событию из бэкенда (запуск модели НЕ блокируется). */
export function showVramRequest(payload: { note: string }) {
  const overlay = document.getElementById("vram-overlay");
  const noteEl = document.getElementById("vram-message");
  if (!overlay || !noteEl) return;

  noteEl.textContent = payload.note;
  overlay.classList.add("show");
  const btnOk = document.getElementById("vram-btn-ok");
  setTimeout(() => btnOk?.focus(), 100);
}