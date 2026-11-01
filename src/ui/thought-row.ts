/// Компактная строка-действие агента в чате — замена общего блока «💭 Мысли (N)»
/// (PRP tasks/01.11.26 Компактные строки действий в чате.md).
///
/// SSOT разметки: это ЕДИНСТВЕННАЯ фабрика DOM строки-действия. Строить innerHTML
/// для мыслей/тулов вне этого модуля запрещено (chat.ts делегирует сюда).
/// Строка: `[иконка] автор/действие · summary (обрезан по ширине) · meta · статус · chevron`,
/// под строкой — панель деталей (полный текст мысли / args+result тула), скрытая
/// по умолчанию. Панель имеет потолок высоты с прокруткой колесом мыши
/// (.thought-row__detail в styles.css), поэтому длинное действие не раздувает чат.
///
/// Правило chevron (конструктив, не угадка): рисуется, если у строки ЕСТЬ детали
/// и (для мысли/сигнала) summary фактически не влезает в линию
/// (замер scrollWidth > clientWidth). У тула детали есть всегда → chevron всегда.
/// Если деталей вообще нет — chevron не рисуется. Фолбэк по длине текста —
/// только для окружения без layout (jsdom: размеры всегда 0), назван честно.

import type { ThoughtMenuCallbacks } from "../types";
import { confirmDialog } from "./confirm";

export type ThoughtRowKind = "thought" | "tool" | "subcall" | "signal";

export interface ThoughtRowOptions {
  kind: ThoughtRowKind;
  /** Иконка перед строкой; по умолчанию берётся из DEFAULT_ROW_ICONS по kind. */
  icon?: string;
  /** Автор действия (агент). Показывается, если не задан label. */
  author?: string;
  /** Текст действия (напр. "author → tool" или "Отчет от сабагента: …"). */
  label?: string;
  /** Короткий текст строки — обрезается по ширине, высоту не растягивает. */
  summary?: string;
  /** Детали для панели: полный текст мысли или args тула. */
  detail?: string;
  /** Мелкая мета справа (⏱ время и т.п.). */
  meta?: string;
  /** uid мысли из истории: наличие ⇒ у строки есть меню ⋮. */
  uid?: string;
  /** Ключ realtime-поиска результата тула (data-tool-key, "author:tool"). */
  toolKey?: string;
  /** ⏳ для realtime-вызова тула без результата. */
  pending?: boolean;
  /** Колбэки меню строки (только при наличии uid). */
  menuCallbacks?: ThoughtMenuCallbacks;
}

/** Иконки по имени тула; всё, что не в карте, получает дефолт 🔧. */
export const TOOL_ICONS: Record<string, string> = {
  excel_inspect: "📊",
  excel_read: "📊",
  excel_apply: "📊",
  excel_audit: "📊",
  excel_render: "📊",
  excel_convert: "📊",
  generate_image: "🎨",
  edit_image: "🎨",
  upscale_image: "🎨",
  image_usage: "🎨",
  generate_video: "🎬",
  todo_list: "📝",
  todo_write: "📝",
  emit_signal: "📡",
  batch_get_agent_report: "📥",
};

const DEFAULT_ROW_ICONS: Record<ThoughtRowKind, string> = {
  thought: "🧠",
  tool: "🔧",
  subcall: "📊",
  signal: "📡",
};

/** jsdom-фолбэк: без layout-движка scrollWidth/clientWidth всегда 0, поэтому
 *  «не влезает» решается по длине текста. Это фолбэк тестового окружения,
 *  а не «умный порог» — в браузере авторитетен замер переполнения. */
const JSDOM_SUMMARY_FALLBACK_CHARS = 120;

function summaryEl(row: HTMLDivElement): HTMLElement | null {
  return row.querySelector<HTMLElement>(".thought-row__summary");
}

function summaryOverflows(row: HTMLDivElement): boolean {
  const s = summaryEl(row);
  if (!s || !s.textContent) return false;
  if (s.scrollWidth > s.clientWidth) return true;
  if (s.scrollWidth === 0 && s.clientWidth === 0 && s.textContent.length > JSDOM_SUMMARY_FALLBACK_CHARS) return true;
  return false;
}

/** Пересчёт видимости chevron по факту переполнения/наличия деталей.
 *  Раскрытое состояние (thought-row--open) НЕ сбрасывается: chevron —
 *  индикатор «есть что развернуть», закрыть панель можно кликом по строке. */
function refreshToggle(row: HTMLDivElement): void {
  const detail = row.querySelector<HTMLElement>(".thought-row__detail");
  const toggle = row.querySelector<HTMLElement>(".thought-row__toggle");
  if (!detail || !toggle) return;
  const hasDetail = detail.childElementCount > 0;
  const isTool = row.classList.contains("thought-row--tool");
  const needed = hasDetail && (isTool || summaryOverflows(row));
  toggle.hidden = !needed;
}

function toggleRow(row: HTMLDivElement): void {
  const detail = row.querySelector<HTMLElement>(".thought-row__detail");
  if (!detail || detail.childElementCount === 0) return;
  const open = row.classList.toggle("thought-row--open");
  detail.hidden = !open;
}

/** Контент панели деталей по виду строки. */
function fillDetail(detail: HTMLDivElement, opts: ThoughtRowOptions): void {
  if (opts.kind === "tool") {
    if (opts.detail !== undefined && opts.detail !== "") {
      const args = document.createElement("div");
      args.className = "tool-thought-args";
      args.textContent = opts.detail;
      detail.appendChild(args);
    }
    return;
  }
  if (opts.detail) {
    const text = document.createElement("div");
    text.className = "thought-row__detail-text";
    text.textContent = opts.detail;
    detail.appendChild(text);
  }
}

/** Меню ⋮ строки из истории: «🗑️ Удалить мысль» и «📋 Клон до этого хода».
 *  Визуальный паттерн msg-menu-* из message-menu.ts, пункты — свои. */
function createRowMenu(uid: string, callbacks: ThoughtMenuCallbacks): HTMLDivElement {
  const wrapper = document.createElement("div");
  wrapper.className = "msg-menu-wrapper";
  const btn = document.createElement("button");
  btn.className = "msg-menu-btn";
  btn.type = "button";
  btn.innerHTML = "⋮";
  btn.title = "Действия";
  const dropdown = document.createElement("div");
  dropdown.className = "msg-menu-dropdown";
  const deleteItem = document.createElement("button");
  deleteItem.className = "msg-menu-item danger";
  deleteItem.type = "button";
  deleteItem.textContent = "🗑️ Удалить мысль";
  deleteItem.addEventListener("click", async (e) => {
    e.stopPropagation();
    dropdown.classList.remove("show");
    const confirmed = await confirmDialog(
      "Удаление мысли",
      "Вы уверены, что хотите удалить это действие из сессии?",
    );
    if (confirmed) {
      callbacks.onDeleteThoughts(null, [uid]);
    }
  });
  const cloneItem = document.createElement("button");
  cloneItem.className = "msg-menu-item";
  cloneItem.type = "button";
  cloneItem.textContent = "📋 Клон до этого хода";
  cloneItem.addEventListener("click", (e) => {
    e.stopPropagation();
    dropdown.classList.remove("show");
    callbacks.onCloneFromThoughts(uid);
  });
  dropdown.appendChild(deleteItem);
  dropdown.appendChild(cloneItem);
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    e.preventDefault();
    dropdown.classList.toggle("show");
  });
  wrapper.appendChild(btn);
  wrapper.appendChild(dropdown);
  return wrapper;
}

/** Главная фабрика: строит компактную строку-действие. */
export function createThoughtRow(opts: ThoughtRowOptions): HTMLDivElement {
  const row = document.createElement("div");
  row.className = `thought-row thought-row--${opts.kind}`;
  if (opts.uid) row.dataset.msgUid = opts.uid;
  if (opts.toolKey) row.dataset.toolKey = opts.toolKey;

  const line = document.createElement("div");
  line.className = "thought-row__line";

  const icon = document.createElement("span");
  icon.className = "thought-row__icon";
  icon.textContent = opts.icon ?? DEFAULT_ROW_ICONS[opts.kind];
  line.appendChild(icon);

  const labelText = opts.label ?? opts.author ?? "";
  const label = document.createElement("span");
  label.className = "thought-row__label";
  label.textContent = labelText;
  if (!labelText) label.hidden = true;
  line.appendChild(label);

  const summary = document.createElement("span");
  summary.className = "thought-row__summary";
  summary.textContent = opts.summary ?? "";
  line.appendChild(summary);

  if (opts.meta) {
    const meta = document.createElement("span");
    meta.className = "thought-row__meta";
    meta.textContent = opts.meta;
    line.appendChild(meta);
  }

  if (opts.pending) {
    const status = document.createElement("span");
    status.className = "thought-row__status";
    status.textContent = "⏳";
    line.appendChild(status);
  }

  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "thought-row__toggle";
  toggle.textContent = "▾";
  toggle.title = "Развернуть";
  toggle.tabIndex = -1;
  toggle.hidden = true;
  toggle.addEventListener("click", (e) => {
    e.stopPropagation();
    toggleRow(row);
  });
  line.appendChild(toggle);

  if (opts.uid && opts.menuCallbacks) {
    line.appendChild(createRowMenu(opts.uid, opts.menuCallbacks));
  }

  const detail = document.createElement("div");
  detail.className = "thought-row__detail";
  detail.hidden = true;
  fillDetail(detail, opts);

  // Клик по строке (кроме меню) — разворот/сворачивание панели деталей.
  row.addEventListener("click", (e) => {
    if ((e.target as HTMLElement).closest(".msg-menu-wrapper")) return;
    toggleRow(row);
  });

  row.appendChild(line);
  row.appendChild(detail);
  refreshToggle(row);
  return row;
}

/** Точечное обновление summary (стрим мысли): текст строки и панели, затем
 *  пересчёт chevron. Раскрытое состояние сохраняется, если панель уже открыта
 *  (юзер читает — пересоздание текста не должно хлопать панелью). */
export function updateThoughtRowSummary(row: HTMLDivElement, summary: string): void {
  const s = summaryEl(row);
  if (!s) return;
  s.textContent = summary;
  const detailText = row.querySelector<HTMLElement>(".thought-row__detail-text");
  if (detailText) detailText.textContent = summary;
  refreshToggle(row);
}

/** Завершение realtime-тула: ⏳ → ✅, результат дописывается в панель деталей. */
export function completeToolRow(row: HTMLDivElement, result: string): void {
  const status = row.querySelector<HTMLElement>(".thought-row__status");
  if (status) status.textContent = "✅";
  const detail = row.querySelector<HTMLElement>(".thought-row__detail");
  if (detail) {
    let res = detail.querySelector<HTMLElement>(".tool-thought-result");
    if (!res) {
      res = document.createElement("div");
      res.className = "tool-thought-result";
      detail.appendChild(res);
    }
    res.textContent = `→ ${result}`;
  }
  refreshToggle(row);
}
