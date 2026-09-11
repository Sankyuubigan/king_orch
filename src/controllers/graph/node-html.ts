import { NODE_LABELS, isDynamicNode } from "./constants";
import type { GraphController } from "./graph-class";

export function buildNodeInnerHtml(this: GraphController, data: any, fallbackId: string = ""): string {
  const typeLabel = NODE_LABELS[data.type] || data.type;
  let agentLine = "";
  if (data.type === "llm_worker" && data.agent) {
    agentLine = `<div class="gn-agent">→ ${this.esc(data.agent)}</div>`;
  }
  if (data.type === "llm_worker" && data.output_type) {
    agentLine += `<div class="gn-agent">${this.esc(data.output_type)}</div>`;
  }
  if (data.type === "llm_worker" && data.inject_reports && data.inject_reports.length > 0) {
    agentLine += `<div class="gn-agent" style="color:#2196f3">📎 Отчеты: ${data.inject_reports.length}</div>`;
  }
  let signalLine = "";
  if (data.type === "signal_router") {
    const parts: string[] = [];
    if (data.signal_name) parts.push(this.esc(data.signal_name));
    if (data.field) parts.push(`.${this.esc(data.field)}`);
    if (parts.length > 0) {
      signalLine = `<div class="gn-agent" style="color:#ff9800">📡 ${parts.join("")}</div>`;
    }
  }
  if (data.type === "condition_check") {
    const parts: string[] = [];
    if (data.field) parts.push(this.esc(data.field));
    if (data.input_object) parts.push(`← ${this.esc(data.input_object)}`);
    if (data.sequential_to) parts.push(`→ ${this.esc(data.sequential_to)}`);
    if (data.true_to) parts.push(`✓ ${this.esc(data.true_to)}`);
    if (data.false_to) parts.push(`✗ ${this.esc(data.false_to)}`);
    signalLine = `<div class="gn-agent" style="color:#26a69a">🎯 ${parts.join(" | ")}</div>`;
  }
  if (data.type === "condition_router") {
    const parts: string[] = [];
    if (data.signal_name) parts.push(this.esc(data.signal_name));
    if (data.logic) parts.push(`logic: ${this.esc(data.logic)}`);
    if (data.true_to) parts.push(`✓ ${this.esc(data.true_to)}`);
    if (data.false_to) parts.push(`✗ ${this.esc(data.false_to)}`);
    signalLine = `<div class="gn-agent" style="color:#5c6bc0">🔀 ${parts.join(" | ")}</div>`;
  }
  let casesLine = "";
  if (isDynamicNode(data.type)) {
    const labels = this.getSwitchCaseKeys(data);
    if (labels.length > 0) {
      casesLine = `<div class="gn-cases">` + labels.map((key, i) =>
        `<div class="gn-case-row">
          <span class="gn-case-idx">${i + 1}</span>
          <span class="gn-case-key">${this.esc(key)}</span>
          <span class="gn-case-dot"></span>
        </div>`
      ).join("") + `</div>`;
    }
  }
  let factsLine = "";
  if (data.type === "llm_fact_extractor") {
    const facts = this.currentWorkflowConfig?.facts as Array<{ id: string }> | undefined;
    if (facts && facts.length > 0) {
      factsLine = `<div class="gn-cases">` + facts.map((f, i) =>
        `<div class="gn-case-row">
          <span class="gn-case-idx">${i + 1}</span>
          <span class="gn-case-key">${this.esc(f.id)}</span>
          <span class="gn-case-dot"></span>
        </div>`
      ).join("") + `</div>`;
    }
  }
  if (data.type === "note") {
    const sysBadge = data.system_message
      ? `<div class="gn-sysmsg-badge">→ чат: ${this.esc((data.system_message || "").slice(0, 40))}${(data.system_message || "").length > 40 ? "…" : ""}</div>`
      : "";
    return `<div class="gn-note-content">${this.esc(data.input || "")}</div>${sysBadge}`;
  }

  const disabledBadge = data.disabled ? `<div class="gn-disabled-badge">⛔ DISABLED</div>` : "";
  return `
    <div class="gn-title">${this.esc(data.id || fallbackId)}</div>
    <div class="gn-type">${typeLabel}</div>
    ${disabledBadge}
    ${agentLine}
    ${signalLine}
    ${casesLine}
    ${factsLine}
  `;
}

export function esc(this: GraphController, s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}