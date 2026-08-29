import { renderMarkdown } from "../utils";
import { createMessageMenu } from "./message-menu";
import type { MessageMenuCallbacks } from "./message-menu";
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
  translateLabel?: string
): HTMLDivElement {
  const msgDiv = document.createElement("div");
  msgDiv.className = `message message-${role}`;
  if (msgUid) msgDiv.dataset.msgUid = msgUid;

  if (role === 'agent' && agentName) {
    const nameSpan = document.createElement("span");
    nameSpan.className = "agent-name";
    nameSpan.innerText = agentName;
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
        const img = document.createElement("img");
        img.className = "msg-attachment-img";
        img.src = `data:${att.mime_type};base64,${att.data_base64}`;
        img.alt = att.file_name || "вложение";
        img.title = att.file_name || "вложение";
        attDiv.appendChild(img);
      } else {
        const chip = document.createElement("div");
        chip.className = "msg-attachment-file";
        chip.textContent = `📎 ${att.file_name || "файл"}`;
        attDiv.appendChild(chip);
      }
    }
    msgDiv.appendChild(attDiv);
  }

  if (timeText && role === 'agent') {
    const timeDiv = document.createElement("div");
    timeDiv.className = "msg-time";
    timeDiv.innerText = timeText;
    msgDiv.appendChild(timeDiv);
  }

  if (msgUid && menuCallbacks && (role === 'user' || role === 'agent')) {
    const menu = createMessageMenu(msgUid, menuCallbacks, role === 'agent' ? translateLabel : undefined);
    msgDiv.appendChild(menu);
  }

  return msgDiv;
}

export function createThoughtElement(agentName: string, thought: string, timeSec?: number): HTMLDivElement {
  const div = document.createElement("div");
  div.className = "message message-thought";
  if (timeSec && timeSec > 0) {
    div.innerHTML = `🧠 <strong>${agentName}</strong> <span class="thought-time">⏱${timeSec.toFixed(1)}с</span>: <em>${thought}</em>`;
  } else {
    div.innerHTML = `🧠 <strong>${agentName}</strong>: <em>${thought}</em>`;
  }
  return div;
}

export function createSubcallElement(call: any, onSubcallClick: (call: any) => void): HTMLDivElement {
  const callDiv = document.createElement("div");
  callDiv.className = "message message-system subcall-msg";
  
  const btn = document.createElement("button");
  btn.className = "btn-subcall";
  btn.innerText = `📊 Отчет от сабагента: ${call.agent_name} (${call.time_sec.toFixed(1)} сек)`;
  btn.onclick = () => onSubcallClick(call);
  
  callDiv.appendChild(btn);
  return callDiv;
}

/** Компактный элемент вызова инструмента для блока мыслей (реалтайм и история).
 *  Событие-вызов создаёт элемент со статусом ⏳, событие-результат дополняет его. */
export function createToolThoughtElement(
  author: string,
  tool: string,
  args?: string,
  result?: string,
  ready = false
): HTMLDivElement {
  const div = document.createElement("div");
  div.className = "message message-thought tool-thought";
  div.dataset.toolKey = `${author}:${tool}`;

  const text = document.createElement("span");
  text.textContent = `🔧 ${author} → ${tool}`;
  div.appendChild(text);

  if (args !== undefined) {
    const argsDiv = document.createElement("div");
    argsDiv.className = "tool-thought-args";
    argsDiv.textContent = args;
    div.appendChild(argsDiv);
  }

  if (result !== undefined) {
    const resultDiv = document.createElement("div");
    resultDiv.className = "tool-thought-result";
    resultDiv.textContent = `→ ${result}`;
    div.appendChild(resultDiv);
  } else if (!ready) {
    const status = document.createElement("span");
    status.className = "tool-thought-status";
    status.textContent = " ⏳";
    div.appendChild(status);
  }
  return div;
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