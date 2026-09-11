import type { WorkflowGraphDef, GraphNodeDef, GraphEdgeDef } from "../../services";

export type { WorkflowGraphDef, GraphNodeDef, GraphEdgeDef };

export interface GraphSnapshot {
  data: Record<string, any>;
}

export interface GraphElements {
  graphContainer: HTMLDivElement;
  graphSidebar: HTMLDivElement;
  graphDetailTitle: HTMLSpanElement;
  graphDetailContent: HTMLDivElement;
  graphSidebarClose: HTMLButtonElement;
  btnOpenWorkflow: HTMLButtonElement;
  btnSaveWorkflow: HTMLButtonElement;
  currentWorkflowName: HTMLSpanElement;
  btnUndo: HTMLButtonElement;
  btnRedo: HTMLButtonElement;
  dirtyIndicator: HTMLSpanElement;
}