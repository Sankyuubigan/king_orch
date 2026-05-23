import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { createMessageElement, createSubcallElement, createToolCallElement, createThoughtElement, Role } from "./ui/render";
import { fetchSessions, loadSession, deleteSession, saveSession, renameSession, openSessionFolder } from "./api/sessions";
import { showToast } from "./ui/toast";
import { initConfirmDialog, confirmDialog } from "./ui/confirm";
import mermaid from "mermaid";

mermaid.initialize({ startOnLoad: false, theme: "dark", securityLevel: "loose" });

async function renderMermaidDiagrams() {
  try { await mermaid.run(); } catch (e) { console.error("Mermaid error:", e); }
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
const chkKvQuant = document.getElementById("chk-kv-quant") as HTMLInputElement;
const btnAddModel = document.getElementById("btn-add-model") as HTMLButtonElement;
const themeSelect = document.getElementById("theme-select") as HTMLSelectElement;
const promptFormatSelect = document.getElementById("prompt-format-select") as HTMLSelectElement;

const tempSlider = document.getElementById("temp-slider") as HTMLInputElement;
const tempValue = document.getElementById("temp-value") as HTMLElement;
const topkSlider = document.getElementById("topk-slider") as HTMLInputElement;
const topkValue = document.getElementById("topk-value") as HTMLElement;
const toppSlider = document.getElementById("topp-slider") as HTMLInputElement;
const toppValue = document.getElementById("topp-value") as HTMLElement;
const minpSlider = document.getElementById("minp-slider") as HTMLInputElement;
const minpValue = document.getElementById("minp-value") as HTMLElement;
const reppenSlider = document.getElementById("reppen-slider") as HTMLInputElement;
const reppenValue = document.getElementById("reppen-value") as HTMLElement;
const prespenSlider = document.getElementById("prespen-slider") as HTMLInputElement;
const prespenValue = document.getElementById("prespen-value") as HTMLElement;
const btnResetParams = document.getElementById("btn-reset-params") as HTMLButtonElement;

const downloadModelSelect = document.getElementById("download-model-select") as HTMLSelectElement;
const btnDownloadModel = document.getElementById("btn-download-model") as HTMLButtonElement;
const downloadProgressContainer = document.getElementById("download-progress-container") as HTMLDivElement;
const downloadProgressBar = document.getElementById("download-progress-bar") as HTMLDivElement;
const downloadStatusLabel = document.getElementById("download-status-label") as HTMLDivElement;

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
let globalChatHistory: {role: string, content: string, sub_calls?: any[], agent_name?: string}[] = [];
let currentSessionId: string | null = null;
let globalDossier: Record<string, string> = {};
let modelsCatalog: any[] = [];
let draftTimeout: number | undefined;

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
    subCalls.forEach(call => { chatHistory.appendChild(createSubcallElement(call, showSubchat)); });
  }
  appendMessageToContainer(chatHistory, role, content, agentName, timeText);
}

function showSubchat(subCall: any) {
  viewChat.classList.remove('active');
  viewSubchat.classList.add('active');
  subchatTitle.innerText = `Сабагент: ${subCall.agent_name}`;
  subchatHistory.innerHTML = '';
  appendMessageToContainer(subchatHistory, 'system', subCall.prompt, 'Отчет контекста сабагента');
  if (subCall.tool_calls) {
    subCall.tool_calls.forEach((tc: any) => { subchatHistory.appendChild(createToolCallElement(tc.tool_name, tc.arguments, tc.result)); });
  }
  appendMessageToContainer(subchatHistory, 'agent', subCall.response, subCall.agent_name, `${subCall.time_sec.toFixed(1)} сек`);
}

btnBackChat?.addEventListener("click", () => { viewSubchat.classList.remove('active'); viewChat.classList.add('active'); });

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

themeSelect?.addEventListener("change", async () => {
  document.documentElement.setAttribute('data-theme', themeSelect.value);
  await invoke("set_theme", { theme: themeSelect.value });
});

promptFormatSelect?.addEventListener("change", async () => {
  await invoke("set_prompt_format", { format: promptFormatSelect.value });
});

async function loadModelParams() {
  const modelPath = modelSelect.value;
  if (!modelPath) return;
  const params: any = await invoke("get_model_params", { modelPath });
  tempSlider.value = params.temperature; tempValue.innerText = params.temperature;
  topkSlider.value = params.top_k; topkValue.innerText = params.top_k;
  toppSlider.value = params.top_p; toppValue.innerText = params.top_p;
  minpSlider.value = params.min_p; minpValue.innerText = params.min_p;
  reppenSlider.value = params.repetition_penalty; reppenValue.innerText = params.repetition_penalty;
  prespenSlider.value = params.presence_penalty; prespenValue.innerText = params.presence_penalty;
}

async function saveModelParams() {
  const modelPath = modelSelect.value;
  if (!modelPath) return;
  const params = {
    temperature: parseFloat(tempSlider.value),
    top_k: parseInt(topkSlider.value, 10),
    top_p: parseFloat(toppSlider.value),
    min_p: parseFloat(minpSlider.value),
    repetition_penalty: parseFloat(reppenSlider.value),
    presence_penalty: parseFloat(prespenSlider.value)
  };
  await invoke("set_model_params", { modelPath, params });
}

[
  { slider: tempSlider, label: tempValue },
  { slider: topkSlider, label: topkValue },
  { slider: toppSlider, label: toppValue },
  { slider: minpSlider, label: minpValue },
  { slider: reppenSlider, label: reppenValue },
  { slider: prespenSlider, label: prespenValue }
].forEach(item => {
  item.slider.addEventListener("input", () => {
    item.label.innerText = item.slider.value;
    saveModelParams();
  });
});

btnResetParams?.addEventListener("click", async () => {
  const modelPath = modelSelect.value;
  if (!modelPath) return;
  await invoke("reset_model_params", { modelPath });
  await loadModelParams();
  showToast("Параметры сброшены на значения по умолчанию.", "success");
});

async function loadCatalog() {
  try {
    modelsCatalog = await invoke("get_models_catalog");
    downloadModelSelect.innerHTML = '<option value="">-- Выберите модель --</option>';
    modelsCatalog.forEach(m => {
      const opt = document.createElement("option");
      opt.value = m.name; opt.text = m.name;
      downloadModelSelect.appendChild(opt);
    });
  } catch (e) {
    downloadModelSelect.innerHTML = '<option value="">Ошибка загрузки каталога</option>';
  }
}

btnDownloadModel?.addEventListener("click", async () => {
  const selectedName = downloadModelSelect.value;
  if (!selectedName) return;
  const model = modelsCatalog.find(m => m.name === selectedName);
  if (!model) return;
  try {
    const savePath = await save({ defaultPath: `${model.name}.gguf`, filters: [{ name: "GGUF", extensions: ["gguf"] }] });
    if (!savePath) return;
    btnDownloadModel.disabled = true;
    downloadProgressContainer.style.display = "block";
    await invoke("download_model", { url: model.download_url, savePath });
    await invoke("add_model", { path: savePath });
    await loadConfig();
    showToast(`Модель ${model.name} успешно скачана!`, "success");
  } catch (e) {
    const errMsg = `Ошибка скачивания: ${e}`;
    showToast(errMsg, "error");
    logToGUI(errMsg);
  } finally {
    btnDownloadModel.disabled = false;
    downloadProgressContainer.style.display = "none";
  }
});

listen("download_progress", (e: any) => {
  const { downloaded, total } = e.payload;
  const mbDown = (downloaded / 1024 / 1024).toFixed(1);
  const mbTotal = (total / 1024 / 1024).toFixed(1);
  const percent = total > 0 ? (downloaded / total) * 100 : 0;
  downloadProgressBar.style.width = `${percent}%`;
  downloadStatusLabel.innerText = `${mbDown} MB / ${mbTotal} MB (${percent.toFixed(1)}%)`;
});

async function loadConfig() {
  logToGUI("Загрузка конфигурации...");
  try {
    const config: any = await invoke("get_config");
    updateModelSelect(config);
    if (config.context_size) { contextSlider.value = config.context_size.toString(); contextValue.innerText = config.context_size.toString(); }
    if (config.kv_quantization !== undefined) chkKvQuant.checked = config.kv_quantization;
    if (config.theme) { themeSelect.value = config.theme; document.documentElement.setAttribute('data-theme', config.theme); }
    if (config.prompt_format) promptFormatSelect.value = config.prompt_format;
    await loadAgents();
    await loadSessionsListUI();
    await loadCatalog();
    await loadModelParams();
  } catch (e) {
    const errMsg = `Ошибка загрузки конфига: ${e}`;
    showToast(errMsg, "error");
    logToGUI(errMsg);
  }
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
    if (orchOption) agentSelect.value = orchOption.value;
  } catch (e) {}
}

document.addEventListener("click", (e) => {
  const dropdowns = document.querySelectorAll('.session-menu-dropdown.show');
  dropdowns.forEach(dd => {
    if (!dd.parentElement?.contains(e.target as Node)) dd.classList.remove('show');
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

      div.addEventListener("click", (e) => {
        if ((e.target as HTMLElement).closest('.session-item-actions')) return;
        openSessionUI(s.id);
      });
      
      const menuBtn = div.querySelector('.btn-session-menu');
      const dropdown = div.querySelector('.session-menu-dropdown');
      
      menuBtn?.addEventListener("click", (e) => {
        e.stopPropagation();
        document.querySelectorAll('.session-menu-dropdown.show').forEach(dd => { if (dd !== dropdown) dd.classList.remove('show'); });
        dropdown?.classList.toggle('show');
      });

      div.querySelector('.btn-rename')?.addEventListener("click", async (e) => {
        e.stopPropagation(); e.preventDefault();
        dropdown?.classList.remove('show');
        const currentTitle = (e.target as HTMLElement).getAttribute('data-title') || '';
        const newTitle = prompt("Введите новое название сессии:", currentTitle);
        if (newTitle && newTitle.trim() !== "" && newTitle !== currentTitle) {
          try {
            await renameSession(s.id, newTitle.trim());
            loadSessionsListUI();
          } catch(err) {
            const msg = `Ошибка переименования: ${err}`;
            showToast(msg, "error");
            logToGUI(msg);
          }
        }
      });

      div.querySelector('.btn-explore')?.addEventListener("click", async (e) => {
        e.stopPropagation(); e.preventDefault();
        dropdown?.classList.remove('show');
        try {
          await openSessionFolder(s.id);
        } catch(err) {
          const msg = `Ошибка открытия папки: ${err}`;
          showToast(msg, "error");
          logToGUI(msg);
        }
      });

      div.querySelector('.btn-delete')?.addEventListener("click", async (e) => {
        e.stopPropagation(); e.preventDefault();
        dropdown?.classList.remove('show');
        await deleteSessionUI(s.id);
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
    // Загрузка dossier (Record<string, string>), с фоллбэком для старых сессий
    globalDossier = session.dossier || {};
    if (!globalDossier && session.state_markdown) {
      globalDossier = { legacy_state: session.state_markdown };
    }
    chatInput.value = session.draft || ""; 
    setTimeout(() => {
      chatInput.style.height = "auto";
      chatInput.style.height = `${chatInput.scrollHeight}px`;
    }, 0);
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
  } catch(e) {
    const msg = `Ошибка загрузки сессии: ${e}`;
    showToast(msg, "error");
    logToGUI(msg);
  }
}

async function deleteSessionUI(id: string) {
  const yes = await confirmDialog(
    "Удаление сессии",
    "Вы уверены, что хотите безвозвратно удалить эту сессию?"
  );
  if (!yes) return;
  try {
    await deleteSession(id);
    if (currentSessionId === id) startNewSession(); else loadSessionsListUI();
    showToast("Сессия удалена.", "success");
  } catch(e) {
    const msg = `Ошибка при удалении: ${e}`;
    showToast(msg, "error");
    logToGUI(msg);
  }
}

function startNewSession() {
  if (isProcessing) return;
  currentSessionId = null;
  globalChatHistory = [];
  globalDossier = {};
  chatHistory.innerHTML = '';
  chatInput.value = ''; 
  chatInput.style.height = "auto";
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

modelSelect?.addEventListener("change", async () => { 
  await invoke("set_last_model", { path: modelSelect.value }); 
  await loadModelParams();
});

btnAddModel?.addEventListener("click", async () => {
  try {
    const selected = await open({ filters: [{ name: "Model", extensions:["gguf"] }] });
    if (selected) {
      const config: any = await invoke("add_model", { path: selected as string });
      updateModelSelect(config);
      await loadModelParams();
    }
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

chatInput.addEventListener("input", () => {
  chatInput.style.height = "auto";
  chatInput.style.height = `${chatInput.scrollHeight}px`;
  if (isProcessing) return;
  if (!currentSessionId && chatInput.value.trim() !== "") {
    currentSessionId = Date.now().toString();
    globalChatHistory = [];
    globalDossier = {};
    saveSession(currentSessionId, globalChatHistory, globalDossier, chatInput.value).then(() => {
      loadSessionsListUI();
    });
  } else if (currentSessionId) {
    clearTimeout(draftTimeout);
    draftTimeout = window.setTimeout(() => {
      saveSession(currentSessionId!, globalChatHistory, globalDossier, chatInput.value);
    }, 500);
  }
});

chatInput.addEventListener("blur", () => {
  if (currentSessionId && !isProcessing) {
    clearTimeout(draftTimeout);
    saveSession(currentSessionId, globalChatHistory, globalDossier, chatInput.value);
  }
});

async function handleSend() {
  const text = chatInput.value.trim();
  if (!text || isProcessing) return;
  const activeAgent = agentSelect.value;
  const modelPath = modelSelect.value;
  if (!modelPath) {
    showToast("Выберите модель GGUF в выпадающем списке!", "error");
    return;
  }

  appendMessage('user', text);
  chatInput.value = "";
  chatInput.style.height = "auto";
  clearTimeout(draftTimeout);
  setProcessingState(true);

  if (!currentSessionId) currentSessionId = Date.now().toString();
  globalChatHistory.push({ role: "user", content: text });
  await saveSession(currentSessionId, globalChatHistory, globalDossier, "");
  loadSessionsListUI();

  const startTime = performance.now();

  try {
    let displayName = agentSelect.options[agentSelect.selectedIndex].text.replace('📁 ', '');
    const historyToSend = globalChatHistory.filter(m => m.role !== 'thought').slice(0, -1);
    const params = {
      temperature: parseFloat(tempSlider.value),
      top_k: parseInt(topkSlider.value, 10),
      top_p: parseFloat(toppSlider.value),
      min_p: parseFloat(minpSlider.value),
      repetition_penalty: parseFloat(reppenSlider.value),
      presence_penalty: parseFloat(prespenSlider.value)
    };

    const response: any = await invoke("chat_request", { 
      modelPath, agentId: activeAgent, message: text, history: historyToSend, 
      contextSize: parseInt(contextSlider.value, 10), kvQuantization: chkKvQuant.checked,
      dossier: globalDossier, modelParams: params
    });
    const durationSec = ((performance.now() - startTime) / 1000).toFixed(1);
    
    globalDossier = response.dossier || {};
    globalChatHistory.push({ role: "assistant", content: response.text, sub_calls: response.sub_calls });
    appendMessage('agent', response.text, displayName, `⏱ Время: ${durationSec} сек.`);
    await saveSession(currentSessionId, globalChatHistory, globalDossier, chatInput.value);
  } catch (error) {
    if (String(error).includes("Отменено") || String(error).includes("Прервано")) {
      appendMessage('system', '⚠️ Обработка прервана.');
    } else {
      const errMsg = `Ошибка: ${error}`;
      showToast(errMsg, "error");
      logToGUI(errMsg);
    }
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
  globalChatHistory.push({ role: "thought", content: payload.thought, agent_name: payload.agent_name });
  if (currentSessionId) saveSession(currentSessionId, globalChatHistory, globalDossier, chatInput.value);
});

initConfirmDialog();
document.addEventListener("DOMContentLoaded", loadConfig);