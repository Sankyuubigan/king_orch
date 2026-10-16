import type {
  WorkflowGraphDef, GraphNodeDef, GraphEdgeDef, GraphGroupDef,
} from "../../services";

export type { WorkflowGraphDef, GraphNodeDef, GraphEdgeDef, GraphGroupDef };

/**
 * Снапшот отмены. `data` — данные нод Drawflow, `groups` — группы.
 *
 * Группы лежат ОТДЕЛЬНО от `data`: они не принадлежат ни одной ноде Drawflow
 * (рамка — наш оверлей, а не drawflow-объект), но обязаны откатываться вместе
 * с нодами, иначе Ctrl+Z оставил бы ноды внутри группы, которой больше нет.
 */
export interface GraphSnapshot {
  data: Record<string, any>;
  groups: GraphGroupDef[];
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