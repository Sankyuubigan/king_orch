import type { GraphController } from "./graph-class";

// ─── Зум холста колесом мыши ───
//
// Контракт колеса принадлежит приложению, а не библиотеке. Корень бага
// «колесо не зумит холст»: зум был делегирован внутреннему обработчику
// drawflow, который в 0.0.60 зумит только с зажатым Ctrl
// (`zoom_enter: e.ctrlKey && ...`), а обычное колесо над холстом молча
// ничего не делает. До переезда на drawflow холст зумил обычным колесом
// (vis-network `zoomView`), поэтому «колесо = зум» — ожидаемое поведение.
//
// Теперь ЛЮБОЕ вертикальное колесо над холстом зумит: обычное и с Ctrl
// (второе заодно гасит страничный зум WebView2). Слушатель висит capture-фазой
// на РОДИТЕЛЕЕ контейнера drawflow и останавливает событие до библиотечного
// слушателя — иначе на Ctrl+wheel сработали бы оба зума (у drawflow слушатель
// на самом контейнере, и перехватить его иначе нельзя: ссылка на bound-функцию
// не экспортируется).

export function setupCanvasZoom(this: GraphController): void {
  if (!this.editor) return;
  const editor = this.editor;
  const container = this.el.graphContainer;
  // Capture предков идёт до слушателей самого контейнера — гарантированный
  // порядок относительно drawflow. Фолбэк на контейнер — для тестовых DOM,
  // где контейнер может быть без родителя.
  const wheelHost = container.parentElement ?? container;

  wheelHost.addEventListener("wheel", (e: WheelEvent) => {
    if (this.editor !== editor) return;
    // Горизонтальное колесо / тачпад-свайп зуму не сигнал.
    if (e.deltaY === 0) return;

    const rect = container.getBoundingClientRect();
    const cx = e.clientX - rect.left;
    const cy = e.clientY - rect.top;
    const oldZoom = editor.zoom;
    // Сетка зума детерминирована: без округления серия шагов накапливает
    // float-хвост (1 + 0.1*N → 1.2000000000000002), и на границе zoom_min
    // возможен лишний микрошаг вместо честного упора.
    const snap = (v: number) => Math.round(v * 1e6) / 1e6;
    const stepped = snap(e.deltaY < 0 ? oldZoom + editor.zoom_value : oldZoom - editor.zoom_value);
    const nextZoom = Math.min(editor.zoom_max, Math.max(editor.zoom_min, stepped));

    // Колесо над холстом не скроллит страницу и не зумит WebView2 — даже на
    // границе диапазона, где зум меняться нечему.
    e.preventDefault();
    if (nextZoom === oldZoom) return;

    // precanvas масштабируется вокруг своего центра (дефолтный transform-origin
    // 50% 50%): экран = rect.left + W/2*(1-zoom) + canvas_x + wx*zoom.
    // Считаем мир под курсором и держим его под курсором на новом зуме.
    const worldX = (cx - rect.width / 2 * (1 - oldZoom) - editor.canvas_x) / oldZoom;
    const worldY = (cy - rect.height / 2 * (1 - oldZoom) - editor.canvas_y) / oldZoom;

    editor.zoom = nextZoom;
    editor.zoom_last_value = nextZoom; // инвариант drawflow: равен zoom между зумами
    editor.canvas_x = snap(cx - rect.width / 2 * (1 - nextZoom) - worldX * nextZoom);
    editor.canvas_y = snap(cy - rect.height / 2 * (1 - nextZoom) - worldY * nextZoom);
    editor.precanvas.style.transform =
      `translate(${editor.canvas_x}px, ${editor.canvas_y}px) scale(${nextZoom})`;
    editor.dispatch("zoom", nextZoom);
    e.stopPropagation();
  }, { capture: true, passive: false });
}
