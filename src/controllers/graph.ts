import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import Drawflow from "drawflow";
import "drawflow/dist/drawflow.min.css";
import { showToast } from "../ui";

interface WorkflowGraphDef {
  team: string;
  name: string;
  file_stem: string;
  visible: boolean;
  config?: any;
  nodes: GraphNodeDef[];
  edges: GraphEdgeDef[];
}

interface GraphNodeDef {
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
  input_object?: string;
  namespace?: string;
  output_type?: string;
  ui_pos?: { x: number; y: number };
}

interface GraphEdgeDef {
  from: string;
  to: string;
  condition?: string;
  case?: string;
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
  btnAddWorker: HTMLButtonElement;
  btnAddSwitch: HTMLButtonElement;
  btnAddSeqSwitch: HTMLButtonElement;
  btnAddExtractor: HTMLButtonElement;
}

const NODE_COLORS: Record<string, string> = {
  llm_worker: "#4caf50",
  llm_classifier: "#42a5f5",
  llm_fact_extractor: "#7e57c2",
  llm_freeform: "#26c6da",
  system_condition: "#ffa726",
  sub_workflow: "#ab47bc",
  switch: "#ef5350",
  llm_sequential_switch: "#e040fb",
  return: "#78909c",
};

const NODE_LABELS: Record<string, string> = {
  llm_worker: "🤖 Worker",
  llm_classifier: "📋 Classifier",
  llm_fact_extractor: "📋 Fact Extractor",
  llm_freeform: "💬 Freeform",
  system_condition: "⚡ Condition",
  sub_workflow: "📦 Sub-workflow",
  switch: "🔀 Switch",
  llm_sequential_switch: "🔀 SeqSwitch",
  return: "🏁 Return",
};

const OUTPUT_COUNT: Record<string, number> = {
  llm_worker: 1,
  llm_classifier: 1,
  llm_fact_extractor: 1,
  llm_freeform: 1,
  system_condition: 1,
  sub_workflow: 1,
  switch: 0,
  llm_sequential_switch: 0,
  return: 0,
};

export class GraphController {
  private el: GraphElements;
  private editor: Drawflow | null = null;
  private currentFilePath: string | null = null;
  private currentWorkflowName: string = "";
  private currentWorkflowConfig: any = null;

  constructor(el: GraphElements) {
    this.el = el;
    this.bindEvents();
  }

  private bindEvents(): void {
    this.el.btnOpenWorkflow.addEventListener("click", () => this.handleOpen());
    this.el.btnSaveWorkflow.addEventListener("click", () => this.handleSave());
    this.el.graphSidebarClose.addEventListener("click", () => this.hideSidebar());
    this.el.btnAddWorker.addEventListener("click", () => this.addNode("llm_worker"));
    this.el.btnAddSwitch.addEventListener("click", () => this.addNode("switch"));
    this.el.btnAddSeqSwitch.addEventListener("click", () => this.addNode("llm_sequential_switch"));
    this.el.btnAddExtractor.addEventListener("click", () => this.addNode("llm_fact_extractor"));
  }

  private ensureEditor(): void {
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

    this.editor.on("click", () => {
      this.hideSidebar();
    });

    this.editor.on("connectionCreated", (...args: any[]) => {
      const d = args[0] || {};
      this.onSwitchConnectionChanged(d.output_id, d.input_id, d.output_class, "created");
    });

    this.editor.on("connectionRemoved", (...args: any[]) => {
      const d = args[0] || {};
      this.onSwitchConnectionChanged(d.output_id, d.input_id, d.output_class, "removed");
    });
  }

  // ─── Системный диалог открытия ───

  private async handleOpen(): Promise<void> {
    try {
      const selected = await openDialog({
        multiple: false,
        filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
      });
      if (!selected) return;

      const wf = await invoke<WorkflowGraphDef>("read_workflow_file", {
        path: selected,
      });

      this.currentFilePath = selected;
      this.currentWorkflowName = wf.name;
      this.currentWorkflowConfig = wf.config ?? null;
      this.el.btnSaveWorkflow.disabled = false;
      this.hideSidebar();
      this.ensureEditor();

      this.el.currentWorkflowName.textContent = `${wf.name} (${wf.file_stem}.yaml)`;

      const nonSwitchEdges = wf.edges.filter((e) => {
        const fn = wf.nodes.find((n) => n.id === e.from);
        return !fn || (fn.type !== "switch" && fn.type !== "llm_sequential_switch");
      });
      const allEdges = [...nonSwitchEdges, ...this.getImplicitSwitchEdges(wf.nodes)];
      const nodePositions = this.computeAutoLayout(wf.nodes, allEdges);

      // Build import data structure with custom IDs as keys
      const importData: import("drawflow").DrawflowExport = {
        drawflow: {
          Home: {
            data: {},
          },
        },
      };
      const nodesData = importData.drawflow.Home.data;

      for (const node of wf.nodes) {
        const pos = node.ui_pos || nodePositions.get(node.id) || { x: 100, y: 100 };
        const outs = (node.type === "switch" || node.type === "llm_sequential_switch") ? this.getSwitchOutputCount(node) : OUTPUT_COUNT[node.type] ?? 1;

        const inputs: Record<string, { connections: any[] }> = {};
        for (let i = 1; i <= 1; i++) inputs[`input_${i}`] = { connections: [] };
        const outputs: Record<string, { connections: any[] }> = {};
        for (let i = 1; i <= outs; i++) outputs[`output_${i}`] = { connections: [] };

        const typeLabel = NODE_LABELS[node.type] || node.type;
        let agentLine = "";
        if (node.type === "llm_worker" && node.agent) {
          agentLine = `<div class="gn-agent">→ ${this.esc(node.agent)}</div>`;
        }
        let casesLine = "";
        if (node.type === "switch" || node.type === "llm_sequential_switch") {
          const labels = this.getSwitchCaseKeys(node);
          if (labels.length > 0) {
            casesLine = `<div class="gn-cases">` + labels.map((key, i) =>
              `<div class="gn-case">${i + 1}: ${this.esc(key)}</div>`
            ).join("") + `</div>`;
          }
        }
        let factsLine = "";
        if (node.type === "llm_fact_extractor") {
          const facts = wf.config?.facts as Array<{ id: string }> | undefined;
          if (facts && facts.length > 0) {
            factsLine = `<div class="gn-cases">` + facts.map((f, i) =>
              `<div class="gn-case">${i + 1}: ${this.esc(f.id)}</div>`
            ).join("") + `</div>`;
          }
        }
        const html = `
          <div class="gn-title">${this.esc(node.id)}</div>
          <div class="gn-type">${typeLabel}</div>
          ${agentLine}
          ${casesLine}
          ${factsLine}
        `;

        nodesData[node.id] = {
          id: node.id,
          name: node.id,
          data: JSON.parse(JSON.stringify(node)),
          class: "",
          html,
          typenode: false,
          inputs,
          outputs,
          pos_x: pos.x,
          pos_y: pos.y,
        };
      }

      this.editor!.import(importData);

      // Навесить обратные соединения для всех загруженных нод
      for (const id of Object.keys(this.editor!.drawflow.drawflow.Home.data)) {
        setTimeout(() => this.enableReverseConnection(id), 0);
      }

      for (const edge of nonSwitchEdges) {
        if (!nodesData[edge.from] || !nodesData[edge.to]) continue;
        try {
          this.editor!.addConnection(edge.from, edge.to, `output_1`, "input_1");
        } catch (connErr) {
          console.error("addConnection error:", connErr);
        }
      }

      // Switch connections come from node data (source of truth)
      for (const node of wf.nodes) {
        if (node.type !== "switch" && node.type !== "llm_sequential_switch") continue;
        const targets: Array<{ to: string; caseKey?: string }> = [];
        if (node.cases_priority) {
          for (const cp of node.cases_priority) targets.push({ to: cp.to, caseKey: cp.key });
        } else if (node.cases) {
          for (const [key, val] of Object.entries(node.cases)) targets.push({ to: val, caseKey: key });
        }
        if (node.default) targets.push({ to: node.default, caseKey: "default" });
        for (const target of targets) {
          if (!target.to || !nodesData[target.to]) continue;
          const outIdx = this.getSwitchOutputIndex(node, target.caseKey);
          try {
            this.editor!.addConnection(node.id, target.to, `output_${outIdx + 1}`, "input_1");
          } catch (connErr) {
            console.error("addConnection error:", connErr);
          }
        }
      }

      this.editor!.zoom_reset();
    } catch (e) {
      console.error("handleOpen error:", e);
      showToast(`Ошибка загрузки: ${e}`, "error");
    }
  }

  private clearEditor(): void {
    if (!this.editor) return;
    const exportData = this.editor.export();
    const nodes = exportData.drawflow.Home.data;
    for (const id of Object.keys(nodes)) {
      this.editor.removeNodeId("node-" + id);
    }
  }

  // ─── Сохранение ───

  private async handleSave(): Promise<void> {
    if (!this.currentFilePath || !this.editor) return;

    try {
      const exportData = this.editor.export();
      const drawflowNodes: Record<string, any> = exportData.drawflow.Home.data;

      const nodes: GraphNodeDef[] = [];
      const edges: GraphEdgeDef[] = [];

      for (const nodeId of Object.keys(drawflowNodes)) {
        const dn = drawflowNodes[nodeId];
        if (dn.data.type === "switch" || dn.data.type === "llm_sequential_switch") {
          this.syncSwitchNodeFromConnections(dn);
        }
        const data = JSON.parse(JSON.stringify(dn.data));
        data.ui_pos = { x: Math.round(dn.pos_x), y: Math.round(dn.pos_y) };
        nodes.push(data);
      }

      for (const nodeId of Object.keys(drawflowNodes)) {
        const dn = drawflowNodes[nodeId];
        for (const inKey of Object.keys(dn.inputs)) {
          for (const conn of dn.inputs[inKey].connections || []) {
            const fromNode = nodes.find((n) => n.id === conn.node);
            if (fromNode && (fromNode.type === "switch" || fromNode.type === "llm_sequential_switch")) {
              continue;
            }
            edges.push({ from: conn.node, to: nodeId });
          }
        }
      }

      const config = this.currentWorkflowConfig ? JSON.parse(JSON.stringify(this.currentWorkflowConfig)) : null;
      if (config?.facts_file) {
        config.facts = [];
      }

      const workflow = { name: this.currentWorkflowName, visible: true, config, nodes, edges };
      await invoke("save_workflow", { path: this.currentFilePath, workflow });
      showToast("✅ Workflow сохранён", "success");
    } catch (e) {
      console.error("handleSave error:", e);
      showToast(`❌ Ошибка сохранения: ${e}`, "error");
    }
  }

  // ─── Редактор ноды в сайдбаре ───

  private showNodeEditor(nodeId: string): void {
    if (!this.editor) return;
    const dn = this.editor.drawflow.drawflow.Home.data[nodeId];
    if (!dn) return;
    const data = dn.data;

    this.el.graphSidebar.classList.add("open");
    this.el.graphDetailTitle.textContent = `✏️ ${nodeId}`;

    const color = NODE_COLORS[data.type] || "#666";
    const typeLabel = NODE_LABELS[data.type] || data.type;

    let html = `<div class="graph-detail-section">
      <div class="detail-label">ID ноды</div>
      <input type="text" id="ge-node-id" class="ge-input" value="${this.esc(data.id || nodeId)}" />
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Тип</div>
      <div class="detail-value" style="border-left:4px solid ${color};padding-left:8px;">${typeLabel}</div>
    </div>`;

    if (data.type === "llm_worker") {
      html += `<div class="graph-detail-section">
        <div class="detail-label">Агент</div>
        <input type="text" id="ge-agent" class="ge-input" placeholder="имя агента" value="${this.esc(data.agent || "")}" />
      </div>`;
      html += `<div class="graph-detail-section">
        <div class="detail-label">Задача (Task)</div>
        <textarea id="ge-task" class="ge-textarea" rows="4">${this.esc(data.task || "")}</textarea>
      </div>`;
    }

    if (data.type === "llm_fact_extractor") {
      const facts = this.currentWorkflowConfig?.facts as Array<{ id: string }> | undefined;
      html += `<div class="graph-detail-section">
        <div class="detail-label">Факты (${facts?.length || 0})</div>
        <div id="ge-facts-list">`;
      if (facts && facts.length > 0) {
        for (const f of facts) {
          html += `<div class="ge-fact-item">${this.esc(f.id)}</div>`;
        }
      } else {
        html += `<span style="color:#888;font-size:12px;">Нет фактов (добавьте в config workflow)</span>`;
      }
      html += `</div></div>`;
    }

    if (data.type === "switch" || data.type === "llm_sequential_switch") {
      if (!data.cases_priority) data.cases_priority = [];
      html += `<div class="graph-detail-section">
        <div class="detail-label">Тип свича</div>
        <select id="ge-switch-type" class="ge-select">
          <option value="switch" ${data.type === "switch" ? "selected" : ""}>🔀 Switch (первый true)</option>
          <option value="llm_sequential_switch" ${data.type === "llm_sequential_switch" ? "selected" : ""}>🔀 SeqSwitch (все true)</option>
        </select>
      </div>`;
      html += `<div class="graph-detail-section">
        <div class="detail-label">Input object (JSON)</div>
        <textarea id="ge-input-object" class="ge-textarea" rows="2">${this.esc(data.input_object || "")}</textarea>
      </div>`;
      html += `<div class="graph-detail-section">
        <div class="detail-label">Кейсы</div>
        <div id="ge-cases-list">`;
      for (let i = 0; i < data.cases_priority.length; i++) {
        const cp = data.cases_priority[i];
        html += `<div class="ge-case-row" data-index="${i}">
          <input class="ge-input ge-case-key" value="${this.esc(cp.key)}" placeholder="ключ" />
          <button class="ge-case-up" title="Вверх">⬆</button>
          <button class="ge-case-down" title="Вниз">⬇</button>
          <button class="ge-case-remove" title="Удалить">🗑</button>
        </div>`;
      }
      html += `</div>
        <button id="ge-case-add" class="btn-secondary" style="margin-top:4px;width:100%;font-size:12px;">+ Добавить кейс</button>
      </div>`;
      html += `<div class="graph-detail-section">
        <div class="detail-label">Default (цель, если ни один не true)</div>
        <input type="text" id="ge-default" class="ge-input" value="${this.esc(data.default || "")}" placeholder="node id" />
      </div>`;
    }

    this.el.graphDetailContent.innerHTML = html;

    const agentInput = document.getElementById("ge-agent") as HTMLInputElement;
    if (agentInput) {
      agentInput.addEventListener("input", () => {
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.agent = agentInput.value;
        this.updateNodeHtml(nodeId);
      });
    }

    const taskTextarea = document.getElementById("ge-task") as HTMLTextAreaElement;
    if (taskTextarea) {
      taskTextarea.addEventListener("change", () => {
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.task = taskTextarea.value;
      });
    }

    const nodeIdInput = document.getElementById("ge-node-id") as HTMLInputElement;
    if (nodeIdInput) {
      nodeIdInput.addEventListener("change", () => {
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.id = nodeIdInput.value;
        this.updateNodeHtml(nodeId);
        this.el.graphDetailTitle.textContent = `✏️ ${nodeIdInput.value}`;
      });
    }

    // ─── Switch/SeqSwitch event handlers ───

    const switchTypeSelect = document.getElementById("ge-switch-type") as HTMLSelectElement;
    if (switchTypeSelect) {
      switchTypeSelect.addEventListener("change", () => {
        const oldType = data.type;
        const newType = switchTypeSelect.value;
        if (oldType !== newType) {
          data.type = newType;
          this.rebuildSwitchOutputs(nodeId);
          this.showNodeEditor(nodeId);
        }
      });
    }

    const inputObjectArea = document.getElementById("ge-input-object") as HTMLTextAreaElement;
    if (inputObjectArea) {
      inputObjectArea.addEventListener("change", () => {
        data.input_object = inputObjectArea.value || undefined;
      });
    }

    const defaultInput = document.getElementById("ge-default") as HTMLInputElement;
    if (defaultInput) {
      defaultInput.addEventListener("change", () => {
        data.default = defaultInput.value || undefined;
        this.rebuildSwitchOutputs(nodeId);
      });
    }

    const addBtn = document.getElementById("ge-case-add");
    if (addBtn) {
      addBtn.addEventListener("click", () => {
        data.cases_priority.push({ key: "", to: "" });
        this.rebuildSwitchOutputs(nodeId);
        this.showNodeEditor(nodeId);
      });
    }

    document.querySelectorAll(".ge-case-remove").forEach((btn) => {
      btn.addEventListener("click", () => {
        const row = (btn as HTMLElement).closest(".ge-case-row") as HTMLElement;
        const idx = parseInt(row.dataset.index || "0", 10);
        data.cases_priority.splice(idx, 1);
        this.rebuildSwitchOutputs(nodeId);
        this.showNodeEditor(nodeId);
      });
    });

    document.querySelectorAll(".ge-case-up").forEach((btn) => {
      btn.addEventListener("click", () => {
        const row = (btn as HTMLElement).closest(".ge-case-row") as HTMLElement;
        const idx = parseInt(row.dataset.index || "0", 10);
        if (idx > 0) {
          const arr = data.cases_priority;
          [arr[idx - 1], arr[idx]] = [arr[idx], arr[idx - 1]];
          this.rebuildSwitchOutputs(nodeId);
          this.showNodeEditor(nodeId);
        }
      });
    });

    document.querySelectorAll(".ge-case-down").forEach((btn) => {
      btn.addEventListener("click", () => {
        const row = (btn as HTMLElement).closest(".ge-case-row") as HTMLElement;
        const idx = parseInt(row.dataset.index || "0", 10);
        const arr = data.cases_priority;
        if (idx < arr.length - 1) {
          [arr[idx], arr[idx + 1]] = [arr[idx + 1], arr[idx]];
          this.rebuildSwitchOutputs(nodeId);
          this.showNodeEditor(nodeId);
        }
      });
    });

    document.querySelectorAll(".ge-case-key").forEach((inp) => {
      inp.addEventListener("input", () => {
        const row = (inp as HTMLElement).closest(".ge-case-row") as HTMLElement;
        const idx = parseInt(row.dataset.index || "0", 10);
        data.cases_priority[idx].key = (inp as HTMLInputElement).value;
        this.updateNodeHtml(nodeId);
      });
    });

  }

  private hideSidebar(): void {
    this.el.graphSidebar.classList.remove("open");
  }

  // ─── Добавление ноды ───

  private addNode(type: string): void {
    this.ensureEditor();
    const id = `${type}_${Date.now()}`;
    const rect = this.el.graphContainer.getBoundingClientRect();
    const cx = (rect.width / 2 - 100) * (1 / (this.editor!.zoom || 1));
    const cy = (rect.height / 2 - 50) * (1 / (this.editor!.zoom || 1));
    const outs = (type === "switch" || type === "llm_sequential_switch") ? 2 : OUTPUT_COUNT[type] ?? 1;

    const inputs: Record<string, { connections: any[] }> = {};
    for (let i = 1; i <= 1; i++) inputs[`input_${i}`] = { connections: [] };
    const outputs: Record<string, { connections: any[] }> = {};
    for (let i = 1; i <= outs; i++) outputs[`output_${i}`] = { connections: [] };

    const nodeData: import("drawflow").DrawflowNode = {
      id,
      name: id,
      data: { id, type },
      class: "",
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

  // ─── Обновление HTML ноды ───

  private updateNodeHtml(nodeId: string): void {
    if (!this.editor) return;
    const dn = this.editor.drawflow.drawflow.Home.data[nodeId];
    if (!dn) return;
    const data = dn.data;
    const el = document.querySelector(`#node-${nodeId} .drawflow_content_node`) as HTMLElement;
    if (!el) return;

    const typeLabel = NODE_LABELS[data.type] || data.type;
    let agentLine = "";
    if (data.type === "llm_worker" && data.agent) {
      agentLine = `<div class="gn-agent">→ ${this.esc(data.agent)}</div>`;
    }
    let casesLine = "";
    if (data.type === "switch" || data.type === "llm_sequential_switch") {
      const labels = this.getSwitchCaseKeys(data);
      if (labels.length > 0) {
        casesLine = `<div class="gn-cases">` + labels.map((key, i) =>
          `<div class="gn-case">${i + 1}: ${this.esc(key)}</div>`
        ).join("") + `</div>`;
      }
    }
    let factsLine = "";
    if (data.type === "llm_fact_extractor") {
      const facts = this.currentWorkflowConfig?.facts as Array<{ id: string }> | undefined;
      if (facts && facts.length > 0) {
        factsLine = `<div class="gn-cases">` + facts.map((f, i) =>
          `<div class="gn-case">${i + 1}: ${this.esc(f.id)}</div>`
        ).join("") + `</div>`;
      }
    }

    el.innerHTML = `
      <div class="gn-title">${this.esc(data.id || nodeId)}</div>
      <div class="gn-type">${typeLabel}</div>
      ${agentLine}
      ${casesLine}
      ${factsLine}
    `;
  }

  private rebuildSwitchOutputs(nodeId: string): void {
    if (!this.editor) return;
    const dn = this.editor.drawflow.drawflow.Home.data[nodeId];
    if (!dn) return;
    const data = dn.data;
    const isSwitch = data.type === "switch" || data.type === "llm_sequential_switch";
    if (!isSwitch) return;

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

  private syncSwitchNodeFromConnections(dn: any): void {
    const data = dn.data;
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

  private onSwitchConnectionChanged(fromNodeId: string, toNodeId: string, outputKey: string, action: "created" | "removed"): void {
    if (!this.editor || !fromNodeId || !toNodeId || !outputKey) return;
    const dn = this.editor.drawflow.drawflow.Home.data[fromNodeId];
    if (!dn) return;
    const data = dn.data;
    if (data.type !== "switch" && data.type !== "llm_sequential_switch") return;
    const match = outputKey.match(/^output_(\d+)$/);
    if (!match) return;
    const outIdx = parseInt(match[1], 10) - 1;
    const caseKeys = this.getSwitchCaseKeys(data);
    if (outIdx < 0 || outIdx >= caseKeys.length) return;
    const caseKey = caseKeys[outIdx];
    if (caseKey === "default") {
      data.default = action === "created" ? toNodeId : undefined;
    } else if (data.cases_priority && outIdx < data.cases_priority.length) {
      data.cases_priority[outIdx].to = action === "created" ? toNodeId : "";
    }
  }

  private enableReverseConnection(nodeId: string): void {
    if (!this.editor) return;
    const nodeEl = document.querySelector(`#node-${nodeId}`);
    if (!nodeEl) return;
    const targetInputEl = nodeEl.querySelector<HTMLElement>('.input');
    if (!targetInputEl) return;
    const precanvas = this.editor.precanvas;
    const zoom = () => this.editor!.zoom;

    nodeEl.querySelectorAll<HTMLElement>('.input').forEach((inputEl) => {
      inputEl.addEventListener('mousedown', (e) => {
        e.stopPropagation();
        const me = e as MouseEvent;

        // Временная SVG линия
        const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
        const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
        path.classList.add("main-path");
        path.setAttributeNS(null, "d", "");
        svg.classList.add("connection");
        svg.classList.add("reverse-drag");
        precanvas.appendChild(svg);
        svg.appendChild(path);

        // Начальная позиция — центр входного порта
        const precanvasRect = precanvas.getBoundingClientRect();
        const inputRect = inputEl.getBoundingClientRect();
        const startX = (inputRect.left + inputRect.width / 2 - precanvasRect.left) / zoom();
        const startY = (inputRect.top + inputRect.height / 2 - precanvasRect.top) / zoom();

        const updateLine = (clientX: number, clientY: number) => {
          const r = precanvas.getBoundingClientRect();
          const z = zoom();
          const endX = (clientX - r.left) / z;
          const endY = (clientY - r.top) / z;
          const dx = Math.abs(endX - startX) * 0.5;
          const d = `M ${startX} ${startY} C ${startX + dx} ${startY} ${endX - dx} ${endY} ${endX} ${endY}`;
          path.setAttributeNS(null, "d", d);
        };

        updateLine(me.clientX, me.clientY);

        const onMouseMove = (ev: MouseEvent) => {
          updateLine(ev.clientX, ev.clientY);
        };

        const onMouseUp = (ev: MouseEvent) => {
          document.removeEventListener('mousemove', onMouseMove);
          document.removeEventListener('mouseup', onMouseUp);
          svg.remove();

          const dropTarget = document.elementFromPoint(ev.clientX, ev.clientY);
          if (!dropTarget) return;
          const outputEl = dropTarget.classList.contains('output')
            ? dropTarget
            : dropTarget.closest<HTMLElement>('.output');
          if (!outputEl) return;
          const sourceNodeEl = outputEl.closest<HTMLElement>('.drawflow-node');
          if (!sourceNodeEl) return;
          const sourceNodeId = sourceNodeEl.id.slice(5);
          if (sourceNodeId === nodeId) return;
          const outputClass = outputEl.classList[1];
          if (!outputClass) return;
          try {
            this.editor!.addConnection(sourceNodeId, nodeId, outputClass, "input_1");
          } catch (_) { }
        };

        document.addEventListener('mousemove', onMouseMove);
        document.addEventListener('mouseup', onMouseUp);
        e.preventDefault();
      }, { capture: true });
    });
  }

  // ─── Утилиты ───

  private getSwitchOutputCount(node: GraphNodeDef): number {
    if (node.cases) return Object.keys(node.cases).length;
    if (node.cases_priority) return node.cases_priority.length + (node.default ? 1 : 0);
    if (node.default) return 1;
    return 2;
  }

  private getSwitchCaseKeys(node: GraphNodeDef): string[] {
    if (node.cases) return Object.keys(node.cases);
    if (node.cases_priority) {
      const keys = node.cases_priority.map((c) => c.key);
      if (node.default) keys.push("default");
      return keys;
    }
    if (node.default) return ["default"];
    return ["default", "other"];
  }

  private getSwitchOutputIndex(node: GraphNodeDef, caseVal?: string): number {
    const keys = this.getSwitchCaseKeys(node);
    const idx = keys.indexOf(caseVal || "default");
    return idx >= 0 ? idx : 0;
  }

  private getImplicitSwitchEdges(nodes: GraphNodeDef[]): GraphEdgeDef[] {
    const implicit: GraphEdgeDef[] = [];
    for (const node of nodes) {
      if (node.type !== "switch" && node.type !== "llm_sequential_switch") continue;
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

  private computeAutoLayout(nodes: GraphNodeDef[], edges: GraphEdgeDef[]): Map<string, { x: number; y: number }> {
    const positions = new Map<string, { x: number; y: number }>();
    const inDegree = new Map<string, number>();
    for (const n of nodes) inDegree.set(n.id, 0);
    for (const e of edges) inDegree.set(e.to, (inDegree.get(e.to) || 0) + 1);

    const levels = new Map<string, number>();
    let queue = nodes.filter((n) => (inDegree.get(n.id) ?? 0) === 0).map((n) => n.id);
    const visited = new Set<string>();
    let level = 0;
    while (queue.length > 0) {
      const next: string[] = [];
      for (const id of queue) {
        if (visited.has(id)) continue;
        visited.add(id);
        levels.set(id, level);
        for (const e of edges.filter((e) => e.from === id)) {
          if (!visited.has(e.to)) next.push(e.to);
        }
      }
      queue = next;
      level++;
    }

    const levelCounts = new Map<number, number>();
    for (const n of nodes) {
      const lvl = levels.get(n.id) ?? 0;
      levelCounts.set(lvl, (levelCounts.get(lvl) || 0) + 1);
    }
    const levelOffsets = new Map<number, number>();
    for (const n of nodes) {
      const lvl = levels.get(n.id) ?? 0;
      const offset = levelOffsets.get(lvl) || 0;
      levelOffsets.set(lvl, offset + 1);
      const total = levelCounts.get(lvl) || 1;
      positions.set(n.id, {
        x: lvl * 320 + 100,
        y: (offset - total / 2) * 140 + 200,
      });
    }
    return positions;
  }

  private esc(s: string): string {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }

  onTabActivated(): void {
    if (!this.editor) {
      this.ensureEditor();
    } else {
      this.editor.zoom_reset();
    }
  }
}
