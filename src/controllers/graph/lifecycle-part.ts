import Drawflow from "drawflow";
import type { ResizeDir } from "./groups-part";
import type { GraphController } from "./graph-class";

export function ensureEditor(this: GraphController): void {
  if (this.editor) return;
  this.editor = new Drawflow(this.el.graphContainer);
  this.editor.start();

  this.editor.on("nodeSelected", (id: string) => {
    this.showNodeEditor(id);
  });

  this.editor.on("nodeCreated", (id: string) => {
    this.updateNodeHtml(id);
    setTimeout(() => this.enableReverseConnection(id), 0);
  });

  // Рамки групп пересоздаются здесь: Drawflow при `import()` чистит `precanvas`,
  // и это ЕДИНСТВЕННЫЙ хук, через который проходят и открытие файла, и Ctrl+Z.
  this.editor.on("import", () => {
    if (!this.editor) return;
    for (const id of Object.keys(this.editor.drawflow.drawflow.Home.data)) {
      this.alignOutputsWithCases(id);
    }
    this.renderGroups();
  });

  this.editor.on("nodeMoved", (id: string) => {
    if (this.isRestoring) return;
    this.onNodeMoved(id);
  });

  this.editor.on("click", (ev: MouseEvent) => {
    // Событие, проброшенное рамкой ребру (см. `interactiveUnderGroup`): оно уже
    // отработало своё назначение у Drawflow, повторно открывать панель нельзя.
    if ((ev as unknown as Record<string, unknown>).__groupFrameForward) return;
    // Клик по рамке открывает инспектор группы, а не закрывает сайдбар:
    // обработчик ниже закрывает его на КАЖДЫЙ клик по холсту. Но рамка лежит
    // над соединениями, поэтому сначала проверяем, что под ней: если там ребро —
    // клик принадлежит ребру, и мы его туда возвращаем.
    const frame = (ev.target as HTMLElement).closest?.(".graph-group") as HTMLElement | null;
    const groupId = frame?.dataset.groupId;
    if (groupId) {
      const under = this.interactiveUnderGroup(ev.clientX, ev.clientY);
      if (under) {
        this.forwardMousedownTo(under, ev);
        return;
      }
      this.onGroupFrameClick(groupId);
      return;
    }
    this.hideSidebar();
  });

  this.editor.on("connectionCreated", (...args: any[]) => {
    if (this.isRestoring) return;
    const d = args[0] || {};
    this.onSwitchConnectionChanged(d.output_id, d.input_id, d.output_class, "created");
  });

  this.editor.on("connectionRemoved", (...args: any[]) => {
    if (this.isRestoring) return;
    const d = args[0] || {};
    this.onSwitchConnectionChanged(d.output_id, d.input_id, d.output_class, "removed");
  });

  // Захват рамки группы: заголовок тянет группу, маркеры — растягивают рамку.
  // Регистрируется ДО `setupCanvasInteraction`, чтобы перехватить mousedown
  // раньше обработчиков выделения и не дать Drawflow начать перетаскивание холста
  // или ноды под рамкой. `stopImmediatePropagation` (а не `stopPropagation`)
  // обязателен: остальные слушатели висят на ЭТОМ ЖЕ элементе, и box-selection с
  // Ctrl по рамке стартовал бы в тот же mousedown.
  this.el.graphContainer.addEventListener("mousedown", (e) => {
    const handle = (e.target as HTMLElement).closest?.(".graph-group-resize") as HTMLElement | null;
    const header = handle
      ? null
      : ((e.target as HTMLElement).closest?.(".graph-group-header") as HTMLElement | null);
    const frame = (handle || header)?.closest(".graph-group") as HTMLElement | null;
    const groupId = frame?.dataset.groupId;
    if (!handle && !groupId) return;
    // Под рамкой может лежать ребро — тогда клик не наш (и не заголовка).
    const under = this.interactiveUnderGroup(e.clientX, e.clientY);
    if (under) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    if (handle) {
      this.beginGroupResize(groupId!, handle.dataset.dir as ResizeDir, e);
    } else {
      this.beginGroupDrag(groupId!, e);
    }
  }, true);

  this.setupCanvasInteraction();
  this.setupCanvasZoom();
  this.setupContextMenu();
  this.setupEdgeDrag();
}

export function onTabActivated(this: GraphController): void {
  if (!this.editor) {
    this.ensureEditor();
  } else {
    this.editor.zoom_reset();
  }
}