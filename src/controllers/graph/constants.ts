export const NODE_COLORS: Record<string, string> = {
  llm_worker: "#4caf50",
  llm_classifier: "#42a5f5",
  llm_fact_extractor: "#7e57c2",
  llm_freeform: "#26c6da",
  system_condition: "#ffa726",
  sub_workflow: "#ab47bc",
  switch: "#ef5350",
  llm_sequential_switch: "#e040fb",
  signal_router: "#ff9800",
  condition_check: "#26a69a",
  condition_router: "#5c6bc0",
  return: "#78909c",
  note: "#e53935",
};

export const NODE_LABELS: Record<string, string> = {
  llm_worker: "🤖 Worker",
  llm_classifier: "📋 Classifier",
  llm_fact_extractor: "📋 Fact Extractor",
  llm_freeform: "💬 Freeform",
  system_condition: "⚡ Condition",
  sub_workflow: "📦 Sub-workflow",
  switch: "🔀 Switch",
  llm_sequential_switch: "🔀 SeqSwitch",
  signal_router: "📡 Signal Router",
  condition_check: "🎯 Condition Check",
  condition_router: "🔀 Condition Router",
  return: "🏁 Return",
  note: "📝 Note",
};

export function isDynamicNode(t: string): boolean {
  return t === "switch" || t === "llm_sequential_switch" || t === "signal_router" || t === "condition_check" || t === "condition_router";
}

export const OUTPUT_COUNT: Record<string, number> = {
  llm_worker: 1,
  llm_classifier: 1,
  llm_fact_extractor: 1,
  llm_freeform: 1,
  system_condition: 1,
  sub_workflow: 1,
  switch: 0,
  llm_sequential_switch: 0,
  signal_router: 0,
  condition_check: 0,
  condition_router: 0,
  return: 0,
  note: 1,
};

export const CONDITION_CHECK_CASES = [
  { key: "true", field: "true_to" as const },
  { key: "false", field: "false_to" as const },
  { key: "seq", field: "sequential_to" as const },
];

export const CONDITION_ROUTER_CASES = [
  { key: "true", field: "true_to" as const },
  { key: "false", field: "false_to" as const },
];

export const MAX_HISTORY = 50;