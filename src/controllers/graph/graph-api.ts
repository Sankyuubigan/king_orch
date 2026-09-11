import type { GraphSnapshot, GraphNodeDef, GraphEdgeDef } from "./types";

export interface GraphApi {
  ensureEditor(): void;
  onTabActivated(): void;

  setupCanvasInteraction(): void;
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

  showNodeEditor(nodeId: string): void;
  hideSidebar(): void;
  renderCasesHtml(data: any): string;
  renderDefaultHtml(data: any): string;

  addNode(type: string, clientX?: number, clientY?: number): void;
  copyNode(nodeId: string): void;
  generateUniqueNodeId(baseType: string): string;
  pasteNode(clientX?: number, clientY?: number): void;
  clearNodeTargets(data: GraphNodeDef): void;
  renameNode(oldKey: string, newKey: string): boolean;
  updateNodeHtml(nodeId: string): void;
  alignOutputsWithCases(nodeId: string): void;

  rebuildSwitchOutputs(nodeId: string): void;
  syncSwitchNodeFromConnections(dn: any): void;
  onSwitchConnectionChanged(fromNodeId: string, toNodeId: string, outputKey: string, action: "created" | "removed"): void;
  getSwitchOutputCount(node: GraphNodeDef): number;
  getSwitchCaseKeys(node: GraphNodeDef): string[];
  getSwitchOutputIndex(node: GraphNodeDef, caseVal?: string): number;
  getImplicitSwitchEdges(nodes: GraphNodeDef[]): GraphEdgeDef[];

  computeAutoLayout(nodes: GraphNodeDef[], edges: GraphEdgeDef[]): Map<string, { x: number; y: number }>;

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
  isNodesEqual(a: Record<string, any>, b: Record<string, any>): boolean;
  resetHistory(): void;
  markDirty(): void;
  markClean(): void;
  updateDirtyIndicator(): void;
  updateUndoButtons(): void;
}