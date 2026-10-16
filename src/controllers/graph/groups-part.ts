import { showToast } from "../../ui";
import { logFront } from "@my-tauri-plugins/plugin-logs";
import type { GraphGroupDef } from "./types";
import type { GraphController } from "./graph-class";

/** Отступ рамки группы от габаритов её нод. */
const FRAME_PADDING = 28;
/** Высота полосы заголовка рамки. */
const HEADER_HEIGHT = 24;
/** Габариты пустой группы (её ещё не наполнили нодами и `ui_pos` не задан). */
const EMPTY_GROUP_W = 260;
const EMPTY_GROUP_H = 120;
/** Размер ноды, когда DOM ещё не отрисован (import, юнит-тесты). */
const FALLBACK_NODE_W = 200;
const FALLBACK_NODE_H = 90;
/** Минимальный размер рамки при растяжении: меньше — в неё нельзя положить ноду. */
const MIN_FRAME_W = 160;
const MIN_FRAME_H = 120;
/**
 * Направления растяжения и стороны, на которых они меняют размер.
 *
 * `w`/`e` трогают только ширину, `n`/`s` — высоту, углы — обе. Сторона, указанная
 * в имени, двигает соответствующий край рамки (и вместе с ним `ui_pos`, если
 * растягивают левый/верхний край).
 */
export const RESIZE_DIRS = [
  { dir: "nw", dx: -1, dy: -1 },
  { dir: "n", dx: 0, dy: -1 },
  { dir: "ne", dx: 1, dy: -1 },
  { dir: "e", dx: 1, dy: 0 },
  { dir: "se", dx: 1, dy: 1 },
  { dir: "s", dx: 0, dy: 1 },
  { dir: "sw", dx: -1, dy: 1 },
  { dir: "w", dx: -1, dy: 0 },
] as const;

export type ResizeDir = (typeof RESIZE_DIRS)[number]["dir"];

// ─── Чтение состояния ────────────────────────────────────────────────────────

/** Группа по id. */
export function groupById(this: GraphController, groupId: string): GraphGroupDef | undefined {
  return this.currentGroups.find((g) => g.id === groupId);
}

/** id всех нод группы — единственный источник «кто входит в рамку». */
export function groupNodeIds(this: GraphController, groupId: string): string[] {
  if (!this.editor) return [];
  const data = this.editor.drawflow.drawflow.Home.data;
  return Object.keys(data).filter((id) => data[id]?.data?.group === groupId);
}

/**
 * Габариты рамки группы в координатах холста.
 *
 * Приоритет ровно один, и он документирован здесь:
 *
 *   1. `ui_pos` + `ui_size` — автор растянул рамку, геометрией владеет он;
 *   2. иначе bbox нод-участников — рамка следует за содержимым сама;
 *   3. иначе `ui_pos` + `EMPTY_GROUP_*` — пустая группа, которой неоткуда взять bbox;
 *   4. иначе `null` — рисовать нечего (группа видна только в списке сайдбара).
 *
 * Раньше приоритетов было два, и пункт 1 вырождался в `ui_pos` без размера: рамку
 * было нельзя сделать просторнее содержимого вообще («очень тесто»). Теперь
 * растяжение пишет явную пару `ui_pos`+`ui_size`, а `fitGroupToNodes` её удаляет.
 */
export function groupFrameRect(
  this: GraphController,
  group: GraphGroupDef,
): { x: number; y: number; w: number; h: number } | null {
  if (group.ui_pos && group.ui_size) {
    return {
      x: group.ui_pos.x,
      y: group.ui_pos.y,
      w: Math.max(MIN_FRAME_W, group.ui_size.w),
      h: Math.max(MIN_FRAME_H, group.ui_size.h),
    };
  }
  const ids = this.groupNodeIds(group.id);
  if (ids.length === 0) {
    const pos = group.ui_pos;
    if (!pos) return null;
    return { x: pos.x, y: pos.y, w: EMPTY_GROUP_W, h: EMPTY_GROUP_H };
  }
  const data = this.editor!.drawflow.drawflow.Home.data;
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const id of ids) {
    const dn = data[id];
    if (!dn) continue;
    const size = this.nodeSize(id);
    if (dn.pos_x < minX) minX = dn.pos_x;
    if (dn.pos_y < minY) minY = dn.pos_y;
    if (dn.pos_x + size.w > maxX) maxX = dn.pos_x + size.w;
    if (dn.pos_y + size.h > maxY) maxY = dn.pos_y + size.h;
  }
  if (!isFinite(minX)) return null;
  return {
    x: minX - FRAME_PADDING,
    y: minY - FRAME_PADDING - HEADER_HEIGHT,
    w: maxX - minX + FRAME_PADDING * 2,
    h: maxY - minY + FRAME_PADDING * 2 + HEADER_HEIGHT,
  };
}

/**
 * Закреплена ли рамка (автор задал ей геометрию сам).
 *
 * Из этого следует разница в поведении: производная рамка едет за нодами сама,
 * закреплённая — стоит на месте, пока её не сдвинет или не растянет автор.
 */
export function isFramePinned(
  this: GraphController,
  group: GraphGroupDef,
): boolean {
  return !!(group.ui_pos && group.ui_size);
}

/**
 * Вернуть рамку к содержимому: удалить явную геометрию.
 *
 * Обратный ход к растяжению — иначе «случайно растянул» нечем отменить, кроме
 * как вручную вытаскивать ноды обратно.
 */
export function fitGroupToNodes(this: GraphController, groupId: string): boolean {
  const group = this.groupById(groupId);
  if (!group || !group.ui_pos && !group.ui_size) return false;
  this.saveCheckpoint();
  delete group.ui_pos;
  delete group.ui_size;
  this.refreshGroupFrames([groupId]);
  this.markDirty();
  return true;
}

/**
 * Размер ноды для расчёта рамки.
 *
 * Из DOM читается ОДИН раз и кэшируется: `getBoundingClientRect` на каждый
 * mousemove заставляет браузер пересчитывать layout (69 нод × 60 fps — реальный
 * лаг, и он не нужен: Drawflow и так знает координаты). Кэш инвалидируется в
 * `updateNodeHtml` и при полной перерисовке рамок.
 */
export function nodeSize(this: GraphController, nodeId: string): { w: number; h: number } {
  const cached = this.nodeSizeCache.get(nodeId);
  if (cached) return cached;
  let size = { w: FALLBACK_NODE_W, h: FALLBACK_NODE_H };
  const el = this.findNodeEl(nodeId);
  if (el) {
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    if (w > 0 && h > 0) size = { w, h };
  }
  this.nodeSizeCache.set(nodeId, size);
  return size;
}

export function invalidateNodeSize(this: GraphController, nodeId: string): void {
  this.nodeSizeCache.delete(nodeId);
}

// ─── Отрисовка ───────────────────────────────────────────────────────────────

/**
 * Контейнер рамок внутри `precanvas`: наследует его `transform`, поэтому рамки
 * корректны при pan/zoom без дублирования состояния вьюпорта.
 */
export function ensureGroupLayer(this: GraphController): HTMLDivElement | null {
  if (!this.editor) return null;
  const precanvas = this.editor.precanvas as unknown as HTMLElement;
  // В тестовых харнессах `precanvas` — простой объект вместо DOM-узла. Рамкам там
  // негде лежать, и это НЕ тишина: сценарий без рамок всё равно работает, данные
  // групп живут в `currentGroups` и уезжают в YAML через `serializeGroups`.
  if (typeof precanvas?.querySelector !== "function") return null;
  let layer = precanvas.querySelector(":scope > .graph-group-layer") as HTMLDivElement | null;
  if (!layer) {
    layer = document.createElement("div");
    layer.className = "graph-group-layer";
    // z-index:1 (см. styles.css) — выше svg-соединений Drawflow (z-index:0) и
    // ниже нод (z-index:2). Вставка первым child нужна только для порядка
    // отрисовки внутри одного z-index и на стек рамок не влияет.
    precanvas.insertBefore(layer, precanvas.firstChild);
  }
  return layer;
}

/** DOM-элемент одной рамки. */
export function buildGroupFrame(this: GraphController, group: GraphGroupDef): HTMLDivElement | null {
  const rect = this.groupFrameRect(group);
  if (!rect) return null;
  const el = document.createElement("div");
  el.className = "graph-group";
  el.dataset.groupId = group.id;
  el.style.left = `${rect.x}px`;
  el.style.top = `${rect.y}px`;
  el.style.width = `${rect.w}px`;
  el.style.height = `${rect.h}px`;

  const status = this.groupFusedStatus(group, this.currentWorkflowStrategy);
  const header = document.createElement("div");
  header.className = "graph-group-header";
  const badge = status.fusable
    ? `<span class="graph-group-badge${status.active ? "" : " is-idle"}" title="${this.esc(status.text)}">⚡</span>`
    : "";
  header.innerHTML =
    `<span class="graph-group-icon">📦</span>` +
    `<span class="graph-group-title">${this.esc(group.title || group.id)}</span>` +
    badge +
    `<span class="graph-group-count">${this.groupNodeIds(group.id).length} нод</span>`;
  header.title = status.text;
  el.appendChild(header);

  // Маркеры растяжения: рамку иначе нечем сделать просторнее содержимого.
  for (const { dir } of RESIZE_DIRS) {
    const handle = document.createElement("div");
    handle.className = "graph-group-resize";
    handle.dataset.dir = dir;
    handle.dataset.groupId = group.id;
    el.appendChild(handle);
  }
  return el;
}

/**
 * Проблемы схлопывания этой группы по ВЕРДИКТУ БЭКЕНДА.
 *
 * Форма макро-ноды — серверная семантика (одна цепочка, один вход, один выход,
 * без роутеров внутри), и дублировать её в TypeScript означало бы завести
 * вторую правду, которая разойдётся с движком при первом же изменении правил.
 * Поэтому редактор показывает РОВНО то, что вернул `read_workflow_file`
 * (`GROUP_FUSED_*`), ничего не решая сам.
 */
export function groupFusedProblems(this: GraphController, groupId: string): string[] {
  const marker = `groups[id=${groupId}]`;
  return (this.yamlDiagnostics || [])
    .filter((d) => d.code.startsWith("GROUP_FUSED_") && d.location.startsWith(marker))
    .map((d) => d.message);
}

/**
 * Что группа будет делать в текущем режиме графа — одна строка для заголовка
 * рамки и для инспектора.
 *
 * Прямая функция от `(group, strategy)`, а не разбросанные по UI условия:
 * подсказка на холсте и подсказка в сайдбаре обязаны говорить одно и то же, иначе
 * одна из них соврёт. Значения `strategy` повторяют `groups::parse_strategy`
 * (`detailed` по умолчанию, `fused` — единственный второй режим; `auto` не бывает).
 */
export function groupFusedStatus(
  this: GraphController,
  group: GraphGroupDef,
  strategy: string | null,
): { fusable: boolean; active: boolean; level: "off" | "on" | "warn"; text: string } {
  const instruction = (group.fused_instruction || "").trim();
  const tools = group.tools || [];
  if (!instruction) {
    return {
      fusable: false,
      active: false,
      level: "off",
      text: "Инструкция FUSED не задана — группа работает только в режиме Detailed.",
    };
  }
  if ((strategy || "detailed") !== "fused") {
    return {
      fusable: true,
      active: false,
      level: "off",
      text: "Граф в режиме Detailed: группа исполняется нода по ноде. Чтобы она схлопывалась, включите режим Fused в чате или поставьте `strategy: fused` в файле графа.",
    };
  }
  if (tools.length > 0) {
    return {
      fusable: true,
      active: false,
      level: "warn",
      text: `Режим Fused, но заданы инструменты (${tools.join(", ")}): схлопывание с инструментами пока не поддержано, прогон даст явную ошибку.`,
    };
  }
  return {
    fusable: true,
    active: true,
    level: "on",
    text: "Режим Fused: группа выполняется одним вызовом LLM по инструкции выше.",
  };
}

/**
 * Полная перерисовка всех рамок.
 *
 * Вызывается из обработчика `"import"` — единственного хука, через который
 * проходят И открытие файла, И отмена/повтор. Drawflow при этом чистит
 * `precanvas.innerHTML`, поэтому слой рамок приходится пересоздавать.
 */
export function renderGroups(this: GraphController): void {
  const layer = this.ensureGroupLayer();
  if (!layer) return;
  this.nodeSizeCache.clear();
  layer.innerHTML = "";
  // Рамки строятся после кадра: только сейчас доступны реальные offsetWidth/offsetHeight нод.
  requestAnimationFrame(() => {
    const liveLayer = this.ensureGroupLayer();
    if (!liveLayer) return;
    liveLayer.innerHTML = "";
    for (const group of this.currentGroups) {
      const el = this.buildGroupFrame(group);
      if (el) liveLayer.appendChild(el);
    }
  });
}

/** Перерисовать рамки указанных групп (или все, если список пуст). */
export function refreshGroupFrames(this: GraphController, groupIds?: string[]): void {
  const layer = this.ensureGroupLayer();
  if (!layer) return;
  const targets = groupIds && groupIds.length > 0 ? groupIds : this.currentGroups.map((g) => g.id);
  for (const id of targets) {
    const group = this.groupById(id);
    if (!group) continue;
    const existing = layer.querySelector(
      `.graph-group[data-group-id="${cssEscapeAttr(id)}"]`,
    ) as HTMLElement | null;
    if (!existing) {
      const el = this.buildGroupFrame(group);
      if (el) layer.appendChild(el);
      continue;
    }
    const rect = this.groupFrameRect(group);
    if (!rect) {
      existing.remove();
      continue;
    }
    existing.style.left = `${rect.x}px`;
    existing.style.top = `${rect.y}px`;
    existing.style.width = `${rect.w}px`;
    existing.style.height = `${rect.h}px`;
    const count = existing.querySelector(".graph-group-count");
    if (count) count.textContent = `${this.groupNodeIds(id).length} нод`;
  }
}

function cssEscapeAttr(value: string): string {
  return value.replace(/["\\]/g, "\\$&");
}

// ─── Хит-тест ────────────────────────────────────────────────────────────────

/**
 * Какая рамка под точкой холста. Возвращает группу самой маленькой рамки: при
 * вложенных рамках бросок должен попасть в маленькую, как в редакторах.
 */
export function groupAtPoint(
  this: GraphController,
  canvasX: number,
  canvasY: number,
): GraphGroupDef | null {
  let best: GraphGroupDef | null = null;
  let bestArea = Infinity;
  for (const group of this.currentGroups) {
    const rect = this.groupFrameRect(group);
    if (!rect) continue;
    if (canvasX < rect.x || canvasX > rect.x + rect.w) continue;
    if (canvasY < rect.y || canvasY > rect.y + rect.h) continue;
    const area = rect.w * rect.h;
    if (area < bestArea) {
      best = group;
      bestArea = area;
    }
  }
  return best;
}

// ─── Мутации ─────────────────────────────────────────────────────────────────

/** Уникальный id группы: `intake_group`, `intake_group_2`, … */
export function generateGroupId(this: GraphController, base: string): string {
  const clean = (base || "group").trim().replace(/[^a-zA-Z0-9_]+/g, "_").replace(/^_+|_+$/g, "") || "group";
  if (!this.groupById(clean)) return clean;
  let n = 2;
  while (this.groupById(`${clean}_${n}`)) n++;
  return `${clean}_${n}`;
}

/**
 * Создать группу под курсором.
 *
 * Имя не спрашивается нативным диалогом: группа создаётся сразу и открывается
 * в инспекторе, где имя и инструкция правятся обычными полями — как все
 * остальные свойства в этом редакторе.
 */
export function createGroup(this: GraphController, clientX: number, clientY: number): GraphGroupDef {
  this.ensureEditor();
  const id = this.generateGroupId(`group_${this.currentGroups.length + 1}`);
  const zoom = this.editor!.zoom || 1;
  const preRect = this.editor!.precanvas.getBoundingClientRect();
  const x = Math.round((clientX - preRect.left) / zoom - EMPTY_GROUP_W / 2);
  const y = Math.round((clientY - preRect.top) / zoom - EMPTY_GROUP_H / 2);

  this.saveCheckpoint();
  const group: GraphGroupDef = {
    id,
    title: "Новая группа",
    ui_pos: { x, y },
  };
  this.currentGroups.push(group);
  this.refreshGroupFrames([id]);
  this.showGroupEditor(id);
  this.markDirty();
  return group;
}

/**
 * Удалить группу. Ноды при этом СОХРАНЯЮТСЯ и просто выходят из неё: автор удалил
 * рамку, а не содержимое. Молча удалять вместе с ней работу — потеря без спроса.
 */
export function deleteGroup(this: GraphController, groupId: string): void {
  if (!this.groupById(groupId)) return;
  this.saveCheckpoint();
  const orphans = this.groupNodeIds(groupId);
  for (const nodeId of orphans) {
    const dn = this.editor?.drawflow.drawflow.Home.data[nodeId];
    if (dn) delete dn.data.group;
  }
  this.currentGroups = this.currentGroups.filter((g) => g.id !== groupId);
  this.refreshGroupFrames();
  for (const nodeId of orphans) this.updateNodeHtml(nodeId);
  this.markDirty();
  showToast(`📦 Группа удалена, ${orphans.length} нод осталось на холсте`, "success");
}

/**
 * Переименовать группу. Тот же контракт, что у `renameNode` у нод: «id ноды,
 * на которую ссылается сторонняя таблица» обязан уехать вместе с переименованием,
 * иначе ссылки `nodes[].group` осиротеют.
 */
export function renameGroup(this: GraphController, oldId: string, newId: string): boolean {
  const group = this.groupById(oldId);
  if (!group || !newId || newId === oldId) return false;
  if (this.groupById(newId)) return false;
  this.saveCheckpoint();
  group.id = newId;
  for (const nodeId of this.groupNodeIds(oldId)) {
    const dn = this.editor?.drawflow.drawflow.Home.data[nodeId];
    if (dn) dn.data.group = newId;
    this.updateNodeHtml(nodeId);
  }
  this.renderGroups();
  this.markDirty();
  return true;
}

/** Привязать ноду к группе (или отвязать, если `groupId` пуст). */
export function assignNodeToGroup(
  this: GraphController,
  nodeId: string,
  groupId: string | null,
): void {
  const dn = this.editor?.drawflow.drawflow.Home.data[nodeId];
  if (!dn) return;
  const previous = typeof dn.data.group === "string" ? dn.data.group : null;
  if ((groupId || null) === previous) return;
  this.saveCheckpoint();
  if (groupId) dn.data.group = groupId;
  else delete dn.data.group;
  this.updateNodeHtml(nodeId);
  this.refreshGroupFrames([previous, groupId].filter((v): v is string => !!v));
  this.markDirty();
}

/**
 * Нода перетащена — пересчитать рамку её группы и определить, куда её бросили.
 *
 * Привязка происходит ПО ОТПУСКАНИЮ, а не на каждом движении мыши: рамка
 * обводит ноду сразу (иначе бросок «на глаз» не попадёт), а менять состав группы
 * в процессе перетаскивания значило бы перекраивать её на лету.
 *
 * Бросок вне всех рамок членство НЕ снимает: молча выбрасывать ноду из группы
 * за то, что автор её сдвинул, — потеря работы без запроса.
 */
export function onNodeMoved(this: GraphController, nodeId: string): void {
  const dn = this.editor?.drawflow.drawflow.Home.data[nodeId];
  if (!dn) return;
  this.invalidateNodeSize(nodeId);
  const previous = typeof dn.data.group === "string" ? dn.data.group : null;
  if (previous) this.refreshGroupFrames([previous]);

  const size = this.nodeSize(nodeId);
  const centerX = dn.pos_x + size.w / 2;
  const centerY = dn.pos_y + size.h / 2;
  const target = this.groupAtPoint(centerX, centerY);
  const next = target ? target.id : previous;
  if (next !== previous) {
    this.assignNodeToGroup(nodeId, next);
  }
}

/**
 * Клик по рамке без перетаскивания — открыть инспектор группы.
 *
 * Заголовок и тело обрабатываются одним и тем же вызовом: различать их незачем,
 * потому что перетаскивание отсекается раньше (mousedown по заголовку уходит в
 * `beginGroupDrag` и до Drawflow не доходит) — то есть сюда попадают только
 * настоящие клики.
 */
export function onGroupFrameClick(this: GraphController, groupId: string): void {
  this.showGroupEditor(groupId);
}

// ─── Кто лежит под рамкой ───────────────────────────────────────────────────

/**
 * Элемент, который оказался бы под курсором, если бы рамки не было.
 *
 * Рамка стоит НАД соединениями (`z-index:1` против svg `z-index:0`), поэтому
 * наивный `pointer-events:auto` на теле рамки сделал бы невидимыми все рёбра
 * внутри группы: и Drawflow (`click()` смотрит на `e.target`), и наш
 * `setupEdgeDrag` (`edge-part.ts:89`) ищут `.main-path` именно в цели события.
 *
 * Вместо угадывания по геометрии спрашиваем у браузера: на миг выключаем
 * hit-test слоя рамок (`pointer-events:none` вместо `visibility`), берём
 * `document.elementFromPoint` и возвращаем всё как было. Ответ — точный, а не
 * «кажется, рядом была линия».
 */
export function interactiveUnderGroup(
  this: GraphController,
  clientX: number,
  clientY: number,
): Element | null {
  const layer = this.ensureGroupLayer();
  if (!layer) return null;
  const previous = layer.style.pointerEvents;
  layer.style.pointerEvents = "none";
  let under: Element | null = null;
  try {
    // В браузере метод есть всегда; отсутствие — среда без hit-test (jsdom),
    // и тогда «под рамкой ничего» — правильный ответ: клик остаётся у рамки.
    if (typeof document.elementFromPoint === "function") {
      under = document.elementFromPoint(clientX, clientY);
    }
  } finally {
    layer.style.pointerEvents = previous;
  }
  if (!(under instanceof Element)) return null;
  return under.closest(".connection") ? under : null;
}

/**
 * Пробросить `mousedown` элементу под рамкой.
 *
 * Событие помечается `__groupFrameForward`, чтобы наш же обработчик клика не
 * открыл инспектор группы поверх только что выбранного ребра.
 */
export function forwardMousedownTo(
  this: GraphController,
  target: Element,
  ev: MouseEvent,
): void {
  const forwarded = new MouseEvent("mousedown", {
    bubbles: true,
    cancelable: true,
    button: ev.button,
    clientX: ev.clientX,
    clientY: ev.clientY,
  });
  (forwarded as unknown as Record<string, unknown>).__groupFrameForward = true;
  target.dispatchEvent(forwarded);
}

// ─── Перетаскивание и растяжение ─────────────────────────────────────────────

/**
 * Перетаскивание группы за заголовок: двигаем все её ноды разом.
 *
 * Смещение ВСЕГДА абсолютное — от точки захвата (`startPos + dx/zoom`). Раньше
 * здесь стояло `dn.pos_x += dx / zoom`, где `dx` уже был суммарным смещением от
 * начала жеста: каждый кадр прибавлял к результату предыдущие кадры, и группа
 * улетала тем дальше, чем дольше её тянули. Drawflow так не разъезжается —
 * он пишет позицию целиком (`offsetLeft - delta`), и ровно поэтому обычные ноды
 * вели себя правильно.
 *
 * Размеры нод не трогаем: двигается МЕСТО, содержимое рамки едет вместе с ней.
 * Закреплённая рамка (`ui_pos`+`ui_size`) едет вместе с нодами, производная —
 * следует за bbox сама.
 */
export function beginGroupDrag(
  this: GraphController,
  groupId: string,
  ev: MouseEvent,
): void {
  const start = { x: ev.clientX, y: ev.clientY };
  const group = this.groupById(groupId);
  const data = this.editor?.drawflow.drawflow.Home.data;
  if (!data) return;
  const ids = this.groupNodeIds(groupId);
  const startPos = new Map<string, { x: number; y: number }>();
  for (const id of ids) {
    const dn = data[id];
    if (dn) startPos.set(id, { x: dn.pos_x, y: dn.pos_y });
  }
  // Точка отсчёта для закреплённой рамки — её нынешний угол, а не bbox нод.
  const pinned = !!group && this.isFramePinned(group);
  const startFramePos = group?.ui_pos ? { x: group.ui_pos.x, y: group.ui_pos.y } : null;

  let started = false;
  this.draggingGroupId = groupId;
  const container = this.el.graphContainer;

  const onMove = (e: MouseEvent) => {
    const dx = e.clientX - start.x;
    const dy = e.clientY - start.y;
    if (!started) {
      if (Math.abs(dx) + Math.abs(dy) < 3) return;
      started = true;
      this.saveCheckpoint();
    }
    const zoom = this.editor?.zoom || 1;
    for (const [id, pos] of startPos) {
      const dn = data[id];
      if (!dn) continue;
      dn.pos_x = pos.x + dx / zoom;
      dn.pos_y = pos.y + dy / zoom;
      const el = this.findNodeEl(id);
      if (el) {
        el.style.left = `${dn.pos_x}px`;
        el.style.top = `${dn.pos_y}px`;
      }
      this.editor!.updateConnectionNodes("node-" + id);
    }
    if (pinned && group && startFramePos) {
      group.ui_pos = {
        x: Math.round(startFramePos.x + dx / zoom),
        y: Math.round(startFramePos.y + dy / zoom),
      };
    }
    this.refreshGroupFrames([groupId]);
  };

  const onUp = () => {
    this.draggingGroupId = null;
    container.removeEventListener("mousemove", onMove);
    container.removeEventListener("mouseup", onUp);
    if (started) {
      logFront(`[groups] группа '${groupId}' перемещена`);
      this.saveCheckpoint();
      this.markDirty();
    }
  };

  container.addEventListener("mousemove", onMove);
  container.addEventListener("mouseup", onUp);
}

/**
 * Растяжение рамки маркером.
 *
 * Двигает ТОЛЬКО рамку: ноды внутри остаются на месте (frame в Figma/Blender),
 * и освободившееся место занимают новые ноды. Жест начинается на текущем прямоугольнике
 * (`groupFrameRect`), каждое движение пишет абсолютные `ui_pos`/`ui_size`, поэтому
 * размер не «накапливается», как это было с перетаскиванием.
 *
 * Один `saveCheckpoint` на весь жест — иначе Ctrl+Z отматывал бы по одному кадру.
 */
export function beginGroupResize(
  this: GraphController,
  groupId: string,
  dir: ResizeDir,
  ev: MouseEvent,
): void {
  const group = this.groupById(groupId);
  const start = this.groupFrameRect(group ?? ({ id: groupId } as GraphGroupDef));
  if (!group || !start) return;
  const axis = RESIZE_DIRS.find((d) => d.dir === dir);
  if (!axis) return;

  const origin = { x: ev.clientX, y: ev.clientY };
  this.saveCheckpoint();
  this.draggingGroupId = groupId;

  const onMove = (e: MouseEvent) => {
    const zoom = this.editor?.zoom || 1;
    const dx = (e.clientX - origin.x) / zoom;
    const dy = (e.clientY - origin.y) / zoom;
    let { x, y, w, h } = start;
    if (axis.dx !== 0) {
      w = Math.max(MIN_FRAME_W, start.w + axis.dx * dx);
      // Левый край тянет за собой левую границу рамки.
      if (axis.dx < 0) x = start.x + (start.w - w);
    }
    if (axis.dy !== 0) {
      h = Math.max(MIN_FRAME_H, start.h + axis.dy * dy);
      if (axis.dy < 0) y = start.y + (start.h - h);
    }
    group.ui_pos = { x: Math.round(x), y: Math.round(y) };
    group.ui_size = { w: Math.round(w), h: Math.round(h) };
    this.refreshGroupFrames([groupId]);
  };

  const onUp = () => {
    this.draggingGroupId = null;
    container.removeEventListener("mousemove", onMove);
    container.removeEventListener("mouseup", onUp);
    logFront(`[groups] рамка группы '${groupId}' растянута (${dir})`);
    this.markDirty();
  };

  const container = this.el.graphContainer;
  container.addEventListener("mousemove", onMove);
  container.addEventListener("mouseup", onUp);
}

// ─── Сериализация ────────────────────────────────────────────────────────────

/** Группы для `save_workflow`. Пустые группы сохраняются: рамка должна пережить перезапуск. */
export function serializeGroups(this: GraphController): GraphGroupDef[] {
  return JSON.parse(JSON.stringify(this.currentGroups));
}

/** Загрузка групп из прочитанного файла. */
export function loadGroups(
  this: GraphController,
  groups: GraphGroupDef[] | null | undefined,
): void {
  this.currentGroups = groups ? JSON.parse(JSON.stringify(groups)) : [];
  for (const group of this.currentGroups) {
    if (!group.id) group.id = "group";
    if (group.title === undefined) group.title = group.id;
  }
}

/**
 * Перенести выделенные ноды в группу — без перетаскивания каждой по очереди.
 *
 * Состав группы задаётся `nodes[].group`, а рамка лишь обводит содержимое, так
 * что «закинуть ноду внутрь рамки» можно и одним действием. Возвращает, сколько
 * нод реально сменило группу (ноды, уже бывшие в ней, не считаются).
 */
export function adoptSelectedNodesIntoGroup(
  this: GraphController,
  groupId: string,
): number {
  if (!this.groupById(groupId)) return 0;
  const ids = [...this.selectedNodes];
  if (ids.length === 0) return 0;
  this.saveCheckpoint();
  const moved: string[] = [];
  for (const nodeId of ids) {
    const dn = this.editor?.drawflow.drawflow.Home.data[nodeId];
    if (!dn || dn.data.group === groupId) continue;
    dn.data.group = groupId;
    moved.push(nodeId);
  }
  if (moved.length === 0) return 0;
  for (const nodeId of moved) this.updateNodeHtml(nodeId);
  this.refreshGroupFrames([groupId]);
  this.markDirty();
  logFront(`[groups] в группу '${groupId}' перенесено нод: ${moved.length}`);
  return moved.length;
}
