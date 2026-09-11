import { NODE_COLORS, NODE_LABELS } from "./constants";
import type { GraphController } from "./graph-class";

// ─── Редактор ноды в сайдбаре ───

export function showNodeEditor(this: GraphController, nodeId: string): void {
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

  if (data.type === "condition_router") {
    if (!data.conditions) data.conditions = [];
    html += `<div class="graph-detail-section">
      <div class="detail-label">Имя сигнала</div>
      <input type="text" id="ge-signal-name" class="ge-input" placeholder="validator_report" value="${this.esc(data.signal_name || "")}" />
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Логика</div>
      <select id="ge-logic" class="ge-select">
        <option value="any" ${data.logic !== "all" ? "selected" : ""}>any — хотя бы одно условие</option>
        <option value="all" ${data.logic === "all" ? "selected" : ""}>all — все условия</option>
      </select>
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Условия</div>
      <div id="ge-conditions-list">`;
    for (let i = 0; i < data.conditions.length; i++) {
      const c = data.conditions[i];
      html += `<div class="ge-case-row" data-index="${i}">
        <input class="ge-input ge-cond-field" value="${this.esc(c.field)}" placeholder="field" style="width:45%;" />
        <span style="color:#888;margin:0 2px;">=</span>
        <input class="ge-input ge-cond-equals" value="${this.esc(String(c.equals))}" placeholder="value" style="width:40%;" />
        <button class="ge-cond-remove" title="Удалить">🗑</button>
      </div>`;
    }
    html += `</div>
      <button id="ge-condition-add" class="btn-secondary" style="margin-top:4px;width:100%;font-size:12px;">+ Добавить условие</button>
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

  // ─── Condition Router event handlers ───

  const logicSelect = document.getElementById("ge-logic") as HTMLSelectElement;
  if (logicSelect) {
    logicSelect.addEventListener("change", () => {
      this.saveCheckpoint();
      data.logic = logicSelect.value === "all" ? "all" : undefined;
      this.updateNodeHtml(nodeId);
    });
  }

  const conditionAddBtn = document.getElementById("ge-condition-add");
  if (conditionAddBtn) {
    conditionAddBtn.addEventListener("click", () => {
      this.saveCheckpoint();
      if (!data.conditions) data.conditions = [];
      data.conditions.push({ field: "", equals: false });
      this.rebuildSwitchOutputs(nodeId);
      this.showNodeEditor(nodeId);
    });
  }

  document.querySelectorAll(".ge-cond-remove").forEach((btn) => {
    btn.addEventListener("click", () => {
      this.saveCheckpoint();
      const row = (btn as HTMLElement).closest(".ge-case-row") as HTMLElement;
      const idx = parseInt(row.dataset.index || "0", 10);
      data.conditions.splice(idx, 1);
      this.rebuildSwitchOutputs(nodeId);
      this.showNodeEditor(nodeId);
    });
  });

  document.querySelectorAll(".ge-cond-field").forEach((inp) => {
    inp.addEventListener("input", () => {
      const row = (inp as HTMLElement).closest(".ge-case-row") as HTMLElement;
      const idx = parseInt(row.dataset.index || "0", 10);
      data.conditions[idx].field = (inp as HTMLInputElement).value;
      this.updateNodeHtml(nodeId);
    });
  });

  document.querySelectorAll(".ge-cond-equals").forEach((inp) => {
    inp.addEventListener("input", () => {
      const row = (inp as HTMLElement).closest(".ge-case-row") as HTMLElement;
      const idx = parseInt(row.dataset.index || "0", 10);
      const val = (inp as HTMLInputElement).value;
      if (val === "true") data.conditions[idx].equals = true;
      else if (val === "false") data.conditions[idx].equals = false;
      else if (!isNaN(Number(val))) data.conditions[idx].equals = Number(val);
      else data.conditions[idx].equals = val;
      this.updateNodeHtml(nodeId);
    });
  });

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

export function hideSidebar(this: GraphController): void {
  if (this.el.graphSidebar.classList.contains("open")) {
    this.saveCheckpoint();
  }
  this.el.graphSidebar.classList.remove("open");
}

// ─── Рендер кейсов и дефолта (общее для switch и signal_router) ───

export function renderCasesHtml(this: GraphController, data: any): string {
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

export function renderDefaultHtml(this: GraphController, data: any): string {
  return `<div class="graph-detail-section">
    <div class="detail-label">Default (цель, если не совпало)</div>
    <input type="text" id="ge-default" class="ge-input" value="${this.esc(data.default || "")}" placeholder="node id" />
  </div>`;
}