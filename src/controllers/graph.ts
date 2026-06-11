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
  return: 0,
};

export class GraphController {
  private el: GraphElements;
  private editor: Drawflow | null = null;
  private currentFilePath: string | null = null;

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
    });

    this.editor.on("click", () => {
      this.el.graphDetailContent.innerHTML = `<p class="graph-hint">Выберите ноду для редактирования</p>`;
      this.el.graphDetailTitle.textContent = "Информация";
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
      this.el.btnSaveWorkflow.disabled = false;
      this.hideSidebar();
      this.ensureEditor();

      this.el.currentWorkflowName.textContent = `${wf.name} (${wf.file_stem}.yaml)`;

      const allEdges = [...wf.edges, ...this.getImplicitSwitchEdges(wf.nodes)];
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
        const outs = node.type === "switch" ? this.getSwitchOutputCount(node) : OUTPUT_COUNT[node.type] ?? 1;

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
        if (node.type === "switch") {
          const labels = this.getSwitchCaseKeys(node);
          if (labels.length > 0) {
            casesLine = `<div class="gn-cases">` + labels.map((key, i) =>
              `<div class="gn-case">${i + 1}: ${this.esc(key)}</div>`
            ).join("") + `</div>`;
          }
        }
        const html = `
          <div class="gn-title">${this.esc(node.id)}</div>
          <div class="gn-type">${typeLabel}</div>
          ${agentLine}
          ${casesLine}
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

      for (const edge of wf.edges) {
        if (!nodesData[edge.from] || !nodesData[edge.to]) continue;
        const fromNode = wf.nodes.find((n) => n.id === edge.from);
        const outIdx = fromNode?.type === "switch" ? this.getSwitchOutputIndex(fromNode, edge.case) : 0;
        try {
          this.editor!.addConnection(edge.from, edge.to, `output_${outIdx + 1}`, "input_1");
        } catch (connErr) {
          console.error("addConnection error:", connErr);
        }
      }

      // Generate implicit edges for switch nodes from cases_priority/cases/default
      for (const node of wf.nodes) {
        if (node.type !== "switch") continue;
        const targets: Array<{ to: string; caseKey?: string }> = [];
        if (node.cases_priority) {
          for (const cp of node.cases_priority) targets.push({ to: cp.to, caseKey: cp.key });
        } else if (node.cases) {
          for (const [key, val] of Object.entries(node.cases)) targets.push({ to: val, caseKey: key });
        }
        if (node.default) targets.push({ to: node.default, caseKey: "default" });
        for (const target of targets) {
          const alreadyExists = wf.edges.some(
            (e) => e.from === node.id && e.to === target.to && (!target.caseKey || e.case === target.caseKey)
          );
          if (alreadyExists) continue;
          if (!nodesData[target.to]) continue;
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
        const data = JSON.parse(JSON.stringify(dn.data));
        data.ui_pos = { x: Math.round(dn.pos_x), y: Math.round(dn.pos_y) };
        nodes.push(data);
      }

      for (const nodeId of Object.keys(drawflowNodes)) {
        const dn = drawflowNodes[nodeId];
        for (const inKey of Object.keys(dn.inputs)) {
          for (const conn of dn.inputs[inKey].connections || []) {
            const fromNode = nodes.find((n) => n.id === conn.node);
            let caseVal: string | undefined;
            if (fromNode && fromNode.type === "switch") {
              caseVal = this.getCaseKeyForOutput(fromNode, parseInt(conn.input.replace("output_", ""), 10) - 1);
            }
            edges.push({ from: conn.node, to: nodeId, case: caseVal });
          }
        }
      }

      const workflow = { name: "", visible: true, config: null, nodes, edges };
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
        <select id="ge-agent" class="ge-select">
          <option value="">— не выбран —</option>
          ${this.loadedAgents().map((a) => `<option value="${this.esc(a.id)}" ${a.id === data.agent ? "selected" : ""}>${this.esc(a.name)}</option>`).join("")}
        </select>
      </div>`;
      html += `<div class="graph-detail-section">
        <div class="detail-label">Задача (Task)</div>
        <textarea id="ge-task" class="ge-textarea" rows="4">${this.esc(data.task || "")}</textarea>
      </div>`;
    }

    if (data.type === "switch") {
      const caseEntries = data.cases ? Object.entries(data.cases as Record<string, string>) : [];
      html += `<div class="graph-detail-section">
        <div class="detail-label">Варианты (cases)</div>
        <div id="ge-cases-list">`;
      for (const [key, val] of caseEntries) {
        html += `<div style="display:flex;gap:6px;margin-bottom:4px;">
          <input class="ge-input" style="flex:1;" value="${this.esc(key)}" readonly />
          <input class="ge-input" style="flex:1;" value="${this.esc(val)}" readonly />
        </div>`;
      }
      if (caseEntries.length === 0) {
        html += `<span style="color:#888;font-size:12px;">Нет вариантов</span>`;
      }
      html += `</div></div>`;
    }

    this.el.graphDetailContent.innerHTML = html;

    const agentSelect = document.getElementById("ge-agent") as HTMLSelectElement;
    if (agentSelect) {
      agentSelect.addEventListener("change", () => {
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.agent = agentSelect.value;
        this.updateNodeHtml(nodeId);
      });
    }

    const taskTextarea = document.getElementById("ge-task") as HTMLTextAreaElement;
    if (taskTextarea) {
      taskTextarea.addEventListener("change", () => {
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.task = taskTextarea.value;
      });
    }
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
    const outs = type === "switch" ? 2 : OUTPUT_COUNT[type] ?? 1;

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
    if (data.type === "switch") {
      const labels = this.getSwitchCaseKeys(data);
      if (labels.length > 0) {
        casesLine = `<div class="gn-cases">` + labels.map((key, i) =>
          `<div class="gn-case">${i + 1}: ${this.esc(key)}</div>`
        ).join("") + `</div>`;
      }
    }

    el.innerHTML = `
      <div class="gn-title">${this.esc(data.id || nodeId)}</div>
      <div class="gn-type">${typeLabel}</div>
      ${agentLine}
      ${casesLine}
    `;
  }

  // ─── Утилиты ───

  private loadedAgents(): Array<{ id: string; name: string }> {
    return [];
  }

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

  private getCaseKeyForOutput(node: GraphNodeDef, outputIdx: number): string | undefined {
    const keys = this.getSwitchCaseKeys(node);
    return keys[outputIdx];
  }

  private getImplicitSwitchEdges(nodes: GraphNodeDef[]): GraphEdgeDef[] {
    const implicit: GraphEdgeDef[] = [];
    for (const node of nodes) {
      if (node.type !== "switch") continue;
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
