import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { store } from "../store";
import { bus } from "../events";
import { createMessageElement, createSubcallElement, createToolCallElement, createThoughtElement, createThoughtsBlock, addToThoughtsBlock, showToast } from "../ui";
import type { Role, MessageMenuCallbacks } from "../ui";
import type { ThoughtMenuCallbacks, Attachment, CatalogEntry } from "../types";
import { saveSession, loadSession, countTokens } from "../services";
import { renderMarkdown } from "../utils";
import mermaid from "mermaid";

mermaid.initialize({ startOnLoad: false, theme: "dark", securityLevel: "loose" });
async function renderMermaid() {
  try {
    await mermaid.run();
  } catch(e) {
    console.error('Mermaid render error:', e);
  }
  document.querySelectorAll('pre.mermaid').forEach(el => {
    if (!el.querySelector('svg')) {
      const code = el.textContent || '';
      el.outerHTML = `<pre style="background:#1e1e1e;color:#ccc;padding:12px;border-radius:6px;overflow-x:auto;white-space:pre-wrap;font-family:monospace">${code.replace(/</g, '&lt;').replace(/>/g, '&gt;')}</pre>`;
    }
  });
}

export interface ChatElements {
  chatHistory: HTMLDivElement;
  chatInput: HTMLTextAreaElement;
  btnSend: HTMLButtonElement;
  btnStop: HTMLButtonElement;
  chatFeedback: HTMLDivElement;
  progressBar: HTMLDivElement;
  statusLabel: HTMLDivElement;
  agentSelect: HTMLSelectElement;
  modelSelect: HTMLSelectElement;
  subchatHistory: HTMLDivElement;
  subchatTitle: HTMLSpanElement;
  btnBackChat: HTMLButtonElement;
  logView: HTMLTextAreaElement;
  contextSlider: HTMLInputElement;
  maxGenSlider: HTMLInputElement;
  chkKvQuantK: HTMLInputElement;
  chkKvQuantV: HTMLInputElement;
  tempSlider: HTMLInputElement;
  topkSlider: HTMLInputElement;
  toppSlider: HTMLInputElement;
  minpSlider: HTMLInputElement;
  reppenSlider: HTMLInputElement;
  prespenSlider: HTMLInputElement;
  viewChat: HTMLDivElement;
  viewSubchat: HTMLDivElement;
  btnAttach: HTMLButtonElement;
  fileInput: HTMLInputElement;
  filePreview: HTMLDivElement;
  tokenCounter: HTMLDivElement;
}

export class ChatController {
  private el: ChatElements;
  private menuCallbacks: MessageMenuCallbacks;
  private thoughtMenuCallbacks: ThoughtMenuCallbacks;
  private attachments: Attachment[] = [];
  private countTimer: number | null = null;

  constructor(el: ChatElements) {
    this.el = el;
    this.menuCallbacks = {
      onDelete: (uid) => this.onDeleteMessage(uid),
      onClone: (uid) => this.onCloneMessage(uid),
      onRunFrom: (uid) => this.onRunFromMessage(uid),
      onCopy: (uid) => this.onCopyMessage(uid),
    };
    this.thoughtMenuCallbacks = {
      onDeleteThoughts: (uid) => this.onDeleteThoughts(uid),
      onCloneFromThoughts: (uid) => this.onCloneFromThoughts(uid),
    };
    this.bindDomEvents();
    this.bindTauriEvents();
    this.bindBusEvents();
    setTimeout(() => {
      this.updateAttachButtonState();
      this.triggerTokenCount();
    }, 100);
  }

  private scrollToBottomIfNearEnd(el: HTMLElement) {
    const threshold = 100;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < threshold;
    if (atBottom) {
      el.scrollTop = el.scrollHeight;
    }
  }

  logToGUI(msg: string) {
    if (this.el.logView) {
      this.el.logView.value += `[${new Date().toLocaleTimeString()}] ${msg}\n`;
      this.scrollToBottomIfNearEnd(this.el.logView);
    }
  }

  private triggerTokenCount() {
    if (this.countTimer) clearTimeout(this.countTimer);
    this.countTimer = window.setTimeout(() => this.updateTokenCounter(), 400);
  }

  private async updateTokenCounter() {
    if (store.isProcessing) return;
    const modelPath = this.el.modelSelect?.value;
    const agentId = this.el.agentSelect?.value;
    const text = this.el.chatInput?.value || "";
    const contextSize = parseInt(this.el.contextSlider?.value || "24576", 10);

    if (!modelPath || !agentId) return;

    let hfModelId = "Xenova/Meta-Llama-3-8B-Instruct";
    const catalogModel = store.modelsCatalog.find((m: CatalogEntry) => modelPath.includes(m.name) || m.name === modelPath);
    if (catalogModel && catalogModel.hf_model_id) {
        hfModelId = catalogModel.hf_model_id;
    }

    try {
        const promptText: string = await invoke("get_prompt_preview", {
            modelPath, 
            agentId, 
            message: text, 
            history: store.chatHistory
        });
        
        const tokens = await countTokens(promptText, hfModelId);

        if (this.el.tokenCounter) {
            this.el.tokenCounter.innerText = `${tokens} / ${contextSize}`;
            this.el.tokenCounter.classList.remove("warning", "danger");
            if (tokens > contextSize) {
                this.el.tokenCounter.classList.add("danger");
                this.el.tokenCounter.title = "Внимание: Лимит превышен! При отправке старые сообщения будут удалены из контекста.";
            } else if (tokens > contextSize * 0.8) {
                this.el.tokenCounter.classList.add("warning");
                this.el.tokenCounter.title = "Контекст заполняется. Близко к лимиту.";
            } else {
                this.el.tokenCounter.title = "Токены: Текущие / Лимит контекста";
            }
        }
    } catch (e) {
        console.error("Ошибка обновления счетчика токенов:", e);
    }
  }

  private async onDeleteMessage(uid: string) {
    const idx = store.msgUidList.indexOf(uid); if (idx === -1) return;
    store.chatHistory.splice(idx, 1); store.msgUidList.splice(idx, 1);
    this.renderChatFromHistory();
    if (store.currentSessionId) await saveSession(store.currentSessionId, store.chatHistory, this.el.chatInput.value);
    this.triggerTokenCount();
    showToast("Сообщение удалено.", "success");
  }

  private async onCloneMessage(uid: string) {
    const idx = store.msgUidList.indexOf(uid); if (idx === -1) return;
    const clonedHistory = store.chatHistory.slice(0, idx + 1);
    const newId = Date.now().toString();
    await saveSession(newId, clonedHistory, "");
    showToast("Клон сессии создан!", "success");
    bus.emit("session:changed");
    bus.emit("session:open", newId);
  }

  private async onCopyMessage(uid: string) {
    const idx = store.msgUidList.indexOf(uid);
    if (idx === -1) return;
    const msg = store.chatHistory[idx];
    try {
      await navigator.clipboard.writeText(msg.content);
      showToast("Сообщение скопировано в буфер обмена", "success");
    } catch (err) {
      showToast(`Ошибка копирования: ${err}`, "error");
    }
  }

  private async onRunFromMessage(uid: string) {
    if (store.isProcessing) return;
    const idx = store.msgUidList.indexOf(uid);
    if (idx === -1) return;

    const activeAgent = this.el.agentSelect.value;
    const modelPath = this.el.modelSelect.value;
    if (!modelPath) { showToast("Выберите модель!", "error"); return; }

    this.logToGUI(`Нажата кнопка 'Отправить и запустить' для сообщения: ${uid}`);

    store.chatHistory = store.chatHistory.slice(0, idx + 1);
    store.msgUidList = store.msgUidList.slice(0, idx + 1);
    this.renderChatFromHistory();

    this.setProcessingState(true);

    if (!store.currentSessionId) store.currentSessionId = Date.now().toString();
    await saveSession(store.currentSessionId, store.chatHistory, this.el.chatInput.value);
    bus.emit("session:changed");

    const preSendLength = store.chatHistory.length;
    const startTime = performance.now();

    try {
      const params = { temperature: parseFloat(this.el.tempSlider.value), top_k: parseInt(this.el.topkSlider.value, 10), top_p: parseFloat(this.el.toppSlider.value), min_p: parseFloat(this.el.minpSlider.value), repetition_penalty: parseFloat(this.el.reppenSlider.value), presence_penalty: parseFloat(this.el.prespenSlider.value) };
      const allHistory = store.chatHistory.slice();

      let mmprojPath: string | null = null;
      try { mmprojPath = await invoke("get_mmproj_path", { modelPath }); } catch (_) {}

      const response: any = await invoke("chat_request", {
        modelPath,
        agentId: activeAgent,
        message: "",
        history: allHistory,
        contextSize: parseInt(this.el.contextSlider.value, 10),
        maxGenTokens: parseInt(this.el.maxGenSlider.value, 10),
        kvQuantKeys: this.el.chkKvQuantK.checked,
        kvQuantValues: this.el.chkKvQuantV.checked,
        modelParams: params,
        attachments: [],
        mmprojPath
      });

      const dur = ((performance.now() - startTime) / 1000);
      const newMessages = response.messages || [];
      const thisRoundThoughts = store.chatHistory.slice(preSendLength).filter((m: any) => m.type === 'thought');

      if (newMessages.length > 0) {
        const oldHistory = newMessages.slice(0, preSendLength);
        const afterOldHistory = newMessages.slice(preSendLength);
        
        afterOldHistory.forEach((m: any) => {
            if (m.author && m.author !== 'user' && m.author !== 'system') {
                m.time_sec = dur;
            }
        });
        
        const oldUids = store.msgUidList.slice(0, preSendLength);
        const thoughtUids = store.msgUidList.slice(preSendLength, preSendLength + thisRoundThoughts.length);
        const remainingMessages = afterOldHistory.slice(thisRoundThoughts.length);
        const newUids = remainingMessages.map(() => store.nextUid());

        store.chatHistory = [...oldHistory, ...thisRoundThoughts, ...remainingMessages];
        store.msgUidList = [...oldUids, ...thoughtUids, ...newUids];
      }

      this.renderChatFromHistory();
      await saveSession(store.currentSessionId, store.chatHistory, this.el.chatInput.value);
      this.triggerTokenCount();

    } catch (error) {
      if (String(error).includes("Отменено") || String(error).includes("Прервано")) {
        store.chatHistory.push({ type: "message", author: "system", content: "⚠️ Прервано." });
        store.msgUidList.push(store.nextUid());
      } else {
        showToast(`Ошибка: ${error}`, "error");
        store.chatHistory.push({ type: "message", author: "system", content: `⚠️ Ошибка: ${error}` });
        store.msgUidList.push(store.nextUid());
      }
      this.renderChatFromHistory();
      if (store.currentSessionId) {
        await saveSession(store.currentSessionId, store.chatHistory, this.el.chatInput.value);
      }
    } finally {
      this.setProcessingState(false);
    }
  }

  private async onDeleteThoughts(assistantUid: string) {
    const idx = store.msgUidList.indexOf(assistantUid); if (idx === -1) return;
    if (store.chatHistory[idx].sub_calls) store.chatHistory[idx].sub_calls = [];
    let i = idx - 1;
    while (i >= 0 && store.chatHistory[i].type === 'thought') { store.chatHistory.splice(i, 1); store.msgUidList.splice(i, 1); i--; }
    this.renderChatFromHistory();
    if (store.currentSessionId) await saveSession(store.currentSessionId, store.chatHistory, this.el.chatInput.value);
    showToast("Блок мыслей удален.", "success");
  }

  private async onCloneFromThoughts(assistantUid: string) {
    const idx = store.msgUidList.indexOf(assistantUid); if (idx === -1) return;
    let first = idx; let i = idx - 1;
    while (i >= 0 && store.chatHistory[i].type === 'thought') { first = i; i--; }
    const cloneIdx = first - 1; if (cloneIdx < 0) { showToast("Нельзя клонировать.", "error"); return; }
    const clonedHistory = store.chatHistory.slice(0, cloneIdx + 1);
    const newId = Date.now().toString();
    await saveSession(newId, clonedHistory, "");
    showToast("Клон сессии создан!", "success");
    bus.emit("session:changed");
    bus.emit("session:open", newId);
  }

  private appendMessageToContainer(container: HTMLDivElement, role: Role, content: string, agentName?: string, timeText?: string) {
    container.appendChild(createMessageElement(role, content, agentName, timeText));
    this.scrollToBottomIfNearEnd(container); renderMermaid();
  }

  appendMessage(role: Role, content: string, agentName?: string, timeText?: string, subCalls?: any[], skipSubcallRender = false, uid?: string) {
    store.activeThoughtsBlock = null;
    if (subCalls && subCalls.length > 0 && !skipSubcallRender) {
      const items = subCalls.map(call => createSubcallElement(call, (c) => this.showSubchat(c)));
      this.el.chatHistory.appendChild(createThoughtsBlock(items, uid, this.thoughtMenuCallbacks));
    }
    const hasMenu = uid !== undefined && (role === 'user' || role === 'agent');
    const msgEl = createMessageElement(role, content, agentName, timeText, hasMenu ? uid : undefined, hasMenu ? this.menuCallbacks : undefined);
    this.el.chatHistory.appendChild(msgEl); this.scrollToBottomIfNearEnd(this.el.chatHistory); renderMermaid();
  }

  renderChatFromHistory() {
    this.el.chatHistory.innerHTML = ''; store.activeThoughtsBlock = null;
    let thoughtsItems: HTMLElement[] = []; let lastAssistantUid: string | undefined;
    for (let i = 0; i < store.chatHistory.length; i++) {
      const msg = store.chatHistory[i]; const uid = store.msgUidList[i];
      if (msg.type === 'thought' || msg.type === 'signal') {
        const content = msg.type === 'signal' ? `[Сигнал] ${msg.content}` : msg.content;
        thoughtsItems.push(createThoughtElement(msg.author || 'Система', content, msg.time_sec));

        if (msg.sub_calls && msg.sub_calls.length > 0) {
          msg.sub_calls.forEach((call: any) => thoughtsItems.push(createSubcallElement(call, (c) => this.showSubchat(c))));
        }
        continue;
      }
      if (msg.type === 'message' && msg.author && msg.author !== 'user' && msg.author !== 'system' && msg.sub_calls && msg.sub_calls.length > 0) {
        lastAssistantUid = uid;
        msg.sub_calls.forEach(call => thoughtsItems.push(createSubcallElement(call, (c) => this.showSubchat(c))));
      }
      if (thoughtsItems.length > 0) {
        this.el.chatHistory.appendChild(createThoughtsBlock(thoughtsItems, lastAssistantUid, this.thoughtMenuCallbacks));
        thoughtsItems = []; lastAssistantUid = undefined;
      }
      const role = (msg.author && msg.author !== 'user') ? (msg.author === 'system' ? 'system' : 'agent') : 'user' as Role;
      const agentName = (msg.author && msg.author !== 'user' && msg.author !== 'system') ? msg.author : undefined;
      const hasMenu = uid && (role === 'user' || role === 'agent');
      const timeText = msg.time_sec ? `${msg.time_sec.toFixed(1)} сек` : undefined;
      this.el.chatHistory.appendChild(createMessageElement(role, msg.content, agentName, timeText, hasMenu ? uid : undefined, hasMenu ? this.menuCallbacks : undefined));
    }
    if (thoughtsItems.length > 0) this.el.chatHistory.appendChild(createThoughtsBlock(thoughtsItems, lastAssistantUid, this.thoughtMenuCallbacks));
    this.scrollToBottomIfNearEnd(this.el.chatHistory); renderMermaid();
  }

  private showSubchat(subCall: any) {
    this.el.viewChat.classList.remove('active'); this.el.viewSubchat.classList.add('active');
    this.el.subchatTitle.innerText = `Сабагент: ${subCall.agent_name}`;
    this.el.subchatHistory.innerHTML = '';
    this.appendMessageToContainer(this.el.subchatHistory, 'system', subCall.prompt, 'Отчет контекста');
    if (subCall.tool_calls) subCall.tool_calls.forEach((tc: any) => this.el.subchatHistory.appendChild(createToolCallElement(tc.tool_name, tc.arguments, tc.result)));
    this.appendMessageToContainer(this.el.subchatHistory, 'agent', subCall.response, subCall.agent_name, `${subCall.time_sec.toFixed(1)} сек`);
  }

  setProcessingState(state: boolean) {
    store.isProcessing = state;
    this.el.modelSelect.disabled = this.el.agentSelect.disabled = this.el.chatInput.disabled = this.el.btnSend.disabled = state;
    this.el.btnStop.disabled = !state;
    if (state) { 
        this.el.chatFeedback.style.display = "block"; 
        this.el.progressBar.style.width = "0%"; 
        this.el.statusLabel.innerText = "Подготовка..."; 
        store.realtimeSubcallKeys.clear(); 
        
        store.rtStreamUid = null;
        store.rtStreamBuffer = "";
        store.rtIsJson = false;
    }
    else { this.el.chatFeedback.style.display = "none"; }
    bus.emit("processing:changed", state);
  }

  async handleSend() {
    const text = this.el.chatInput.value.trim(); if (!text && this.attachments.length === 0) return; if (store.isProcessing) return;
    const activeAgent = this.el.agentSelect.value; const modelPath = this.el.modelSelect.value;
    if (!modelPath) { showToast("Выберите модель!", "error"); return; }
    const userUid = store.nextUid(); store.msgUidList.push(userUid);
    const displayText = text || (this.attachments.length > 0 ? `[${this.attachments.length} файлов]` : '');
    this.appendMessage('user', displayText, undefined, undefined, undefined, false, userUid);
    this.el.chatInput.value = ""; this.el.chatInput.style.height = "auto"; clearTimeout(store.draftTimeout);
    this.el.filePreview.innerHTML = '';
    const attachments = [...this.attachments];
    this.attachments = [];
    this.setProcessingState(true);
    if (!store.currentSessionId) store.currentSessionId = Date.now().toString();
    const preSendLength = store.chatHistory.length;
    store.chatHistory.push({ type: "message", author: "user", content: displayText });
    await saveSession(store.currentSessionId, store.chatHistory, ""); bus.emit("session:changed");
    const startTime = performance.now();
    try {
      const displayName = this.el.agentSelect.options[this.el.agentSelect.selectedIndex].text.replace(/^[📁📊]\s*/, '');
      const params = { temperature: parseFloat(this.el.tempSlider.value), top_k: parseInt(this.el.topkSlider.value, 10), top_p: parseFloat(this.el.toppSlider.value), min_p: parseFloat(this.el.minpSlider.value), repetition_penalty: parseFloat(this.el.reppenSlider.value), presence_penalty: parseFloat(this.el.prespenSlider.value) };
      const allHistory = store.chatHistory.slice();
      let mmprojPath: string | null = null;
      try { mmprojPath = await invoke("get_mmproj_path", { modelPath }); } catch (_) {}
      const response: any = await invoke("chat_request", { 
          modelPath, 
          agentId: activeAgent, 
          message: text, 
          history: allHistory, 
          contextSize: parseInt(this.el.contextSlider.value, 10), 
          maxGenTokens: parseInt(this.el.maxGenSlider.value, 10), 
          kvQuantKeys: this.el.chkKvQuantK.checked, 
          kvQuantValues: this.el.chkKvQuantV.checked, 
          modelParams: params, 
          attachments, 
          mmprojPath 
      });
      const dur = ((performance.now() - startTime) / 1000);
      const newMessages = response.messages || [];
      const thisRoundThoughts = store.chatHistory.slice(preSendLength + 1).filter((m: any) => m.type === 'thought');
      if (newMessages.length > 0) {
        const oldHistory = newMessages.slice(0, preSendLength);
        const userMsg = newMessages.slice(preSendLength, preSendLength + 1);
        const afterUserMsg = newMessages.slice(preSendLength + 1);
        
        afterUserMsg.forEach((m: any) => {
            if (m.author && m.author !== 'user' && m.author !== 'system') {
                m.time_sec = dur;
            }
        });
        
        const oldUids = store.msgUidList.slice(0, preSendLength + 1);
        const thoughtUids = store.msgUidList.slice(preSendLength + 1); 
        const newUids = afterUserMsg.map(() => store.nextUid());
        
        store.chatHistory = [...oldHistory, ...userMsg, ...thisRoundThoughts, ...afterUserMsg];
        store.msgUidList = [...oldUids, ...thoughtUids, ...newUids];
      }
      const agentUid = store.nextUid(); store.msgUidList.push(agentUid);
      const hasRT = response.sub_calls && response.sub_calls.some((c: any) => store.realtimeSubcallKeys.has(`${c.agent_name}:${c.time_sec.toFixed(2)}`));
      if (response.text) {
        this.renderChatFromHistory();
      } else if (newMessages.length > 0) {
        this.renderChatFromHistory();
      }
      await saveSession(store.currentSessionId, store.chatHistory, this.el.chatInput.value);
    } catch (error) {
      if (String(error).includes("Отменено") || String(error).includes("Прервано")) {
          store.chatHistory.push({ type: "message", author: "system", content: "⚠️ Прервано." });
          store.msgUidList.push(store.nextUid());
      } else {
          showToast(`Ошибка: ${error}`, "error");
          store.chatHistory.push({ type: "message", author: "system", content: `⚠️ Ошибка: ${error}` });
          store.msgUidList.push(store.nextUid());
      }
      this.renderChatFromHistory();
      if (store.currentSessionId) {
        await saveSession(store.currentSessionId, store.chatHistory, this.el.chatInput.value);
      }
    } finally { 
        this.setProcessingState(false); 
        this.triggerTokenCount();
    }
  }

  private triggerDraftSave() {
    if (store.isProcessing) return;
    if (!store.currentSessionId && this.el.chatInput.value.trim() !== "") {
      store.currentSessionId = Date.now().toString(); store.chatHistory = [];
      saveSession(store.currentSessionId, store.chatHistory, this.el.chatInput.value).then(() => bus.emit("session:changed"));
    } else if (store.currentSessionId) {
      clearTimeout(store.draftTimeout);
      store.draftTimeout = window.setTimeout(() => { saveSession(store.currentSessionId!, store.chatHistory, this.el.chatInput.value); }, 500);
    }
  }

  startNewSession() {
    if (store.isProcessing) return;
    store.currentSessionId = null; store.resetForNewSession(); this.thoughtDedupSet.clear();
    this.el.chatHistory.innerHTML = ''; this.el.chatInput.value = ''; this.el.chatInput.style.height = "auto";
    this.el.filePreview.innerHTML = ''; this.attachments = [];
    this.updateAttachButtonState();
    this.appendMessage('system', 'Новая сессия. Выберите агента и напишите запрос.');
    bus.emit("session:changed"); bus.emit("tab:switch", 'chat');
    this.triggerTokenCount();
  }

  async openSession(id: string) {
    if (store.isProcessing) return;
    try {
      const session = await loadSession(id);
      store.currentSessionId = id; store.chatHistory = session.messages;
      this.el.chatInput.value = session.draft || "";
      setTimeout(() => { this.el.chatInput.style.height = "auto"; this.el.chatInput.style.height = `${this.el.chatInput.scrollHeight}px`; }, 0);
      store.uidCounter = 0; store.msgUidList = store.chatHistory.map(() => store.nextUid());
      this.renderChatFromHistory(); bus.emit("session:changed"); bus.emit("tab:switch", 'chat');
      this.triggerTokenCount();
    } catch(e) { showToast(`Ошибка: ${e}`, "error"); }
  }

  private async updateAttachButtonState() {
    const btn = this.el.btnAttach;
    const modelPath = this.el.modelSelect?.value;
    if (!modelPath) { btn.disabled = true; btn.classList.remove('btn-attach-active'); btn.classList.add('btn-attach-inactive'); btn.title = 'Сначала выберите модель'; return; }
    let hasMmproj = false;
    try { hasMmproj = !!(await invoke("get_mmproj_path", { modelPath })); } catch (_) {}
    if (hasMmproj) {
      btn.disabled = false;
      btn.classList.remove('btn-attach-inactive');
      btn.classList.add('btn-attach-active');
      btn.title = 'Прикрепить файл (изображение/аудио)';
    } else {
      btn.disabled = true;
      btn.classList.remove('btn-attach-active');
      btn.classList.add('btn-attach-inactive');
      btn.title = 'mmproj не найден — мультимодальный режим недоступен';
    }
  }

  private async handleFileSelect(files: FileList | null) {
    if (!files || store.isProcessing) return;
    for (let i = 0; i < files.length; i++) {
      const file = files[i];
      if (file.size > 20 * 1024 * 1024) { showToast(`Файл ${file.name} слишком большой (>20MB)`, "error"); continue; }
      const dataBase64 = await this.fileToBase64(file);
      this.attachments.push({ file_name: file.name, mime_type: file.type, data_base64: dataBase64 });
      this.addFilePreview(file.name, file.type, dataBase64);
    }
    this.el.fileInput.value = '';
    this.triggerTokenCount();
  }

  private fileToBase64(file: File): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result as string;
        resolve(result.split(',')[1] || result);
      };
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(file);
    });
  }

  private addFilePreview(fileName: string, mimeType: string, dataBase64: string) {
    const div = document.createElement('div');
    div.className = 'file-preview-item';
    const isImage = mimeType.startsWith('image/');
    if (isImage) {
      const img = document.createElement('img');
      img.src = `data:${mimeType};base64,${dataBase64}`;
      div.appendChild(img);
    }
    const nameSpan = document.createElement('span');
    nameSpan.className = 'file-preview-name';
    nameSpan.textContent = fileName;
    div.appendChild(nameSpan);
    const remove = document.createElement('span');
    remove.className = 'file-preview-remove';
    remove.textContent = '✕';
    remove.addEventListener('click', () => {
      const idx = this.attachments.findIndex(a => a.file_name === fileName);
      if (idx !== -1) this.attachments.splice(idx, 1);
      div.remove();
      this.triggerTokenCount();
    });
    div.appendChild(remove);
    this.el.filePreview.appendChild(div);
  }

  private bindDomEvents() {
    this.el.btnSend?.addEventListener("click", () => this.handleSend());
    this.el.chatInput?.addEventListener("keydown", (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); this.handleSend(); } });
    this.el.btnStop?.addEventListener("click", async () => { if (this.el.btnStop.disabled) return; this.el.btnStop.disabled = true; this.el.btnStop.innerText = "Стоп..."; this.appendMessage('system', 'Остановка...'); await invoke("stop_processing"); this.el.btnStop.innerText = "⏹ Стоп"; });
    this.el.btnBackChat?.addEventListener("click", () => { this.el.viewSubchat.classList.remove('active'); this.el.viewChat.classList.add('active'); });
    this.el.chatInput.addEventListener("input", () => { 
        this.el.chatInput.style.height = "auto"; 
        this.el.chatInput.style.height = `${this.el.chatInput.scrollHeight}px`; 
        this.triggerDraftSave(); 
        this.triggerTokenCount();
    });
    this.el.chatInput.addEventListener("blur", () => { if (store.currentSessionId && !store.isProcessing) { clearTimeout(store.draftTimeout); saveSession(store.currentSessionId, store.chatHistory, this.el.chatInput.value); } });
    this.el.btnAttach?.addEventListener("click", () => { if (!this.el.btnAttach.disabled) this.el.fileInput.click(); });
    this.el.fileInput?.addEventListener("change", (e) => this.handleFileSelect((e.target as HTMLInputElement).files));
    this.el.modelSelect?.addEventListener("change", () => { this.updateAttachButtonState(); this.triggerTokenCount(); });
    this.el.agentSelect?.addEventListener("change", () => { this.triggerTokenCount(); });
  }

  private thoughtDedupSet = new Set<string>();

  private bindTauriEvents() {
    listen("progress", (e) => { this.el.progressBar.style.width = `${e.payload}%`; });
    listen("status", (e) => { this.el.statusLabel.innerText = e.payload as string; });
    listen("log", (e) => { this.logToGUI(e.payload as string); });
    
    // СТРИМИНГ ОТВЕТА В ЧАТ В РЕАЛЬНОМ ВРЕМЕНИ
    listen("stream_chunk", (e) => {
        if (!store.isProcessing) return;
        const chunk = e.payload as string;
        store.rtStreamBuffer += chunk;
        
        // Если это внутренний JSON (вызов сабагента или тулзы) — скрываем от глаз пользователя!
        if (store.rtStreamBuffer.trimStart().startsWith("{") || store.rtStreamBuffer.trimStart().startsWith("```json")) {
            store.rtIsJson = true;
            return;
        }
        
        // Если это начало ответа, создаем пустое сообщение в UI
        if (!store.rtStreamUid) {
            store.rtStreamUid = store.nextUid();
            const displayName = this.el.agentSelect.options[this.el.agentSelect.selectedIndex].text.replace(/^[📁📊]\s*/, '');
            this.appendMessage('agent', '', displayName, undefined, undefined, false, store.rtStreamUid);
        }
        
        // Обновляем тело сообщения (renderMarkdown автоматически скроет <think> теги из текста)
        const msgEl = this.el.chatHistory.querySelector(`[data-msg-uid="${store.rtStreamUid}"]`);
        if (msgEl) {
            const contentDiv = msgEl.querySelector('div:nth-child(2)');
            if (contentDiv) {
                contentDiv.innerHTML = renderMarkdown(store.rtStreamBuffer);
                this.scrollToBottomIfNearEnd(this.el.chatHistory);
            }
        }
        
        // Печатаем "Мысли" агента в реальном времени в раскрывающийся блок!
        const thinkMatch = store.rtStreamBuffer.match(/<think>([\s\S]*?)(?:<\/think>|$)/i);
        if (thinkMatch) {
            const thoughtContent = thinkMatch[1].trim();
            if (!store.activeThoughtsBlock && msgEl) {
                const item = createThoughtElement("Агент", thoughtContent);
                item.id = "rt-thought";
                store.activeThoughtsBlock = createThoughtsBlock([item]);
                this.el.chatHistory.insertBefore(store.activeThoughtsBlock, msgEl);
            } else if (store.activeThoughtsBlock) {
                let item = store.activeThoughtsBlock.querySelector('#rt-thought');
                if (!item) {
                    item = createThoughtElement("Агент", thoughtContent);
                    item.id = "rt-thought";
                    addToThoughtsBlock(store.activeThoughtsBlock, item as HTMLElement);
                } else {
                    item.innerHTML = `🧠 <strong>Агент</strong>: <em>${thoughtContent}</em>`;
                }
            }
        }
    });

    listen("subcall_done", (e) => {
      const call = e.payload as any;
      store.realtimeSubcallKeys.add(`${call.agent_name}:${call.time_sec.toFixed(2)}`);
      const item = createSubcallElement(call, (c) => this.showSubchat(c));
      if (store.activeThoughtsBlock) { addToThoughtsBlock(store.activeThoughtsBlock, item); }
      else { store.activeThoughtsBlock = createThoughtsBlock([item], undefined, undefined); this.el.chatHistory.appendChild(store.activeThoughtsBlock); }
      this.scrollToBottomIfNearEnd(this.el.chatHistory);
    });
    
    listen("agent_thought", (e) => {
      const payload = e.payload as { author: string, thought: string, time_sec: number };
      const dedupKey = `${payload.author}:${payload.thought.substring(0, 200)}`;
      if (this.thoughtDedupSet.has(dedupKey)) return;
      this.thoughtDedupSet.add(dedupKey);
      const item = createThoughtElement(payload.author, payload.thought, payload.time_sec);
      if (store.activeThoughtsBlock) { addToThoughtsBlock(store.activeThoughtsBlock, item); }
      else { store.activeThoughtsBlock = createThoughtsBlock([item], undefined, undefined); this.el.chatHistory.appendChild(store.activeThoughtsBlock); }
      this.scrollToBottomIfNearEnd(this.el.chatHistory);
      store.chatHistory.push({ type: "thought", content: payload.thought, author: payload.author, time_sec: payload.time_sec });
      store.msgUidList.push(store.nextUid());
    });
  }

  private bindBusEvents() {
    bus.on("session:new", () => this.startNewSession());
    bus.on("session:open", (id: string) => this.openSession(id));
    bus.on("config:loaded", () => { this.updateAttachButtonState(); this.triggerTokenCount(); });
  }
}