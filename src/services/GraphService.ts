import { invoke } from "@tauri-apps/api/core";

export interface GraphNodeDef {
  id: string;
  type: string;
  agent?: string;
  task?: string;
  input?: string;
  action?: string;
  required?: string[];
  workflow?: string;
  cases?: Record<string, string>;
  default?: string;
  cases_priority?: Array<{ key: string; to: string }>;
  conditions?: Array<{ field: string; equals: any }>;
  logic?: string;
  input_object?: string;
  inject_reports?: string[];
  output_type?: string;
  ui_pos?: { x: number; y: number };
  signal_name?: string;
  field?: string;
  sequential_to?: string;
  true_to?: string;
  false_to?: string;
  system_message?: string;
  disabled?: boolean;
}

export interface GraphEdgeDef {
  from: string;
  to: string;
  condition?: string;
  case?: string;
}

export interface WorkflowGraphDef {
  team?: string;
  name: string;
  file_stem?: string;
  visible: boolean;
  config?: any;
  nodes: GraphNodeDef[];
  edges: GraphEdgeDef[];
}

export async function readWorkflowFile(path: string): Promise<WorkflowGraphDef> {
  return invoke<WorkflowGraphDef>("read_workflow_file", { path });
}

export async function saveWorkflow(path: string, workflow: WorkflowGraphDef): Promise<void> {
  return invoke<void>("save_workflow", { path, workflow });
}