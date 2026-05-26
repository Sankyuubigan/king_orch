import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { createMessageElement, createSubcallElement, createToolCallElement, createThoughtElement, Role } from "./ui/render";
import { fetchSessions, loadSession, deleteSession, saveSession, renameSession, openSessionFolder } from "./api/sessions";
import { showToast } from "./ui/toast";
import { initConfirmDialog, confirmDialog } from "./ui/confirm";
import { createThoughtsBlock, addToThoughtsBlock } from "./ui/thoughts-block";
import { MessageMenuCallbacks } from "./ui/message-menu";
import mermaid from "mermaid";

mermaid.initialize({ startOnLoad: false, theme: "dark", securityLevel: "loose" });
async function renderMermaid() { try { await mermaid.run(); } catch(e) { console.error(e); } }

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const modelSelect = $<HTMLSelectElement>("model-select");
const agentSelect = $<HTMLSelectElement>("agent-select");
const chatHistory = $<HTMLDivElement>("chat-history");
const subchatHistory = $<HTMLDivElement>("subchat-history");
const chatInput = $<HTMLTextAreaElement>("chat-input");
const btnSend = $<HTMLButtonElement>("btn-send");
const btnStop = $<HTMLButtonElement>("btn-stop");
const chatFeedback = $<HTMLDivElement>("chat-feedback");
const progressBar = $<HTMLDivElement>("progress-bar");
const statusLabel = $<HTMLDivElement>("status-label");
const contextSlider = $<HTMLInputElement>("context-slider");
const contextValue = $<HTMLElement>("context-value");
const chkKvQuant = $<HTMLInputElement>("chk-kv-quant");
const btnAddModel = $<HTMLButtonElement>("btn-add-model");
const themeSelect = $<HTMLSelectElement>("theme-select");
const promptFormatSelect = $<HTMLSelectElement>("prompt-format-select");
const tempSlider = $<HTMLInputElement>("temp-slider"); const tempValue = $<HTMLElement>("temp-value");
const topkSlider = $<HTMLInputElement>("topk-slider"); const topkValue = $<HTMLElement>("topk-value");
const toppSlider = $<HTMLInputElement>("topp-slider"); const toppValue = $<HTMLElement>("topp-value");
const minpSlider = $<HTMLInputElement>("minp-slider"); const minpValue = $<HTMLElement>("minp-value");
const reppenSlider = $<HTMLInputElement>("reppen-slider"); const reppenValue = $<HTMLElement>("reppen-value");
const prespenSlider = $<HTMLInputElement>("prespen-slider"); const prespenValue = $<HTMLElement>("prespen-value");
const btnResetParams = $<HTMLButtonElement>("btn-reset-params");
const downloadModelSelect = $<HTMLSelectElement>("download-model-select");
const btnDownloadModel = $<HTMLButtonElement>("btn-download-model");
const downloadProgressContainer = $<HTMLDivElement>("download-progress-container");
const downloadProgressBar = $<HTMLDivElement>("download-progress-bar");
const downloadStatusLabel = $<HTMLDivElement>("download-status-label");
const tabChat = $<HTMLButtonElement>("tab-chat"); const tabSettings = $<HTMLButtonElement>("tab-settings"); const tabLogs = $<HTMLButtonElement>("tab-logs");
const viewChat = $<HTMLDivElement>("view-chat"); const viewSubchat = $<HTMLDivElement>("view-subchat");
const viewSettings = $<HTMLDivElement>("view-settings"); const viewLogs = $<HTMLDivElement>("view-logs");
const logView = $<HTMLTextAreaElement>("log-view"); const btnClearLogs = $<HTMLButtonElement>("btn-clear-logs");
const btnBackChat = $<HTMLButtonElement>("btn-back-chat"); const subchatTitle = $<HTMLSpanElement>("subchat-title");
const btnNewSession = $<HTMLButtonElement>("btn-new-session"); const sessionList = $<HTMLDivElement>("session-list");

let isProcessing = false;
let globalChatHistory: {role: string, content: string, sub_calls?: any[], agent_name?: string}[] = [];
let currentSessionId: string | null = null;
let globalDossier: Record<string, string> = {};
let modelsCatalog: any[] = [];
let draftTimeout: number | undefined;
let uidCounter = 0;
let msgUidList: string[] = [];
let activeThoughtsBlock: HTMLDivElement | null = null;
const realtimeSubcallKeys = new Set<string>();

function nextUid(): string { return `msg_${uidCounter++}`; }

export function logToGUI(msg: string) {
  if (logView) { logView.value += `[${new Date().toLocaleTimeString()}] ${msg}\n`; logView.scrollTop = logView.scrollHeight; }
}

// === Меню сообщений: удаление и клонирование ===
const menuCallbacks: MessageMenuCallbacks = {
  onDelete: async (uid: string) => {
    const idx = msgUidList.indexOf(uid);
    if (idx === -1) return;
    globalChatHistory.splice(idx, 1);
    msgUidList.splice(idx, 1);
    renderChatFromHistory();
    if (currentSessionId) await saveSession(currentSessionId, globalChatHistory, globalDossier, chatInput.value);
    showToast("Сообщение удалено.", "success");
  },
  onClone: async (uid: string) => {
    const idx = msgUidList.indexOf(uid);
    if (idx === -1) return;
    const clonedHistory = globalChatHistory.slice(0, idx + 1);
    const newId = Date.now().toString();
    await saveSession(newId, clonedHistory, { ...globalDossier }, "");
    showToast("Клон сессии создан!", "success");
    await loadSessionsListUI();
    await openSessionUI(newId);
  }
};

// === Рендеринг чата ===

function appendMessageToContainer(container: HTMLDivElement, role: Role, content: string, agentName?: string, timeText?: string) {
  container.appendChild(createMessageElement(role, content, agentName, timeText));
  container.scrollTop = container.scrollHeight;
  renderMermaid();
}

function appendMessage(role: Role, content: string, agentName?: string, timeText?: string, subCalls?: any[], skipSubcallRender = false, uid?: string) {
  activeThoughtsBlock = null;
  if (subCalls && subCalls.length > 0 && !skipSubcallRender) {
    const items = subCalls.map(call => createSubcallElement(call, showSubchat));
    chatHistory.appendChild(createThoughtsBlock(items));
  }
  const hasMenu = uid !== undefined && (role === 'user' || role === 'agent');
  const msgEl = createMessageElement(role, content, agentName, timeText, hasMenu ? uid : undefined, hasMenu ? menuCallbacks : undefined);
  chatHistory.appendChild(msgEl);
  chatHistory.scrollTop = chatHistory.scrollHeight;
  renderMermaid();
}

function renderChatFromHistory() {
  chatHistory.innerHTML = '';
  activeThoughtsBlock = null;
  let thoughtsItems: HTMLElement[] = [];
  for (let i = 0; i < globalChatHistory.length; i++) {
    const msg = globalChatHistory[i];
    const uid = msgUidList[i];
    if (msg.role === 'thought') { thoughtsItems.push(createThoughtElement(msg.agent_name || 'Агент', msg.content)); continue; }
    if (msg.role === 'assistant' && msg.sub_calls && msg.sub_calls.length > 0) {
      msg.sub_calls.forEach(call => { thoughtsItems.push(createSubcallElement(call, showSubchat)); });
    }
    if (thoughtsItems.length > 0) { chatHistory.appendChild(createThoughtsBlock(thoughtsItems)); thoughtsItems = []; }
    const role = (msg.role === 'assistant' ? 'agent' : msg.role) as Role;
    const agentName = msg.role === 'assistant' ? 'Агент' : undefined;
    const hasMenu = uid && (role === 'user' || role === 'agent');
    chatHistory.appendChild(createMessageElement(role, msg.content, agentName, undefined, hasMenu ? uid : undefined, hasMenu ? menuCallbacks : undefined));
  }
  if (thoughtsItems.length > 0) { chatHistory.appendChild(createThoughtsBlock(thoughtsItems)); }
  chatHistory.scrollTop = chatHistory.scrollHeight;
  renderMermaid();
}

function showSubchat(subCall: any) {
  viewChat.classList.remove('active'); viewSubchat.classList.add('active');
  subchatTitle.innerText = `Сабагент: ${subCall.agent_name}`;
  subchatHistory.innerHTML = '';
  appendMessageToContainer(subchatHistory, 'system', subCall.prompt, 'Отчет контекста сабагента');
  if (subCall.tool_calls) subCall.tool_calls.forEach((tc: any) => { subchatHistory.appendChild(createToolCallElement(tc.tool_name, tc.arguments, tc.result)); });
  appendMessageToContainer(subchatHistory, 'agent', subCall.response, subCall.agent_name, `${subCall.time_sec.toFixed(1)} сек`);
}

btnBackChat?.addEventListener("click", () => { viewSubchat.classList.remove('active'); viewChat.classList.add('active'); });

function switchTab(tab: 'chat' | 'settings' | 'logs') {
  tabChat.classList.toggle('active', tab === 'chat'); tabSettings.classList.toggle('active', tab === 'settings'); tabLogs.classList.toggle('active', tab === 'logs');
  viewChat.classList.toggle('active', tab === 'chat'); viewSubchat.classList.remove('active'); viewSettings.classList.toggle('active', tab === 'settings'); viewLogs.classList.toggle('active', tab === 'logs');
}
tabChat?.addEventListener("click", () => switchTab('chat'));
tabSettings?.addEventListener("click", () => switchTab('settings'));
tabLogs?.addEventListener("click", () => switchTab('logs'));
btnClearLogs?.addEventListener("click", () => { logView.value = ""; });
contextSlider?.addEventListener("input", () => { contextValue.innerText = contextSlider.value; });
themeSelect?.addEventListener("change", async () => { document.documentElement.setAttribute('data-theme', themeSelect.value); await invoke("set_theme", { theme: themeSelect.value }); });
promptFormatSelect?.addEventListener("change", async () => { await invoke("set_prompt_format", { format: promptFormatSelect.value }); });

// === Параметры модели ===
async function loadModelParams() {
  const p = modelSelect.value; if (!p) return;
  const params: any = await invoke("get_model_params", { modelPath: p });
  tempSlider.value = params.temperature; tempValue.innerText = params.temperature;
  topkSlider.value = params.top_k; topkValue.innerText = params.top_k;
  toppSlider.value = params.top_p; toppValue.innerText = params.top_p;
  minpSlider.value = params.min_p; minpValue.innerText = params.min_p;
  reppenSlider.value = params.repetition_penalty; reppenValue.innerText = params.repetition_penalty;
  prespenSlider.value = params.presence_penalty; prespenValue.innerText = params.presence_penalty;
}
async function saveModelParams() {
  const p = modelSelect.value; if (!p) return;
  await invoke("set_model_params", { modelPath: p, params: {
    temperature: parseFloat(tempSlider.value), top_k: parseInt(topkSlider.value, 10),
    top_p: parseFloat(toppSlider.value), min_p: parseFloat(minpSlider.value),
    repetition_penalty: parseFloat(reppenSlider.value), presence_penalty: parseFloat(prespenSlider.value)
  }});
}
[{ s: tempSlider, l: tempValue }, { s: topkSlider, l: topkValue }, { s: toppSlider, l: toppValue }, { s: minpSlider, l: minpValue }, { s: reppenSlider, l: reppenValue }, { s: prespenSlider, l: prespenValue }].forEach(({ s, l }) => { s?.addEventListener("input", () => { l.innerText = s.value; saveModelParams(); }); });
btnResetParams?.addEventListener("click", async () => { const p = modelSelect.value; if (!p) return; await invoke("reset_model_params", { modelPath: p }); await loadModelParams(); showToast("Параметры сброшены.", "success"); });

// === Скачивание моделей ===
async function loadCatalog() { try { modelsCatalog = await invoke("get_models_catalog"); downloadModelSelect.innerHTML = '<option value="">-- Выберите модель --</option>'; modelsCatalog.forEach(m => { const o = document.createElement("option"); o.value = m.name; o.text = m.name; downloadModelSelect.appendChild(o); }); } catch(e) { downloadModelSelect.innerHTML = '<option value="">Ошибка загрузки</option>'; } }
btnDownloadModel?.addEventListener("click", async () => {
  const name = downloadModelSelect.value; if (!name) return;
  const model = modelsCatalog.find(m => m.name === name); if (!model) return;
  try { const savePath = await save({ defaultPath: `${model.name}.gguf`, filters: [{ name: "GGUF", extensions: ["gguf"] }] }); if (!savePath) return; btnDownloadModel.disabled = true; downloadProgressContainer.style.display = "block"; await invoke("download_model", { url: model.download_url, savePath }); await invoke("add_model", { path: savePath }); await loadConfig(); showToast(`Модель ${model.name} скачана!`, "success"); } catch(e) { showToast(`Ошибка: ${e}`, "error"); logToGUI(`${e}`); } finally { btnDownloadModel.disabled = false; downloadProgressContainer.style.display = "none"; }
});
listen("download_progress", (e: any) => { const { downloaded, total } = e.payload; const pct = total > 0 ? (downloaded / total) * 100 : 0; downloadProgressBar.style.width = `${pct}%`; downloadStatusLabel.innerText = `${(downloaded/1024/1024).toFixed(1)} MB / ${(total/1024/1024).toFixed(1)} MB (${pct.toFixed(1)}%)`; });

// === Конфигурация ===
async function loadConfig() {
  logToGUI("Загрузка конфигурации...");
  try { const config: any = await invoke("get_config"); updateModelSelect(config); if (config.context_size) { contextSlider.value = config.context_size.toString(); contextValue.innerText = config.context_size.toString(); } if (config.kv_quantization !== undefined) chkKvQuant.checked = config.kv_quantization; if (config.theme) { themeSelect.value = config.theme; document.documentElement.setAttribute('data-theme', config.theme); } if (config.prompt_format) promptFormatSelect.value = config.prompt_format; await loadAgents(); await loadSessionsListUI(); await loadCatalog(); await loadModelParams(); } catch(e) { showToast(`Ошибка: ${e}`, "error"); logToGUI(`${e}`); }
}
async function loadAgents() { try { const agents: any[] = await invoke("get_agents"); agentSelect.innerHTML = ''; for (const a of agents) { if (!a.is_hidden) { const o = document.createElement("option"); o.value = `agent_${a.id}`; o.text = `📁 ${a.name} (${a.id})`; agentSelect.appendChild(o); } } const orch = Array.from(agentSelect.options).find(o => o.value === 'agent_therapist_communicator'); if (orch) agentSelect.value = orch.value; } catch(e) {} }

// === Сессии UI ===
document.addEventListener("click", (e) => { document.querySelectorAll('.session-menu-dropdown.show').forEach(dd => { if (!dd.parentElement?.contains(e.target as Node)) dd.classList.remove('show'); }); });

async function loadSessionsListUI() {
  try { const sessions = await fetchSessions(); sessionList.innerHTML = ""; for (const s of sessions) { const div = document.createElement("div"); div.className = `session-item ${s.id === currentSessionId ? 'active' : ''}`; div.innerHTML = `<span class="session-title" title="${s.title}">${s.title}</span><div class="session-item-actions"><button class="btn-session-menu">⋮</button><div class="session-menu-dropdown"><button class="session-menu-item btn-rename" data-id="${s.id}" data-title="${s.title}">✏️ Переименовать</button><button class="session-menu-item btn-explore" data-id="${s.id}">📁 Открыть в проводнике</button><button class="session-menu-item danger btn-delete" data-id="${s.id}">🗑️ Удалить</button></div></div>`; div.addEventListener("click", (e) => { if (!(e.target as HTMLElement).closest('.session-item-actions')) openSessionUI(s.id); }); const menuBtn = div.querySelector('.btn-session-menu'); const dropdown = div.querySelector('.session-menu-dropdown'); menuBtn?.addEventListener("click", (e) => { e.stopPropagation(); document.querySelectorAll('.session-menu-dropdown.show').forEach(dd => { if (dd !== dropdown) dd.classList.remove('show'); }); dropdown?.classList.toggle('show'); }); div.querySelector('.btn-rename')?.addEventListener("click", async (e) => { e.stopPropagation(); dropdown?.classList.remove('show'); const cur = (e.target as HTMLElement).getAttribute('data-title') || ''; const newT = prompt("Новое название:", cur); if (newT && newT.trim() !== "" && newT !== cur) { try { await renameSession(s.id, newT.trim()); loadSessionsListUI(); } catch(err) { showToast(`Ошибка: ${err}`, "error"); } } }); div.querySelector('.btn-explore')?.addEventListener("click", async (e) => { e.stopPropagation(); dropdown?.classList.remove('show'); try { await openSessionFolder(s.id); } catch(err) { showToast(`Ошибка: ${err}`, "error"); } }); div.querySelector('.btn-delete')?.addEventListener("click", async (e) => { e.stopPropagation(); dropdown?.classList.remove('show'); await deleteSessionUI(s.id); }); sessionList.appendChild(div); } } catch(e) {}
}

async function openSessionUI(id: string) {
  if (isProcessing) return;
  try { const session = await loadSession(id); currentSessionId = id; globalChatHistory = session.messages; globalDossier = session.dossier || {}; if (!globalDossier && session.state_markdown) globalDossier = { legacy_state: session.state_markdown }; chatInput.value = session.draft || ""; setTimeout(() => { chatInput.style.height = "auto"; chatInput.style.height = `${chatInput.scrollHeight}px`; }, 0); uidCounter = 0; msgUidList = globalChatHistory.map(() => nextUid()); renderChatFromHistory(); loadSessionsListUI(); switchTab('chat'); } catch(e) { showToast(`Ошибка: ${e}`, "error"); logToGUI(`${e}`); }
}

async function deleteSessionUI(id: string) {
  const yes = await confirmDialog("Удаление сессии", "Вы уверены, что хотите безвозвратно удалить эту сессию?"); if (!yes) return;
  try { await deleteSession(id); if (currentSessionId === id) startNewSession(); else loadSessionsListUI(); showToast("Сессия удалена.", "success"); } catch(e) { showToast(`Ошибка: ${e}`, "error"); }
}

function startNewSession() {
  if (isProcessing) return;
  currentSessionId = null; globalChatHistory = []; globalDossier = {}; realtimeSubcallKeys.clear(); uidCounter = 0; msgUidList = []; activeThoughtsBlock = null;
  chatHistory.innerHTML = ''; chatInput.value = ''; chatInput.style.height = "auto";
  appendMessage('system', 'Новая сессия начата. Выберите агента и напишите запрос.');
  loadSessionsListUI(); switchTab('chat');
}
btnNewSession?.addEventListener("click", startNewSession);

function updateModelSelect(config: any) { modelSelect.innerHTML = ""; for (const m of config.models) { const o = document.createElement("option"); o.value = m; o.text = m.split(/[/\\]/).pop() || m; modelSelect.appendChild(o); } if (config.last_model && config.models.includes(config.last_model)) modelSelect.value = config.last_model; }
modelSelect?.addEventListener("change", async () => { await invoke("set_last_model", { path: modelSelect.value }); await loadModelParams(); });
btnAddModel?.addEventListener("click", async () => { try { const sel = await open({ filters: [{ name: "Model", extensions: ["gguf"] }] }); if (sel) { const cfg: any = await invoke("add_model", { path: sel as string }); updateModelSelect(cfg); await loadModelParams(); } } catch(e) {} });

function setProcessingState(state: boolean) {
  isProcessing = state; modelSelect.disabled = agentSelect.disabled = chatInput.disabled = btnSend.disabled = btnNewSession.disabled = state; btnStop.disabled = !state;
  if (state) { chatFeedback.style.display = "block"; progressBar.style.width = "0%"; statusLabel.innerText = "Подготовка..."; realtimeSubcallKeys.clear(); } else { chatFeedback.style.display = "none"; }
}

chatInput.addEventListener("input", () => {
  chatInput.style.height = "auto"; chatInput.style.height = `${chatInput.scrollHeight}px`;
  if (isProcessing) return;
  if (!currentSessionId && chatInput.value.trim() !== "") { currentSessionId = Date.now().toString(); globalChatHistory = []; globalDossier = {}; saveSession(currentSessionId, globalChatHistory, globalDossier, chatInput.value).then(() => loadSessionsListUI()); }
  else if (currentSessionId) { clearTimeout(draftTimeout); draftTimeout = window.setTimeout(() => { saveSession(currentSessionId!, globalChatHistory, globalDossier, chatInput.value); }, 500); }
});
chatInput.addEventListener("blur", () => { if (currentSessionId && !isProcessing) { clearTimeout(draftTimeout); saveSession(currentSessionId, globalChatHistory, globalDossier, chatInput.value); } });

// === Отправка сообщения ===
async function handleSend() {
  const text = chatInput.value.trim(); if (!text || isProcessing) return;
  const activeAgent = agentSelect.value; const modelPath = modelSelect.value;
  if (!modelPath) { showToast("Выберите модель GGUF!", "error"); return; }
  const userUid = nextUid(); msgUidList.push(userUid);
  appendMessage('user', text, undefined, undefined, undefined, false, userUid);
  chatInput.value = ""; chatInput.style.height = "auto"; clearTimeout(draftTimeout);
  setProcessingState(true);
  if (!currentSessionId) currentSessionId = Date.now().toString();
  globalChatHistory.push({ role: "user", content: text });
  await saveSession(currentSessionId, globalChatHistory, globalDossier, ""); loadSessionsListUI();
  const startTime = performance.now();
  try {
    const displayName = agentSelect.options[agentSelect.selectedIndex].text.replace('📁 ', '');
    const historyToSend = globalChatHistory.filter(m => m.role !== 'thought').slice(0, -1);
    const params = { temperature: parseFloat(tempSlider.value), top_k: parseInt(topkSlider.value, 10), top_p: parseFloat(toppSlider.value), min_p: parseFloat(minpSlider.value), repetition_penalty: parseFloat(reppenSlider.value), presence_penalty: parseFloat(prespenSlider.value) };
    const response: any = await invoke("chat_request", { modelPath, agentId: activeAgent, message: text, history: historyToSend, contextSize: parseInt(contextSlider.value, 10), kvQuantization: chkKvQuant.checked, dossier: globalDossier, modelParams: params });
    const dur = ((performance.now() - startTime) / 1000).toFixed(1);
    globalDossier = response.dossier || {};
    globalChatHistory.push({ role: "assistant", content: response.text, sub_calls: response.sub_calls });
    const agentUid = nextUid(); msgUidList.push(agentUid);
    const hasRT = response.sub_calls && response.sub_calls.some((c: any) => realtimeSubcallKeys.has(`${c.agent_name}:${c.time_sec.toFixed(2)}`));
    appendMessage('agent', response.text, displayName, `⏱ Время: ${dur} сек.`, response.sub_calls, hasRT, agentUid);
    await saveSession(currentSessionId, globalChatHistory, globalDossier, chatInput.value);
  } catch (error) {
    if (String(error).includes("Отменено") || String(error).includes("Прервано")) appendMessage('system', '⚠️ Обработка прервана.');
    else { showToast(`Ошибка: ${error}`, "error"); logToGUI(`${error}`); }
  } finally { setProcessingState(false); }
}
btnSend?.addEventListener("click", handleSend);
chatInput?.addEventListener("keydown", (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); handleSend(); } });
btnStop?.addEventListener("click", async () => { if (btnStop.disabled) return; btnStop.disabled = true; btnStop.innerText = "Останавливаю..."; appendMessage('system', 'Отправлен сигнал остановки...'); await invoke("stop_processing"); btnStop.innerText = "⏹ Стоп"; });

// === События ===
listen("progress", (e) => { progressBar.style.width = `${e.payload}%`; });
listen("status", (e) => { statusLabel.innerText = e.payload as string; });
listen("log", (e) => { logToGUI(e.payload as string); });

listen("subcall_done", (e) => {
  const call = e.payload as any;
  realtimeSubcallKeys.add(`${call.agent_name}:${call.time_sec.toFixed(2)}`);
  const item = createSubcallElement(call, showSubchat);
  if (activeThoughtsBlock) { addToThoughtsBlock(activeThoughtsBlock, item); }
  else { activeThoughtsBlock = createThoughtsBlock([item]); chatHistory.appendChild(activeThoughtsBlock); }
  chatHistory.scrollTop = chatHistory.scrollHeight;
});

listen("agent_thought", (e) => {
  const payload = e.payload as {agent_name: string, thought: string};
  const item = createThoughtElement(payload.agent_name, payload.thought);
  if (activeThoughtsBlock) { addToThoughtsBlock(activeThoughtsBlock, item); }
  else { activeThoughtsBlock = createThoughtsBlock([item]); chatHistory.appendChild(activeThoughtsBlock); }
  chatHistory.scrollTop = chatHistory.scrollHeight;
  globalChatHistory.push({ role: "thought", content: payload.thought, agent_name: payload.agent_name });
  msgUidList.push(nextUid());
  if (currentSessionId) saveSession(currentSessionId, globalChatHistory, globalDossier, chatInput.value);
});

initConfirmDialog();
document.addEventListener("DOMContentLoaded", loadConfig);