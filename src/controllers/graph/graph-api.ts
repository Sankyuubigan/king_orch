import type { GraphSnapshot, GraphNodeDef, GraphEdgeDef, GraphGroupDef } from "./types";
import type { GraphDiagnostic } from "../../services";
import type { ResizeDir } from "./groups-part";

export interface GraphApi {
  ensureEditor(): void;
  onTabActivated(): void;
  setYamlDiagnostics(diagnostics: GraphDiagnostic[]): void;

  /**
   * DOM-узел ноды Drawflow в границах СВОЕГО экземпляра студии.
   *
   * Нужен, потому что студий может быть несколько: id нод задаёт автор графа, и в двух
   * открытых графах они совпадают (`user_message`, `router`…), поэтому поиск по документу
   * попадал бы в чужой граф.
   */
  findNodeEl(nodeId: string): HTMLElement | null;

  setupCanvasInteraction(): void;
  setupCanvasZoom(): void;
  selectNode(id: string): void;
  deselectAllNodes(): void;
  startBoxSelection(e: MouseEvent): void;
  updateBoxSelection(e: MouseEvent): void;
  endBoxSelection(_e: MouseEvent): void;

  setupContextMenu(): void;
  toggleNodeDisabled(nodeId: string): void;

  handleOpen(): Promise<void>;
  clearEditor(): void;
  handleSave(): Promise<void>;
  resolveEntryForEditor(wf: { entry?: string | null; nodes: GraphNodeDef[] }): string | null;

  showNodeEditor(nodeId: string): void;
  showGroupEditor(groupId: string): void;
  hideSidebar(): void;
  renderCasesHtml(data: any): string;
  renderDefaultHtml(data: any): string;

  // ─── Группы нод (Group) ────────────────────────────────────────────────
  groupById(groupId: string): GraphGroupDef | undefined;
  groupNodeIds(groupId: string): string[];
  groupFrameRect(group: GraphGroupDef): { x: number; y: number; w: number; h: number } | null;
  isFramePinned(group: GraphGroupDef): boolean;
  fitGroupToNodes(groupId: string): boolean;
  groupFusedStatus(
    group: GraphGroupDef,
    strategy: string | null,
  ): { fusable: boolean; active: boolean; level: "off" | "on" | "warn"; text: string };
  /** Проблемы схлопывания группы по вердикту бэкенда (`GROUP_FUSED_*`). */
  groupFusedProblems(groupId: string): string[];
  groupAtPoint(canvasX: number, canvasY: number): GraphGroupDef | null;
  nodeSize(nodeId: string): { w: number; h: number };
  invalidateNodeSize(nodeId: string): void;
  ensureGroupLayer(): HTMLDivElement | null;
  buildGroupFrame(group: GraphGroupDef): HTMLDivElement | null;
  renderGroups(): void;
  refreshGroupFrames(groupIds?: string[]): void;
  generateGroupId(base: string): string;
  createGroup(clientX: number, clientY: number): GraphGroupDef;
  deleteGroup(groupId: string): void;
  renameGroup(oldId: string, newId: string): boolean;
  assignNodeToGroup(nodeId: string, groupId: string | null): void;
  onNodeMoved(nodeId: string): void;
  onGroupFrameClick(groupId: string): void;
  interactiveUnderGroup(clientX: number, clientY: number): Element | null;
  forwardMousedownTo(target: Element, ev: MouseEvent): void;
  beginGroupDrag(groupId: string, ev: MouseEvent): void;
  beginGroupResize(groupId: string, dir: ResizeDir, ev: MouseEvent): void;
  adoptSelectedNodesIntoGroup(groupId: string): number;
  serializeGroups(): GraphGroupDef[];
  loadGroups(groups: GraphGroupDef[] | null | undefined): void;

  addNode(type: string, clientX?: number, clientY?: number): void;
  copyNode(nodeId: string): void;
  generateUniqueNodeId(baseType: string): string;
  generateEntryNodeId(): string;
  hasSourceNode(): boolean;
  pasteNode(clientX?: number, clientY?: number): void;
  clearNodeTargets(data: GraphNodeDef): void;
  renameNode(oldKey: string, newKey: string): boolean;
  updateNodeHtml(nodeId: string): void;
  alignOutputsWithCases(nodeId: string): void;

  rebuildSwitchOutputs(nodeId: string): void;
  syncSwitchNodeFromConnections(dn: any): void;
  canReceiveEdge(targetId: string): boolean;
  onSwitchConnectionChanged(fromNodeId: string, toNodeId: string, outputKey: string, action: "created" | "removed"): void;
  getSwitchOutputCount(node: GraphNodeDef): number;
  getSwitchCaseKeys(node: GraphNodeDef): string[];
  getSwitchOutputIndex(node: GraphNodeDef, caseVal?: string): number;
  getImplicitSwitchEdges(nodes: GraphNodeDef[]): GraphEdgeDef[];

  computeAutoLayout(nodes: GraphNodeDef[], edges: GraphEdgeDef[], entryNodeId?: string | null): Map<string, { x: number; y: number }>;

  buildNodeInnerHtml(data: any, fallbackId?: string): string;
  esc(s: string): string;

  enableReverseConnection(nodeId: string): void;
  setupEdgeDrag(): void;
  beginEdgeReconnect(data: {
    sourceId: string; targetId: string; svg: SVGElement; isSourceHalf: boolean;
    clickX: number; clickY: number; outputKey: string;
  }, _ev: MouseEvent): void;
  restoreEdge(conn: { sourceId: string; targetId: string; outputKey: string }): void;

  captureSnapshot(): GraphSnapshot;
  applySnapshot(snap: GraphSnapshot): void;
  saveCheckpoint(): void;
  undo(): void;
  redo(): void;
  syncDirtyAfterRestore(): void;
  groupsEqual(a: GraphGroupDef[], b: GraphGroupDef[] | undefined): boolean;
  isNodesEqual(a: Record<string, any>, b: Record<string, any>): boolean;
  resetHistory(): void;
  markDirty(): void;
  markClean(): void;
  updateDirtyIndicator(): void;
  updateUndoButtons(): void;
}