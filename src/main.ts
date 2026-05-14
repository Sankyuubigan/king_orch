import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { createMessageElement, createSubcallElement, createToolCallElement, createThoughtElement, Role } from "./ui/render";
import { fetchSessions, loadSession, deleteSession, saveSession, renameSession, openSessionFolder } from "./api/sessions";
import mermaid from "mermaid";

mermaid.initialize({
  startOnLoad: false,
  theme: "dark",
  securityLevel: "loose",
});

async function renderMermaidDiagrams() {
  try {
    await mermaid.run();
  } catch (e) {
    console.error("Mermaid render error:", e);
  }
}

const modelSelect = document.getElementById("model-select") as HTMLSelectElement;
const agentSelect = document.getElementById("agent-select") as HTMLSelectElement;
const chatHistory = document.getElementById("chat-history") as HTMLDivElement;
const subchatHistory = document.getElementById("subchat-history") as HTMLDivElement;
const chatInput = document.getElementById("chat-input") as HTMLTextAreaElement;
const btnSend = document.getElementById("btn-send") as HTMLButtonElement;
const btnStop = document.getElementById("btn-stop") as HTMLButtonElement;
const chatFeedback = document.getElementById("chat-feedback") as HTMLDivElement;
const progressBar = document.getElementById("progress-bar") as HTMLDivElement;
const statusLabel = document.getElementById("status-label") as HTMLDivElement;

const contextSlider = document.getElementById("context-slider") as HTMLInputElement;
const contextValue = document.getElementById("context-value") as HTMLElement;
const confSlider = document.getElementById("conf-slider") as HTMLInputElement;
const confValue = document.getElementById("conf-value") as HTMLElement;
const chkKvQuant = document.getElementById("chk-kv-quant") as HTMLInputElement;
const btnAddModel = document.getElementById("btn-add-model") as HTMLButtonElement;
const themeSelect = document.getElementById("theme-select") as HTMLSelectElement;
const promptFormatSelect = document.getElementById("prompt-format-select") as HTMLSelectElement;

const tabChat = document.getElementById("tab-chat") as HTMLButtonElement;
const tabSettings = document.getElementById("tab-settings") as HTMLButtonElement;
const tabLogs = document.getElementById("tab-logs") as HTMLButtonElement;
const viewChat = document.getElementById("view-chat") as HTMLDivElement;
const viewSubchat = document.getElementById("view-subchat") as HTMLDivElement;
const viewSettings = document.getElementById("view-settings") as HTMLDivElement;
const viewLogs = document.getElementById("view-logs") as HTMLDivElement;
const logView = document.getElementById("log-view") as HTMLTextAreaElement;
const btnClearLogs = document.getElementById("btn-clear-logs") as HTMLButtonElement;
const btnBackChat = document.getElementById("btn-back-chat") as HTMLButtonElement;
const subchatTitle = document.getElementById("subchat-title") as HTMLSpanElement;

const btnNewSession = document.getElementById("btn-new-session") as HTMLButtonElement;
const sessionList = document.getElementById("session-list") as HTMLDivElement;

let isProcessing = false;
let globalChatHistory: {role: string, content: string, sub_calls?: any[], agent_name?: string}[] =[];
let currentSessionId: string | null = null;
let globalStateMarkdown: string = ""; // Глобальное состояние сессии

export function logToGUI(msg: string) {
  if (logView) {
    logView.value += `[${new Date().toLocaleTimeString()}] ${msg}\n`;
    logView.scrollTop = logView.scrollHeight;
  }
}

function appendMessageToContainer(container: HTMLDivElement, role: Role, content: string, agentName?: string, timeText?: string) {
  const msgEl = createMessageElement(role, content, agentName, timeText);
  container.appendChild(msgEl);
  container.scrollTop = container.scrollHeight;
  renderMermaidDiagrams();
}

function appendMessage(role: Role, content: string, agentName?: string, timeText?: string, subCalls?: any[]) {
  if (subCalls && subCalls.length > 0) {
    subCalls.forEach(call => {
      chatHistory.appendChild(createSubcallElement(call, showSubchat));
    });
  }
  appendMessageToContainer(chatHistory, role, content, agentName, timeText);
}

function showSubchat(subCall: any) {
  viewChat.classList.remove('active');
  viewSubchat.classList.add('active');
  subchatTitle.innerText = `Сабагент: ${subCall.agent_name}`;
  subchatHistory.innerHTML = '';
  
  appendMessageToContainer(subchatHistory, 'system', subCall.prompt, 'Отчет контекста сабагента');
  
  if (subCall.tool_calls && subCall.tool_calls.length > 0) {
    subCall.tool_calls.forEach((tc: any) => {
      subchatHistory.appendChild(createToolCallElement(tc.tool_name, tc.arguments, tc.result));
    });
  }

  appendMessageToContainer(subchatHistory, 'agent', subCall.response, subCall.agent_name, `${subCall.time_sec.toFixed(1)} сек`);
}

btnBackChat?.addEventListener("click", () => {
  viewSubchat.classList.remove('active');
  viewChat.classList.add('active');
});

function switchTab(tab: 'chat' | 'settings' | 'logs') {
  tabChat.classList.toggle('active', tab === 'chat');
  tabSettings.classList.toggle('active', tab === 'settings');
  tabLogs.classList.toggle('active', tab === 'logs');

  viewChat.classList.toggle('active', tab === 'chat');
  viewSubchat.classList.remove('active');
  viewSettings.classList.toggle('active', tab === 'settings');
  viewLogs.classList.toggle('active', tab === 'logs');
}

tabChat?.addEventListener("click", () => switchTab('chat'));
tabSettings?.addEventListener("click", () => switchTab('settings'));
tabLogs?.addEventListener("click", () => switchTab('logs'));
btnClearLogs?.addEventListener("click", () => { logView.value = ""; });

contextSlider?.addEventListener("input", () => { contextValue.innerText = contextSlider.value; });
confSlider?.addEventListener("input", async () => {
  confValue.innerText = confSlider.value;
  await invoke("set_config_value", { key: "confidence_threshold", value: parseFloat(confSlider.value) });
});

themeSelect?.addEventListener("change", async () => {
  document.documentElement.setAttribute('data-theme', themeSelect.value);
  await invoke("set_theme", { theme: themeSelect.value });
});

promptFormatSelect?.addEventListener("change", async () => {
  await invoke("set_prompt_format", { format: promptFormatSelect.value });
});

async function loadConfig() {
  logToGUI("Загрузка конфигурации...");
  try {
    const config: any = await invoke("get_config");
    updateModelSelect(config);
    
    if (config.context_size) { contextSlider.value = config.context_size.toString(); contextValue.innerText = config.context_size.toString(); }
    if (config.confidence_threshold !== undefined) { confSlider.value = config.confidence_threshold.toString(); confValue.innerText = config.confidence_threshold.toString(); }
    if (config.kv_quantization !== undefined) chkKvQuant.checked = config.kv_quantization;
    if (config.theme) { themeSelect.value = config.theme; document.documentElement.setAttribute('data-theme', config.theme); }
    if (config.prompt_format) promptFormatSelect.value = config.prompt_format;
    
    await loadAgents();
    await loadSessionsListUI();
  } catch (e) { logToGUI(`Ошибка загрузки конфига: ${e}`); }
}

async function loadAgents() {
  try {
    const agents: any[] = await invoke("get_agents");
    agentSelect.innerHTML = '';
    for (const agent of agents) {
      if (!agent.is_hidden) {
        const option = document.createElement("option");
        option.value = `agent_${agent.id}`;
        option.text = `📁 ${agent.name} (${agent.id})`;
        agentSelect.appendChild(option);
      }
    }
    
    const orchOption = Array.from(agentSelect.options).find(opt => opt.value === 'agent_therapist_communicator');
    if (orchOption) {
      agentSelect.value = orchOption.value;
    }
  } catch (e) {}
}

// Закрытие дропдаунов при клике вне их
document.addEventListener("click", (e) => {
  const dropdowns = document.querySelectorAll('.session-menu-dropdown.show');
  dropdowns.forEach(dd => {
    if (!dd.parentElement?.contains(e.target as Node)) {
      dd.classList.remove('show');
    }
  });
});

async function loadSessionsListUI() {
  try {
    const sessions = await fetchSessions();
    sessionList.innerHTML = "";
    for (const s of sessions) {
      const div = document.createElement("div");
      div.className = `session-item ${s.id === currentSessionId ? 'active' : ''}`;
      
      div.innerHTML = `
        <span class="session-title" title="${s.title}">${s.title}</span>
        <div class="session-item-actions">
          <button class="btn-session-menu">⋮</button>
          <div class="session-menu-dropdown">
            <button class="session-menu-item btn-rename" data-id="${s.id}" data-title="${s.title}">✏️ Переименовать</button>
            <button class="session-menu-item btn-explore" data-id="${s.id}">📁 Открыть в проводнике</button>
            <button class="session-menu-item danger btn-delete" data-id="${s.id}">🗑️ Удалить</button>
          </div>
        </div>
      `;
      
      div.querySelector('.session-title')?.addEventListener("click", () => openSessionUI(s.id));
      
      const menuBtn = div.querySelector('.btn-session-menu');
      const dropdown = div.querySelector('.session-menu-dropdown');
      
      menuBtn?.addEventListener("click", (e) => {
        e.stopPropagation();
        // Скрываем все остальные
        document.querySelectorAll('.session-menu-dropdown.show').forEach(dd => {
          if (dd !== dropdown) dd.classList.remove('show');
        });
        dropdown?.classList.toggle('show');
      });

      div.querySelector('.btn-rename')?.addEventListener("click", async (e) => {
        e.stopPropagation();
        dropdown?.classList.remove('show');
        const currentTitle = (e.target as HTMLElement).getAttribute('data-title') || '';
        const newTitle = prompt("Введите новое название сессии:", currentTitle);
        if (newTitle && newTitle.trim() !== "" && newTitle !== currentTitle) {
          await renameSession(s.id, newTitle.trim());
          loadSessionsListUI();
        }
      });

      div.querySelector('.btn-explore')?.addEventListener("click", async (e) => {
        e.stopPropagation();
        dropdown?.classList.remove('show');
        await openSessionFolder(s.id);
      });

      div.querySelector('.btn-delete')?.addEventListener("click", async (e) => {
        e.stopPropagation();
        dropdown?.classList.remove('show');
        deleteSessionUI(s.id);
      });
      
      sessionList.appendChild(div);
    }
  } catch (e) {}
}

async function openSessionUI(id: string) {
  if (isProcessing) return;
  try {
    const session = await loadSession(id);
    currentSessionId = id;
    globalChatHistory = session.messages;
    globalStateMarkdown = session.state_markdown || ""; // Загружаем состояние
    
    chatHistory.innerHTML = '';
    appendMessage('system', 'Сессия загружена.');
    for (const msg of globalChatHistory) {
      if (msg.role === 'thought') {
        chatHistory.appendChild(createThoughtElement(msg.agent_name || 'Агент', msg.content));
      } else {
        const role = msg.role === 'assistant' ? 'agent' : msg.role;
        appendMessage(role as Role, msg.content, msg.role === 'assistant' ? 'Агент' : undefined, undefined, msg.sub_calls);
      }
    }
    loadSessionsListUI();
    switchTab('chat');
  } catch(e) {}
}

async function deleteSessionUI(id: string) {
  if (confirm("Вы уверены, что хотите безвозвратно удалить эту сессию?")) {
    try {
      await deleteSession(id);
      if (currentSessionId === id) startNewSession(); else loadSessionsListUI();
    } catch(e) {}
  }
}

function startNewSession() {
  if (isProcessing) return;
  currentSessionId = null;
  globalChatHistory =[];
  globalStateMarkdown = ""; // Сбрасываем состояние
  chatHistory.innerHTML = '';
  appendMessage('system', 'Новая сессия начата. Выберите агента и напишите запрос.');
  loadSessionsListUI();
  switchTab('chat');
}

btnNewSession?.addEventListener("click", startNewSession);

function updateModelSelect(config: any) {
  modelSelect.innerHTML = "";
  for (const model of config.models) {
    const option = document.createElement("option");
    option.value = model; option.text = model.split(/[/\\]/).pop() || model;
    modelSelect.appendChild(option);
  }
  if (config.last_model && config.models.includes(config.last_model)) modelSelect.value = config.last_model;
}

modelSelect?.addEventListener("change", async () => { await invoke("set_last_model", { path: modelSelect.value }); });

btnAddModel?.addEventListener("click", async () => {
  try {
    const selected = await open({ filters: [{ name: "Model", extensions:["gguf"] }] });
    if (selected) updateModelSelect(await invoke("add_model", { path: selected as string }));
  } catch (error) {}
});

function setProcessingState(state: boolean) {
  isProcessing = state;
  modelSelect.disabled = agentSelect.disabled = chatInput.disabled = btnSend.disabled = btnNewSession.disabled = state;
  btnStop.disabled = !state;
  if (state) {
    chatFeedback.style.display = "block"; progressBar.style.width = "0%"; statusLabel.innerText = "Подготовка...";
  } else { chatFeedback.style.display = "none"; }
}

agentSelect?.addEventListener("change", () => {
  appendMessage('system', `Вы переключились на: ${agentSelect.options[agentSelect.selectedIndex].text}.`);
});

async function handleSend() {
  const text = chatInput.value.trim();
  if (!text || isProcessing) return;
  const activeAgent = agentSelect.value;
  const modelPath = modelSelect.value;
  if (!modelPath) return appendMessage('system', 'Ошибка: Выберите модель GGUF!');

  appendMessage('user', text);
  chatInput.value = "";
  setProcessingState(true);

  if (!currentSessionId) currentSessionId = Date.now().toString();
  globalChatHistory.push({ role: "user", content: text });
  await saveSession(currentSessionId, globalChatHistory, globalStateMarkdown);
  loadSessionsListUI();

  const startTime = performance.now();

  try {
    let displayName = agentSelect.options[agentSelect.selectedIndex].text.replace('📁 ', '');
    
    // Мы фильтруем "мысли" из истории перед отправкой в LLM, чтобы не засорять контекст
    const historyToSend = globalChatHistory
      .filter(m => m.role !== 'thought')
      .slice(0, -1);

    const response: any = await invoke("chat_request", { 
      modelPath, 
      agentId: activeAgent, 
      message: text, 
      history: historyToSend, 
      contextSize: parseInt(contextSlider.value, 10), 
      kvQuantization: chkKvQuant.checked,
      currentState: globalStateMarkdown
    });
    const durationSec = ((performance.now() - startTime) / 1000).toFixed(1);
    
    globalStateMarkdown = response.new_state;

    globalChatHistory.push({ role: "assistant", content: response.text, sub_calls: response.sub_calls });
    appendMessage('agent', response.text, displayName, `⏱ Время: ${durationSec} сек.`);
    
    await saveSession(currentSessionId, globalChatHistory, globalStateMarkdown);
  } catch (error) {
    appendMessage('system', String(error).includes("Отменено") ? '⚠️ Обработка прервана.' : `❌ Ошибка: ${error}`);
  } finally { setProcessingState(false); }
}

btnSend?.addEventListener("click", handleSend);
chatInput?.addEventListener("keydown", (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); handleSend(); } });

btnStop?.addEventListener("click", async () => {
  if (btnStop.disabled) return;
  btnStop.disabled = true; btnStop.innerText = "Останавливаю...";
  appendMessage('system', 'Отправлен сигнал остановки...');
  await invoke("stop_processing");
  btnStop.innerText = "⏹ Стоп";
});

listen("progress", (e) => { progressBar.style.width = `${e.payload}%`; });
listen("status", (e) => { statusLabel.innerText = e.payload as string; });
listen("log", (e) => { logToGUI(e.payload as string); });

listen("subcall_done", (e) => {
  const call = e.payload as any;
  chatHistory.appendChild(createSubcallElement(call, showSubchat));
  chatHistory.scrollTop = chatHistory.scrollHeight;
});

listen("agent_thought", (e) => {
  const payload = e.payload as {agent_name: string, thought: string};
  const el = createThoughtElement(payload.agent_name, payload.thought);
  chatHistory.appendChild(el);
  chatHistory.scrollTop = chatHistory.scrollHeight;
  
  // Сохраняем мысль в историю, чтобы она осталась при перезагрузке сессии
  globalChatHistory.push({ 
    role: "thought", 
    content: payload.thought, 
    agent_name: payload.agent_name 
  });
  if (currentSessionId) {
    saveSession(currentSessionId, globalChatHistory, globalStateMarkdown);
  }
});

document.addEventListener("DOMContentLoaded", loadConfig);