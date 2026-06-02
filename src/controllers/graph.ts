import { invoke } from "@tauri-apps/api/core";
import { Network, DataSet } from "vis-network/standalone";
import { showToast } from "../ui";

// ─── Типы данных с бэкенда ───

interface GraphResponse {
  teams: string[];
  workflows: WorkflowGraphDef[];
  agents: GraphAgentDef[];
}

interface WorkflowGraphDef {
  team: string;
  name: string;
  file_stem: string;
  visible_agents: string[];
  config?: { statuses: Array<{ id: string; description: string }> };
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
  namespace?: string;
}

interface GraphEdgeDef {
  from: string;
  to: string;
  condition?: string;
  case?: string;
}

interface GraphAgentDef {
  team: string;
  id: string;
  name: string;
  description: string;
  is_hidden: boolean;
  mode: string;
}

// ─── Стили узлов по типу ───

const NODE_STYLES: Record<string, { shape: string; color: string; label: string }> = {
  llm_worker:       { shape: "box",        color: "#4caf50", label: "Worker" },
  llm_classifier:   { shape: "diamond",    color: "#42a5f5", label: "Classifier" },
  system_condition: { shape: "hexagon",    color: "#ffa726", label: "Condition" },
  sub_workflow:     { shape: "database",   color: "#ab47bc", label: "Sub-workflow" },
  switch:           { shape: "triangle",   color: "#ef5350", label: "Switch" },
  return:           { shape: "dot",        color: "#78909c", label: "Return" },
};

// ─── Workflow color map ───

const WF_COLORS: Record<string, string> = {
  main_conversation_flow: "#42a5f5",
  triage_flow:            "#66bb6a",
  analysis_flow:          "#ffa726",
  treatment_flow:         "#ab47bc",
};

// ─── Интерфейс для элементов DOM ───

export interface GraphElements {
  graphContainer: HTMLDivElement;
  graphSidebar: HTMLDivElement;
  graphTeamSelect: HTMLSelectElement;
  graphDetailTitle: HTMLSpanElement;
  graphDetailContent: HTMLDivElement;
  graphSidebarClose: HTMLButtonElement;
}

// ─── Контроллер ───

const ENTRY_NODE = "__start__";
const USER_NODE = "__user__";

export class GraphController {
  private el: GraphElements;
  private network: Network | null = null;
  private allData: GraphResponse | null = null;
  private nodeDataMap = new Map<string, any>();

  constructor(el: GraphElements) {
    this.el = el;
    this.bindEvents();
  }

  private bindEvents(): void {
    this.el.graphTeamSelect.addEventListener("change", () => this.loadGraph());
    this.el.graphSidebarClose.addEventListener("click", () => this.hideSidebar());
  }

  async loadData(): Promise<void> {
    try {
      this.allData = await invoke<GraphResponse>("get_workflow_graphs");
      this.populateTeamSelect();
      if (this.allData.teams.length > 0 && this.el.graphTeamSelect.value) {
        this.loadGraph();
      }
    } catch (e) {
      showToast(`Ошибка загрузки графа: ${e}`, "error");
    }
  }

  private populateTeamSelect(): void {
    const sel = this.el.graphTeamSelect;
    sel.innerHTML = "";
    for (const team of this.allData!.teams) {
      const opt = document.createElement("option");
      opt.value = team;
      opt.textContent = team === "psychotherapist" ? "🧠 Психотерапевт" : team;
      sel.appendChild(opt);
    }
    if (sel.options.length > 0) {
      sel.selectedIndex = 0;
    }
  }

  private loadGraph(): void {
    const team = this.el.graphTeamSelect.value;
    if (!team || !this.allData) return;

    const workflows = this.allData.workflows.filter(w => w.team === team);
    const agents = this.allData.agents.filter(a => a.team === team);

    const visNodes: any[] = [];
    const visEdges: any[] = [];
    this.nodeDataMap.clear();
    this.el.graphSidebar.classList.remove("open");

    // === Prepare workflow metadata ===
    const firstNodePerWorkflow: Record<string, string> = {};
    const entryWorkflows: string[] = [];

    for (const wf of workflows) {
      if (wf.nodes.length > 0) {
        firstNodePerWorkflow[wf.file_stem] = `${wf.file_stem}.${wf.nodes[0].id}`;
      }
      if (wf.visible_agents && wf.visible_agents.length > 0) {
        entryWorkflows.push(wf.file_stem);
      }
    }

    // Collect set of nodes referencing user_message
    const userMsgNodeIds = new Set<string>();
    for (const wf of workflows) {
      for (const node of wf.nodes) {
        const fullId = `${wf.file_stem}.${node.id}`;
        if (node.input && node.input.includes("user_message")) {
          userMsgNodeIds.add(fullId);
        }
        if (node.task && node.task.includes("user_message")) {
          userMsgNodeIds.add(fullId);
        }
      }
    }

    // Precompute cross-workflow targets: sub_workflow node → target first node id
    const subWfTargets: Map<string, string> = new Map();
    for (const wf of workflows) {
      for (const node of wf.nodes) {
        if (node.type === "sub_workflow" && node.workflow) {
          const targetStem = node.workflow.replace(/\.yaml$/, "").replace(/\.yml$/, "");
          const targetFirst = firstNodePerWorkflow[targetStem];
          if (targetFirst) {
            subWfTargets.set(`${wf.file_stem}.${node.id}`, targetFirst);
          }
        }
      }
    }

    // === 1) START node (entry) ===
    if (entryWorkflows.length > 0) {
      const firstEntry = entryWorkflows[0];
      const targetId = firstNodePerWorkflow[firstEntry];
      this.nodeDataMap.set(ENTRY_NODE, { type: "special", label: "🚀 START", description: "Точка входа в систему" });
      visNodes.push({
        id: ENTRY_NODE,
        label: "🚀 START",
        shape: "star",
        color: {
          background: "#ffd70033",
          border: "#ffd700",
          highlight: { background: "#ffd70055", border: "#ffffff" },
          hover: { background: "#ffd70044", border: "#ffd700" },
        },
        font: { size: 14, color: "#ffd700", multi: "md" },
        margin: { top: 8, bottom: 8, left: 14, right: 14 },
        borderWidth: 2,
      });
      if (targetId) {
        visEdges.push({
          from: ENTRY_NODE,
          to: targetId,
          color: "#ffd700",
          width: 2,
          arrows: "to",
        });
      }
    }

    // === 2) User Input node ===
    if (userMsgNodeIds.size > 0) {
      this.nodeDataMap.set(USER_NODE, { type: "special", label: "👤 User Input", description: "Сообщение пользователя (user_message)" });
      visNodes.push({
        id: USER_NODE,
        label: "👤 User",
        shape: "ellipse",
        color: {
          background: "#fff17633",
          border: "#fff176",
          highlight: { background: "#fff17655", border: "#ffffff" },
          hover: { background: "#fff17644", border: "#fff176" },
        },
        font: { size: 14, color: "#fff176", multi: "md" },
        margin: { top: 6, bottom: 6, left: 12, right: 12 },
        borderWidth: 2,
      });
      for (const targetId of userMsgNodeIds) {
        visEdges.push({
          from: USER_NODE,
          to: targetId,
          label: "user_message",
          font: { size: 10, color: "#fff176", strokeWidth: 0 },
          color: "#fff176",
          width: 1,
          dashes: true,
          arrows: "to",
        });
      }
    }

    // === 3) Workflow step nodes ===
    for (const wf of workflows) {
      const prefix = wf.file_stem.replace(/_flow$/, "").replace(/_/g, " ");
      for (const node of wf.nodes) {
        const fullId = `${wf.file_stem}.${node.id}`;
        const style = NODE_STYLES[node.type] || NODE_STYLES.llm_worker;
        const wfColor = WF_COLORS[wf.file_stem] || "#888";
        const borderColor = node.type === "sub_workflow" ? wfColor : style.color;

        const lines: string[] = [];
        // First line: node id (monospace look)
        lines.push(node.id);

        // Workflow context on second line
        const typeLabel = `${node.type === "llm_worker" ? "🤖" : node.type === "llm_classifier" ? "📋" : node.type === "system_condition" ? "⚡" : node.type === "sub_workflow" ? "📦" : node.type === "switch" ? "🔀" : "🏁"} ${style.label} [${prefix}]`;
        lines.push(typeLabel);

        // Agent name on third line for workers
        if (node.type === "llm_worker" && node.agent) {
          const agent = agents.find(a => a.id === node.agent);
          lines.push(agent ? `→ ${agent.name || node.agent}` : `→ ${node.agent}`);
        }

        // Statuses for classifiers
        if (node.type === "llm_classifier" && wf.config?.statuses && wf.config.statuses.length > 0) {
          const names = wf.config.statuses.map((s: any) => s.id).join(", ");
          lines.push(`statuses: ${names}`);
        }

        // Cases for switches
        if (node.type === "switch" && node.cases) {
          lines.push(Object.keys(node.cases).join(" / "));
        }

        // Action for conditions
        if (node.type === "system_condition" && node.action) {
          lines.push(`action: ${node.action}`);
        }

        // Target for sub_workflow
        if (node.type === "sub_workflow" && node.workflow) {
          const targetStem = node.workflow.replace(/\.yaml$/, "");
          lines.push(`→ ${targetStem}`);
        }

        this.nodeDataMap.set(fullId, {
          type: "workflow-node",
          node,
          wf_name: wf.name,
          file_stem: wf.file_stem,
        });

        const borderWidth = node.type === "llm_classifier" ? 3 : node.type === "sub_workflow" ? 2 : 2;
        const bgAlpha = node.type === "llm_classifier" ? "33" : "22";

        visNodes.push({
          id: fullId,
          label: lines.join("\n"),
          shape: style.shape,
          color: {
            background: borderColor + bgAlpha,
            border: borderColor,
            highlight: { background: borderColor + "55", border: "#ffffff" },
            hover: { background: borderColor + "44", border: borderColor },
          },
          font: { size: 12, color: "#e0e0e0", multi: "md", align: "center" },
          margin: { top: 7, bottom: 7, left: 10, right: 10 },
          borderWidth,
          borderWidthSelected: 3,
        });
      }
    }

    // === 4) Within-workflow edges ===
    for (const wf of workflows) {
      for (const edge of wf.edges) {
        const fromId = `${wf.file_stem}.${edge.from}`;
        const toId = `${wf.file_stem}.${edge.to}`;
        const label = edge.case || edge.condition || "";
        visEdges.push({
          from: fromId,
          to: toId,
          label,
          font: { size: 11, color: "#ffe082", strokeWidth: 0 },
          color: { color: "#90a4ae", highlight: "#fff" },
          width: 2,
          arrows: "to",
          smooth: { enabled: true, type: "curvedCW", roundness: 0.15 },
          dashes: edge.condition ? [5, 5] : false,
        });
      }
    }

    // === 5) Cross-workflow edges (sub_workflow → target first node) ===
    for (const [fromId, toId] of subWfTargets) {
      // Find the label for this sub_workflow
      let subLabel = "";
      for (const wf of workflows) {
        for (const node of wf.nodes) {
          if (`${wf.file_stem}.${node.id}` === fromId && node.workflow) {
            subLabel = "→ " + node.workflow.replace(/\.yaml$/, "");
          }
        }
      }
      // Check target node exists
      if (visNodes.some((n: any) => n.id === toId)) {
        visEdges.push({
          from: fromId,
          to: toId,
          label: subLabel,
          font: { size: 10, color: "#ce93d8", strokeWidth: 0 },
          color: { color: "#ce93d8" },
          width: 2,
          dashes: [6, 4],
          arrows: "to",
        });
      }
    }

    // === 6) Agent nodes ===
    const connectedAgents = new Set<string>();
    for (const wf of workflows) {
      for (const node of wf.nodes) {
        if (node.type === "llm_worker" && node.agent) {
          connectedAgents.add(node.agent);
        }
      }
    }

    for (const agent of agents) {
      if (!connectedAgents.has(agent.id)) continue;

      const agentId = `_agent_${agent.id}`;
      this.nodeDataMap.set(agentId, { type: "agent", agent });

      const label = agent.name || agent.id;
      visNodes.push({
        id: agentId,
        label,
        shape: "ellipse",
        color: {
          background: "#26a69a22",
          border: "#26a69a",
          highlight: { background: "#26a69a55", border: "#ffffff" },
          hover: { background: "#26a69a44", border: "#26a69a" },
        },
        font: { size: 11, color: "#80cbc4" },
        margin: { top: 5, bottom: 5, left: 10, right: 10 },
        borderWidth: 1,
      });

      for (const wf of workflows) {
        for (const node of wf.nodes) {
          if (node.type === "llm_worker" && node.agent === agent.id) {
            const nodeId = `${wf.file_stem}.${node.id}`;
            visEdges.push({
              from: agentId,
              to: nodeId,
              color: "#26a69a66",
              width: 1,
              dashes: [3, 3],
              arrows: "to",
            });
          }
        }
      }
    }

    // === 7) Build network ===
    if (this.network) {
      this.network.destroy();
      this.network = null;
    }

    const data = { nodes: new DataSet(visNodes), edges: new DataSet(visEdges) };
    const options = {
      layout: {
        hierarchical: {
          enabled: true,
          direction: "UD",
          sortMethod: "directed",
          levelSeparation: 200,
          nodeSpacing: 180,
          treeSpacing: 250,
          edgeMinimization: true,
          blockShifting: true,
        },
      },
      physics: { enabled: false },
      edges: {
        smooth: { enabled: true, type: "curvedCW", roundness: 0.15 },
      },
      nodes: {
        shape: "box",
        font: { size: 13, color: "#e0e0e0", multi: "md" },
        borderWidth: 2,
        color: {
          border: "#666",
          background: "#2a2a2a",
          highlight: { border: "#ffffff" },
        },
      },
      interaction: {
        hover: true,
        tooltipDelay: 200,
        navigationButtons: true,
        keyboard: true,
        zoomView: true,
        dragView: true,
      },
      groups: {
        agent: { shape: "ellipse" },
      },
    };

    this.network = new Network(this.el.graphContainer, data, options);

    // Click handler
    this.network.on("click", (params: any) => {
      if (params.nodes.length > 0) {
        this.showNodeDetail(params.nodes[0], workflows, agents);
      } else {
        this.hideSidebar();
      }
    });

    this.el.graphDetailContent.innerHTML = `<p class="graph-hint">Кликните на узел для просмотра деталей</p>`;
    this.el.graphDetailTitle.textContent = "Информация";
  }

  private showNodeDetail(
    nodeId: string,
    allWorkflows: WorkflowGraphDef[],
    allAgents: GraphAgentDef[],
  ): void {
    const data = this.nodeDataMap.get(nodeId);
    if (!data) return;

    this.el.graphSidebar.classList.add("open");

    const parts: string[] = [];

    if (data.type === "special") {
      this.el.graphDetailTitle.textContent = data.label;
      parts.push(`<div class="graph-detail-section">
        <div class="detail-label">Описание</div>
        <div class="detail-value">${data.description || ""}</div>
      </div>`);
    } else if (data.type === "agent") {
      const agent = data.agent as GraphAgentDef;
      this.el.graphDetailTitle.textContent = `🧑 ${agent.name || agent.id}`;
      parts.push(`<div class="graph-detail-section">
        <div class="detail-label">ID</div>
        <div class="detail-value code">${agent.id}</div>
      </div>`);
      parts.push(`<div class="graph-detail-section">
        <div class="detail-label">Имя</div>
        <div class="detail-value">${agent.name}</div>
      </div>`);
      parts.push(`<div class="graph-detail-section">
        <div class="detail-label">Описание</div>
        <div class="detail-value">${agent.description}</div>
      </div>`);
      parts.push(`<div class="graph-detail-section">
        <div class="detail-label">Режим</div>
        <div class="detail-value">${agent.mode}</div>
      </div>`);
      parts.push(`<div class="graph-detail-section">
        <div class="detail-label">Скрыт в UI</div>
        <div class="detail-value">${agent.is_hidden ? "Да" : "Нет"}</div>
      </div>`);
    } else {
      const node = data.node as GraphNodeDef;
      const style = NODE_STYLES[node.type];
      const colorBox = style?.color || "#666";
      this.el.graphDetailTitle.textContent = `${node.id} [${data.file_stem}]`;

      parts.push(`<div class="graph-detail-section">
        <div class="detail-label">Тип</div>
        <div class="detail-value" style="border-left: 4px solid ${colorBox}; padding-left: 8px;">
          ${style?.label || node.type}
        </div>
      </div>`);
      parts.push(`<div class="graph-detail-section">
        <div class="detail-label">Граф</div>
        <div class="detail-value">${data.wf_name} <span class="detail-sub">${data.file_stem}.yaml</span></div>
      </div>`);
      if (node.agent) {
        const agent = allAgents.find(a => a.id === node.agent);
        parts.push(`<div class="graph-detail-section">
          <div class="detail-label">Агент</div>
          <div class="detail-value">${agent ? agent.name : node.agent}
            <span class="detail-sub">${node.agent}</span>
          </div>
        </div>`);
      }
      if (node.task) {
        parts.push(`<div class="graph-detail-section">
          <div class="detail-label">Задача</div>
          <div class="detail-value pre-wrap">${node.task}</div>
        </div>`);
      }
      if (node.input) {
        parts.push(`<div class="graph-detail-section">
          <div class="detail-label">Вход</div>
          <div class="detail-value code">${node.input}</div>
        </div>`);
      }
      if (node.type === "llm_classifier") {
        const wf = allWorkflows.find(w => w.file_stem === data.file_stem);
        if (wf?.config?.statuses && wf.config.statuses.length > 0) {
          const rows = wf.config.statuses.map((s: any) =>
            `<div class="detail-case"><span class="case-key">${s.id}</span> ${s.description}</div>`
          ).join("");
          parts.push(`<div class="graph-detail-section">
            <div class="detail-label">Статусы классификации</div>
            <div class="detail-value">${rows}</div>
          </div>`);
        }
      }
      if (node.action) {
        parts.push(`<div class="graph-detail-section">
          <div class="detail-label">Действие</div>
          <div class="detail-value code">${node.action}</div>
        </div>`);
      }
      if (node.required && node.required.length > 0) {
        const names = node.required.map((id: string) => {
          const a = allAgents.find(ag => ag.id === id);
          return a ? `${a.name} (${id})` : id;
        });
        parts.push(`<div class="graph-detail-section">
          <div class="detail-label">Требуемые агенты</div>
          <div class="detail-value">${names.join(", ")}</div>
        </div>`);
      }
      if (node.workflow) {
        const wf = allWorkflows.find(w => node.workflow && w.file_stem === node.workflow.replace(/\.yaml$/, ""));
        parts.push(`<div class="graph-detail-section">
          <div class="detail-label">Вызывает граф</div>
          <div class="detail-value">${wf ? wf.name : node.workflow}
            <span class="detail-sub">${node.workflow}</span>
          </div>
        </div>`);
      }
      if (node.cases) {
        const html = Object.entries(node.cases)
          .map(([k, v]) => `<div class="detail-case"><span class="case-key">${k}</span> → ${v}</div>`)
          .join("");
        parts.push(`<div class="graph-detail-section">
          <div class="detail-label">Варианты (switch)</div>
          <div class="detail-value">${html}</div>
        </div>`);
      }
    }

    this.el.graphDetailContent.innerHTML = parts.join("");
  }

  private hideSidebar(): void {
    this.el.graphSidebar.classList.remove("open");
  }

  onTabActivated(): void {
    if (!this.allData) {
      this.loadData();
    } else {
      this.loadGraph();
      setTimeout(() => this.network?.redraw(), 100);
    }
  }
}
