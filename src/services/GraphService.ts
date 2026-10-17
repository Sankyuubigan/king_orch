import { invoke } from "@tauri-apps/api/core";

/**
 * ЗЕРКАЛО схемы YAML. Владелец схемы — `parser.rs` (`WorkflowDef` / `NodeDef` /
 * `GroupDef`); этот файл — его TypeScript-отражение и он НЕ является вторым
 * источником правды.
 *
 * Расхождение ловится гейтом `test/graph.maximal-roundtrip.test.ts`: Rust отдаёт
 * YAML со всеми возможными полями, фронт прогоняет open→save и обязан вернуть
 * их без потерь. Добавляешь поле в Rust — падает тест, пока не проверишь здесь.
 */
export interface GraphNodeDef {
  id: string;
  type: string;
  /**
   * Принадлежность к группе нод (`WorkflowGraphDef.groups[].id`).
   *
   * В Detailed ничего не меняет. В Fused ноды одной группы схлопываются в один
   * вызов LLM. Пусто — нода вне групп.
   */
  group?: string;
  agent?: string;
  task?: string;
  input?: string;
  action?: string;
  required?: string[];
  workflow?: string;
  cases?: Record<string, string>;
  default?: string;
  cases_priority?: Array<{ key: string; to: string }>;
  conditions?: ConditionNode[];
  logic?: string;
  input_object?: string;
  inject_response?: string[];
  output_type?: string;
  ui_pos?: { x: number; y: number };
  signal_name?: string;
  field?: string;
  switch_field?: string;
  statuses?: any;
  problems?: string;
  sequential_to?: string;
  true_to?: string;
  false_to?: string;
  system_message?: string;
  disabled?: boolean;
  llm_params?: string;
  max_visits?: number;
  rules_file?: string;
}

export type ConditionNode =
  | { field: string; equals: any }
  | { logic?: string; conditions: ConditionNode[] };

export interface GraphEdgeDef {
  from: string;
  to: string;
  condition?: string;
  case?: string;
}

/** Ключ выхода склеенного вызова группы: контракт `FactDef` без нового типа. */
export interface GraphFusedKeyDef {
  id: string;
  criteria?: string | null;
  values?: string[];
}

/**
 * Группа нод — аннотация графа для FUSED-режима и визуальная рамка на холсте.
 *
 * Группы НЕ хранят `on_success`/`interrupts`: переход группа→группа уже выражен
 * обычным ребром между нодами разных групп, а прерывание на человека — нода
 * `note`. Второй источник правды разошёлся бы с рёбрами при первом редактировании.
 */
export interface GraphGroupDef {
  id: string;
  title?: string;
  fused_instruction?: string | null;
  /** Агент-исполнитель группы: его `.md` даёт системный промпт и `mcp_servers`. */
  fused_agent?: string | null;
  tools?: string[];
  fused_keys?: GraphFusedKeyDef[];
  /** Явные координаты рамки; приоритет над производным bbox нод. */
  ui_pos?: { x: number; y: number };
  /**
   * Явный размер рамки: появляется, когда автор растянул её.
   *
   * Пара с `ui_pos` = «рамкой владеет автор» (в Figma/Blender так ведёт себя
   * frame): растяжение двигает только рамку, ноды внутри остаются на месте, и
   * свободное место под новые ноды появляется само собой. Без этой пары рамка
   * производная от bbox нод.
   */
  ui_size?: { w: number; h: number };
}

export interface WorkflowGraphDef {
  team?: string;
  name: string;
  description?: string | null;
  file_stem?: string;
  config?: any;
  /**
   * ID ноды — точки входа графа. Движок стартует ровно от неё и без этого
   * поля не запускает граф, поэтому при сохранении поле всегда заполнено.
   */
  entry?: string;
  /**
   * Предлагать ли граф в каталоге «Доступные агенты» (`visible: false` скрывает).
   *
   * Поле обязано ездить через редактор: `save_workflow` пишет `WorkflowDef` поверх
   * файла, поэтому забытое здесь `visible` вернуло бы скрытый граф в каталог.
   * Отсутствие поля = `true` (дефолт backend'а).
   */
  visible?: boolean;
  /** Режим исполнения: `detailed` (по умолчанию) | `fused`. Режима `auto` нет. */
  strategy?: string | null;
  /**
   * Группы нод. Поле обязано ездить через редактор: `save_workflow` сериализует
   * `WorkflowDef` поверх файла, поэтому забытое здесь `groups` выбросило бы всю
   * секцию из YAML при первом сохранении из визуального редактора.
   */
  groups?: GraphGroupDef[];
  nodes: GraphNodeDef[];
  edges: GraphEdgeDef[];
}

export interface GraphDiagnostic {
  code: string;
  location: string;
  message: string;
}

export interface WorkflowGraphReadResult {
  workflow: WorkflowGraphDef;
  diagnostics: GraphDiagnostic[];
}

export async function readWorkflowFile(path: string): Promise<WorkflowGraphReadResult> {
  return invoke<WorkflowGraphReadResult>("read_workflow_file", { path });
}

export async function saveWorkflow(path: string, workflow: WorkflowGraphDef): Promise<void> {
  return invoke<void>("save_workflow", { path, workflow });
}