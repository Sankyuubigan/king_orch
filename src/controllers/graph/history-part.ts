import type { DrawflowExport } from "drawflow";
import { isDynamicNode, MAX_HISTORY } from "./constants";
import type { GraphSnapshot } from "./types";
import type { GraphController } from "./graph-class";

// ─── Undo/Redo + Dirty State ───

export function captureSnapshot(this: GraphController): GraphSnapshot {
  const editor = this.editor;
  if (!editor) return { data: {} };
  return {
    data: JSON.parse(JSON.stringify(editor.drawflow.drawflow.Home.data)),
  };
}

export function applySnapshot(this: GraphController, snap: GraphSnapshot): void {
  if (!this.editor) return;
  this.isRestoring = true;

  const editor = this.editor;

  // 1. Собираем все соединения из снапшота ДО очистки
  const snapDataRaw = JSON.parse(JSON.stringify(snap.data));
  const connectionsToRestore: Array<{ from: string; to: string; output: string; input: string }> = [];
  for (const nodeId of Object.keys(snapDataRaw)) {
    const dn = snapDataRaw[nodeId];
    for (const inKey of Object.keys(dn.inputs || {})) {
      for (const conn of dn.inputs[inKey].connections || []) {
        if (snapDataRaw[conn.node]) {
          connectionsToRestore.push({
            from: conn.node,
            to: nodeId,
            output: conn.input || "output_1",
            input: inKey,
          });
        }
      }
    }
  }

  // 2. Удаляем все текущие ноды (isRestoring=true — подавляем connectionRemoved)
  const currentNodes = Object.keys(editor.drawflow.drawflow.Home.data);
  for (const id of currentNodes) {
    editor.removeNodeId("node-" + id);
  }

  // 3. Собираем import-данные (с ПУСТЫМИ connections — Drawflow не умеет создавать их из данных)
  const importData: DrawflowExport = {
    drawflow: { Home: { data: {} } },
  };
  const snapData = JSON.parse(JSON.stringify(snap.data));

  for (const id of Object.keys(snapData)) {
    const dn = snapData[id];
    // Обнуляем connections — создадим их вручную после import
    for (const inKey of Object.keys(dn.inputs || {})) {
      dn.inputs[inKey].connections = [];
    }
    for (const outKey of Object.keys(dn.outputs || {})) {
      dn.outputs[outKey].connections = [];
    }
    // Обновляем HTML и class
    dn.html = this.buildNodeInnerHtml(dn.data, id);
    let nodeClass = "";
    const t = dn.data.type;
    if (t === "note") nodeClass = "node-type-note";
    else if (isDynamicNode(t)) nodeClass = "dynamic-node";
    if (dn.data.disabled) nodeClass = nodeClass ? `${nodeClass} node-disabled` : "node-disabled";
    dn.class = nodeClass;
  }
  importData.drawflow.Home.data = snapData;

  // 5. Импортируем (isRestoring=true — подавляем connectionCreated/Removed)
  editor.import(importData);

  // 6. Вручную создаём все соединения (Drawflow import их не восстанавливает)
  for (const conn of connectionsToRestore) {
    try {
      editor.addConnection(conn.from, conn.to, conn.output, conn.input);
    } catch (_) { }
  }

  // 7. Ждём кадр для пост-обработки (выравнивание + SVG + reverse)
  requestAnimationFrame(() => {
    if (!this.editor) return;
    this.isRestoring = true;

    // Drawflow import() сбрасывает transform precanvas — восстанавливаем
    const transform = `translate(${this.editor.canvas_x}px, ${this.editor.canvas_y}px) scale(${this.editor.zoom})`;
    this.editor.precanvas.style.transform = transform;

    for (const id of Object.keys(this.editor.drawflow.drawflow.Home.data)) {
      this.alignOutputsWithCases(id);
    }

    for (const id of Object.keys(this.editor.drawflow.drawflow.Home.data)) {
      this.editor.updateConnectionNodes("node-" + id);
    }

    for (const id of Object.keys(this.editor.drawflow.drawflow.Home.data)) {
      this.enableReverseConnection(id);
    }

    this.hideSidebar();
    this.deselectAllNodes();
    this.isRestoring = false;
  });
}

export function saveCheckpoint(this: GraphController): void {
  if (this.isRestoring) return;
  this.undoStack.push(this.captureSnapshot());
  if (this.undoStack.length > MAX_HISTORY) {
    this.undoStack.shift();
  }
  this.redoStack = [];
  this.markDirty();
  this.updateUndoButtons();
}

export function undo(this: GraphController): void {
  if (this.undoStack.length === 0 || !this.editor) return;
  this.redoStack.push(this.captureSnapshot());
  const snap = this.undoStack.pop()!;
  this.applySnapshot(snap);
  this.syncDirtyAfterRestore();
  this.updateUndoButtons();
}

export function redo(this: GraphController): void {
  if (this.redoStack.length === 0 || !this.editor) return;
  this.undoStack.push(this.captureSnapshot());
  const snap = this.redoStack.pop()!;
  this.applySnapshot(snap);
  this.syncDirtyAfterRestore();
  this.updateUndoButtons();
}

export function syncDirtyAfterRestore(this: GraphController): void {
  if (!this.pristineSnapshot) return;
  const current = this.captureSnapshot();
  if (this.isNodesEqual(current.data, this.pristineSnapshot.data)) {
    this.markClean();
  } else {
    this.markDirty();
  }
}

// Сравнение только значимых полей нод (игнорируем служебные поля Drawflow)
export function isNodesEqual(this: GraphController, a: Record<string, any>, b: Record<string, any>): boolean {
  const aKeys = Object.keys(a);
  const bKeys = Object.keys(b);
  if (aKeys.length !== bKeys.length) return false;
  for (const key of aKeys) {
    const an = a[key];
    const bn = b[key];
    if (!bn) return false;
    if (JSON.stringify(an.data) !== JSON.stringify(bn.data)) return false;
    if (JSON.stringify(an.inputs) !== JSON.stringify(bn.inputs)) return false;
    if (JSON.stringify(an.outputs) !== JSON.stringify(bn.outputs)) return false;
    if (an.pos_x !== bn.pos_x || an.pos_y !== bn.pos_y) return false;
  }
  return true;
}

export function resetHistory(this: GraphController): void {
  this.undoStack = [];
  this.redoStack = [];
  this.pristineSnapshot = this.captureSnapshot();
  this.markClean();
  this.updateUndoButtons();
}

export function markDirty(this: GraphController): void {
  this.isDirty = true;
  this.updateDirtyIndicator();
}

export function markClean(this: GraphController): void {
  this.isDirty = false;
  this.updateDirtyIndicator();
}

export function updateDirtyIndicator(this: GraphController): void {
  const el = this.el.dirtyIndicator;
  if (!this.currentFilePath) {
    el.textContent = "";
    el.title = "";
    return;
  }
  if (this.isDirty) {
    el.textContent = " ●";
    el.style.color = "#e53935";
    el.title = "Есть несохранённые изменения";
  } else {
    el.textContent = " ●";
    el.style.color = "#4caf50";
    el.title = "Сохранено";
  }
}

export function updateUndoButtons(this: GraphController): void {
  this.el.btnUndo.disabled = this.undoStack.length === 0;
  this.el.btnRedo.disabled = this.redoStack.length === 0;
}