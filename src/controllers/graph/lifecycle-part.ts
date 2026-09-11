import Drawflow from "drawflow";
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

  this.editor.on("import", () => {
    if (!this.editor) return;
    for (const id of Object.keys(this.editor.drawflow.drawflow.Home.data)) {
      this.alignOutputsWithCases(id);
    }
  });

  this.editor.on("click", () => {
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

  this.setupCanvasInteraction();
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