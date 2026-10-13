import type { WorkflowGraphDef, GraphNodeDef, GraphEdgeDef } from "../../services";

export type { WorkflowGraphDef, GraphNodeDef, GraphEdgeDef };

export interface GraphSnapshot {
  data: Record<string, any>;
}

export interface GraphElements {
  /**
   * Корень экземпляра студии (граница поиска элементов).
   *
   * Нужен потому, что студий может быть НЕСКОЛЬКО (по одной на вкладку): поля редактора
   * ноды (`ge-*`) и ноды холста (`node-*`) обязаны искаться внутри своего экземпляра, иначе
   * две студии находили бы элементы друг друга. Необязателен: тесты и одиночный экземпляр
   * исторически передают только сами элементы.
   */
  root?: HTMLElement;
  graphContainer: HTMLDivElement;
  graphSidebar: HTMLDivElement;
  graphDetailTitle: HTMLSpanElement;
  graphDetailContent: HTMLDivElement;
  graphSidebarClose: HTMLButtonElement;
  btnOpenWorkflow: HTMLButtonElement;
  btnSaveWorkflow: HTMLButtonElement;
  currentWorkflowName: HTMLSpanElement;
  yamlWarning: HTMLDetailsElement;
  yamlWarningList: HTMLUListElement;
  btnUndo: HTMLButtonElement;
  btnRedo: HTMLButtonElement;
  dirtyIndicator: HTMLSpanElement;
}