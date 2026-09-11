import type { GraphController } from "./graph-class";

// ─── Контекстное меню (ПКМ) на нодах ───

export function setupContextMenu(this: GraphController): void {
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
        <div class="ctx-item${this.copiedNode ? "" : " disabled"}" data-action="paste-node">
          <span class="ctx-icon">📌</span>
          Вставить ноду
        </div>
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
            <div class="ctx-item" data-type="condition_router">🔀 Condition Router</div>
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

      // Вставка скопированной ноды
      this.ctxMenu!.querySelector('[data-action="paste-node"]')?.addEventListener('click', (ev) => {
        ev.stopPropagation();
        if (!this.copiedNode) return;
        this.pasteNode(cmX, cmY);
        this.ctxMenu?.classList.remove('open');
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
      <div class="ctx-item" data-action="copy-node">
        <span class="ctx-icon">📋</span>
        Копировать ноду
      </div>
      <div class="ctx-item${this.copiedNode ? "" : " disabled"}" data-action="paste-node">
        <span class="ctx-icon">📌</span>
        Вставить ноду
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

    this.ctxMenu!.querySelector('[data-action="copy-node"]')?.addEventListener('click', () => {
      this.copyNode(nodeId);
      this.ctxMenu?.classList.remove('open');
    });

    this.ctxMenu!.querySelector('[data-action="paste-node"]')?.addEventListener('click', () => {
      if (!this.copiedNode) return;
      this.pasteNode(e.clientX, e.clientY);
      this.ctxMenu?.classList.remove('open');
    });
  });
}

export function toggleNodeDisabled(this: GraphController, nodeId: string): void {
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