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
  signal_name?: string;
  field?: string;
  disabled?: boolean;
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
  btnAddSignalRouter: HTMLButtonElement;
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
  signal_router: "#ff9800",
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
  signal_router: "📡 Signal Router",
  return: "🏁 Return",
};

function isDynamicNode(t: string): boolean {
  return t === "switch" || t === "llm_sequential_switch" || t === "signal_router";
}

const OUTPUT_COUNT: Record<string, number> = {
  llm_worker: 1,
  llm_classifier: 1,
  llm_fact_extractor: 1,
  llm_freeform: 1,
  system_condition: 1,
  sub_workflow: 1,
  switch: 0,
  llm_sequential_switch: 0,
  signal_router: 0,
  return: 0,
};

export class GraphController {
  private el: GraphElements;
  private editor: Drawflow | null = null;
  private currentFilePath: string | null = null;
  private currentWorkflowName: string = "";
  private currentWorkflowConfig: any = null;
  private selectedNodes: Set<string> = new Set();
  private isSelecting: boolean = false;
  private selectStart: { x: number; y: number } | null = null;
  private selectionRectEl: HTMLDivElement | null = null;
  private multiDragRaf: number | null = null;
  private multiDragInitial: Map<string, { x: number; y: number }> | null = null;
  private multiDragPrimary: string | null = null;
  private ctxMenu: HTMLDivElement | null = null;

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
    this.el.btnAddSignalRouter.addEventListener("click", () => this.addNode("signal_router"));
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

    this.editor.on("import", () => {
      if (!this.editor) return;
      for (const id of Object.keys(this.editor.drawflow.drawflow.Home.data)) {
        this.alignOutputsWithCases(id);
      }
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

    this.setupCanvasInteraction();
    this.setupContextMenu();
  }

  // ─── Селекторное выделение (LMB box selection) + RMB pan + multi-drag ───

  private setupCanvasInteraction(): void {
    if (!this.editor) return;
    const editor = this.editor;
    const container = this.el.graphContainer;

    // Создаём overlay для рамки выделения
    this.selectionRectEl = document.createElement('div');
    this.selectionRectEl.className = 'graph-selection-rect';
    this.selectionRectEl.style.display = 'none';
    container.appendChild(this.selectionRectEl);

    // ─── Capture-phase mousedown ───
    container.addEventListener('mousedown', (e: MouseEvent) => {
      const target = e.target as HTMLElement;

      // LMB на холсте → box selection
      if (e.button === 0 && (target.classList.contains('drawflow') || target.classList.contains('parent-drawflow'))) {
        e.stopPropagation();
        e.preventDefault();
        this.deselectAllNodes();
        this.startBoxSelection(e);
        return;
      }

      // LMB на ноде → управление выделением
      if (e.button === 0) {
        const nodeEl = target.closest('.drawflow-node') as HTMLElement | null;
        if (nodeEl) {
          const id = nodeEl.id.slice(5);
          if (!this.selectedNodes.has(id)) {
            this.deselectAllNodes();
            this.selectNode(id);
          }
          // Если уже выделена — оставляем все выделенными (multi-drag)
          return;
        }
      }
    }, true);

    // ─── Capture-phase mousemove ───
    container.addEventListener('mousemove', (e: MouseEvent) => {
      if (this.isSelecting) {
        this.updateBoxSelection(e);
      }
    }, true);

    // ─── Capture-phase mouseup ───
    container.addEventListener('mouseup', (e: MouseEvent) => {
      if (this.isSelecting) {
        this.endBoxSelection(e);
      }
      // Очистка multi-drag
      this.multiDragInitial = null;
      this.multiDragPrimary = null;
      if (this.multiDragRaf !== null) {
        cancelAnimationFrame(this.multiDragRaf);
        this.multiDragRaf = null;
      }
    }, true);

    // ─── Multi-drag: intercept mousedown на выделенной ноде ───
    container.addEventListener('mousedown', (e: MouseEvent) => {
      if (e.button !== 0) return;
      const nodeEl = (e.target as HTMLElement).closest('.drawflow-node') as HTMLElement | null;
      if (!nodeEl) return;
      const id = nodeEl.id.slice(5);
      if (!this.selectedNodes.has(id) || this.selectedNodes.size <= 1) return;

      // Сохраняем начальные позиции всех выделенных нод
      this.multiDragPrimary = id;
      this.multiDragInitial = new Map();
      for (const sid of this.selectedNodes) {
        const dn = editor.drawflow.drawflow.Home.data[sid];
        if (dn) {
          this.multiDragInitial.set(sid, { x: dn.pos_x, y: dn.pos_y });
        }
      }

      // Запускаем RAF-цикл синхронизации позиций
      const animate = () => {
        if (!editor.drag || !this.multiDragPrimary || !this.multiDragInitial) {
          this.multiDragRaf = null;
          return;
        }
        const primary = editor.drawflow.drawflow.Home.data[this.multiDragPrimary];
        if (primary) {
          const ini = this.multiDragInitial.get(this.multiDragPrimary);
          if (ini) {
            const dx = primary.pos_x - ini.x;
            const dy = primary.pos_y - ini.y;
            if (dx !== 0 || dy !== 0) {
              for (const sid of this.selectedNodes) {
                if (sid === this.multiDragPrimary) continue;
                const sini = this.multiDragInitial.get(sid);
                if (!sini) continue;
                const dn = editor.drawflow.drawflow.Home.data[sid];
                if (!dn) continue;
                dn.pos_x = sini.x + dx;
                dn.pos_y = sini.y + dy;
                const el = document.getElementById('node-' + sid);
                if (el) {
                  el.style.top = dn.pos_y + 'px';
                  el.style.left = dn.pos_x + 'px';
                }
              }
              for (const sid of this.selectedNodes) {
                editor.updateConnectionNodes('node-' + sid);
              }
            }
          }
        }
        this.multiDragRaf = requestAnimationFrame(animate);
      };
      this.multiDragRaf = requestAnimationFrame(animate);
    }, true);
  }

  private selectNode(id: string): void {
    this.selectedNodes.add(id);
    const el = document.getElementById('node-' + id);
    if (el) el.classList.add('selected');
  }

  private deselectAllNodes(): void {
    for (const id of this.selectedNodes) {
      const el = document.getElementById('node-' + id);
      if (el) el.classList.remove('selected');
    }
    this.selectedNodes.clear();
  }

  private startBoxSelection(e: MouseEvent): void {
    const container = this.el.graphContainer;
    const rect = container.getBoundingClientRect();
    this.selectStart = {
      x: e.clientX - rect.left,
      y: e.clientY - rect.top,
    };
    this.isSelecting = true;
    if (this.selectionRectEl) {
      this.selectionRectEl.style.display = 'block';
      this.selectionRectEl.style.left = this.selectStart.x + 'px';
      this.selectionRectEl.style.top = this.selectStart.y + 'px';
      this.selectionRectEl.style.width = '0';
      this.selectionRectEl.style.height = '0';
    }
  }

  private updateBoxSelection(e: MouseEvent): void {
    if (!this.selectStart || !this.selectionRectEl) return;
    const container = this.el.graphContainer;
    const rect = container.getBoundingClientRect();
    const cx = e.clientX - rect.left;
    const cy = e.clientY - rect.top;

    const x = Math.min(this.selectStart.x, cx);
    const y = Math.min(this.selectStart.y, cy);
    const w = Math.abs(cx - this.selectStart.x);
    const h = Math.abs(cy - this.selectStart.y);

    this.selectionRectEl.style.left = x + 'px';
    this.selectionRectEl.style.top = y + 'px';
    this.selectionRectEl.style.width = w + 'px';
    this.selectionRectEl.style.height = h + 'px';
  }

  private endBoxSelection(_e: MouseEvent): void {
    this.isSelecting = false;
    if (this.selectionRectEl) {
      this.selectionRectEl.style.display = 'none';
    }
    if (!this.selectStart || !this.editor) return;

    // Получаем границы рамки выделения (с учётом зума)
    const containerRect = this.el.graphContainer.getBoundingClientRect();
    const selLeft = parseFloat(this.selectionRectEl!.style.left);
    const selTop = parseFloat(this.selectionRectEl!.style.top);
    const selRight = selLeft + parseFloat(this.selectionRectEl!.style.width);
    const selBottom = selTop + parseFloat(this.selectionRectEl!.style.height);

    // Минимальный размер для детекта (чтобы простой клик не выделял)
    if (selRight - selLeft < 5 && selBottom - selTop < 5) {
      this.selectStart = null;
      return;
    }

    const data = this.editor.drawflow.drawflow.Home.data;

    for (const id of Object.keys(data)) {
      const nodeEl = document.getElementById('node-' + id);
      if (!nodeEl) continue;
      const nodeRect = nodeEl.getBoundingClientRect();
      // Проверяем пересечение ноды с рамкой (мировые координаты)
      const nx = nodeRect.left - containerRect.left;
      const ny = nodeRect.top - containerRect.top;
      const nr = nx + nodeRect.width;
      const nb = ny + nodeRect.height;

      if (nx < selRight && nr > selLeft && ny < selBottom && nb > selTop) {
        this.selectNode(id);
      }
    }

    this.selectStart = null;
  }

  // ─── Контекстное меню (ПКМ) на нодах ───

  private setupContextMenu(): void {
    // Создаём элемент меню
    this.ctxMenu = document.createElement('div');
    this.ctxMenu.className = 'graph-context-menu';
    document.body.appendChild(this.ctxMenu);

    // Закрытие по клику вне меню
    document.addEventListener('mousedown', (e) => {
      if (this.ctxMenu && !this.ctxMenu.contains(e.target as Node)) {
        this.ctxMenu.classList.remove('open');
      }
    });

    // Закрытие по Escape
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && this.ctxMenu) {
        this.ctxMenu.classList.remove('open');
      }
    });

    // ПКМ на холсте графа
    this.el.graphContainer.addEventListener('contextmenu', (e) => {
      const nodeEl = (e.target as HTMLElement).closest('.drawflow-node') as HTMLElement;
      if (!nodeEl) {
        // Клик вне ноды — закрываем меню
        if (this.ctxMenu) this.ctxMenu.classList.remove('open');
        return;
      }
      e.preventDefault();
      e.stopPropagation();

      const nodeId = nodeEl.id.slice(5); // "node-xxx" → "xxx"

      const dn = this.editor?.drawflow.drawflow.Home.data[nodeId];
      const isDisabled = dn?.data?.disabled ? true : false;

      this.ctxMenu!.innerHTML = `
        <div class="ctx-item" data-action="toggle-disabled">
          <span class="ctx-icon">${isDisabled ? "✅" : "⛔"}</span>
          ${isDisabled ? "Включить ноду" : "Отключить ноду"}
        </div>
      `;

      // Позиционируем меню
      this.ctxMenu!.style.left = Math.min(e.clientX, window.innerWidth - 200) + 'px';
      this.ctxMenu!.style.top = Math.min(e.clientY, window.innerHeight - 100) + 'px';
      this.ctxMenu!.classList.add('open');

      // Обработчик клика по пункту меню
      this.ctxMenu!.querySelector('[data-action="toggle-disabled"]')?.addEventListener('click', () => {
        this.toggleNodeDisabled(nodeId);
        this.ctxMenu?.classList.remove('open');
      });
    });
  }

  private toggleNodeDisabled(nodeId: string): void {
    if (!this.editor) return;
    const dn = this.editor.drawflow.drawflow.Home.data[nodeId];
    if (!dn) return;
    dn.data.disabled = !dn.data.disabled || undefined;
    this.updateNodeHtml(nodeId);
    // Обновляем сайдбар, если он открыт для этой ноды
    if (this.el.graphSidebar.classList.contains('open')) {
      const titleEl = this.el.graphDetailTitle;
      if (titleEl.textContent?.includes(nodeId)) {
        this.showNodeEditor(nodeId);
      }
    }
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
        return !fn || !isDynamicNode(fn.type);
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

      // Нормализация: конвертируем старый cases → cases_priority
      for (const node of wf.nodes) {
        if (isDynamicNode(node.type) && node.cases && typeof node.cases === "object" && !Array.isArray(node.cases)) {
          node.cases_priority = Object.entries(node.cases).map(([key, to]) => ({ key, to: to as string }));
          delete node.cases;
        }
      }

      for (const node of wf.nodes) {
        const pos = node.ui_pos || nodePositions.get(node.id) || { x: 100, y: 100 };
        const outs = isDynamicNode(node.type) ? this.getSwitchOutputCount(node) : OUTPUT_COUNT[node.type] ?? 1;

        const inputs: Record<string, { connections: any[] }> = {};
        for (let i = 1; i <= 1; i++) inputs[`input_${i}`] = { connections: [] };
        const outputs: Record<string, { connections: any[] }> = {};
        for (let i = 1; i <= outs; i++) outputs[`output_${i}`] = { connections: [] };

        const typeLabel = NODE_LABELS[node.type] || node.type;
        let agentLine = "";
        if (node.type === "llm_worker" && node.agent) {
          agentLine = `<div class="gn-agent">→ ${this.esc(node.agent)}</div>`;
        }
        if (node.type === "llm_worker" && node.output_type) {
          agentLine += `<div class="gn-agent">${this.esc(node.output_type)}</div>`;
        }
        let signalLine = "";
        if (node.type === "signal_router") {
          const parts: string[] = [];
          if (node.signal_name) parts.push(this.esc(node.signal_name));
          if (node.field) parts.push(`.${this.esc(node.field)}`);
          if (parts.length > 0) {
            signalLine = `<div class="gn-agent" style="color:#ff9800">📡 ${parts.join("")}</div>`;
          }
        }
        let casesLine = "";
        if (isDynamicNode(node.type)) {
          const labels = this.getSwitchCaseKeys(node);
          if (labels.length > 0) {
            casesLine = `<div class="gn-cases">` + labels.map((key, i) =>
              `<div class="gn-case-row">
                <span class="gn-case-idx">${i + 1}</span>
                <span class="gn-case-key">${this.esc(key)}</span>
                <span class="gn-case-dot"></span>
              </div>`
            ).join("") + `</div>`;
          }
        }
        let factsLine = "";
        if (node.type === "llm_fact_extractor") {
          const facts = wf.config?.facts as Array<{ id: string }> | undefined;
          if (facts && facts.length > 0) {
            factsLine = `<div class="gn-cases">` + facts.map((f, i) =>
              `<div class="gn-case-row">
                <span class="gn-case-idx">${i + 1}</span>
                <span class="gn-case-key">${this.esc(f.id)}</span>
                <span class="gn-case-dot"></span>
              </div>`
            ).join("") + `</div>`;
          }
        }
        const disabledBadge = node.disabled ? `<div class="gn-disabled-badge">⛔ DISABLED</div>` : "";
        const html = `
          <div class="gn-title">${this.esc(node.id)}</div>
          <div class="gn-type">${typeLabel}</div>
          ${disabledBadge}
          ${agentLine}
          ${signalLine}
          ${casesLine}
          ${factsLine}
        `;

        let nodeClass = isDynamicNode(node.type) ? "dynamic-node" : "";
        if (node.disabled) nodeClass = nodeClass ? `${nodeClass} node-disabled` : "node-disabled";

        nodesData[node.id] = {
          id: node.id,
          name: node.id,
          data: JSON.parse(JSON.stringify(node)),
          class: nodeClass,
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

      // Switch/SignalRouter connections come from node data (source of truth)
      for (const node of wf.nodes) {
        if (!isDynamicNode(node.type)) continue;
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
    if (isDynamicNode(dn.data.type)) {
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
            if (fromNode && isDynamicNode(fromNode.type)) {
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

    const isDisabled = !!data.disabled;

    let html = `<div class="graph-detail-section">
      <div class="detail-label">ID ноды</div>
      <input type="text" id="ge-node-id" class="ge-input" value="${this.esc(data.id || nodeId)}" />
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Тип</div>
      <div class="detail-value" style="border-left:4px solid ${color};padding-left:8px;">${typeLabel}</div>
    </div>
    <div class="graph-detail-section ge-disabled-row">
      <div class="detail-label">Состояние</div>
      <label class="ge-toggle">
        <input type="checkbox" id="ge-disabled" ${isDisabled ? "checked" : ""} />
        <span class="ge-toggle-slider"></span>
        <span class="ge-toggle-label">${isDisabled ? "⛔ Отключена" : "✅ Включена"}</span>
      </label>
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
      html += `<div class="graph-detail-section">
        <div class="detail-label">Тип вывода</div>
        <select id="ge-output-type" class="ge-select">
          <option value="message" ${data.output_type === "message" ? "selected" : ""}>message</option>
          <option value="thought" ${!data.output_type || data.output_type === "thought" ? "selected" : ""}>thought</option>
        </select>
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

    if (data.type === "signal_router") {
      if (data.cases && typeof data.cases === "object" && !Array.isArray(data.cases)) {
        data.cases_priority = Object.entries(data.cases).map(([key, to]) => ({ key, to: to as string }));
        delete data.cases;
      }
      if (!data.cases_priority) data.cases_priority = [];
      html += `<div class="graph-detail-section">
        <div class="detail-label">Имя сигнала</div>
        <input type="text" id="ge-signal-name" class="ge-input" placeholder="validator_report" value="${this.esc(data.signal_name || "")}" />
      </div>
      <div class="graph-detail-section">
        <div class="detail-label">Поле в сигнале</div>
        <input type="text" id="ge-field" class="ge-input" placeholder="verdict" value="${this.esc(data.field || "")}" />
      </div>`;
      html += this.renderCasesHtml(data);
      html += this.renderDefaultHtml(data);
    }

    if (data.type === "switch" || data.type === "llm_sequential_switch") {
      // Конвертируем старый формат cases → cases_priority, чтобы не дублировались при сохранении
      if (data.cases && typeof data.cases === "object" && !Array.isArray(data.cases)) {
        data.cases_priority = Object.entries(data.cases).map(([key, to]) => ({ key, to: to as string }));
        delete data.cases;
      }
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
      html += this.renderCasesHtml(data);
      html += this.renderDefaultHtml(data);
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

    const outputTypeSelect = document.getElementById("ge-output-type") as HTMLSelectElement;
    if (outputTypeSelect) {
      outputTypeSelect.addEventListener("change", () => {
        data.output_type = outputTypeSelect.value === "thought" ? undefined : outputTypeSelect.value;
        this.updateNodeHtml(nodeId);
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

    const disabledToggle = document.getElementById("ge-disabled") as HTMLInputElement;
    if (disabledToggle) {
      disabledToggle.addEventListener("change", () => {
        const isNowDisabled = disabledToggle.checked;
        data.disabled = isNowDisabled || undefined;
        // обновляем текст рядом с переключателем
        const label = disabledToggle.parentElement?.querySelector('.ge-toggle-label');
        if (label) label.textContent = isNowDisabled ? "⛔ Отключена" : "✅ Включена";
        this.updateNodeHtml(nodeId);
      });
    }

    const signalNameInput = document.getElementById("ge-signal-name") as HTMLInputElement;
    if (signalNameInput) {
      signalNameInput.addEventListener("change", () => {
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.signal_name = signalNameInput.value || undefined;
      });
    }

    const fieldInput = document.getElementById("ge-field") as HTMLInputElement;
    if (fieldInput) {
      fieldInput.addEventListener("change", () => {
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.field = fieldInput.value || undefined;
      });
    }

    // ─── Switch/SeqSwitch/SignalRouter event handlers ───

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
    const outs = isDynamicNode(type) ? 2 : OUTPUT_COUNT[type] ?? 1;

    const inputs: Record<string, { connections: any[] }> = {};
    for (let i = 1; i <= 1; i++) inputs[`input_${i}`] = { connections: [] };
    const outputs: Record<string, { connections: any[] }> = {};
    for (let i = 1; i <= outs; i++) outputs[`output_${i}`] = { connections: [] };

    const nodeData: import("drawflow").DrawflowNode = {
      id,
      name: id,
      data: { id, type },
      class: isDynamicNode(type) ? "dynamic-node" : "",
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
    if (data.type === "llm_worker" && data.output_type) {
      agentLine += `<div class="gn-agent">${this.esc(data.output_type)}</div>`;
    }
    let signalLine = "";
    if (data.type === "signal_router") {
      const parts: string[] = [];
      if (data.signal_name) parts.push(this.esc(data.signal_name));
      if (data.field) parts.push(`.${this.esc(data.field)}`);
      if (parts.length > 0) {
        signalLine = `<div class="gn-agent" style="color:#ff9800">📡 ${parts.join("")}</div>`;
      }
    }
    let casesLine = "";
    if (isDynamicNode(data.type)) {
      const labels = this.getSwitchCaseKeys(data);
      if (labels.length > 0) {
        casesLine = `<div class="gn-cases">` + labels.map((key, i) =>
          `<div class="gn-case-row">
            <span class="gn-case-idx">${i + 1}</span>
            <span class="gn-case-key">${this.esc(key)}</span>
            <span class="gn-case-dot"></span>
          </div>`
        ).join("") + `</div>`;
      }
    }
    let factsLine = "";
    if (data.type === "llm_fact_extractor") {
      const facts = this.currentWorkflowConfig?.facts as Array<{ id: string }> | undefined;
      if (facts && facts.length > 0) {
        factsLine = `<div class="gn-cases">` + facts.map((f, i) =>
          `<div class="gn-case-row">
            <span class="gn-case-idx">${i + 1}</span>
            <span class="gn-case-key">${this.esc(f.id)}</span>
            <span class="gn-case-dot"></span>
          </div>`
        ).join("") + `</div>`;
      }
    }

    const disabledBadge = data.disabled ? `<div class="gn-disabled-badge">⛔ DISABLED</div>` : "";

    el.innerHTML = `
      <div class="gn-title">${this.esc(data.id || nodeId)}</div>
      <div class="gn-type">${typeLabel}</div>
      ${disabledBadge}
      ${agentLine}
      ${signalLine}
      ${casesLine}
      ${factsLine}
    `;

    const nodeEl = el.closest('.drawflow-node') as HTMLElement;
    if (nodeEl) {
      nodeEl.classList.toggle('node-disabled', !!data.disabled);
    }

    this.alignOutputsWithCases(nodeId);
  }

  private rebuildSwitchOutputs(nodeId: string): void {
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
    if (!isDynamicNode(data.type)) return;
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

  // ─── Рендер кейсов и дефолта (общее для switch и signal_router) ───

  private renderCasesHtml(data: any): string {
    let html = `<div class="graph-detail-section">
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
    return html;
  }

  private renderDefaultHtml(data: any): string {
    return `<div class="graph-detail-section">
      <div class="detail-label">Default (цель, если не совпало)</div>
      <input type="text" id="ge-default" class="ge-input" value="${this.esc(data.default || "")}" placeholder="node id" />
    </div>`;
  }

  // ─── Выравнивание выходов напротив строк кейсов ───

  private alignOutputsWithCases(nodeId: string): void {
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
      if (!isDynamicNode(node.type)) continue;
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
