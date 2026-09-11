import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { DrawflowExport } from "drawflow";
import { showToast } from "../../ui";
import { trackError } from "../../telemetry";
import { readWorkflowFile, saveWorkflow } from "../../services";
import type { WorkflowGraphDef, GraphNodeDef, GraphEdgeDef } from "./types";
import { isDynamicNode, OUTPUT_COUNT } from "./constants";
import type { GraphController } from "./graph-class";

// ─── Системный диалог открытия ───

export async function handleOpen(this: GraphController): Promise<void> {
    try {
      const selected = await openDialog({
        multiple: false,
        filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
      });
      if (!selected) return;

      const wf = await readWorkflowFile(selected);

      this.currentFilePath = selected;
      this.currentWorkflowName = wf.name;
      this.currentWorkflowConfig = wf.config ?? null;
      this.el.btnSaveWorkflow.disabled = false;
      this.hideSidebar();
      this.ensureEditor();

      this.el.currentWorkflowName.textContent = `${wf.name} (${wf.file_stem || ""}.yaml)`;

      const nonSwitchEdges = wf.edges.filter((e) => {
        const fn = wf.nodes.find((n) => n.id === e.from);
        return !fn || !isDynamicNode(fn.type);
      });
      const allEdges = [...nonSwitchEdges, ...this.getImplicitSwitchEdges(wf.nodes)];
      const nodePositions = this.computeAutoLayout(wf.nodes, allEdges);

      // Build import data structure with custom IDs as keys
      const importData: DrawflowExport = {
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
          void trackError("graph.restoreConnection", connErr);
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
        } else if (node.type === "condition_router") {
          if (node.true_to) targets.push({ to: node.true_to, caseKey: "true" });
          if (node.false_to) targets.push({ to: node.false_to, caseKey: "false" });
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
            void trackError("graph.restoreConnection.switch", connErr);
          }
        }
      }

      this.editor!.zoom_reset();
      this.resetHistory();
    } catch (e) {
      console.error("handleOpen error:", e);
      showToast(`Ошибка загрузки: ${e}`, "error");
      void trackError("graph.open", e);
    }
}

export function clearEditor(this: GraphController): void {
  if (!this.editor) return;
  this.saveCheckpoint();
  const exportData = this.editor.export();
  const nodes = exportData.drawflow.Home.data;
  for (const id of Object.keys(nodes)) {
    this.editor.removeNodeId("node-" + id);
  }
}

// ─── Сохранение ───

export async function handleSave(this: GraphController): Promise<void> {
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

      const workflow: WorkflowGraphDef = { name: this.currentWorkflowName, visible: true, config, nodes, edges };
      const saveResult = await saveWorkflow(this.currentFilePath, workflow);
      void saveResult;
      this.pristineSnapshot = this.captureSnapshot();
      this.markClean();
      showToast("✅ Workflow сохранён", "success");
    } catch (e) {
      // Файл НЕ сохранён (бэкенд отклоняет невалидный YAML до записи).
      // Оставляем dirty-флаг — данные на холсте считаются несохранёнными.
      console.error("[handleSave] Сохранение НЕ выполнено, файл не перезаписан:", e);
      showToast(`❌ Не сохранено: ${e}`, "error");
      void trackError("graph.save", e);
    }
}