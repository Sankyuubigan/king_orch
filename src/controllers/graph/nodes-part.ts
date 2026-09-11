import type { DrawflowNode } from "drawflow";
import { showToast } from "../../ui";
import { isDynamicNode, OUTPUT_COUNT } from "./constants";
import type { GraphNodeDef } from "./types";
import type { GraphController } from "./graph-class";

// ─── Добавление ноды ───

export function addNode(this: GraphController, type: string, clientX?: number, clientY?: number): void {
  this.ensureEditor();
  this.saveCheckpoint();
  const id = `${type}_${Date.now()}`;
  const zoom = this.editor!.zoom || 1;
  let cx: number, cy: number;
  if (clientX !== undefined && clientY !== undefined) {
    const preRect = this.editor!.precanvas.getBoundingClientRect();
    cx = (clientX - preRect.left) / zoom;
    cy = (clientY - preRect.top) / zoom;
  } else {
    const preRect = this.editor!.precanvas.getBoundingClientRect();
    cx = (preRect.width / 2 - 100) * (1 / zoom);
    cy = (preRect.height / 2 - 50) * (1 / zoom);
  }
  const outs = type === "condition_check" ? 3 : type === "condition_router" ? 2 : (isDynamicNode(type) ? 2 : OUTPUT_COUNT[type] ?? 1);

  const inputs: Record<string, { connections: any[] }> = {};
  for (let i = 1; i <= 1; i++) inputs[`input_${i}`] = { connections: [] };
  const outputs: Record<string, { connections: any[] }> = {};
  for (let i = 1; i <= outs; i++) outputs[`output_${i}`] = { connections: [] };

  let nodeClass = "";
  if (type === "note") nodeClass = "node-type-note";
  else if (isDynamicNode(type)) nodeClass = "dynamic-node";

  const nodeData: DrawflowNode = {
    id,
    name: id,
    data: { id, type },
    class: nodeClass,
    html: "",
    typenode: false,
    inputs,
    outputs,
    pos_x: cx + Math.random() * 80 - 40,
    pos_y: cy + Math.random() * 80 - 40,
  };

  this.editor!.drawflow.drawflow.Home.data[id] = nodeData;
  this.editor!.addNodeImport(nodeData, this.editor!.precanvas);
  this.editor!.dispatch("nodeCreated", id);

  this.updateNodeHtml(id);
}

// ─── Копирование / вставка ноды ───

export function copyNode(this: GraphController, nodeId: string): void {
  if (!this.editor) return;
  const dn = this.editor.drawflow.drawflow.Home.data[nodeId];
  if (!dn) return;
  this.copiedNode = JSON.parse(JSON.stringify(dn));
  showToast(`📋 Скопирована нода: ${nodeId}`, "success");
}

export function generateUniqueNodeId(this: GraphController, baseType: string): string {
  const data = this.editor ? this.editor.drawflow.drawflow.Home.data : {};
  const ts = Date.now();
  let id = `${baseType}_${ts}`;
  let n = 1;
  while (data[id]) {
    id = `${baseType}_${ts}_${n}`;
    n++;
  }
  return id;
}

export function pasteNode(this: GraphController, clientX?: number, clientY?: number): void {
  if (!this.copiedNode) return;
  this.ensureEditor();
  this.saveCheckpoint();

  const newId = this.generateUniqueNodeId(this.copiedNode.data.type || "node");
  const zoom = this.editor!.zoom || 1;
  let cx: number, cy: number;
  if (clientX !== undefined && clientY !== undefined) {
    const preRect = this.editor!.precanvas.getBoundingClientRect();
    cx = (clientX - preRect.left) / zoom;
    cy = (clientY - preRect.top) / zoom;
  } else {
    const preRect = this.editor!.precanvas.getBoundingClientRect();
    cx = (preRect.width / 2 - 100) * (1 / zoom);
    cy = (preRect.height / 2 - 50) * (1 / zoom);
  }

  const clone = JSON.parse(JSON.stringify(this.copiedNode)) as DrawflowNode;
  clone.id = newId;
  clone.name = newId;
  clone.data.id = newId;
  clone.data.ui_pos = undefined;
  this.clearNodeTargets(clone.data);

  // Копия вставляется без связей — пустые порты
  clone.inputs = {};
  clone.outputs = {};
  clone.inputs.input_1 = { connections: [] };
  const outs = isDynamicNode(clone.data.type) ? this.getSwitchOutputCount(clone.data) : OUTPUT_COUNT[clone.data.type] ?? 1;
  for (let i = 1; i <= outs; i++) clone.outputs[`output_${i}`] = { connections: [] };

  clone.pos_x = cx + Math.random() * 80 - 40;
  clone.pos_y = cy + Math.random() * 80 - 40;

  this.editor!.drawflow.drawflow.Home.data[newId] = clone;
  this.editor!.addNodeImport(clone, this.editor!.precanvas);
  this.editor!.dispatch("nodeCreated", newId);
  this.updateNodeHtml(newId);
  showToast(`📌 Вставлена нода: ${newId}`, "success");
}

export function clearNodeTargets(this: GraphController, data: GraphNodeDef): void {
  if (data.type === "condition_check") {
    data.true_to = undefined;
    data.false_to = undefined;
    data.sequential_to = undefined;
    return;
  }
  if (data.type === "condition_router") {
    data.true_to = undefined;
    data.false_to = undefined;
    return;
  }
  if (data.cases_priority) {
    for (const cp of data.cases_priority) cp.to = "";
  }
  if (data.default) data.default = undefined;
}

// ─── Переименование ноды (drawflow-ключ + data.id) ───

export function renameNode(this: GraphController, oldKey: string, newKey: string): boolean {
  if (!this.editor) return false;
  if (oldKey === newKey) return true;
  const trimmed = newKey.trim();
  if (!trimmed) return false;

  const data = this.editor.drawflow.drawflow.Home.data;

  if (data[trimmed]) {
    showToast(`❌ Узел с ID "${trimmed}" уже существует`, "error");
    return false;
  }

  const nodeData = data[oldKey];
  if (!nodeData) {
    console.error(`[renameNode] oldKey "${oldKey}" не найден`);
    return false;
  }

  this.saveCheckpoint();

  // 1. Перемещаем запись под новый ключ
  data[trimmed] = nodeData;
  delete data[oldKey];

  // 2. Обновляем identity-поля ноды
  data[trimmed].id = trimmed;
  data[trimmed].name = trimmed;
  data[trimmed].data.id = trimmed;

  // 3. Обновляем conn.node во всех inputs/outputs всех нод
  for (const nodeId of Object.keys(data)) {
    const dn = data[nodeId];
    for (const inKey of Object.keys(dn.inputs)) {
      for (const conn of dn.inputs[inKey].connections) {
        if (conn.node === oldKey) conn.node = trimmed;
      }
    }
    for (const outKey of Object.keys(dn.outputs)) {
      for (const conn of dn.outputs[outKey].connections) {
        if (conn.node === oldKey) conn.node = trimmed;
      }
    }
  }

  // 4. Обновляем target-ссылки в data динамических нод
  for (const nodeId of Object.keys(data)) {
    const nd = data[nodeId].data;
    if (!isDynamicNode(nd.type)) continue;
    if (nd.type === "condition_check") {
      if (nd.sequential_to === oldKey) nd.sequential_to = trimmed;
      if (nd.true_to === oldKey) nd.true_to = trimmed;
      if (nd.false_to === oldKey) nd.false_to = trimmed;
    } else if (nd.type === "condition_router") {
      if (nd.true_to === oldKey) nd.true_to = trimmed;
      if (nd.false_to === oldKey) nd.false_to = trimmed;
    } else {
      if (nd.cases_priority) {
        for (const cp of nd.cases_priority) {
          if (cp.to === oldKey) cp.to = trimmed;
        }
      }
      if (nd.default === oldKey) nd.default = trimmed;
    }
  }

  // 5. Обновляем DOM-атрибут id ноды
  const domEl = document.getElementById(`node-${oldKey}`);
  if (domEl) domEl.id = `node-${trimmed}`;

  // 6. Обновляем CSS-классы на SVG-соединениях
  const svgParent = this.editor.precanvas;
  if (svgParent) {
    const inNodes = svgParent.querySelectorAll(`.node_in_node-${oldKey}`);
    for (const el of inNodes) {
      el.classList.remove(`node_in_node-${oldKey}`);
      el.classList.add(`node_in_node-${trimmed}`);
    }
    const outNodes = svgParent.querySelectorAll(`.node_out_node-${oldKey}`);
    for (const el of outNodes) {
      el.classList.remove(`node_out_node-${oldKey}`);
      el.classList.add(`node_out_node-${trimmed}`);
    }
  }

  // 7. Перерисовываем SVG-пути
  this.editor.updateConnectionNodes(`node-${trimmed}`);

  // 8. Перерендериваем HTML ноды
  this.updateNodeHtml(trimmed);

  // 9. Обновляем заголовок сайдбара
  this.el.graphDetailTitle.textContent = `✏️ ${trimmed}`;

  return true;
}

// ─── Обновление HTML ноды ───

export function updateNodeHtml(this: GraphController, nodeId: string): void {
  if (!this.editor) return;
  const dn = this.editor.drawflow.drawflow.Home.data[nodeId];
  if (!dn) return;
  const data = dn.data;
  const el = document.querySelector(`#node-${nodeId} .drawflow_content_node`) as HTMLElement;
  if (!el) return;

  el.innerHTML = this.buildNodeInnerHtml(data, nodeId);

  const nodeEl = el.closest('.drawflow-node') as HTMLElement;
  if (nodeEl) {
    nodeEl.classList.toggle('node-disabled', !!data.disabled);
  }

  this.alignOutputsWithCases(nodeId);
}

// ─── Выравнивание выходов напротив строк кейсов ───

export function alignOutputsWithCases(this: GraphController, nodeId: string): void {
  if (!this.editor) return;
  const dn = this.editor.drawflow.drawflow.Home.data[nodeId];
  if (!dn || !isDynamicNode(dn.data.type)) return;
  const nodeEl = document.getElementById('node-' + nodeId);
  if (!nodeEl) return;
  const caseRows = nodeEl.querySelectorAll('.gn-case-row');
  if (caseRows.length === 0) return;
  const outputsContainer = nodeEl.querySelector('.outputs') as HTMLElement;
  if (!outputsContainer) return;
  const outputs = nodeEl.querySelectorAll('.output') as NodeListOf<HTMLElement>;
  if (outputs.length === 0) return;

  // Контейнер выходов — абсолютно, не участвует в flex-раскладке
  outputsContainer.style.position = 'absolute';
  outputsContainer.style.right = '0';
  outputsContainer.style.top = '0';
  outputsContainer.style.height = '100%';
  outputsContainer.style.width = '20px';
  outputsContainer.style.pointerEvents = 'none';
  outputsContainer.style.paddingTop = '0';

  // Каждый выход — абсолютно напротив своей строки кейса
  const zoom = this.editor.zoom || 1;
  const nodeRect = nodeEl.getBoundingClientRect();
  const borderTop = parseFloat(getComputedStyle(nodeEl).borderTopWidth) || 0;

  for (let i = 0; i < Math.min(caseRows.length, outputs.length); i++) {
    const row = caseRows[i] as HTMLElement;
    const out = outputs[i] as HTMLElement;
    const rowRect = row.getBoundingClientRect();
    const outHeight = out.offsetHeight || 18;
    const rowCenter = (rowRect.top + rowRect.height / 2 - nodeRect.top - borderTop) / zoom;
    out.style.position = 'absolute';
    out.style.top = (rowCenter - outHeight / 2) + 'px';
    out.style.right = '-3px';
    out.style.marginBottom = '0';
    out.style.pointerEvents = 'auto';
  }
}