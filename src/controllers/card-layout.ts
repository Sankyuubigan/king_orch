/**
 * Плотная упаковка карточек разделов (bin packing в 2 колонки).
 *
 * ЗАЧЕМ. Обычный CSS Grid привязан к рядам: короткая карточка занимает
 * строку целиком, и под ней остаётся дыра, пока высокая соседка тянет
 * высоту. CSS multi-column (masonry) дыр не оставляет, но раскладывает
 * элементы «сверху донизу» и сам решает, что во что попадёт: первые две
 * карточки (оба движка на странице «Движки») оказываются друг под другом.
 *
 * Здесь раскладка считается явно — жадно, в ту колонку, которая сейчас
 * ниже. Результат: две низкие карточки встают одна под другой рядом с
 * одной высокой, между рядами ровный gap, первые две карточки всегда
 * рядом (после первой вторая идёт в другую колонку — она там ниже).
 *
 * КАК. Координаты отдаются CSS через `grid-row`. В упакованном режиме
 * контейнер живёт с `grid-auto-rows: 1px` (styles.css, .cards-grid[data-packed]),
 * поэтому «строка» = 1px, а карточка высоты H с отступом gap занимает
 * `span (H + gap)` строк. Отступ не схлопывается, потому что карточка
 * выровнена по началу своей области (`align-items: start`), а не растянута.
 *
 * ДОМ НЕ ПЕРЕСТАВЛЯЕТСЯ. Карточки остаются внутри своих контейнеров —
 * это обязательно: ленивая материализация в tabs.ts определяет «уже
 * смонтировано» по наличию детей у [data-lazy-content], и перенос карточек
 * наружу сбросил бы этот счётчик и привёл к дубликатам. Меняются только
 * CSS-координаты.
 *
 * ЕДИНАЯ ЛОГИКА. Один и тот же раскладчик обслуживает Настройки, Движки и
 * тест-панели Студии — отличается только разметка.
 */
import { logFront } from "@my-tauri-plugins/plugin-logs";

const GRID_SELECTOR = ".cards-grid";
/** Ленивая обёртка: плагины монтируются в неё, сетка может быть как внутри
 *  неё (движки), так и вокруг неё (настройки) — наблюдаем оба случая. */
const LAZY_SELECTOR = "[data-lazy-content]";
/** Ширина, ниже которой раскладка выключается (та же точка, что в styles.css). */
const NARROW_QUERY = "(max-width: 980px)";

interface Measured {
  el: HTMLElement;
  /** Естественная высота карточки, px. */
  height: number;
  /** Карточка на всю ширину: отдельная полоса под обеими колонками. */
  full: boolean;
}

const grids = new Set<HTMLElement>();
/** Уже подписанные элементы: ResizeObserver/MutationObserver не любят дубли. */
const observed = new WeakSet<Element>();
const pending = new Set<HTMLElement>();

let frame = 0;
let started = false;
let ro: ResizeObserver | null = null;
let mo: MutationObserver | null = null;

/** Раскладка в одну колонку: пакить нечего, обычный поток уже без дыр. */
function isSingleColumn(): boolean {
  return (
    document.documentElement.dataset.cardLayout === "list" ||
    window.matchMedia(NARROW_QUERY).matches
  );
}

/**
 * Карточки контейнера в порядке DOM.
 * Два пути попадания в поток: прямые дети и дети ленивой обёртки
 * [data-lazy-content]. Обёртка растворена через display: contents, но в
 * DOM остаётся родителем — поэтому список строится по DOM-дереву вручную.
 */
function collectCards(grid: HTMLElement): HTMLElement[] {
  const cards: HTMLElement[] = [];
  for (const child of Array.from(grid.children)) {
    const node = child as HTMLElement;
    if (node.matches(LAZY_SELECTOR)) {
      for (const inner of Array.from(node.children)) cards.push(inner as HTMLElement);
    } else {
      cards.push(node);
    }
  }
  return cards;
}

/** Снимает раскладку — сетка возвращается в «запасной» режим из styles.css. */
function release(cards: HTMLElement[], grid: HTMLElement) {
  for (const el of cards) {
    el.style.gridColumn = "";
    el.style.gridRow = "";
  }
  grid.removeAttribute("data-packed");
}

function schedule(grid: HTMLElement) {
  pending.add(grid);
  if (frame) return;
  frame = requestAnimationFrame(() => {
    frame = 0;
    const queue = [...pending];
    pending.clear();
    for (const g of queue) pack(g);
  });
}

/**
 * Раскладывает одну сетку. Синхронная и идемпотентная: зовётся из
 * ResizeObserver, поэтому обязана сходиться за один проход.
 */
function pack(grid: HTMLElement) {
  const cards = collectCards(grid);
  if (cards.length === 0 || isSingleColumn()) {
    release(cards, grid);
    return;
  }

  // Замер в естественном потоке: сначала снимаем размещение — при
  // grid-auto-rows: 1px карточка без span схлопнулась бы в 1px, и мы
  // измерили бы не содержимое, а ничего.
  release(cards, grid);
  const measured: Measured[] = cards.map((el) => ({
    el,
    height: Math.ceil(el.getBoundingClientRect().height),
    full: el.dataset.span === "full",
  }));

  // Вертикальный зазор берём из самого CSS (пока сетка распакованная),
  // чтобы ширину раскладки и видимый отступ задавал один источник правды.
  const rowGap = parseFloat(getComputedStyle(grid).rowGap);
  const gap = Number.isFinite(rowGap) ? rowGap : 0;

  // Низ каждой колонки в px. Сетка из 1px-строк, поэтому координата — это
  // номер строки минус один.
  const bottom = [0, 0];

  for (const card of measured) {
    const step = card.height + gap;
    if (card.full) {
      // Полоса на всю ширину идёт ниже обеих колонок.
      const top = Math.max(bottom[0], bottom[1]);
      card.el.style.gridColumn = "1 / -1";
      card.el.style.gridRow = `${top + 1} / span ${step}`;
      bottom[0] = top + step;
      bottom[1] = top + step;
    } else {
      // Жадно — в самую низкую колонку. Первые две карточки автоматически
      // разводятся по разным колонкам: после первой вторая ниже, чем она.
      const col = bottom[0] <= bottom[1] ? 0 : 1;
      card.el.style.gridColumn = String(col + 1);
      card.el.style.gridRow = `${bottom[col] + 1} / span ${step}`;
      bottom[col] += step;
    }
  }

  // Флаг ставим последним: пока координаты есть не у всех карточек, он
  // схлопнул бы «лишние» в 1px.
  grid.setAttribute("data-packed", "");
}

/** Подписывает новые сетки и их карточки на наблюдение. */
function sync(root: ParentNode = document) {
  for (const grid of root.querySelectorAll<HTMLElement>(GRID_SELECTOR)) grids.add(grid);

  if (!ro) return;
  for (const grid of grids) {
    // Сама сетка — ленивый монтаж добавляет карточки и меняет её высоту.
    if (!observed.has(grid)) {
      observed.add(grid);
      ro.observe(grid);
    }
    // Ленивая обёртка приходит в сетку в двух ипостасях: в неё (движки) или
    // она в неё (настройки). Подписываем оба — смена детей любой из них
    // означает «набор карточек изменился».
    for (const box of [grid, ...grid.querySelectorAll<HTMLElement>(LAZY_SELECTOR)]) {
      if (!observed.has(box)) {
        observed.add(box);
        mo?.observe(box, { childList: true });
      }
    }
    // Карточки: панели плагинов подгружают каталоги асинхронно и меняют
    // высоту уже после первой раскладки.
    for (const card of collectCards(grid)) {
      if (!observed.has(card)) {
        observed.add(card);
        ro.observe(card);
      }
    }
  }
}

function repackAll() {
  sync();
  for (const grid of grids) schedule(grid);
}

/**
 * Запуск наблюдателей. Один раз из main.ts после сборки UI.
 * До старта сетки живут в запасном режиме — разметка от JS не зависит.
 */
export function initCardLayout() {
  if (started) return;
  started = true;

  if ("ResizeObserver" in window) {
    ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const target = entry.target as HTMLElement;
        if (target.matches(GRID_SELECTOR)) {
          schedule(target);
        } else {
          const owner = target.closest<HTMLElement>(GRID_SELECTOR);
          if (owner) schedule(owner);
        }
      }
      // Новые карточки могли приехать вместе с resize их сетки.
      sync();
    });
  }

  if ("MutationObserver" in window) {
    mo = new MutationObserver((records) => {
      for (const rec of records) {
        const target = rec.target as HTMLElement;
        const owner = target.closest<HTMLElement>(GRID_SELECTOR);
        // Обёртка движков — предок самой сетки, closest() даст null;
        // repackAll() ниже подхватит сетку, которая появилась внутри.
        if (owner) schedule(owner);
      }
      repackAll();
    });
  }

  window.addEventListener("resize", repackAll);
  window.matchMedia(NARROW_QUERY).addEventListener("change", repackAll);
  // Смена режима «Сетка / Список» ставится атрибутом на <html> (SettingsController).
  new MutationObserver(repackAll).observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-card-layout"],
  });

  repackAll();
  logFront("[card-layout] bin-packing инициализирован");
}

/** Переупаковать сетки по запросу (после смены числа карточек в разделе). */
export function refreshCardLayout() {
  if (!started) return;
  repackAll();
}