import { NODE_COLORS, NODE_LABELS } from "./constants";
import {
  buildKnownFields,
  conditionAt,
  conditionGroupList,
  conditionSiblings,
  isConditionGroup,
  parseEqualsValue,
  renderConditionsTreeHtml,
} from "./condition-editor";
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
      <div class="detail-label">Инжект мысли (фаза 1, ID агентов)</div>
      <input type="text" id="ge-inject-thoughts" class="ge-input" placeholder="через запятую: soma_translator" value="${this.esc((data.inject_thoughts || []).join(', '))}" />
    </div>`;
    html += `<div class="graph-detail-section">
      <div class="detail-label">Инжект ответы (фаза 2, ID агентов)</div>
      <input type="text" id="ge-inject-response" class="ge-input" placeholder="через запятую: validator, synthesizer" value="${this.esc((data.inject_response || []).join(', '))}" />
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
    if (!Array.isArray(data.conditions)) data.conditions = [];
    const facts = (this.currentWorkflowConfig?.facts || []) as Array<{ id: string; values?: string[] }>;
    const allNodeData = this.editor
      ? (Object.values(this.editor.drawflow.drawflow.Home.data) as any[]).map((dn) => dn.data)
      : [];
    const knownFields = buildKnownFields(allNodeData);
    const fieldOptions = Array.from(new Set([...facts.map((f) => f.id), ...knownFields]));
    html += `<div class="graph-detail-section">
      <div class="detail-label">Условия (дерево: группы — скобки, без JSON)</div>
      <datalist id="ge-cond-fields-list">
        ${fieldOptions.map((f) => `<option value="${this.esc(f)}"></option>`).join("")}
      </datalist>
      <div id="ge-conditions-tree">${renderConditionsTreeHtml(data, { esc: (s) => this.esc(s), facts, knownFields: fieldOptions })}</div>
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

  const thoughtsInput = document.getElementById("ge-inject-thoughts") as HTMLInputElement;
  if (thoughtsInput) {
    thoughtsInput.addEventListener("change", () => {
      this.saveCheckpoint();
      const val = thoughtsInput.value.trim();
      data.inject_thoughts = val ? val.split(',').map((s: string) => s.trim()) : undefined;
      this.updateNodeHtml(nodeId);
    });
  }

  const responseInput = document.getElementById("ge-inject-response") as HTMLInputElement;
  if (responseInput) {
    responseInput.addEventListener("change", () => {
      this.saveCheckpoint();
      const val = responseInput.value.trim();
      data.inject_response = val ? val.split(',').map((s: string) => s.trim()) : undefined;
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

  // ─── Condition Router event handlers (дерево условий) ───

  document.querySelectorAll(".ge-cond-logic").forEach((sel) => {
    sel.addEventListener("change", () => {
      this.saveCheckpoint();
      const path = JSON.parse((sel as HTMLElement).dataset.path || "[]") as number[];
      const isRoot = (sel as HTMLElement).dataset.isroot === "1";
      const logic = (sel as HTMLSelectElement).value === "all" ? "all" : "any";
      if (isRoot) {
        data.logic = logic === "all" ? "all" : undefined;
      } else {
        const target = conditionAt({ conditions: data.conditions }, path);
        if (target && isConditionGroup(target)) target.logic = logic;
      }
      this.updateNodeHtml(nodeId);
    });
  });

  document.querySelectorAll(".ge-cond-field").forEach((inp) => {
    inp.addEventListener("input", () => {
      const path = JSON.parse((inp as HTMLElement).dataset.path || "[]") as number[];
      const target = conditionAt({ conditions: data.conditions }, path);
      if (target && !isConditionGroup(target)) target.field = (inp as HTMLInputElement).value;
      this.updateNodeHtml(nodeId);
    });
  });

  document.querySelectorAll(".ge-cond-equals").forEach((inp) => {
    const apply = () => {
      const path = JSON.parse((inp as HTMLElement).dataset.path || "[]") as number[];
      const target = conditionAt({ conditions: data.conditions }, path);
      if (target && !isConditionGroup(target)) {
        target.equals = parseEqualsValue((inp as HTMLInputElement).value);
      }
      this.updateNodeHtml(nodeId);
    };
    inp.addEventListener("input", apply);
    inp.addEventListener("change", () => {
      this.saveCheckpoint();
      apply();
    });
  });

  document.querySelectorAll(".ge-cond-add").forEach((btn) => {
    btn.addEventListener("click", () => {
      this.saveCheckpoint();
      const path = JSON.parse((btn as HTMLElement).dataset.path || "[]") as number[];
      conditionGroupList({ conditions: data.conditions }, path).push({ field: "", equals: false });
      this.showNodeEditor(nodeId);
    });
  });

  document.querySelectorAll(".ge-cond-add-group").forEach((btn) => {
    btn.addEventListener("click", () => {
      this.saveCheckpoint();
      const path = JSON.parse((btn as HTMLElement).dataset.path || "[]") as number[];
      conditionGroupList({ conditions: data.conditions }, path).push({ logic: "any", conditions: [] });
      this.showNodeEditor(nodeId);
    });
  });

  document.querySelectorAll(".ge-cond-remove").forEach((btn) => {
    btn.addEventListener("click", () => {
      this.saveCheckpoint();
      const path = JSON.parse((btn as HTMLElement).dataset.path || "[]") as number[];
      const siblings = conditionSiblings({ conditions: data.conditions }, path);
      if (!siblings) return;
      siblings.splice(path[path.length - 1], 1);
      this.showNodeEditor(nodeId);
    });
  });

  document.querySelectorAll(".ge-cond-up").forEach((btn) => {
    btn.addEventListener("click", () => {
      this.saveCheckpoint();
      const path = JSON.parse((btn as HTMLElement).dataset.path || "[]") as number[];
      const siblings = conditionSiblings({ conditions: data.conditions }, path);
      if (!siblings) return;
      const i = path[path.length - 1];
      if (i > 0) {
        [siblings[i - 1], siblings[i]] = [siblings[i], siblings[i - 1]];
        this.showNodeEditor(nodeId);
      }
    });
  });

  document.querySelectorAll(".ge-cond-down").forEach((btn) => {
    btn.addEventListener("click", () => {
      this.saveCheckpoint();
      const path = JSON.parse((btn as HTMLElement).dataset.path || "[]") as number[];
      const siblings = conditionSiblings({ conditions: data.conditions }, path);
      if (!siblings) return;
      const i = path[path.length - 1];
      if (i < siblings.length - 1) {
        [siblings[i], siblings[i + 1]] = [siblings[i + 1], siblings[i]];
        this.showNodeEditor(nodeId);
      }
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