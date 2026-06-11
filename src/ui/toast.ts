import { bus } from "../events";

const TOAST_DURATION = 5000;

export function showToast(message: string, type: 'error' | 'success' | 'info' = 'error') {
  const container = document.getElementById('toast-container');
  if (!container) return;

  const toast = document.createElement('div');
  toast.className = `toast toast-${type}`;
  toast.textContent = message;

  container.appendChild(toast);

  if (type === "error") {
    bus.emit("log", `❌ ${message}`);
  }

  // Анимация появления
  requestAnimationFrame(() => {
    toast.classList.add('toast-show');
  });

  // Автоудаление через 5 секунд
  setTimeout(() => {
    toast.classList.remove('toast-show');
    toast.classList.add('toast-hide');
    setTimeout(() => {
      if (toast.parentNode) toast.parentNode.removeChild(toast);
    }, 400);
  }, TOAST_DURATION);
}