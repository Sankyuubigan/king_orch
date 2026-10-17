import { showToast } from "../../ui";
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

  this.inspectingGroupId = null;
  this.el.graphSidebar.classList.add("open");
  this.el.graphDetailTitle.textContent = `✏️ ${nodeId}`;

  const color = NODE_COLORS[data.type] || "#666";
  const typeLabel = NODE_LABELS[data.type] || data.type;

  const isDisabled = !!data.disabled;
  const isEntry = this.currentEntryNodeId === (data.id || nodeId);

  const groupOptions = [
    `<option value=""${data.group ? "" : " selected"}>— без группы —</option>`,
    ...this.currentGroups.map(
      (g) =>
        `<option value="${this.esc(g.id)}"${data.group === g.id ? " selected" : ""}>${this.esc(g.title || g.id)}</option>`,
    ),
  ].join("");

  let html = `<div class="graph-detail-section">
      <div class="detail-label">ID ноды</div>
      <input type="text" id="ge-node-id" class="ge-input" value="${this.esc(data.id || nodeId)}" />
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Тип</div>
      <div class="detail-value" style="border-left:4px solid ${color};padding-left:8px;">${typeLabel}</div>
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Группа нод</div>
      <select id="ge-node-group" class="ge-input">${groupOptions}</select>
      <div class="detail-hint">Группа — это рамка на холсте. В режиме Detailed она ничего не меняет; в режиме Fused ноды группы схлопываются в один вызов LLM.</div>
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Точка входа графа</div>
      ${
        isEntry
          ? `<div class="detail-value ge-entry-badge">🚪 Это вход графа — с него начинается прогон</div>`
          : `<div class="detail-hint">Вход графа — нода «Сообщение юзера». Назначить входом другую ноду нельзя: только она даёт текст пользователя.</div>`
      }
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
      <div class="detail-label">Инжект ответов (ID агентов)</div>
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

  // Экземпляр студии может быть не один (вкладок студии несколько), поэтому поиск полей
  // идёт В ПРЕДЕЛАХ сайдбара этого контроллера, а не по всему документу. Разметка сайдбара
  // пересоздаётся при каждом открытии ноды, значит лишние id внутри неё уже удалены.
  const editorRoot = this.el.graphDetailContent;

  /** Поле редактора ноды в границах ЭТОГО сайдбара (не по всему документу). */
  const getField = <T extends Element>(id: string): T => editorRoot.querySelector<T>(`#${id}`) as T;

  const groupSelect = getField<HTMLSelectElement>("ge-node-group");
  if (groupSelect) {
    groupSelect.addEventListener("change", () => {
      this.assignNodeToGroup(nodeId, groupSelect.value || null);
    });
  }

  const agentInput = getField<HTMLInputElement>("ge-agent");
  if (agentInput) {
    agentInput.addEventListener("input", () => {
      this.editor!.drawflow.drawflow.Home.data[nodeId].data.agent = agentInput.value;
      this.updateNodeHtml(nodeId);
    });
  }

  const taskTextarea = getField<HTMLTextAreaElement>("ge-task");
  if (taskTextarea) {
    taskTextarea.addEventListener("change", () => {
      this.saveCheckpoint();
      this.editor!.drawflow.drawflow.Home.data[nodeId].data.task = taskTextarea.value;
    });
  }

  const outputTypeSelect = getField<HTMLSelectElement>("ge-output-type");
  if (outputTypeSelect) {
    outputTypeSelect.addEventListener("change", () => {
      this.saveCheckpoint();
      data.output_type = outputTypeSelect.value === "thought" ? undefined : outputTypeSelect.value;
      this.updateNodeHtml(nodeId);
    });
  }

  const responseInput = getField<HTMLInputElement>("ge-inject-response");
  if (responseInput) {
    responseInput.addEventListener("change", () => {
      this.saveCheckpoint();
      const val = responseInput.value.trim();
      data.inject_response = val ? val.split(',').map((s: string) => s.trim()) : undefined;
      this.updateNodeHtml(nodeId);
    });
  }

  const nodeIdInput = getField<HTMLInputElement>("ge-node-id");
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

  const disabledToggle = getField<HTMLInputElement>("ge-disabled");
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

  const signalNameInput = getField<HTMLInputElement>("ge-signal-name");
  if (signalNameInput) {
    signalNameInput.addEventListener("change", () => {
      this.saveCheckpoint();
      this.editor!.drawflow.drawflow.Home.data[nodeId].data.signal_name = signalNameInput.value || undefined;
    });
  }

  const fieldInput = getField<HTMLInputElement>("ge-field");
  if (fieldInput) {
    fieldInput.addEventListener("change", () => {
      this.saveCheckpoint();
      this.editor!.drawflow.drawflow.Home.data[nodeId].data.field = fieldInput.value || undefined;
      this.updateNodeHtml(nodeId);
    });
  }

  const sequentialToInput = getField<HTMLInputElement>("ge-sequential-to");
  if (sequentialToInput) {
    sequentialToInput.addEventListener("change", () => {
      this.saveCheckpoint();
      data.sequential_to = sequentialToInput.value || undefined;
      this.updateNodeHtml(nodeId);
    });
  }

  const trueToInput = getField<HTMLInputElement>("ge-true-to");
  if (trueToInput) {
    trueToInput.addEventListener("change", () => {
      this.saveCheckpoint();
      data.true_to = trueToInput.value || undefined;
      this.updateNodeHtml(nodeId);
    });
  }

  const falseToInput = getField<HTMLInputElement>("ge-false-to");
  if (falseToInput) {
    falseToInput.addEventListener("change", () => {
      this.saveCheckpoint();
      data.false_to = falseToInput.value || undefined;
      this.updateNodeHtml(nodeId);
    });
  }

  // ─── Switch/SeqSwitch/SignalRouter event handlers ───

  const switchTypeSelect = getField<HTMLSelectElement>("ge-switch-type");
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

  const inputObjectArea = getField<HTMLTextAreaElement>("ge-input-object");
  if (inputObjectArea) {
    inputObjectArea.addEventListener("change", () => {
      this.saveCheckpoint();
      data.input_object = inputObjectArea.value || undefined;
    });
  }

  const defaultInput = getField<HTMLInputElement>("ge-default");
  if (defaultInput) {
    defaultInput.addEventListener("change", () => {
      this.saveCheckpoint();
      data.default = defaultInput.value || undefined;
      this.rebuildSwitchOutputs(nodeId);
    });
  }

  const noteContent = getField<HTMLTextAreaElement>("ge-note-content");
  if (noteContent) {
    noteContent.addEventListener("change", () => {
      this.saveCheckpoint();
      this.editor!.drawflow.drawflow.Home.data[nodeId].data.input = noteContent.value || undefined;
      this.updateNodeHtml(nodeId);
    });
  }

  const freeformInput = getField<HTMLTextAreaElement>("ge-freeform-input");
  if (freeformInput) {
    freeformInput.addEventListener("change", () => {
      this.saveCheckpoint();
      this.editor!.drawflow.drawflow.Home.data[nodeId].data.input = freeformInput.value || undefined;
      this.updateNodeHtml(nodeId);
    });
  }

  const noteSysMsg = getField<HTMLTextAreaElement>("ge-note-sysmsg");
  if (noteSysMsg) {
    noteSysMsg.addEventListener("change", () => {
      this.saveCheckpoint();
      this.editor!.drawflow.drawflow.Home.data[nodeId].data.system_message = noteSysMsg.value || undefined;
      this.updateNodeHtml(nodeId);
    });
  }

  // ─── Condition Router event handlers (дерево условий) ───

  this.el.graphDetailContent.querySelectorAll(".ge-cond-logic").forEach((sel) => {
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

  this.el.graphDetailContent.querySelectorAll(".ge-cond-field").forEach((inp) => {
    inp.addEventListener("input", () => {
      const path = JSON.parse((inp as HTMLElement).dataset.path || "[]") as number[];
      const target = conditionAt({ conditions: data.conditions }, path);
      if (target && !isConditionGroup(target)) target.field = (inp as HTMLInputElement).value;
      this.updateNodeHtml(nodeId);
    });
  });

  this.el.graphDetailContent.querySelectorAll(".ge-cond-equals").forEach((inp) => {
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

  this.el.graphDetailContent.querySelectorAll(".ge-cond-add").forEach((btn) => {
    btn.addEventListener("click", () => {
      this.saveCheckpoint();
      const path = JSON.parse((btn as HTMLElement).dataset.path || "[]") as number[];
      conditionGroupList({ conditions: data.conditions }, path).push({ field: "", equals: false });
      this.showNodeEditor(nodeId);
    });
  });

  this.el.graphDetailContent.querySelectorAll(".ge-cond-add-group").forEach((btn) => {
    btn.addEventListener("click", () => {
      this.saveCheckpoint();
      const path = JSON.parse((btn as HTMLElement).dataset.path || "[]") as number[];
      conditionGroupList({ conditions: data.conditions }, path).push({ logic: "any", conditions: [] });
      this.showNodeEditor(nodeId);
    });
  });

  this.el.graphDetailContent.querySelectorAll(".ge-cond-remove").forEach((btn) => {
    btn.addEventListener("click", () => {
      this.saveCheckpoint();
      const path = JSON.parse((btn as HTMLElement).dataset.path || "[]") as number[];
      const siblings = conditionSiblings({ conditions: data.conditions }, path);
      if (!siblings) return;
      siblings.splice(path[path.length - 1], 1);
      this.showNodeEditor(nodeId);
    });
  });

  this.el.graphDetailContent.querySelectorAll(".ge-cond-up").forEach((btn) => {
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

  this.el.graphDetailContent.querySelectorAll(".ge-cond-down").forEach((btn) => {
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

  const addBtn = getField<HTMLButtonElement>("ge-case-add");
  if (addBtn) {
    addBtn.addEventListener("click", () => {
      this.saveCheckpoint();
      data.cases_priority.push({ key: "", to: "" });
      this.rebuildSwitchOutputs(nodeId);
      this.showNodeEditor(nodeId);
    });
  }

  this.el.graphDetailContent.querySelectorAll(".ge-case-remove").forEach((btn) => {
    btn.addEventListener("click", () => {
      this.saveCheckpoint();
      const row = (btn as HTMLElement).closest(".ge-case-row") as HTMLElement;
      const idx = parseInt(row.dataset.index || "0", 10);
      data.cases_priority.splice(idx, 1);
      this.rebuildSwitchOutputs(nodeId);
      this.showNodeEditor(nodeId);
    });
  });

  this.el.graphDetailContent.querySelectorAll(".ge-case-up").forEach((btn) => {
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

  this.el.graphDetailContent.querySelectorAll(".ge-case-down").forEach((btn) => {
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

  this.el.graphDetailContent.querySelectorAll(".ge-case-key").forEach((inp) => {
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
  this.inspectingGroupId = null;
  this.el.graphSidebar.classList.remove("open");
}

// ─── Редактор группы нод ───

/**
 * Сайдбар группы нод.
 *
 * Раньше `showNodeEditor` был ЕДИНСТВЕННЫМ писателем в `graphDetailContent`, и
 * инспектор группы добавил бы второй формат в то же место. Он остаётся отдельной
 * функцией, но использует ту же тройку (`.open` + заголовок + innerHTML), что и
 * нода, — иначе закрытие сайдбара и Esc будут вести себя по-разному.
 */
export function showGroupEditor(this: GraphController, groupId: string): void {
  const group = this.groupById(groupId);
  if (!group) return;

  this.inspectingGroupId = groupId;
  this.el.graphSidebar.classList.add("open");
  this.el.graphDetailTitle.textContent = `📦 ${group.title || group.id}`;

  const memberIds = this.groupNodeIds(groupId);
  const members = memberIds.length > 0
    ? memberIds.map((id) => `<li>${this.esc(id)}</li>`).join("")
    : `<li class="detail-hint">пусто — перетащи ноды в рамку или выбери группу в инспекторе ноды</li>`;

  const tools = (group.tools || []).join(", ");
  // Живая подсказка вместо молчаливой рамки: без неё группа в Detailed выглядит
  // «нерабочей», и правильный ответ на вопрос автора («что задать, чтобы
  // работало») приходилось держать в голове.
  //
  // Если бэкенд сказал, что форму макро-ноды группа не имеет, — оптимистичная
  // строка была бы ложью («выполняется одним вызовом» при роутере внутри).
  // Поэтому при таком вердикте показываем ЕГО текст, а не свой.
  const problems = this.groupFusedProblems(groupId);
  const status = this.groupFusedStatus(group, this.currentWorkflowStrategy);
  const shownStatus =
    problems.length > 0
      ? { ...status, active: false, level: "warn" as const, text: problems[0] }
      : status;
  const moreProblems = problems.slice(1);
  const pinned = this.isFramePinned(group);
  const selectedCount = this.selectedNodes.size;
  this.el.graphDetailContent.innerHTML = `
    <div class="graph-detail-section">
      <div class="detail-label">Что группа сделает</div>
      <div class="ge-group-status is-${shownStatus.level}">${this.esc(shownStatus.text)}</div>
      ${moreProblems.map((p) => `<div class="ge-group-status is-warn">${this.esc(p)}</div>`).join("")}
      <div class="detail-hint">Режим исполнения выбирается в чате (тумблер действует на одну сессию) или полем <code>strategy:</code> в файле графа. Форму схлопывания проверяет движок — его вердикт показан выше.</div>
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">ID группы</div>
      <input type="text" id="ge-group-id" class="ge-input" value="${this.esc(group.id)}" />
      <div class="detail-hint">ID входит в YAML как ссылка из <code>nodes[].group</code>. С пробелами и спецсимволами — нет.</div>
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Название</div>
      <input type="text" id="ge-group-title" class="ge-input" value="${this.esc(group.title || "")}" />
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Инструкция FUSED</div>
      <textarea id="ge-group-fused" class="ge-input" rows="8"
        placeholder="Опиши одним куском то, что делает вся группа. В режиме Fused группа выполняется ОДНИМ вызовом LLM по этой инструкции.">${this.esc(group.fused_instruction || "")}</textarea>
      <div class="detail-hint">Пусто — группа работает только в режиме Detailed (рамка остаётся, схлопывания нет). Ссылки вида <code>{{ nodes.X.output }}</code> здесь запрещены: в FUSED эти ноды не выполняются по отдельности.</div>
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Агент-исполнитель</div>
      <input type="text" id="ge-group-agent" class="ge-input" value="${this.esc(group.fused_agent || "")}"
        placeholder="например neuro_healer" />
      <div class="detail-hint">Его <code>.md</code> даёт системный промпт и <code>mcp_servers</code>. Обязателен, если заданы инструменты.</div>
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Инструменты</div>
      <input type="text" id="ge-group-tools" class="ge-input" value="${this.esc(tools)}"
        placeholder="ReadSection, через запятую" />
      <div class="detail-hint">Схлопывание с инструментами пока не поддержано: в Fused такая группа даст явную ошибку и продолжит работать только в Detailed.</div>
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Рамка</div>
      <div class="detail-hint">${pinned
        ? "Размер задан автором: растяжение и перенос двигают только рамку, ноды внутри остаются на месте."
        : "Размер выводится из габаритов нод группы. Растяните рамку маркером — появится свободное место под новые ноды."}</div>
      <button id="ge-group-fit" class="btn-secondary" style="width:100%; margin-top:6px;"
        ${pinned ? "" : "disabled"}>📐 По размеру нод</button>
    </div>
    <div class="graph-detail-section">
      <div class="detail-label">Ноды группы (${memberIds.length})</div>
      <ul class="ge-group-members">${members}</ul>
      <button id="ge-group-adopt" class="btn-secondary" style="width:100%; margin-top:6px;"
        ${selectedCount > 0 ? "" : "disabled"}>
        📥 Перенести выделенные ноды в группу (выделено ${selectedCount})
      </button>
    </div>
    <div class="graph-detail-section">
      <button id="ge-group-delete" class="btn-secondary" style="width:100%;">🗑 Удалить группу (ноды останутся)</button>
    </div>`;

  const root = this.el.graphDetailContent;
  const field = <T extends Element>(id: string): T => root.querySelector<T>(`#${id}`) as T;

  const fitButton = field<HTMLButtonElement>("ge-group-fit");
  if (fitButton) {
    fitButton.addEventListener("click", () => {
      if (this.fitGroupToNodes(groupId)) {
        showToast("📐 Рамка снова по размеру нод", "success");
        this.showGroupEditor(groupId);
      }
    });
  }

  const adoptButton = field<HTMLButtonElement>("ge-group-adopt");
  if (adoptButton) {
    adoptButton.addEventListener("click", () => {
      const moved = this.adoptSelectedNodesIntoGroup(groupId);
      showToast(
        moved > 0 ? `📥 В группу перенесено нод: ${moved}` : "📥 Ничего не перенесено",
        moved > 0 ? "success" : "error",
      );
      this.showGroupEditor(groupId);
    });
  }

  const idInput = field<HTMLInputElement>("ge-group-id");
  if (idInput) {
    idInput.addEventListener("change", () => {
      const next = idInput.value.trim();
      if (!next || next === group.id || this.groupById(next)) {
        idInput.value = group.id;
        if (next && this.groupById(next)) {
          showToast(`❌ Группа «${next}» уже существует`, "error");
        }
        return;
      }
      this.renameGroup(groupId, next);
      this.showGroupEditor(next);
    });
  }

  const titleInput = field<HTMLInputElement>("ge-group-title");
  if (titleInput) {
    titleInput.addEventListener("input", () => {
      group.title = titleInput.value;
      this.refreshGroupFrames([groupId]);
    });
  }

  const fusedTextarea = field<HTMLTextAreaElement>("ge-group-fused");
  if (fusedTextarea) {
    fusedTextarea.addEventListener("change", () => {
      this.saveCheckpoint();
      const text = fusedTextarea.value;
      if (text.trim()) group.fused_instruction = text;
      else delete group.fused_instruction;
      this.renderGroups();
      this.showGroupEditor(groupId);
    });
  }

  const agentInput = field<HTMLInputElement>("ge-group-agent");
  if (agentInput) {
    agentInput.addEventListener("change", () => {
      this.saveCheckpoint();
      const text = agentInput.value.trim();
      if (text) group.fused_agent = text;
      else delete group.fused_agent;
    });
  }

  const toolsInput = field<HTMLInputElement>("ge-group-tools");
  if (toolsInput) {
    toolsInput.addEventListener("change", () => {
      this.saveCheckpoint();
      const list = toolsInput.value
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
      if (list.length > 0) group.tools = list;
      else delete group.tools;
      this.renderGroups();
      this.showGroupEditor(groupId);
    });
  }

  const delButton = field<HTMLButtonElement>("ge-group-delete");
  if (delButton) {
    delButton.addEventListener("click", () => {
      this.deleteGroup(groupId);
      this.hideSidebar();
    });
  }
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