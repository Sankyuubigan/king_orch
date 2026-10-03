import type { GraphNodeDef, GraphEdgeDef } from "./types";
import type { GraphController } from "./graph-class";

export function computeAutoLayout(
  this: GraphController,
  nodes: GraphNodeDef[],
  edges: GraphEdgeDef[],
  entryNodeId?: string | null,
): Map<string, { x: number; y: number }> {
  const positions = new Map<string, { x: number; y: number }>();
  const inDegree = new Map<string, number>();
  for (const n of nodes) inDegree.set(n.id, 0);
  for (const e of edges) inDegree.set(e.to, (inDegree.get(e.to) || 0) + 1);

  const levels = new Map<string, number>();
  // Раскладка отсчитывает уровни от точки входа. Объявленный `entry` —
  // единственный источник правды о старте; без него берём ноду без входящих
  // рёбер (как раньше), но не смешиваем две разные эвристики в одну.
  const declaredEntry = entryNodeId && nodes.some((n) => n.id === entryNodeId) ? entryNodeId : null;
  let queue = declaredEntry
    ? [declaredEntry]
    : nodes.filter((n) => (inDegree.get(n.id) ?? 0) === 0).map((n) => n.id);
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