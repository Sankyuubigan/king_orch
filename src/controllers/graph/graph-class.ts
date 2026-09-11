import Drawflow from "drawflow";
import "drawflow/dist/drawflow.min.css";
import type { DrawflowNode } from "drawflow";
import type { GraphElements, GraphSnapshot } from "./types";
import type { GraphApi } from "./graph-api";

export class GraphController {
  el: GraphElements;
  editor: Drawflow | null = null;
  currentFilePath: string | null = null;
  currentWorkflowName: string = "";
  currentWorkflowConfig: any = null;
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
  pristineSnapshot: GraphSnapshot | null = null;
  copiedNode: DrawflowNode | null = null;

  constructor(el: GraphElements) {
    this.el = el;
    this.bindEvents();
  }

  private bindEvents(): void {
    this.el.btnOpenWorkflow.addEventListener("click", () => this.handleOpen());
    this.el.btnSaveWorkflow.addEventListener("click", () => this.handleSave());
    this.el.graphSidebarClose.addEventListener("click", () => this.hideSidebar());
    this.el.btnUndo.addEventListener("click", () => this.undo());
    this.el.btnRedo.addEventListener("click", () => this.redo());
    this.setupKeyboardShortcuts();
  }

  private setupKeyboardShortcuts(): void {
    document.addEventListener("keydown", (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "z") {
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