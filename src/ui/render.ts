import { renderMarkdown } from "../utils";
import { createMessageMenu } from "./message-menu";
import type { MessageMenuCallbacks } from "./message-menu";
import { createThoughtRow, completeToolRow, TOOL_ICONS } from "./thought-row";
import { createImageAttachmentElement } from "./image-attachment";
import type { ImageAttachmentCallbacks } from "./image-attachment";
import { createVideoAttachmentElement } from "./video-attachment";
import { createFileAttachmentElement } from "./file-attachment";
import type { Attachment } from "../types";


export type Role = 'user' | 'agent' | 'system';

export function createMessageElement(
  role: Role, 
  content: string, 
  agentName?: string, 
  timeText?: string,
  msgUid?: string,
  menuCallbacks?: MessageMenuCallbacks,
  attachments?: Attachment[],
  translateLabel?: string,
  modelName?: string,
  imageCallbacks?: ImageAttachmentCallbacks
): HTMLDivElement {
  const msgDiv = document.createElement("div");
  msgDiv.className = `message message-${role}`;
  if (msgUid) msgDiv.dataset.msgUid = msgUid;

  if (role === 'agent' && agentName) {
    const nameSpan = document.createElement("span");
    nameSpan.className = "agent-name";
    nameSpan.textContent = agentName;
    msgDiv.appendChild(nameSpan);
  }

  const contentDiv = document.createElement("div");
  contentDiv.className = "msg-content";
  contentDiv.innerHTML = renderMarkdown(content);
  msgDiv.appendChild(contentDiv);

  if (attachments && attachments.length > 0) {
    const attDiv = document.createElement("div");
    attDiv.className = "msg-attachments";
    for (const att of attachments) {
      if (att.mime_type?.startsWith("image/")) {
        attDiv.appendChild(createImageAttachmentElement(att, imageCallbacks));
      } else if (att.mime_type?.startsWith("video/")) {
        attDiv.appendChild(createVideoAttachmentElement(att, imageCallbacks));
      } else {
        // Документы (.xlsx/.csv/.pdf и пр.) — карточка с проводником (файл
        // на диске) или сохранением (только base64).
        attDiv.appendChild(createFileAttachmentElement(att, imageCallbacks));
      }
    }
    msgDiv.appendChild(attDiv);
  }

  if (timeText && role === 'agent') {
    const timeDiv = document.createElement("div");
    timeDiv.className = "msg-time";
    if (modelName) {
      const modelSpan = document.createElement("span");
      modelSpan.className = "msg-model";
      modelSpan.innerText = modelName;
      timeDiv.appendChild(modelSpan);
    }
    const timeSpan = document.createElement("span");
    timeSpan.className = "msg-time-value";
    timeSpan.innerText = timeText;
    timeDiv.appendChild(timeSpan);
    msgDiv.appendChild(timeDiv);
  }

  if (msgUid && menuCallbacks && (role === 'user' || role === 'agent')) {
    const menu = createMessageMenu(msgUid, menuCallbacks, role === 'agent' ? translateLabel : undefined);
    // Угол под кнопку ⋮ вырезается float-распоркой: текст первой строки
    // обтекает её и физически не может попасть под кнопку (styles.css).
    const spacer = document.createElement("div");
    spacer.className = "msg-menu-spacer";
    contentDiv.prepend(spacer);
    msgDiv.appendChild(menu);
  }

  return msgDiv;
}

/** Мысль агента как компактная строка-действие (делегация в thought-row.ts —
 *  SSOT разметки). Полный текст доступен в панели деталей. */
export function createThoughtElement(agentName: string, thought: string, timeSec?: number): HTMLDivElement {
  return createThoughtRow({
    kind: "thought",
    author: agentName,
    summary: thought,
    detail: thought,
    meta: timeSec && timeSec > 0 ? `⏱${timeSec.toFixed(1)}с` : undefined,
  });
}

/** Отчёт сабагента как кликабельная строка-действие (деталей нет — chevron не нужен). */
export function createSubcallElement(call: any, onSubcallClick: (call: any) => void): HTMLDivElement {
  const row = createThoughtRow({
    kind: "subcall",
    label: `Отчет от сабагента: ${call.agent_name} (${call.time_sec.toFixed(1)} сек)`,
  });
  row.addEventListener("click", () => onSubcallClick(call));
  return row;
}

/** Компактная строка вызова инструмента (реалтайм и история).
 *  Событие-вызов создаёт строку со статусом ⏳, событие-результат
 *  дополняет её через completeToolRow. */
export function createToolThoughtElement(
  author: string,
  tool: string,
  args?: string,
  result?: string,
  ready = false
): HTMLDivElement {
  const row = createThoughtRow({
    kind: "tool",
    icon: TOOL_ICONS[tool] ?? "🔧",
    label: `${author} → ${tool}`,
    summary: args ?? "",
    detail: args,
    toolKey: `${author}:${tool}`,
    pending: result === undefined && !ready,
  });
  if (result !== undefined) completeToolRow(row, result);
  return row;
}

export function createToolCallElement(toolName: string, args: string, result: string): HTMLDivElement {  const div = document.createElement("div");
  div.className = "tool-call-block message";
  
  const header = document.createElement("div");
  header.className = "tool-call-header";
  header.innerText = `🔧 Использован инструмент: ${toolName}`;
  
  const argsDiv = document.createElement("div");
  argsDiv.className = "tool-call-args";
  argsDiv.innerText = `Аргументы: ${args}`;
  
  const resultDiv = document.createElement("div");
  resultDiv.className = "tool-call-result";
  resultDiv.innerText = result;
  
  div.appendChild(header);
  div.appendChild(argsDiv);
  div.appendChild(resultDiv);
  
  return div;
}