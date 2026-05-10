import { renderMarkdown } from "../utils/markdown";

export type Role = 'user' | 'agent' | 'system';

export function createMessageElement(
  role: Role, 
  content: string, 
  agentName?: string, 
  timeText?: string
): HTMLDivElement {
  const msgDiv = document.createElement("div");
  msgDiv.className = `message message-${role}`;

  if (role === 'agent' && agentName) {
    const nameSpan = document.createElement("span");
    nameSpan.className = "agent-name";
    nameSpan.innerText = agentName;
    msgDiv.appendChild(nameSpan);
  }

  const contentDiv = document.createElement("div");
  contentDiv.innerHTML = role === 'system' ? content : renderMarkdown(content);
  msgDiv.appendChild(contentDiv);

  if (timeText && role === 'agent') {
    const timeDiv = document.createElement("div");
    timeDiv.className = "msg-time";
    timeDiv.innerText = timeText;
    msgDiv.appendChild(timeDiv);
  }

  return msgDiv;
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