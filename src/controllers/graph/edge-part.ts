import type { GraphController } from "./graph-class";

// ─── Reverse-connection: drag из input в чужой output ───

export function enableReverseConnection(this: GraphController, nodeId: string): void {
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

export function setupEdgeDrag(this: GraphController): void {
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

export function beginEdgeReconnect(this: GraphController, data: {
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

export function restoreEdge(this: GraphController, conn: { sourceId: string; targetId: string; outputKey: string }): void {
  if (!this.editor) return;
  const { sourceId, targetId, outputKey } = conn;

  // Если ноды уже нет на холсте — не восстанавливаем
  if (!this.editor.drawflow.drawflow.Home.data[sourceId] ||
      !this.editor.drawflow.drawflow.Home.data[targetId]) return;

  try {
    this.editor.addConnection(sourceId, targetId, outputKey, "input_1");
  } catch (_) { }
}