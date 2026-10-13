import Drawflow from "drawflow";
import "drawflow/dist/drawflow.min.css";
import type { DrawflowNode } from "drawflow";
import type { GraphDiagnostic } from "../../services";
import type { GraphElements, GraphSnapshot } from "./types";
import type { GraphApi } from "./graph-api";

export class GraphController {
  el: GraphElements;
  editor: Drawflow | null = null;
  currentFilePath: string | null = null;
  currentWorkflowName: string = "";
  currentWorkflowConfig: any = null;
  /**
   * Видимость графа в каталоге «Доступные агенты» (`visible` в корне YAML).
   *
   * Хранится в контроллере, потому что `handleSave` собирает `WorkflowDef` заново
   * из своих полей: не помнить здесь значение — значит записать `visible: true`
   * поверх `visible: false` при первом же сохранении и вернуть скрытый граф в каталог.
   */
  currentWorkflowVisible: boolean = true;
  /**
   * ID узла — точки входа графа (поле `entry` в YAML).
   *
   * Вход в workflow ОБЯЗАТЕЛЕН и объявлен в файле: движок стартует ровно от этой
   * ноды (`parser::resolve_entry`). Позиция ноды в `nodes:` объявлением не является
   * — порядок переписывает редактор при сохранении.
   */
  currentEntryNodeId: string | null = null;
  selectedNodes: Set<string> = new Set();
  isSelecting: boolean = false;
  selectStart: { x: number; y: number } | null = null;
  selectionRectEl: HTMLDivElement | null = null;
  multiDragRaf: number | null = null;
  multiDragInitial: Map<string, { x: number; y: number }> | null = null;
  multiDragPrimary: string | null = null;
  ctxMenu: HTMLDivElement | null = null;
  undoStack: GraphSnapshot[] = [];
  redoStack: GraphSnapshot[] = [];
  isRestoring: boolean = false;
  isDirty: boolean = false;
  yamlDiagnostics: GraphDiagnostic[] = [];
  pristineSnapshot: GraphSnapshot | null = null;
  copiedNode: DrawflowNode | null = null;
  /**
   * Отписки от ГЛОБАЛЬНЫХ подписок (document/window/body), которые переживают
   * удаление DOM экземпляра.
   *
   * Всё, что висит на элементе внутри корня студии, умирает вместе с `root.remove()`.
   * А вот слушатель на `document` держит контроллер (и его элементы) живым после
   * закрытия вкладки, а узел контекстного меню, добавленный в `document.body`, остаётся
   * в интерфейсе навсегда. Части графа (`menu-part` и др.) регистрируют такие
   * подписки через `addGlobalListener`, а `destroy()` откатывает их все.
   */
  globalDisposers: (() => void)[] = [];

  constructor(el: GraphElements) {
    this.el = el;
    this.bindEvents();
  }

  /**
   * Зарегистрировать подписку, которая переживёт удаление корня экземпляра.
   *
   * Возвращает функцию отписки; вызывать её вручную не нужно — `destroy()` откатит
   * всё записанное в `globalDisposers`.
   */
  addGlobalListener<T extends Event>(
    target: EventTarget,
    type: string,
    handler: (e: T) => void,
    options?: AddEventListenerOptions,
  ): void {
    const ev = handler as EventListener;
    target.addEventListener(type, ev, options);
    this.globalDisposers.push(() => target.removeEventListener(type, ev, options));
  }

  /**
   * Освободить экземпляр редактора: снять глобальные подписки и убрать всё,
   * что контроллер добавил в документ (контекстное меню).
   *
   * Вызывается при закрытии вкладки студии (`AgentStudioTab.dispose`). Холст Drawflow
   * освобождать отдельно не нужно: у библиотеки нет `destroy()`, но его слушатели висят
   * на элементах ВНУТРИ контейнера, который уходит вместе с корнем экземпляра.
   */
  destroy(): void {
    for (const dispose of this.globalDisposers.splice(0)) dispose();
    this.ctxMenu?.remove();
    this.ctxMenu = null;
    this.editor = null;
  }

  /**
   * DOM-узел ноды Drawflow по её id, В ГРАНИЦАХ своего экземпляра студии.
   *
   * Drawflow вешает на ноды `id="node-<id>"`, а id нод задаёт автор графа — в двух
   * открытых графах они совпадают (например `user_message`). Поэтому `document.getElementById`
   * вернул бы ноду ПЕРВОЙ студии, и правки ушли бы в чужой граф. Ищем внутри корня экземпляра.
   *
   * `root` необязателен (тесты и одиночный экземпляр передают элементы без корня) — тогда
   * поведение прежнее, как до появления мультивкладок.
   */
  findNodeEl(nodeId: string): HTMLElement | null {
    return (
      this.el.root?.querySelector<HTMLElement>(`#node-${nodeId}`) ??
      document.getElementById(`node-${nodeId}`)
    );
  }

  private bindEvents(): void {
    this.el.btnOpenWorkflow.addEventListener("click", () => this.handleOpen());
    this.el.btnSaveWorkflow.addEventListener("click", () => this.handleSave());
    this.el.graphSidebarClose.addEventListener("click", () => this.hideSidebar());
    this.el.btnUndo.addEventListener("click", () => this.undo());
    this.el.btnRedo.addEventListener("click", () => this.redo());
    this.setupKeyboardShortcuts();
  }

  /**
   * Активна ли страница ЭТОГО экземпляра (её `.view` имеет класс `active`).
   *
   * Нужна для горячих клавиш: `keydown` слушается на документе, поэтому при нескольких
   * открытых студиях Ctrl+Z без этой проверки отменил бы действие сразу во ВСЕХ графах
   * (`tasks/` — «горячие клавиши только у активной вкладки»).
   */
  private isStudioActive(): boolean {
    const root = this.el.root;
    if (!root) return true; // одиночный экземпляр/тесты: проверять нечего
    return root.classList.contains("active");
  }

  private setupKeyboardShortcuts(): void {
    this.addGlobalListener<KeyboardEvent>(document, "keydown", (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "z") {
        // Неактивная вкладка студии не реагирует: иначе одна отмена меняла бы чужой граф.
        if (!this.isStudioActive()) return;
        if (e.shiftKey) {
          e.preventDefault();
          this.redo();
        } else {
          e.preventDefault();
          this.undo();
        }
      }
    });
  }
}

export interface GraphController extends GraphApi {}