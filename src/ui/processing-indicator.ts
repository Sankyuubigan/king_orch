/// Индикатор обработки для потока чата: SVG-крутилка + статус + маячок + таймер.
///
/// Замена горизонтального прогрессбара: проценты статусов были фейковыми полосами
/// (10/20/60/80), которые между событиями замирали и выглядели как зависание —
/// особенно в очереди облачного провайдера. Индикатор вместо этого показывает
/// то, что честно известно: «обработка идёт» (крутилка крутится всегда),
/// «процесс сдвинулся» (пульс-волна на каждом новом статусе) и «сколько уже
/// ждём» (таймер тикает каждую секунду даже при полном молчании движка).
///
/// Компонент чистый: не знает ни о контроллере, ни о событиях — жизненным
/// циклом управляет ChatController (setProcessingState / onStatus /
/// onStreamChunk). Визуально строка всегда последняя в flex-колонке чата
/// (CSS `order`), DOM-позиция не важна.

export interface ProcessingIndicator {
  /** Корневой элемент строки (.processing-row) — вставляется в chat-history. */
  el: HTMLDivElement;
  /** Показать новый статус: смена текста (с анимацией) + пульс-волна. Повтор того же текста — без анимации. */
  setStatus(text: string): void;
  /** Повтор пульса вручную (волна от крутилки + вспышка маячка). */
  pulse(): void;
  /** Показ/скрытие строки (скрывается при видимом стриме ответа, возвращается со следующим статусом). */
  show(visible: boolean): void;
  /** Запустить счётчик прошедшего времени от метки старта обработки. */
  startTimer(fromTs: number): void;
  stopTimer(): void;
  /** Полное освобождение: таймер + вынос из DOM. */
  destroy(): void;
}

/** Счётчик 0:07 / 12:34 — минуты без ведущего нуля, секунды всегда две цифры. */
function formatElapsed(ms: number): string {
  const totalSec = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

let gradSeq = 0;

/** Повтор CSS-анимации: снять класс, форсировать reflow, вернуть класс. */
function retrigger(el: HTMLElement, className: string) {
  el.classList.remove(className);
  // чтение geometry в браузере форсирует пересчёт стилей — снятая анимация
  // гарантированно завершена до повторного запуска; в jsdom просто no-op.
  void (el as HTMLElement & { offsetWidth: number }).offsetWidth;
  el.classList.add(className);
}

export function createProcessingIndicator(): ProcessingIndicator {
  const row = document.createElement("div");
  row.className = "processing-row";

  // ── Крутилка: трек + градиентная дуга + волна-пульс вокруг ──
  const spinWrap = document.createElement("div");
  spinWrap.className = "pi-spin-wrap";
  const gradId = `pi-grad-${++gradSeq}`;
  spinWrap.innerHTML =
    `<svg class="pi-spinner" viewBox="0 0 40 40" width="22" height="22" aria-hidden="true">` +
    `<defs><linearGradient id="${gradId}" x1="0%" y1="0%" x2="100%" y2="100%">` +
    `<stop offset="0%" stop-color="#38bdf8"/><stop offset="100%" stop-color="#c084fc"/>` +
    `</linearGradient></defs>` +
    `<circle class="pi-track" cx="20" cy="20" r="16"/>` +
    `<circle class="pi-arc" cx="20" cy="20" r="16" stroke="url(#${gradId})"/>` +
    `</svg>` +
    `<span class="pi-wave"></span>`;
  // Градиент живёт в defs этого же SVG — при нескольких вкладках id не конфликтуют.

  // ── Статус: текст справа от крутилки ──
  const status = document.createElement("span");
  status.className = "pi-status";
  status.textContent = "";

  // ── Маячок: вспышка на смену статуса, между событиями тихо дышит ──
  const dot = document.createElement("span");
  dot.className = "pi-dot";

  // ── Таймер прошедшего времени ──
  const timer = document.createElement("span");
  timer.className = "pi-timer";
  timer.textContent = "0:00";
  timer.title = "Время с начала обработки";

  row.append(spinWrap, status, dot, timer);

  let lastStatusText = "";
  let timerHandle: number | null = null;

  const waveEl = spinWrap.querySelector<HTMLElement>(".pi-wave");

  const stopTimer = () => {
    if (timerHandle !== null) {
      clearInterval(timerHandle);
      timerHandle = null;
    }
  };

  const startTimer = (fromTs: number) => {
    stopTimer();
    const tick = () => {
      // Вкладку могли закрыть среди обработки: строка выпала из документа,
      // но интернал держал бы её в памяти — гасим себя сами.
      if (!row.isConnected) {
        stopTimer();
        return;
      }
      timer.textContent = formatElapsed(Date.now() - fromTs);
    };
    tick();
    timerHandle = window.setInterval(tick, 1000);
  };

  const pulse = () => {
    if (waveEl) retrigger(waveEl, "pi-ping");
    retrigger(dot, "pi-flash");
  };

  return {
    el: row,
    setStatus(text: string) {
      if (text === lastStatusText) return;
      lastStatusText = text;
      status.textContent = text;
      retrigger(status, "pi-status-in");
      pulse();
    },
    pulse,
    show(visible: boolean) {
      row.classList.toggle("pi-hidden", !visible);
    },
    startTimer,
    stopTimer,
    destroy() {
      stopTimer();
      row.remove();
    },
  };
}
