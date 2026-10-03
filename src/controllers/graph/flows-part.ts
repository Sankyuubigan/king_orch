import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { DrawflowExport } from "drawflow";
import { showToast } from "../../ui";
import { trackError } from "../../telemetry";
import { readWorkflowFile, saveWorkflow } from "../../services";
import type { WorkflowGraphDef, GraphNodeDef, GraphEdgeDef } from "./types";
import { isDynamicNode, OUTPUT_COUNT, ENTRY_NODE_TYPE, inputCount } from "./constants";
import type { GraphController } from "./graph-class";

// ─── Системный диалог открытия ───

export async function handleOpen(this: GraphController): Promise<void> {
    try {
      const selected = await openDialog({
        multiple: false,
        filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
      });
      if (!selected) return;

      const readResult = await readWorkflowFile(selected);
      const wf = readResult.workflow;
      const diagnostics = [...readResult.diagnostics];
      this.setYamlDiagnostics(diagnostics);

      this.currentFilePath = selected;
      this.currentWorkflowName = wf.name;
      this.currentWorkflowConfig = wf.config ?? null;
      this.el.btnSaveWorkflow.disabled = false;
      this.hideSidebar();
      this.ensureEditor();

      this.el.currentWorkflowName.textContent = `${wf.name} (${wf.file_stem || ""}.yaml)`;

      const nonSwitchEdgeEntries = wf.edges.flatMap((edge, index) => {
        const fromNode = wf.nodes.find((node) => node.id === edge.from);
        if (fromNode && isDynamicNode(fromNode.type)) {
          diagnostics.push({
            code: "DYNAMIC_EDGE_NOT_RENDERED",
            location: `edges[${index}]`,
            message: `ребро '${edge.from}' → '${edge.to}' из динамической ноды нельзя точно отобразить и сохранить: маршрут хранится в полях ноды`,
          });
          return [];
        }
        return [{ edge, index }];
      });
      const nonSwitchEdges = nonSwitchEdgeEntries.map(({ edge }) => edge);
      const allEdges = [...nonSwitchEdges, ...this.getImplicitSwitchEdges(wf.nodes)];
      // entry определяем после подсчёта ВСЕХ рёбер: маршруты динамических нод
      // хранятся в полях нод, и по одним `edges:` терминальные ноды выглядели бы
      // корнями графа. От точки входа потом считается и раскладка.
      this.currentEntryNodeId = this.resolveEntryForEditor({
        entry: wf.entry,
        nodes: wf.nodes,
        edges: allEdges,
      });
      const nodePositions = this.computeAutoLayout(wf.nodes, allEdges, this.currentEntryNodeId);

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
        const ins = inputCount(node.type);
        for (let i = 1; i <= ins; i++) inputs[`input_${i}`] = { connections: [] };
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

      for (const { edge, index } of nonSwitchEdgeEntries) {
        if (!nodesData[edge.from] || !nodesData[edge.to]) {
          diagnostics.push({
            code: "EDGE_ENDPOINT_MISSING",
            location: `edges[${index}]`,
            message: `ребро '${edge.from}' → '${edge.to}' не может быть отображено: один из endpoint отсутствует среди nodes[].id`,
          });
          continue;
        }
        try {
          this.editor!.addConnection(edge.from, edge.to, `output_1`, "input_1");
        } catch (connErr) {
          diagnostics.push({
            code: "IMPORT_CONNECTION_FAILED",
            location: `edges[${index}]`,
            message: `Drawflow не смог восстановить связь '${edge.from}' → '${edge.to}': ${connErr}`,
          });
        }
      }

      // Switch/SignalRouter/ConditionCheck connections come from node data (source of truth)
      for (const node of wf.nodes) {
        if (!isDynamicNode(node.type)) continue;
        const targets: Array<{ to: string; caseKey?: string; location: string }> = [];
        if (node.type === "condition_check") {
          if (node.true_to) targets.push({ to: node.true_to, caseKey: "true", location: `nodes[id=${node.id}].true_to` });
          if (node.false_to) targets.push({ to: node.false_to, caseKey: "false", location: `nodes[id=${node.id}].false_to` });
          if (node.sequential_to) targets.push({ to: node.sequential_to, caseKey: "seq", location: `nodes[id=${node.id}].sequential_to` });
        } else if (node.type === "condition_router") {
          if (node.true_to) targets.push({ to: node.true_to, caseKey: "true", location: `nodes[id=${node.id}].true_to` });
          if (node.false_to) targets.push({ to: node.false_to, caseKey: "false", location: `nodes[id=${node.id}].false_to` });
          if (node.sequential_to) targets.push({ to: node.sequential_to, caseKey: "seq", location: `nodes[id=${node.id}].sequential_to` });
        } else if (node.cases_priority) {
          for (const [caseIndex, cp] of node.cases_priority.entries()) {
            targets.push({
              to: cp.to,
              caseKey: cp.key,
              location: `nodes[id=${node.id}].cases_priority[${caseIndex}]`,
            });
          }
        } else if (node.cases) {
          for (const [key, val] of Object.entries(node.cases)) {
            targets.push({ to: val, caseKey: key, location: `nodes[id=${node.id}].cases.${key}` });
          }
        }
        if (node.default) targets.push({ to: node.default, caseKey: "default", location: `nodes[id=${node.id}].default` });
        for (const target of targets) {
          if (!target.to || !nodesData[target.to]) {
            diagnostics.push({
              code: "NODE_TARGET_MISSING",
              location: target.location,
              message: `маршрут '${target.to || "(пусто)"}' не будет отображён: целевая нода отсутствует`,
            });
            continue;
          }
          if (!this.canReceiveEdge(target.to)) {
            diagnostics.push({
              code: "EDGE_INTO_SOURCE_NODE",
              location: target.location,
              message: `маршрут '${node.id}' → '${target.to}' не будет отображён: у ноды-источника нет входов`,
            });
            continue;
          }
          const outIdx = this.getSwitchOutputIndex(node, target.caseKey);
          try {
            this.editor!.addConnection(node.id, target.to, `output_${outIdx + 1}`, "input_1");
          } catch (connErr) {
            diagnostics.push({
              code: "IMPORT_CONNECTION_FAILED",
              location: target.location,
              message: `Drawflow не смог восстановить связь '${node.id}' → '${target.to}': ${connErr}`,
            });
          }
        }
      }

      this.editor!.zoom_reset();
      this.resetHistory();
      this.setYamlDiagnostics(diagnostics);
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

// ─── Точка входа графа ───

/**
 * Определяет `entry` для редактора.
 *
 * Порядок: объявленное значение → единственная нода типа `user_message` → единственная
 * нода без входящих рёбер. Последний шаг нужен только для миграции старых файлов:
 * он ничего не пишет сам, а лишь показывает вход на холсте, чтобы пользователь его
 * увидел и подтвердил сохранением. Движок принимает только явно записанное `entry`.
 *
 * ВАЖНО: передавать нужно ВСЕ рёбра, включая неявные маршруты динамических нод
 * (`getImplicitSwitchEdges`). По одним `edges:` терминальные ноды выглядят
 * корнями графа, и точку входа вывести не из чего.
 */
export function resolveEntryForEditor(
  this: GraphController,
  wf: { entry?: string | null; nodes: GraphNodeDef[]; edges: GraphEdgeDef[] },
): string | null {
  const declared = (wf.entry ?? "").trim();
  if (declared && wf.nodes.some((n) => n.id === declared)) return declared;

  const sourceNodes = wf.nodes.filter((n) => n.type === ENTRY_NODE_TYPE);
  if (sourceNodes.length === 1) return sourceNodes[0].id;
  if (sourceNodes.length > 1) return null;

  const hasIncoming = new Set(wf.edges.map((e) => e.to));
  const roots = wf.nodes.filter((n) => !hasIncoming.has(n.id));
  return roots.length === 1 ? roots[0].id : null;
}

/**
 * Назначает ноду точкой входа графа.
 *
 * Порты не трогаем: молча выбрасывать входящие рёбра — это потеря работы
 * пользователя. Если у новой точки входа есть входящие рёбра — это покажет
 * диагностика `ENTRY_HAS_INCOMING_EDGE`, а не тихая правка графа.
 */
export function setEntryNode(this: GraphController, nodeId: string): void {
  const dn = this.editor?.drawflow.drawflow.Home.data[nodeId];
  if (!dn) return;
  const previousId = this.currentEntryNodeId;
  if (previousId === nodeId) return;

  this.saveCheckpoint();
  this.currentEntryNodeId = nodeId;
  if (previousId) this.updateNodeHtml(previousId);
  this.updateNodeHtml(nodeId);
  showToast(`🚪 Вход графа: ${nodeId}`, "success");
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

      // Точка входа — объявление, а не позиция в `nodes:` (порядок нод здесь
      // задаётся хешем drawflow). Без неё движок не запустит граф, поэтому
      // отсутствие входа — ошибка сохранения, а не тихая потеря.
      const entryNodeId = this.resolveEntryForEditor({
        entry: this.currentEntryNodeId,
        nodes,
        // Полный набор рёбер: маршруты динамических нод хранятся в полях нод
        // и в `edges:` не попадают, но для определения корней они значимы.
        edges: [...edges, ...this.getImplicitSwitchEdges(nodes)],
      });
      if (!entryNodeId) {
        const message = "❌ Не задана точка входа: добавь ноду «Сообщение юзера» и назначь её входом (кнопка в сайдбаре)";
        console.error(`[handleSave] ${message}`);
        showToast(message, "error");
        return;
      }

      const config = this.currentWorkflowConfig ? JSON.parse(JSON.stringify(this.currentWorkflowConfig)) : null;
      if (config?.facts_file) {
        config.facts = [];
      }

      const workflow: WorkflowGraphDef = { name: this.currentWorkflowName, config, entry: entryNodeId, nodes, edges };
      const saveResult = await saveWorkflow(this.currentFilePath, workflow);
      void saveResult;
      this.pristineSnapshot = this.captureSnapshot();
      this.setYamlDiagnostics([]);
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