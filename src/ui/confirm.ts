let resolvePromise: ((value: boolean) => void) | null = null;

export function initConfirmDialog() {
  const overlay = document.getElementById('confirm-overlay');
  const btnYes = document.getElementById('confirm-btn-yes');
  const btnNo = document.getElementById('confirm-btn-no');

  if (btnYes) {
    btnYes.addEventListener('click', () => {
      if (resolvePromise) {
        resolvePromise(true);
        resolvePromise = null;
      }
      if (overlay) overlay.classList.remove('show');
    });
  }

  if (btnNo) {
    btnNo.addEventListener('click', () => {
      if (resolvePromise) {
        resolvePromise(false);
        resolvePromise = null;
      }
      if (overlay) overlay.classList.remove('show');
    });
  }

  if (overlay) {
    overlay.addEventListener('click', (e) => {
      if (e.target === overlay) {
        if (resolvePromise) {
          resolvePromise(false);
          resolvePromise = null;
        }
        overlay.classList.remove('show');
      }
    });
  }
}

export function confirmDialog(title: string, message: string): Promise<boolean> {
  return new Promise((resolve) => {
    const overlay = document.getElementById('confirm-overlay');
    const titleEl = document.getElementById('confirm-title');
    const messageEl = document.getElementById('confirm-message');

    if (!overlay || !titleEl || !messageEl) {
      resolve(confirm(message));
      return;
    }

    titleEl.textContent = title;
    messageEl.textContent = message;
    resolvePromise = resolve;
    overlay.classList.add('show');

    const btnNo = document.getElementById('confirm-btn-no');
    setTimeout(() => btnNo?.focus(), 100);
  });
}