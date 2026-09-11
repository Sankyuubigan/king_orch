import type { GraphController } from "./graph-class";

// ─── Селекторное выделение (LMB box selection) + RMB pan + multi-drag ───

export function setupCanvasInteraction(this: GraphController): void {
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

export function selectNode(this: GraphController, id: string): void {
  this.selectedNodes.add(id);
  const el = document.getElementById('node-' + id);
  if (el) el.classList.add('selected');
}

export function deselectAllNodes(this: GraphController): void {
  for (const id of this.selectedNodes) {
    const el = document.getElementById('node-' + id);
    if (el) el.classList.remove('selected');
  }
  this.selectedNodes.clear();
}

export function startBoxSelection(this: GraphController, e: MouseEvent): void {
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

export function updateBoxSelection(this: GraphController, e: MouseEvent): void {
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

export function endBoxSelection(this: GraphController, _e: MouseEvent): void {
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