import { renderMarkdown } from "../utils";
import { createMessageMenu } from "./message-menu";
import type { MessageMenuCallbacks } from "./message-menu";
import type { ThoughtMenuCallbacks } from "../types";

export type Role = 'user' | 'agent' | 'system';

export function createMessageElement(
  role: Role, 
  content: string, 
  agentName?: string, 
  timeText?: string,
  msgUid?: string,
  menuCallbacks?: MessageMenuCallbacks
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
  contentDiv.innerHTML = renderMarkdown(content);
  msgDiv.appendChild(contentDiv);

  if (timeText && role === 'agent') {
    const timeDiv = document.createElement("div");
    timeDiv.className = "msg-time";
    timeDiv.innerText = timeText;
    msgDiv.appendChild(timeDiv);
  }

  if (msgUid && menuCallbacks && (role === 'user' || role === 'agent')) {
    const menu = createMessageMenu(msgUid, menuCallbacks);
    msgDiv.appendChild(menu);
  }

  return msgDiv;
}

export function createThoughtElement(agentName: string, thought: string): HTMLDivElement {
  const div = document.createElement("div");
  div.className = "message message-thought";
  div.innerHTML = `🧠 <strong>${agentName}</strong>: <em>${thought}</em>`;
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

export function createToolCallElement(toolName: string, args: string, result: string): HTMLDivElement {
  const div = document.createElement("div");
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