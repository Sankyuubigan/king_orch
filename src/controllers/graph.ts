import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import Drawflow from "drawflow";
import "drawflow/dist/drawflow.min.css";
import { showToast } from "../ui";

interface GraphSnapshot {
  data: Record<string, any>;
}

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
  btnUndo: HTMLButtonElement;
  btnRedo: HTMLButtonElement;
  dirtyIndicator: HTMLSpanElement;
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
  condition_check: "#26a69a",
  return: "#78909c",
  note: "#e53935",
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
  condition_check: "🎯 Condition Check",
  return: "🏁 Return",
  note: "📝 Note",
};

function isDynamicNode(t: string): boolean {
  return t === "switch" || t === "llm_sequential_switch" || t === "signal_router" || t === "condition_check";
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
  condition_check: 0,
  return: 0,
  note: 1,
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
  private undoStack: GraphSnapshot[] = [];
  private redoStack: GraphSnapshot[] = [];
  private isRestoring: boolean = false;
  private isDirty: boolean = false;
  private pristineSnapshot: GraphSnapshot | null = null;

  private static readonly CONDITION_CHECK_CASES = [
    { key: "true", field: "true_to" as const },
    { key: "false", field: "false_to" as const },
    { key: "seq", field: "sequential_to" as const },
  ];

  constructor(el: GraphElements) {
    this.el = el;
    this.bindEvents();
  }

  private bindEvents(): void {
    this.el.btnOpenWorkflow.addEventListener("click", () => this.handleOpen());
    this.el.btnSaveWorkflow.addEventListener("click", () => this.handleSave());
    this.el.graphSidebarClose.addEventListener("click", () => this.hideSidebar());
    this.el.btnUndo.addEventListener("click", () => this.undo());
    this.el.btnRedo.addEventListener("click", () => this.redo());
    this.setupKeyboardShortcuts();
  }

  private setupKeyboardShortcuts(): void {
    document.addEventListener("keydown", (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "z") {
        if (e.shiftKey) {
          e.preventDefault();
          this.redo();
        } else {
          e.preventDefault();
          this.undo();
        }
      }
    });
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
      if (this.isRestoring) return;
      const d = args[0] || {};
      this.onSwitchConnectionChanged(d.output_id, d.input_id, d.output_class, "created");
    });

    this.editor.on("connectionRemoved", (...args: any[]) => {
      if (this.isRestoring) return;
      const d = args[0] || {};
      this.onSwitchConnectionChanged(d.output_id, d.input_id, d.output_class, "removed");
    });

    this.setupCanvasInteraction();
    this.setupContextMenu();
    this.setupEdgeDrag();
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

      if (e.button === 0) {
        const nodeEl = target.closest('.drawflow-node') as HTMLElement | null;
        const canvasEl = target.closest('.drawflow') || target.closest('.parent-drawflow');

        if (nodeEl) {
          const id = nodeEl.id.slice(5);
          if (!this.selectedNodes.has(id)) {
            this.deselectAllNodes();
            this.selectNode(id);
          }
          return;
        }

        if (canvasEl) {
          this.deselectAllNodes();
          if (e.ctrlKey || e.metaKey) {
            e.stopPropagation();
            e.preventDefault();
            this.startBoxSelection(e);
          }
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

    // ─── Совместный снапшот для Drawflow-native connection + multi-drag ───
    container.addEventListener('mousedown', (e: MouseEvent) => {
      if (e.button !== 0) return;
      const outputEl = (e.target as HTMLElement).closest('.output');
      if (outputEl && this.editor && !this.isRestoring) {
        this.saveCheckpoint();
      }
    }, true);

    // ─── Снапшот для перетаскивания одной ноды ───
    container.addEventListener('mousedown', (e: MouseEvent) => {
      if (e.button !== 0) return;
      if (this.isRestoring) return;
      if (!this.editor) return;
      const outputEl = (e.target as HTMLElement).closest('.output');
      if (outputEl) return;
      const nodeEl = (e.target as HTMLElement).closest('.drawflow-node');
      if (nodeEl) {
        this.saveCheckpoint();
      }
    }, true);

    // ─── Multi-drag: intercept mousedown на выделенной ноде ───
    container.addEventListener('mousedown', (e: MouseEvent) => {
      if (e.button !== 0) return;
      const nodeEl = (e.target as HTMLElement).closest('.drawflow-node') as HTMLElement | null;
      if (!nodeEl) return;
      const id = nodeEl.id.slice(5);
      if (!this.selectedNodes.has(id) || this.selectedNodes.size <= 1) return;

      this.saveCheckpoint();

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
        // Клик вне ноды — меню создания
        e.preventDefault();
        e.stopPropagation();
        this.ctxMenu!.innerHTML = `
          <div class="ctx-item has-submenu" data-action="create">
            <span class="ctx-icon">➕</span>
            Создать
            <span class="ctx-submenu-arrow">▶</span>
            <div class="ctx-submenu-list">
              <div class="ctx-item" data-type="llm_worker">🤖 Worker</div>
              <div class="ctx-item" data-type="switch">🔀 Switch</div>
              <div class="ctx-item" data-type="llm_sequential_switch">🔀 SeqSwitch</div>
              <div class="ctx-item" data-type="signal_router">📡 Signal Router</div>
              <div class="ctx-item" data-type="condition_check">🎯 Condition</div>
              <div class="ctx-item" data-type="llm_fact_extractor">📋 Extractor</div>
              <div class="ctx-item" data-type="llm_freeform">💬 Freeform</div>
              <div class="ctx-item" data-type="note">📝 Note</div>
            </div>
          </div>
        `;
        this.ctxMenu!.style.left = Math.min(e.clientX, window.innerWidth - 220) + 'px';
        this.ctxMenu!.style.top = Math.min(e.clientY, window.innerHeight - 200) + 'px';
        this.ctxMenu!.classList.add('open');

        // Клик по подпункту подменю
        const cmX = e.clientX;
        const cmY = e.clientY;
        this.ctxMenu!.querySelectorAll('.ctx-submenu-list .ctx-item').forEach((item) => {
          item.addEventListener('click', (ev) => {
            ev.stopPropagation();
            const type = (item as HTMLElement).dataset.type!;
            this.addNode(type, cmX, cmY);
            this.ctxMenu?.classList.remove('open');
          });
        });
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
    this.saveCheckpoint();
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

        const html = this.buildNodeInnerHtml(node, node.id);

        let nodeClass = "";
        if (node.type === "note") nodeClass = "node-type-note";
        else if (isDynamicNode(node.type)) nodeClass = "dynamic-node";
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

      this.isRestoring = true;
      this.editor!.import(importData);
      this.isRestoring = false;

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

      // Switch/SignalRouter/ConditionCheck connections come from node data (source of truth)
      for (const node of wf.nodes) {
        if (!isDynamicNode(node.type)) continue;
        const targets: Array<{ to: string; caseKey?: string }> = [];
        if (node.type === "condition_check") {
          if (node.true_to) targets.push({ to: node.true_to, caseKey: "true" });
          if (node.false_to) targets.push({ to: node.false_to, caseKey: "false" });
          if (node.sequential_to) targets.push({ to: node.sequential_to, caseKey: "seq" });
        } else if (node.cases_priority) {
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
      this.resetHistory();
    } catch (e) {
      console.error("handleOpen error:", e);
      showToast(`Ошибка загрузки: ${e}`, "error");
    }
  }

  private clearEditor(): void {
    if (!this.editor) return;
    this.saveCheckpoint();
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
            if (!fromNode) {
              console.warn(`[handleSave] fromNode не найден для conn.node="${conn.node}" (target="${nodeId}"). drawflow-ключ расходится с data.id — это баг renameNode`);
            }
            if (fromNode && isDynamicNode(fromNode.type)) {
              continue;
            }
            edges.push({ from: conn.node, to: nodeId });
          }
        }
      }

      // Валидация: каждое ребро должно ссылаться на существующий node.id
      const badEdges = edges.filter(e => !nodes.find(n => n.id === e.from) || !nodes.find(n => n.id === e.to));
      if (badEdges.length > 0) {
        console.error("[handleSave] Битые рёбра (ссылаются на несуществующие node.id):", JSON.stringify(badEdges));
        showToast(`⚠️ ${badEdges.length} ребер имеют неверные ID нод — данные могут потеряться`, "error");
      }

      const config = this.currentWorkflowConfig ? JSON.parse(JSON.stringify(this.currentWorkflowConfig)) : null;
      if (config?.facts_file) {
        config.facts = [];
      }

      const workflow = { name: this.currentWorkflowName, visible: true, config, nodes, edges };
      const saveResult = await invoke("save_workflow", { path: this.currentFilePath, workflow });
      void saveResult;
      this.pristineSnapshot = this.captureSnapshot();
      this.markClean();
      showToast("✅ Workflow сохранён", "success");
    } catch (e) {
      // Файл НЕ сохранён (бэкенд отклоняет невалидный YAML до записи).
      // Оставляем dirty-флаг — данные на холсте считаются несохранёнными.
      console.error("[handleSave] Сохранение НЕ выполнено, файл не перезаписан:", e);
      showToast(`❌ Не сохранено: ${e}`, "error");
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
      html += `<div class="graph-detail-section">
        <div class="detail-label">Прикрепить отчеты (ID агентов)</div>
        <input type="text" id="ge-inject-reports" class="ge-input" placeholder="через запятую: soma_translator, curator" value="${this.esc((data.inject_reports || []).join(', '))}" />
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

    if (data.type === "condition_check") {
      html += `<div class="graph-detail-section">
        <div class="detail-label">Input object (JSON)</div>
        <textarea id="ge-input-object" class="ge-textarea" rows="2">${this.esc(data.input_object || "{{ nodes.extract_facts.output }}")}</textarea>
      </div>
      <div class="graph-detail-section">
        <div class="detail-label">Поле для проверки</div>
        <input type="text" id="ge-field" class="ge-input" placeholder="has_somatic" value="${this.esc(data.field || "")}" />
      </div>
      <div class="graph-detail-section">
        <div class="detail-label">Sequential → цель (всегда)</div>
        <input type="text" id="ge-sequential-to" class="ge-input" placeholder="node id" value="${this.esc(data.sequential_to || "")}" />
      </div>
      <div class="graph-detail-section">
        <div class="detail-label">True → цель</div>
        <input type="text" id="ge-true-to" class="ge-input" placeholder="node id" value="${this.esc(data.true_to || "")}" />
      </div>
      <div class="graph-detail-section">
        <div class="detail-label">False → цель</div>
        <input type="text" id="ge-false-to" class="ge-input" placeholder="node id" value="${this.esc(data.false_to || "")}" />
      </div>`;
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

    if (data.type === "llm_freeform") {
      html += `<div class="graph-detail-section">
        <div class="detail-label">Промпт (input)</div>
        <textarea id="ge-freeform-input" class="ge-textarea" rows="4" placeholder="Текст инструкции для LLM">${this.esc(data.input || "")}</textarea>
      </div>`;
    }

    if (data.type === "note") {
      html += `<div class="graph-detail-section">
        <div class="detail-label">Текст заметки (только в редакторе графа)</div>
        <textarea id="ge-note-content" class="ge-textarea" rows="4">${this.esc(data.input || "")}</textarea>
      </div>`;
      html += `<div class="graph-detail-section">
        <div class="detail-label">Системное сообщение пользователю (system_message)</div>
        <textarea id="ge-note-sysmsg" class="ge-textarea" rows="4" placeholder="Текст, который появится в чате как системное сообщение">${this.esc(data.system_message || "")}</textarea>
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
        this.saveCheckpoint();
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.task = taskTextarea.value;
      });
    }

    const outputTypeSelect = document.getElementById("ge-output-type") as HTMLSelectElement;
    if (outputTypeSelect) {
      outputTypeSelect.addEventListener("change", () => {
        this.saveCheckpoint();
        data.output_type = outputTypeSelect.value === "thought" ? undefined : outputTypeSelect.value;
        this.updateNodeHtml(nodeId);
      });
    }

    const injectInput = document.getElementById("ge-inject-reports") as HTMLInputElement;
    if (injectInput) {
      injectInput.addEventListener("change", () => {
        this.saveCheckpoint();
        const val = injectInput.value.trim();
        data.inject_reports = val ? val.split(',').map((s: string) => s.trim()) : undefined;
        this.updateNodeHtml(nodeId);
      });
    }

    const nodeIdInput = document.getElementById("ge-node-id") as HTMLInputElement;
    if (nodeIdInput) {
      nodeIdInput.addEventListener("change", () => {
        const newId = nodeIdInput.value.trim();
        if (newId && newId !== nodeId) {
          if (this.renameNode(nodeId, newId)) {
            this.showNodeEditor(newId);
          } else {
            nodeIdInput.value = nodeId;
          }
        }
      });
    }

    const disabledToggle = document.getElementById("ge-disabled") as HTMLInputElement;
    if (disabledToggle) {
      disabledToggle.addEventListener("change", () => {
        this.saveCheckpoint();
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
        this.saveCheckpoint();
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.signal_name = signalNameInput.value || undefined;
      });
    }

    const fieldInput = document.getElementById("ge-field") as HTMLInputElement;
    if (fieldInput) {
      fieldInput.addEventListener("change", () => {
        this.saveCheckpoint();
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.field = fieldInput.value || undefined;
        this.updateNodeHtml(nodeId);
      });
    }

    const sequentialToInput = document.getElementById("ge-sequential-to") as HTMLInputElement;
    if (sequentialToInput) {
      sequentialToInput.addEventListener("change", () => {
        this.saveCheckpoint();
        data.sequential_to = sequentialToInput.value || undefined;
        this.updateNodeHtml(nodeId);
      });
    }

    const trueToInput = document.getElementById("ge-true-to") as HTMLInputElement;
    if (trueToInput) {
      trueToInput.addEventListener("change", () => {
        this.saveCheckpoint();
        data.true_to = trueToInput.value || undefined;
        this.updateNodeHtml(nodeId);
      });
    }

    const falseToInput = document.getElementById("ge-false-to") as HTMLInputElement;
    if (falseToInput) {
      falseToInput.addEventListener("change", () => {
        this.saveCheckpoint();
        data.false_to = falseToInput.value || undefined;
        this.updateNodeHtml(nodeId);
      });
    }

    // ─── Switch/SeqSwitch/SignalRouter event handlers ───

    const switchTypeSelect = document.getElementById("ge-switch-type") as HTMLSelectElement;
    if (switchTypeSelect) {
      switchTypeSelect.addEventListener("change", () => {
        const oldType = data.type;
        const newType = switchTypeSelect.value;
        if (oldType !== newType) {
          this.saveCheckpoint();
          data.type = newType;
          this.rebuildSwitchOutputs(nodeId);
          this.showNodeEditor(nodeId);
        }
      });
    }

    const inputObjectArea = document.getElementById("ge-input-object") as HTMLTextAreaElement;
    if (inputObjectArea) {
      inputObjectArea.addEventListener("change", () => {
        this.saveCheckpoint();
        data.input_object = inputObjectArea.value || undefined;
      });
    }

    const defaultInput = document.getElementById("ge-default") as HTMLInputElement;
    if (defaultInput) {
      defaultInput.addEventListener("change", () => {
        this.saveCheckpoint();
        data.default = defaultInput.value || undefined;
        this.rebuildSwitchOutputs(nodeId);
      });
    }

    const noteContent = document.getElementById("ge-note-content") as HTMLTextAreaElement;
    if (noteContent) {
      noteContent.addEventListener("change", () => {
        this.saveCheckpoint();
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.input = noteContent.value || undefined;
        this.updateNodeHtml(nodeId);
      });
    }

    const freeformInput = document.getElementById("ge-freeform-input") as HTMLTextAreaElement;
    if (freeformInput) {
      freeformInput.addEventListener("change", () => {
        this.saveCheckpoint();
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.input = freeformInput.value || undefined;
        this.updateNodeHtml(nodeId);
      });
    }

    const noteSysMsg = document.getElementById("ge-note-sysmsg") as HTMLTextAreaElement;
    if (noteSysMsg) {
      noteSysMsg.addEventListener("change", () => {
        this.saveCheckpoint();
        this.editor!.drawflow.drawflow.Home.data[nodeId].data.system_message = noteSysMsg.value || undefined;
        this.updateNodeHtml(nodeId);
      });
    }

    const addBtn = document.getElementById("ge-case-add");
    if (addBtn) {
      addBtn.addEventListener("click", () => {
        this.saveCheckpoint();
        data.cases_priority.push({ key: "", to: "" });
        this.rebuildSwitchOutputs(nodeId);
        this.showNodeEditor(nodeId);
      });
    }

    document.querySelectorAll(".ge-case-remove").forEach((btn) => {
      btn.addEventListener("click", () => {
        this.saveCheckpoint();
        const row = (btn as HTMLElement).closest(".ge-case-row") as HTMLElement;
        const idx = parseInt(row.dataset.index || "0", 10);
        data.cases_priority.splice(idx, 1);
        this.rebuildSwitchOutputs(nodeId);
        this.showNodeEditor(nodeId);
      });
    });

    document.querySelectorAll(".ge-case-up").forEach((btn) => {
      btn.addEventListener("click", () => {
        this.saveCheckpoint();
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
        this.saveCheckpoint();
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
    if (this.el.graphSidebar.classList.contains("open")) {
      this.saveCheckpoint();
    }
    this.el.graphSidebar.classList.remove("open");
  }

  // ─── Добавление ноды ───

  private addNode(type: string, clientX?: number, clientY?: number): void {
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
    const outs = type === "condition_check" ? 3 : (isDynamicNode(type) ? 2 : OUTPUT_COUNT[type] ?? 1);

    const inputs: Record<string, { connections: any[] }> = {};
    for (let i = 1; i <= 1; i++) inputs[`input_${i}`] = { connections: [] };
    const outputs: Record<string, { connections: any[] }> = {};
    for (let i = 1; i <= outs; i++) outputs[`output_${i}`] = { connections: [] };

    let nodeClass = "";
    if (type === "note") nodeClass = "node-type-note";
    else if (isDynamicNode(type)) nodeClass = "dynamic-node";

    const nodeData: import("drawflow").DrawflowNode = {
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

  // ─── Переименование ноды (drawflow-ключ + data.id) ───

  private renameNode(oldKey: string, newKey: string): boolean {
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

  private updateNodeHtml(nodeId: string): void {
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
    if (data.type === "condition_check") {
      for (let i = 0; i < GraphController.CONDITION_CHECK_CASES.length; i++) {
        const cc = GraphController.CONDITION_CHECK_CASES[i];
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

  private onSwitchConnectionChanged(fromNodeId: string, toNodeId: string, outputKey: string, action: "created" | "removed"): void {
    if (!this.editor || !fromNodeId || !toNodeId || !outputKey) return;
    const dn = this.editor.drawflow.drawflow.Home.data[fromNodeId];
    if (!dn) return;
    const data = dn.data;
    if (!isDynamicNode(data.type)) return;
    const match = outputKey.match(/^output_(\d+)$/);
    if (!match) return;
    const outIdx = parseInt(match[1], 10) - 1;

    if (data.type === "condition_check") {
      const cc = GraphController.CONDITION_CHECK_CASES[outIdx];
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
        this.saveCheckpoint();
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

  // ─── Drag-to-reconnect ребёр (перетаскивание связи) ───

  private setupEdgeDrag(): void {
    if (!this.editor) return;
    const precanvas = this.editor.precanvas;

    precanvas.addEventListener('mousedown', (e: MouseEvent) => {
      if (e.button !== 0) return;
      const path = (e.target as HTMLElement).closest<SVGPathElement>('.main-path');
      if (!path) return;
      const svg = path.closest<SVGElement>('.connection');
      if (!svg) return;

      let sourceId = '', targetId = '';
      for (const cls of svg.classList) {
        if (cls.startsWith('node_out_node-')) sourceId = cls.slice('node_out_node-'.length);
        if (cls.startsWith('node_in_node-')) targetId = cls.slice('node_in_node-'.length);
      }
      if (!sourceId || !targetId) return;

      const d = path.getAttribute('d') || '';
      const nums = d.match(/[-\d.]+/g);
      if (!nums || nums.length < 6) return;
      const x1 = parseFloat(nums[0]), y1 = parseFloat(nums[1]);
      const xN = parseFloat(nums[nums.length - 2]), yN = parseFloat(nums[nums.length - 1]);

      const z = this.editor!.zoom || 1;
      const r = precanvas.getBoundingClientRect();
      const mx = (e.clientX - r.left) / z;
      const my = (e.clientY - r.top) / z;

      const isSourceHalf = Math.hypot(mx - x1, my - y1) < Math.hypot(mx - xN, my - yN);

      // Находим outputKey из CSS-классов SVG (всегда отражает актуальное состояние)
      let outputKey = 'output_1';
      for (const cls of svg.classList) {
        if (/^output_\d+$/.test(cls)) {
          outputKey = cls;
          break;
        }
      }

      // НЕ стопаем propagation — пусть Drawflow обработает клик (выделение связи)
      // Начнём reconnect только если мышь реально поехала
      const dragData = { sourceId, targetId, svg, isSourceHalf, outputKey, clickX: e.clientX, clickY: e.clientY };

      const onMouseMove = (ev: MouseEvent) => {
        const dx = ev.clientX - dragData.clickX;
        const dy = ev.clientY - dragData.clickY;
        if (Math.abs(dx) > 4 || Math.abs(dy) > 4) {
          document.removeEventListener('mousemove', onMouseMove);
          document.removeEventListener('mouseup', onMouseUp);
          this.beginEdgeReconnect(dragData, ev);
        }
      };

      const onMouseUp = () => {
        document.removeEventListener('mousemove', onMouseMove);
        document.removeEventListener('mouseup', onMouseUp);
        // Клик без драга — Drawflow сам обработал выделение связи
      };

      document.addEventListener('mousemove', onMouseMove);
      document.addEventListener('mouseup', onMouseUp);
    });
  }

  private beginEdgeReconnect(data: {
    sourceId: string; targetId: string; svg: SVGElement; isSourceHalf: boolean;
    clickX: number; clickY: number; outputKey: string;
  }, _ev: MouseEvent): void {
    if (!this.editor) return;
    this.saveCheckpoint();
    const editor = this.editor;
    const precanvas = editor.precanvas;
    const allData = editor.drawflow.drawflow.Home.data;
    const { sourceId, targetId, svg, isSourceHalf, outputKey } = data;

    const originalConn = { sourceId, targetId, outputKey };

    // Удаляем соединение из данных Drawflow
    if (allData[targetId]?.inputs?.['input_1']) {
      allData[targetId].inputs['input_1'].connections =
        allData[targetId].inputs['input_1'].connections.filter((c: any) => c.node !== sourceId);
    }
    const sourceOutputs = allData[sourceId]?.outputs?.[outputKey];
    if (sourceOutputs) {
      sourceOutputs.connections =
        sourceOutputs.connections.filter((c: any) => c.node !== targetId);
    }
    svg.remove();

    editor.dispatch('connectionRemoved', {
      output_id: sourceId, input_id: targetId, output_class: outputKey, input_class: 'input_1',
    });

    document.body.classList.add('edge-reconnecting');

    const getPortCenter = (nodeId: string, selector: string): { x: number; y: number } | null => {
      const nodeEl = document.getElementById('node-' + nodeId);
      if (!nodeEl) return null;
      const portEl = nodeEl.querySelector<HTMLElement>(selector);
      if (!portEl) return null;
      const pr = portEl.getBoundingClientRect();
      const cr = precanvas.getBoundingClientRect();
      const z = editor.zoom || 1;
      return {
        x: (pr.left + pr.width / 2 - cr.left) / z,
        y: (pr.top + pr.height / 2 - cr.top) / z,
      };
    };

    let startPort: { x: number; y: number } | null;
    if (isSourceHalf) {
      startPort = getPortCenter(targetId, '.input');
    } else {
      startPort = getPortCenter(sourceId, '.output.' + outputKey);
    }
    if (!startPort) {
      document.body.classList.remove('edge-reconnecting');
      this.restoreEdge(originalConn);
      return;
    }

    const tempSvg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    const tempPath = document.createElementNS("http://www.w3.org/2000/svg", "path");
    tempPath.classList.add("main-path");
    tempSvg.classList.add("connection", "reverse-drag");
    precanvas.appendChild(tempSvg);
    tempSvg.appendChild(tempPath);

    const updateLine = (cx: number, cy: number) => {
      const z2 = editor.zoom || 1;
      const r2 = precanvas.getBoundingClientRect();
      const endX = (cx - r2.left) / z2;
      const endY = (cy - r2.top) / z2;
      const dx = Math.abs(endX - startPort!.x) * 0.5;
      const d = `M ${startPort!.x} ${startPort!.y} C ${startPort!.x + dx} ${startPort!.y} ${endX - dx} ${endY} ${endX} ${endY}`;
      tempPath.setAttributeNS(null, "d", d);
    };

    updateLine(data.clickX, data.clickY);

    const onMouseMove = (ev: MouseEvent) => updateLine(ev.clientX, ev.clientY);

    const onKeyDown = (ev: KeyboardEvent) => {
      if (ev.key === 'Escape') {
        cleanup();
        this.restoreEdge(originalConn);
      }
    };

    const cleanup = () => {
      document.removeEventListener('mousemove', onMouseMove);
      document.removeEventListener('mouseup', onMouseUp);
      document.removeEventListener('keydown', onKeyDown);
      tempSvg.remove();
      document.body.classList.remove('edge-reconnecting');
    };

    const onMouseUp = (ev: MouseEvent) => {
      cleanup();

      const dropTarget = document.elementFromPoint(ev.clientX, ev.clientY);
      if (!dropTarget) { this.restoreEdge(originalConn); return; }

      if (isSourceHalf) {
        const outputEl = dropTarget.classList.contains('output')
          ? dropTarget : dropTarget.closest<HTMLElement>('.output');
        if (!outputEl) { this.restoreEdge(originalConn); return; }
        const newSourceNodeEl = outputEl.closest<HTMLElement>('.drawflow-node');
        if (!newSourceNodeEl) { this.restoreEdge(originalConn); return; }
        const newSourceId = newSourceNodeEl.id.slice(5);
        if (newSourceId === targetId) { this.restoreEdge(originalConn); return; }
        const newOutputClass = outputEl.classList[1];
        if (!newOutputClass) { this.restoreEdge(originalConn); return; }
        if (allData[newSourceId]?.data?.disabled) { this.restoreEdge(originalConn); return; }
        try {
          editor.addConnection(newSourceId, targetId, newOutputClass, "input_1");
          this.saveCheckpoint();
        }
        catch (_) { this.restoreEdge(originalConn); }
      } else {
        const inputEl = dropTarget.classList.contains('input')
          ? dropTarget : dropTarget.closest<HTMLElement>('.input');
        if (!inputEl) { this.restoreEdge(originalConn); return; }
        const newTargetNodeEl = inputEl.closest<HTMLElement>('.drawflow-node');
        if (!newTargetNodeEl) { this.restoreEdge(originalConn); return; }
        const newTargetId = newTargetNodeEl.id.slice(5);
        if (newTargetId === sourceId) { this.restoreEdge(originalConn); return; }
        const inputClass = inputEl.classList[1];
        if (!inputClass) { this.restoreEdge(originalConn); return; }
        if (allData[newTargetId]?.data?.disabled) { this.restoreEdge(originalConn); return; }
        try {
          editor.addConnection(sourceId, newTargetId, outputKey, inputClass);
          this.saveCheckpoint();
        }
        catch (_) { this.restoreEdge(originalConn); }
      }
    };

    document.addEventListener('mousemove', onMouseMove);
    document.addEventListener('mouseup', onMouseUp);
    document.addEventListener('keydown', onKeyDown);
  }

  private restoreEdge(conn: { sourceId: string; targetId: string; outputKey: string }): void {
    if (!this.editor) return;
    const { sourceId, targetId, outputKey } = conn;

    // Если ноды уже нет на холсте — не восстанавливаем
    if (!this.editor.drawflow.drawflow.Home.data[sourceId] ||
        !this.editor.drawflow.drawflow.Home.data[targetId]) return;

    try {
      this.editor.addConnection(sourceId, targetId, outputKey, "input_1");
    } catch (_) { }
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
    if (node.type === "condition_check") return 3;
    if (node.cases) return Object.keys(node.cases).length;
    if (node.cases_priority) return node.cases_priority.length + (node.default ? 1 : 0);
    if (node.default) return 1;
    return 2;
  }

  private getSwitchCaseKeys(node: GraphNodeDef): string[] {
    if (node.type === "condition_check") return GraphController.CONDITION_CHECK_CASES.map(c => c.key);
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
      if (node.type === "condition_check") {
        if (node.true_to) implicit.push({ from: node.id, to: node.true_to });
        if (node.false_to) implicit.push({ from: node.id, to: node.false_to });
        if (node.sequential_to) implicit.push({ from: node.id, to: node.sequential_to });
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

  private buildNodeInnerHtml(data: any, fallbackId: string = ""): string {
    const typeLabel = NODE_LABELS[data.type] || data.type;
    let agentLine = "";
    if (data.type === "llm_worker" && data.agent) {
      agentLine = `<div class="gn-agent">→ ${this.esc(data.agent)}</div>`;
    }
    if (data.type === "llm_worker" && data.output_type) {
      agentLine += `<div class="gn-agent">${this.esc(data.output_type)}</div>`;
    }
    if (data.type === "llm_worker" && data.inject_reports && data.inject_reports.length > 0) {
      agentLine += `<div class="gn-agent" style="color:#2196f3">📎 Отчеты: ${data.inject_reports.length}</div>`;
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
    if (data.type === "condition_check") {
      const parts: string[] = [];
      if (data.field) parts.push(this.esc(data.field));
      if (data.input_object) parts.push(`← ${this.esc(data.input_object)}`);
      if (data.sequential_to) parts.push(`→ ${this.esc(data.sequential_to)}`);
      if (data.true_to) parts.push(`✓ ${this.esc(data.true_to)}`);
      if (data.false_to) parts.push(`✗ ${this.esc(data.false_to)}`);
      signalLine = `<div class="gn-agent" style="color:#26a69a">🎯 ${parts.join(" | ")}</div>`;
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
    if (data.type === "note") {
      const sysBadge = data.system_message
        ? `<div class="gn-sysmsg-badge">→ чат: ${this.esc((data.system_message || "").slice(0, 40))}${(data.system_message || "").length > 40 ? "…" : ""}</div>`
        : "";
      return `<div class="gn-note-content">${this.esc(data.input || "")}</div>${sysBadge}`;
    }

    const disabledBadge = data.disabled ? `<div class="gn-disabled-badge">⛔ DISABLED</div>` : "";
    return `
      <div class="gn-title">${this.esc(data.id || fallbackId)}</div>
      <div class="gn-type">${typeLabel}</div>
      ${disabledBadge}
      ${agentLine}
      ${signalLine}
      ${casesLine}
      ${factsLine}
    `;
  }

  private esc(s: string): string {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }

  // ─── Undo/Redo + Dirty State ───

  private static readonly MAX_HISTORY = 50;

  private captureSnapshot(): GraphSnapshot {
    const editor = this.editor;
    if (!editor) return { data: {} };
    return {
      data: JSON.parse(JSON.stringify(editor.drawflow.drawflow.Home.data)),
    };
  }

  private applySnapshot(snap: GraphSnapshot): void {
    if (!this.editor) return;
    this.isRestoring = true;

    const editor = this.editor;

    // 1. Собираем все соединения из снапшота ДО очистки
    const snapDataRaw = JSON.parse(JSON.stringify(snap.data));
    const connectionsToRestore: Array<{ from: string; to: string; output: string; input: string }> = [];
    for (const nodeId of Object.keys(snapDataRaw)) {
      const dn = snapDataRaw[nodeId];
      for (const inKey of Object.keys(dn.inputs || {})) {
        for (const conn of dn.inputs[inKey].connections || []) {
          if (snapDataRaw[conn.node]) {
            connectionsToRestore.push({
              from: conn.node,
              to: nodeId,
              output: conn.input || "output_1",
              input: inKey,
            });
          }
        }
      }
    }

    // 2. Удаляем все текущие ноды (isRestoring=true — подавляем connectionRemoved)
    const currentNodes = Object.keys(editor.drawflow.drawflow.Home.data);
    for (const id of currentNodes) {
      editor.removeNodeId("node-" + id);
    }

    // 3. Собираем import-данные (с ПУСТЫМИ connections — Drawflow не умеет создавать их из данных)
    const importData: import("drawflow").DrawflowExport = {
      drawflow: { Home: { data: {} } },
    };
    const snapData = JSON.parse(JSON.stringify(snap.data));

    for (const id of Object.keys(snapData)) {
      const dn = snapData[id];
      // Обнуляем connections — создадим их вручную после import
      for (const inKey of Object.keys(dn.inputs || {})) {
        dn.inputs[inKey].connections = [];
      }
      for (const outKey of Object.keys(dn.outputs || {})) {
        dn.outputs[outKey].connections = [];
      }
      // Обновляем HTML и class
      dn.html = this.buildNodeInnerHtml(dn.data, id);
      let nodeClass = "";
      const t = dn.data.type;
      if (t === "note") nodeClass = "node-type-note";
      else if (isDynamicNode(t)) nodeClass = "dynamic-node";
      if (dn.data.disabled) nodeClass = nodeClass ? `${nodeClass} node-disabled` : "node-disabled";
      dn.class = nodeClass;
    }
    importData.drawflow.Home.data = snapData;

    // 5. Импортируем (isRestoring=true — подавляем connectionCreated/Removed)
    editor.import(importData);

    // 6. Вручную создаём все соединения (Drawflow import их не восстанавливает)
    for (const conn of connectionsToRestore) {
      try {
        editor.addConnection(conn.from, conn.to, conn.output, conn.input);
      } catch (_) { }
    }

    // 7. Ждём кадр для пост-обработки (выравнивание + SVG + reverse)
    requestAnimationFrame(() => {
      if (!this.editor) return;
      this.isRestoring = true;

      // Drawflow import() сбрасывает transform precanvas — восстанавливаем
      const transform = `translate(${this.editor.canvas_x}px, ${this.editor.canvas_y}px) scale(${this.editor.zoom})`;
      this.editor.precanvas.style.transform = transform;

      for (const id of Object.keys(this.editor.drawflow.drawflow.Home.data)) {
        this.alignOutputsWithCases(id);
      }

      for (const id of Object.keys(this.editor.drawflow.drawflow.Home.data)) {
        this.editor.updateConnectionNodes("node-" + id);
      }

      for (const id of Object.keys(this.editor.drawflow.drawflow.Home.data)) {
        this.enableReverseConnection(id);
      }

      this.hideSidebar();
      this.deselectAllNodes();
      this.isRestoring = false;
    });
  }

  private saveCheckpoint(): void {
    if (this.isRestoring) return;
    this.undoStack.push(this.captureSnapshot());
    if (this.undoStack.length > GraphController.MAX_HISTORY) {
      this.undoStack.shift();
    }
    this.redoStack = [];
    this.markDirty();
    this.updateUndoButtons();
  }

  private undo(): void {
    if (this.undoStack.length === 0 || !this.editor) return;
    this.redoStack.push(this.captureSnapshot());
    const snap = this.undoStack.pop()!;
    this.applySnapshot(snap);
    this.syncDirtyAfterRestore();
    this.updateUndoButtons();
  }

  private redo(): void {
    if (this.redoStack.length === 0 || !this.editor) return;
    this.undoStack.push(this.captureSnapshot());
    const snap = this.redoStack.pop()!;
    this.applySnapshot(snap);
    this.syncDirtyAfterRestore();
    this.updateUndoButtons();
  }

  private syncDirtyAfterRestore(): void {
    if (!this.pristineSnapshot) return;
    const current = this.captureSnapshot();
    if (this.isNodesEqual(current.data, this.pristineSnapshot.data)) {
      this.markClean();
    } else {
      this.markDirty();
    }
  }

  // Сравнение только значимых полей нод (игнорируем служебные поля Drawflow)
  private isNodesEqual(a: Record<string, any>, b: Record<string, any>): boolean {
    const aKeys = Object.keys(a);
    const bKeys = Object.keys(b);
    if (aKeys.length !== bKeys.length) return false;
    for (const key of aKeys) {
      const an = a[key];
      const bn = b[key];
      if (!bn) return false;
      if (JSON.stringify(an.data) !== JSON.stringify(bn.data)) return false;
      if (JSON.stringify(an.inputs) !== JSON.stringify(bn.inputs)) return false;
      if (JSON.stringify(an.outputs) !== JSON.stringify(bn.outputs)) return false;
      if (an.pos_x !== bn.pos_x || an.pos_y !== bn.pos_y) return false;
    }
    return true;
  }

  private resetHistory(): void {
    this.undoStack = [];
    this.redoStack = [];
    this.pristineSnapshot = this.captureSnapshot();
    this.markClean();
    this.updateUndoButtons();
  }

  private markDirty(): void {
    this.isDirty = true;
    this.updateDirtyIndicator();
  }

  private markClean(): void {
    this.isDirty = false;
    this.updateDirtyIndicator();
  }

  private updateDirtyIndicator(): void {
    const el = this.el.dirtyIndicator;
    if (!this.currentFilePath) {
      el.textContent = "";
      el.title = "";
      return;
    }
    if (this.isDirty) {
      el.textContent = " ●";
      el.style.color = "#e53935";
      el.title = "Есть несохранённые изменения";
    } else {
      el.textContent = " ●";
      el.style.color = "#4caf50";
      el.title = "Сохранено";
    }
  }

  private updateUndoButtons(): void {
    this.el.btnUndo.disabled = this.undoStack.length === 0;
    this.el.btnRedo.disabled = this.redoStack.length === 0;
  }

  onTabActivated(): void {
    if (!this.editor) {
      this.ensureEditor();
    } else {
      this.editor.zoom_reset();
    }
  }
}
