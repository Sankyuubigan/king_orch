import { isDynamicNode, CONDITION_CHECK_CASES, CONDITION_ROUTER_CASES } from "./constants";
import type { GraphNodeDef, GraphEdgeDef } from "./types";
import type { GraphController } from "./graph-class";

export function rebuildSwitchOutputs(this: GraphController, nodeId: string): void {
  if (!this.editor) return;
  const dn = this.editor.drawflow.drawflow.Home.data[nodeId];
  if (!dn) return;
  const data = dn.data;
  if (!isDynamicNode(data.type)) return;

  // Сохраняем данные ДО удаления, т.к. connectionRemoved занулит их
  const targets: string[] = [];
  if (data.cases_priority) {
    for (const cp of data.cases_priority) targets.push(cp.to);
  }
  if (data.default) targets.push(data.default);
  const savedDefault = data.default;

  const oldKeys = Object.keys(dn.outputs)
    .filter(k => /^output_\d+$/.test(k))
    .sort((a, b) => parseInt(b.replace("output_", ""), 10) - parseInt(a.replace("output_", ""), 10));
  for (const key of oldKeys) {
    this.editor!.removeNodeOutput(nodeId, key);
  }

  // Восстанавливаем default, который мог быть занулён connectionRemoved
  data.default = savedDefault;

  const caseKeys = this.getSwitchCaseKeys(data);
  const newCount = caseKeys.length;

  for (let i = 0; i < newCount; i++) {
    this.editor!.addNodeOutput(nodeId);
  }

  for (let i = 0; i < targets.length; i++) {
    const targetId = targets[i];
    if (targetId && this.editor.drawflow.drawflow.Home.data[targetId]) {
      try {
        this.editor!.addConnection(nodeId, targetId, `output_${i + 1}`, "input_1");
      } catch (_) { }
    }
  }

  this.updateNodeHtml(nodeId);
}

export function syncSwitchNodeFromConnections(this: GraphController, dn: any): void {
  const data = dn.data;
  if (data.type === "condition_check") {
    for (let i = 0; i < CONDITION_CHECK_CASES.length; i++) {
      const cc = CONDITION_CHECK_CASES[i];
      const outKey = `output_${i + 1}`;
      const conns = dn.outputs[outKey]?.connections || [];
      data[cc.field] = conns.length > 0 ? conns[0].node : undefined;
    }
    return;
  }
  if (data.type === "condition_router") {
    for (let i = 0; i < CONDITION_ROUTER_CASES.length; i++) {
      const cc = CONDITION_ROUTER_CASES[i];
      const outKey = `output_${i + 1}`;
      const conns = dn.outputs[outKey]?.connections || [];
      data[cc.field] = conns.length > 0 ? conns[0].node : undefined;
    }
    return;
  }
  const caseKeys = this.getSwitchCaseKeys(data);
  if (data.cases_priority) {
    for (let i = 0; i < data.cases_priority.length; i++) {
      const outKey = `output_${i + 1}`;
      const conns = dn.outputs[outKey]?.connections || [];
      data.cases_priority[i].to = conns.length > 0 ? conns[0].node : "";
    }
  }
  if (data.default) {
    const defaultIdx = caseKeys.indexOf("default");
    if (defaultIdx >= 0) {
      const outKey = `output_${defaultIdx + 1}`;
      const conns = dn.outputs[outKey]?.connections || [];
      data.default = conns.length > 0 ? conns[0].node : undefined;
    }
  }
}

export function onSwitchConnectionChanged(this: GraphController, fromNodeId: string, toNodeId: string, outputKey: string, action: "created" | "removed"): void {
  if (!this.editor || !fromNodeId || !toNodeId || !outputKey) return;
  const dn = this.editor.drawflow.drawflow.Home.data[fromNodeId];
  if (!dn) return;
  const data = dn.data;
  if (!isDynamicNode(data.type)) return;
  const match = outputKey.match(/^output_(\d+)$/);
  if (!match) return;
  const outIdx = parseInt(match[1], 10) - 1;

  if (data.type === "condition_check") {
    const cc = CONDITION_CHECK_CASES[outIdx];
    if (cc) data[cc.field] = action === "created" ? toNodeId : undefined;
    return;
  }

  if (data.type === "condition_router") {
    const cc = CONDITION_ROUTER_CASES[outIdx];
    if (cc) data[cc.field] = action === "created" ? toNodeId : undefined;
    return;
  }

  const caseKeys = this.getSwitchCaseKeys(data);
  if (outIdx < 0 || outIdx >= caseKeys.length) return;
  const caseKey = caseKeys[outIdx];
  if (caseKey === "default") {
    data.default = action === "created" ? toNodeId : undefined;
  } else if (data.cases_priority && outIdx < data.cases_priority.length) {
    data.cases_priority[outIdx].to = action === "created" ? toNodeId : "";
  }
}

// ─── Утилиты switch ───

export function getSwitchOutputCount(this: GraphController, node: GraphNodeDef): number {
  if (node.type === "condition_check") return 3;
  if (node.type === "condition_router") return 2;
  if (node.cases) return Object.keys(node.cases).length;
  if (node.cases_priority) return node.cases_priority.length + (node.default ? 1 : 0);
  if (node.default) return 1;
  return 2;
}

export function getSwitchCaseKeys(this: GraphController, node: GraphNodeDef): string[] {
  if (node.type === "condition_check") return CONDITION_CHECK_CASES.map(c => c.key);
  if (node.type === "condition_router") return CONDITION_ROUTER_CASES.map(c => c.key);
  if (node.cases) return Object.keys(node.cases);
  if (node.cases_priority) {
    const keys = node.cases_priority.map((c) => c.key);
    if (node.default) keys.push("default");
    return keys;
  }
  if (node.default) return ["default"];
  return ["default", "other"];
}

export function getSwitchOutputIndex(this: GraphController, node: GraphNodeDef, caseVal?: string): number {
  const keys = this.getSwitchCaseKeys(node);
  const idx = keys.indexOf(caseVal || "default");
  return idx >= 0 ? idx : 0;
}

export function getImplicitSwitchEdges(this: GraphController, nodes: GraphNodeDef[]): GraphEdgeDef[] {
  const implicit: GraphEdgeDef[] = [];
  for (const node of nodes) {
    if (!isDynamicNode(node.type)) continue;
    if (node.type === "condition_check") {
      if (node.true_to) implicit.push({ from: node.id, to: node.true_to });
      if (node.false_to) implicit.push({ from: node.id, to: node.false_to });
      if (node.sequential_to) implicit.push({ from: node.id, to: node.sequential_to });
      continue;
    }
    if (node.type === "condition_router") {
      if (node.true_to) implicit.push({ from: node.id, to: node.true_to });
      if (node.false_to) implicit.push({ from: node.id, to: node.false_to });
      continue;
    }
    if (node.cases_priority) {
      for (const cp of node.cases_priority) {
        implicit.push({ from: node.id, to: cp.to });
      }
    }
    if (node.cases) {
      for (const val of Object.values(node.cases)) {
        implicit.push({ from: node.id, to: val });
      }
    }
    if (node.default) {
      implicit.push({ from: node.id, to: node.default });
    }
  }
  return implicit;
}