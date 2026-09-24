import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { store } from "../store";
import { bus } from "../events";
import { createMessageElement, createSubcallElement, createToolCallElement, createToolThoughtElement, createThoughtElement, createThoughtsBlock, addToThoughtsBlock, showToast, showPermissionRequest, showVramRequest } from "../ui";
import type { Role, MessageMenuCallbacks } from "../ui";
import type { ThoughtMenuCallbacks, Attachment, ChatMessage } from "../types";
import { saveSession, loadSession, countTokens } from "../services";
import { getEngineStatus, getMmprojPath, ensureMmproj, getModelCapabilities, getModelsCatalog, estimatePromptMemory, type CatalogEntry } from "@my-tauri-plugins/plugin-llama-engine";
import { chatCompletion as nineRouterChat } from "@my-tauri-plugins/plugin-9router";
import { renderMarkdown, stripStreamArtifacts, extractChannelThought, NINE_ROUTER_MODEL_PREFIX, fillModelSelect, fillAgentSelect } from "../utils";
import { logFront } from "@my-tauri-plugins/plugin-logs";
import { trackError } from "../telemetry";
import mermaid from "mermaid";

mermaid.initialize({ startOnLoad: false, theme: "dark", securityLevel: "loose" });

/// Watchdog зависшей обработки: если движок молчит дольше лимита, а обработка
/// формально идёт — снимаем состояние (кнопки/стоп возвращаются юзеру).
const PROCESSING_IDLE_LIMIT_MS = 15 * 60 * 1000; // 15 минут тишины движка
const PROCESSING_CHECK_MS = 30 * 1000;           // проверка раз в 30 сек

/// Названия целевых языков для переводчика сообщений.
/// `menu` — текст пункта контекстного меню, `footer` — подпись внутри сообщения.
const TRANSLATOR_LANGS: Record<string, { menu: string; footer: string }> = {
  ru: { menu: "Перевести на русский язык", footer: "перевод на русский язык:" },
  en: { menu: "Перевести на английский язык", footer: "перевод на английский язык:" },
};

async function renderMermaid() {
  try {
    await mermaid.run();
  } catch(e) {
    console.error('Mermaid render error:', e);
    void trackError("mermaid.render", e);
  }
  document.querySelectorAll('pre.mermaid').forEach(el => {
    if (!el.querySelector('svg')) {
      const code = el.textContent || '';
      el.outerHTML = `<pre style="background:#1e1e1e;color:#ccc;padding:12px;border-radius:6px;overflow-x:auto;white-space:pre-wrap;font-family:monospace">${code.replace(/</g, '&lt;').replace(/>/g, '&gt;')}</pre>`;
    }
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Маршрутизация событий движка: глобальные Tauri-listen регистрируются ОДИН
// раз (initChatEventRouter, см. src/controllers/chat-router.ts), а все события
// (прогресс, статус, стриминг, мысли, режим движка) перенаправляются в
// КОНТРОЛЛЕР вкладки, которая в данный момент обрабатывает запрос.
// Обработка СТРОГО последовательная (одна генерация за раз).
// ─────────────────────────────────────────────────────────────────────────────

let activeProcessingChat: ChatController | null = null;

function isProcessingActive(controller: ChatController): boolean {
  return activeProcessingChat !== null && activeProcessingChat === controller;
}

/** Снятие статуса «активная обработка» — только сам владелец может освободить слот. */
export function releaseGlobalProcessing(controller: ChatController) {
  if (activeProcessingChat === controller) {
    activeProcessingChat = null;
    store.isProcessing = false;
  }
}

/** Контроллер вкладки, обрабатывающей текущий запрос (для роутера и настроек). */
export function getActiveProcessingChat(): ChatController | null {
  return activeProcessingChat;
}

/** Захват слота активной обработки (строго один контроллер за раз). */
export function setActiveProcessingChat(controller: ChatController) {
  activeProcessingChat = controller;
  store.isProcessing = true;
}

// ─────────────────────────────────────────────────────────────────────────────

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
  maxGenSlider: HTMLInputElement;
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
  engineBadge: HTMLDivElement;
  btnSetWorkdir: HTMLButtonElement;
  currentWorkdir: HTMLSpanElement;
}

/// Колбэки вкладки (вызывает TabController).
export interface ChatHooks {
  /// Вкладка стала чат-табом: получила реальную сессию (отправка либо
  /// открыта существующая по session:open). Таб-контроллер меняет тип вкладки
  /// main → chat, убирает экран новой вкладки и персистит.
  onBecameChat?: (sessionId: string) => void;
  /// Мain-вкладка получила черновик (сессия создана при наборе текста).
  /// Таб-контроллер регистрирует sessionId у вкладки БЕЗ конвертации в чат.
  onDraftSession?: (sessionId: string) => void;
}

/**
 * Состояние ОДНОЙ чат-вкладки. Каждая вкладка имеет собственную сессию,
 * историю, черновик и стриминг. Глобальный флаг `store.isProcessing` остаётся
 * верным и единым для приложения (обработка идёт где-то).
 */
export class ChatTabState {
  sessionId: string | null = null;
  history: ChatMessage[] = [];
  uidList: string[] = [];
  uidCounter = 0;

  /** Раскрывающийся блок «Мысли агентов» текущей вкладки. */
  activeThoughtsBlock: HTMLDivElement | null = null;
  realtimeSubcallKeys = new Set<string>();

  /// Для watchdog зависшей обработки.
  processingStartedAt = 0;
  lastActivityAt = 0;

  /** Таймер автосейва черновика (персист раз в 500мс). */
  draftTimeout: number | undefined;

  /// Вкладка уже показана как чат (main-экран снят). Ставится при реальной
  /// конвертации main → chat (отправка) либо при открытии сессии.
  isChatTab = false;

  // Стриминг текста
  rtStreamUid: string | null = null;
  rtStreamBuffer: string = "";
  rtIsJson: boolean = false;

  // Стриминг мыслей
  rtThoughtUid: string | null = null;
  rtThoughtBuffer: string = "";
  rtThoughtAuthor: string = "";

  /** Идёт ли обработка ИМЕННО в этой вкладке. */
  isProcessing = false;

  /** «Вкладка имеет реальную сессию» — для таб-ленты (типы main/chat). */
  hasSession(): boolean {
    return this.sessionId !== null && this.sessionId !== "";
  }

  nextUid(): string {
    return `msg_${this.uidCounter++}`;
  }

  resetForNewSession() {
    this.history = [];
    this.uidList = [];
    this.uidCounter = 0;
    this.realtimeSubcallKeys.clear();
    this.activeThoughtsBlock = null;
    this.rtStreamUid = null;
    this.rtStreamBuffer = "";
    this.rtIsJson = false;
    this.rtThoughtUid = null;
    this.rtThoughtBuffer = "";
    this.rtThoughtAuthor = "";
    this.isProcessing = false;
    this.processingStartedAt = 0;
    this.lastActivityAt = 0;
  }
}

export class ChatController {
  readonly state = new ChatTabState();
  private el: ChatElements;
  private hooks: ChatHooks;
  private menuCallbacks: MessageMenuCallbacks;
  private thoughtMenuCallbacks: ThoughtMenuCallbacks;
  private attachments: Attachment[] = [];
  private modelAudioCapable: boolean = false;
  private countTimer: number | null = null;
  private lastPromptTokens: number = 0;
  private processingWatchdog: number | null = null;
  /// Кэш каталога моделей плагина (для tokenizer_id в счётчике токенов).
  private catalogCache: CatalogEntry[] | null = null;
  private thoughtDedupSet = new Set<string>();

  /// Публичные ссылки на страницы (для чужих контроллеров: вкладки, сессии).
  get mainView(): HTMLDivElement { return this.el.viewChat; }
  get subView(): HTMLDivElement { return this.el.viewSubchat; }

  constructor(el: ChatElements, hooks?: ChatHooks) {
    this.el = el;
    this.hooks = hooks || {};
    this.menuCallbacks = {
      onDelete: (uid) => this.onDeleteMessage(uid),
      onClone: (uid) => this.onCloneMessage(uid),
      onRunFrom: (uid) => this.onRunFromMessage(uid),
      onCopy: (uid) => this.onCopyMessage(uid),
      onEdit: (uid) => this.onEditMessage(uid),
      onTranslate: (uid) => this.onTranslateMessage(uid),
      onSaveImage: (uid) => this.onSaveImageMessage(uid),
    };
    this.thoughtMenuCallbacks = {
      onDeleteThoughts: (uid, uids) => this.onDeleteThoughts(uid, uids),
      onCloneFromThoughts: (uid) => this.onCloneFromThoughts(uid),
    };
    this.bindDomEvents();
    this.bindBusEvents();
    setTimeout(() => {
      this.updateAttachButtonState();
      this.triggerTokenCount();
      this.refreshEngineBadgeInit();
    }, 100);
  }

  /** Текущее значение выбора модели (для SettingsController при загрузке параметров). */
  get modelSelectValue(): string | null {
    return this.el.modelSelect?.value || null;
  }

  // ── Публичный API для роутера событий (src/controllers/chat-router.ts) ──

  onProgress(pct: number) {
    if (!isProcessingActive(this)) return;
    this.state.lastActivityAt = Date.now();
    this.el.progressBar.style.width = `${pct}%`;
  }

  onStatus(text: string) {
    if (!isProcessingActive(this)) return;
    this.state.lastActivityAt = Date.now();
    this.el.statusLabel.innerText = text;
  }

  onStreamChunk(rawPayload: any) {
    if (!isProcessingActive(this)) return;
    // Бэкенд шлёт объект { kind, author, text }; для обратной совместимости
    // (старые вызовы) payload может прийти строкой — считаем его message.
    const payload = rawPayload;
    const kind: string = typeof payload === "string" ? "message" : (payload.kind || "message");
    const chunk: string = typeof payload === "string" ? payload : (payload.text || "");
    const author: string = (payload && typeof payload.author === "string") ? payload.author : "";

    // ── МЫСЛИ: печатаем в блок «Мысли агентов» в реальном времени ──
    if (kind === "thought") {
      // Новый агент → новый пузырь мыслей
      if (this.state.rtThoughtUid && this.state.rtThoughtAuthor && author && author !== this.state.rtThoughtAuthor) {
        this.state.rtThoughtUid = null;
        this.state.rtThoughtBuffer = "";
        this.state.rtThoughtAuthor = "";
      }
      this.state.rtThoughtBuffer += chunk;

      // Прячем внутренние JSON-вызовы (сабагент/инструмент) от глаз пользователя
      if (this.state.rtThoughtBuffer.trimStart().startsWith("{") || this.state.rtThoughtBuffer.trimStart().startsWith("```json")) {
        return;
      }

      const label = author || "Агент";
      if (!this.state.rtThoughtUid) {
        this.state.rtThoughtUid = this.state.nextUid();
        this.state.rtThoughtAuthor = label;
        const item = createThoughtElement(label, this.state.rtThoughtBuffer);
        item.id = "rt-thought-stream";
        this.state.activeThoughtsBlock = createThoughtsBlock([item]);
        this.el.chatHistory.appendChild(this.state.activeThoughtsBlock);
      } else {
        const item = this.state.activeThoughtsBlock?.querySelector('#rt-thought-stream');
        if (item) {
          item.innerHTML = `🧠 <strong>${label}</strong>: <em>${this.state.rtThoughtBuffer}</em>`;
        }
      }
      this.scrollToBottomIfNearEnd(this.el.chatHistory);
      return;
    }

    // ── СООБЩЕНИЕ: старая логика основного чата ──
    this.state.rtStreamBuffer += chunk;

    // Если это внутренний JSON (вызов сабагента или тулзы) — скрываем от глаз пользователя!
    if (this.state.rtStreamBuffer.trimStart().startsWith("{") || this.state.rtStreamBuffer.trimStart().startsWith("```json")) {
      this.state.rtIsJson = true;
      return;
    }

    // Если это начало ответа, создаем пустое сообщение в UI
    if (!this.state.rtStreamUid) {
      this.state.rtStreamUid = this.state.nextUid();
      const displayName = (author && author.trim())
        ? author
        : this.el.agentSelect.options[this.el.agentSelect.selectedIndex].text.replace(/^[📁📊]\s*/, '');
      this.appendMessage('agent', '', displayName, undefined, undefined, false, this.state.rtStreamUid);
    }

    // Обновляем тело сообщения. Скрываем служебные теги LLM (<|channel>...<channel|> и
    // <|turn>) перед подачей в renderMarkdown (теги могут прийти внутри чанка).
    const visibleText = stripStreamArtifacts(this.state.rtStreamBuffer);
    const msgEl = this.el.chatHistory.querySelector(`[data-msg-uid="${this.state.rtStreamUid}"]`);
    if (msgEl) {
      const contentDiv = msgEl.querySelector('div:nth-child(2)');
      if (contentDiv) {
        contentDiv.innerHTML = renderMarkdown(visibleText);
        this.scrollToBottomIfNearEnd(this.el.chatHistory);
      }
    }

    // Печатаем "Мысли" агента (формат <|channel>thought>...) в раскрывающийся блок.
    const channelThought = extractChannelThought(this.state.rtStreamBuffer);
    if (channelThought) {
      if (!this.state.activeThoughtsBlock && msgEl) {
        const item = createThoughtElement("Агент", channelThought);
        item.id = "rt-channel-thought";
        this.state.activeThoughtsBlock = createThoughtsBlock([item]);
        this.el.chatHistory.insertBefore(this.state.activeThoughtsBlock, msgEl);
      } else if (this.state.activeThoughtsBlock) {
        let item = this.state.activeThoughtsBlock.querySelector('#rt-channel-thought');
        if (!item) {
          item = createThoughtElement("Агент", channelThought);
          item.id = "rt-channel-thought";
          addToThoughtsBlock(this.state.activeThoughtsBlock, item as HTMLElement);
        } else {
          item.innerHTML = `🧠 <strong>Агент</strong>: <em>${channelThought}</em>`;
        }
      }
    }
  }

  onSubcallDone(call: any) {
    if (!isProcessingActive(this)) return;
    this.state.realtimeSubcallKeys.add(`${call.agent_name}:${call.time_sec.toFixed(2)}`);
    const item = createSubcallElement(call, (c) => this.showSubchat(c));
    if (this.state.activeThoughtsBlock) { addToThoughtsBlock(this.state.activeThoughtsBlock, item); }
    else { this.state.activeThoughtsBlock = createThoughtsBlock([item], undefined, undefined); this.el.chatHistory.appendChild(this.state.activeThoughtsBlock); }
    this.scrollToBottomIfNearEnd(this.el.chatHistory);
  }

  onAgentThought(payload: { author: string, thought: string, time_sec: number }) {
    if (!isProcessingActive(this)) return;
    const dedupKey = `${payload.author}:${payload.thought.substring(0, 200)}`;
    if (this.thoughtDedupSet.has(dedupKey)) return;
    this.thoughtDedupSet.add(dedupKey);
    const item = createThoughtElement(payload.author, payload.thought, payload.time_sec);
    if (this.state.activeThoughtsBlock) { addToThoughtsBlock(this.state.activeThoughtsBlock, item); }
    else { this.state.activeThoughtsBlock = createThoughtsBlock([item], undefined, undefined); this.el.chatHistory.appendChild(this.state.activeThoughtsBlock); }
    this.scrollToBottomIfNearEnd(this.el.chatHistory);
  }

  onAgentToolCall(p: { author: string, tool: string, args?: string, result?: string }) {
    if (!isProcessingActive(this)) return;
    const key = `${p.author}:${p.tool}`;
    if (p.result !== undefined) {
      const item = this.state.activeThoughtsBlock?.querySelector(`[data-tool-key="${key}"]`);
      if (item) {
        item.querySelector(".tool-thought-status")?.remove();
        const resultDiv = document.createElement("div");
        resultDiv.className = "tool-thought-result";
        resultDiv.textContent = `→ ${p.result}`;
        item.appendChild(resultDiv);
        this.scrollToBottomIfNearEnd(this.el.chatHistory);
      }
      return;
    }
    const item = createToolThoughtElement(p.author, p.tool, p.args);
    if (this.state.activeThoughtsBlock) { addToThoughtsBlock(this.state.activeThoughtsBlock, item); }
    else { this.state.activeThoughtsBlock = createThoughtsBlock([item], undefined, undefined); this.el.chatHistory.appendChild(this.state.activeThoughtsBlock); }
    this.scrollToBottomIfNearEnd(this.el.chatHistory);
  }

  onEngineMode(p: { mode?: string, tok_per_sec?: number, detail?: string }) {
    if (!isProcessingActive(this)) return;
    this.updateEngineBadge(p.mode || "cpu", p.tok_per_sec || 0, p.detail || "");
  }

  onToolPermission(payload: any) {
    if (!isProcessingActive(this)) return;
    showPermissionRequest(payload);
  }

  onVramNotice(payload: any) {
    if (!isProcessingActive(this)) return;
    showVramRequest(payload);
  }

  // ── Вспомогательные ──

  private scrollToBottomIfNearEnd(el: HTMLElement) {
    const threshold = 100;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < threshold;
    if (atBottom) {
      el.scrollTop = el.scrollHeight;
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
    const contextSize = store.contextSize;

    if (!modelPath || !agentId) return;
    // Облачные комбо 9Router: локальный счётчик токенов/VRAM неприменим.
    if (modelPath.startsWith(NINE_ROUTER_MODEL_PREFIX)) {
      if (this.el.tokenCounter) this.el.tokenCounter.innerText = "";
      return;
    }

    let tokenizerId = "Xenova/Meta-Llama-3-8B-Instruct";
    if (!this.catalogCache) {
      try { this.catalogCache = await getModelsCatalog(); } catch (_) { this.catalogCache = []; }
    }
    const catalogModel = (this.catalogCache || []).find((m: CatalogEntry) => modelPath.includes(m.name) || m.name === modelPath);
    if (catalogModel && catalogModel.tokenizer_id) {
        tokenizerId = catalogModel.tokenizer_id;
    }

    try {
        // kv_quant_* — legacy (Variant A: всегда false). KV-spec (BeeLlama KVarN)
        // резолвится на бэкенде по engine_source — хардкод здесь не влияет на OOM.
        const kvQuantKeys = false;
        const kvQuantValues = false;
        const maxGen = parseInt(this.el.maxGenSlider?.value || "2048", 10);

        const promptText = await invoke<string>("get_prompt_preview", {
            modelPath,
            agentId,
            message: text,
            history: this.state.history
        });

        const tokens = await countTokens(promptText, tokenizerId);
        this.lastPromptTokens = tokens;

        const vram = await estimatePromptMemory(modelPath, contextSize, kvQuantKeys, kvQuantValues, tokens, maxGen);

        if (this.el.tokenCounter) {
            const isGraph = this.el.agentSelect.options[this.el.agentSelect.selectedIndex]?.text.startsWith("📁") ?? false;
            const graphHint = isGraph ? " Оценка по самому тяжёлому агенту графа." : "";
            // Формат: (расчётная потребность + уже занятая VRAM) / точный объём VRAM в MiB.
            const memStr = vram && vram.vram_used_mb > 0 && vram.vram_total_mb > 0
                ? ` (~${Math.round(vram.need_mb + vram.vram_used_mb)} МБ / ${Math.round(vram.vram_total_mb)} МБ)`
                : (isFinite(vram?.need_mb) && (vram?.need_mb ?? 0) > 0 ? ` (~${Math.round(vram.need_mb)} МБ)` : "");
            this.el.tokenCounter.innerText = `${tokens}${memStr}`;
            this.el.tokenCounter.classList.remove("warning", "danger");
            if (tokens > contextSize) {
                this.el.tokenCounter.classList.add("danger");
                this.el.tokenCounter.title = "Внимание: Лимит превышен! При отправке старые сообщения будут удалены из контекста." + graphHint;
            } else if (tokens > contextSize * 0.8) {
                this.el.tokenCounter.classList.add("warning");
                this.el.tokenCounter.title = "Контекст заполняется. Близко к лимиту." + graphHint;
            } else {
                this.el.tokenCounter.title = "Токены: текущие / Лимит контекста. Память: оценка (потребность + занято) / объём VRAM." + graphHint;
            }
        }
    } catch (e) {
        console.error("Ошибка обновления счетчика токенов:", e);
        void trackError("chat.updateTokenCounter", e);
    }
  }

  // ── Операции над сообщениями ──

  private async onDeleteMessage(uid: string) {
    const idx = this.state.uidList.indexOf(uid); if (idx === -1) return;
    this.state.history.splice(idx, 1); this.state.uidList.splice(idx, 1);
    this.renderChatFromHistory();
    if (this.state.hasSession()) await this.persistSession();
    this.triggerTokenCount();
    showToast("Сообщение удалено.", "success");
  }

  private async onCloneMessage(uid: string) {
    const idx = this.state.uidList.indexOf(uid); if (idx === -1) return;
    const clonedHistory = this.state.history.slice(0, idx + 1);
    const newId = Date.now().toString();
    await saveSession(newId, clonedHistory, "", this.el.modelSelect?.value, this.el.agentSelect?.value);
    showToast("Клон сессии создан!", "success");
    bus.emit("session:changed");
    bus.emit("session:open", newId);
  }

  private async onSaveImageMessage(uid: string) {
    const idx = this.state.uidList.indexOf(uid);
    if (idx === -1) return;
    const msg = this.state.history[idx];
    const images = (msg.attachments || []).filter((a) => a.mime_type?.startsWith("image/"));
    if (images.length === 0) {
      showToast("В сообщении нет изображений", "error");
      return;
    }
    // Первое изображение — сразу в диалог, остальные — по очереди после сохранения.
    for (const img of images) {
      const ext = img.mime_type === "image/jpeg" ? "jpg" : img.mime_type === "image/webp" ? "webp" : "png";
      const defName = (img.file_name || `image.${ext}`).replace(/\.[a-z0-9]+$/i, "") + `.${ext}`;
      const savePath = await saveDialog({ defaultPath: defName, filters: [{ name: "Изображения", extensions: [ext] }] });
      if (!savePath) return;
      try {
        await invoke("save_image_file", { path: savePath, dataBase64: img.data_base64 });
        showToast(`Изображение сохранено: ${savePath}`, "success");
      } catch (err) {
        showToast(`Ошибка сохранения: ${err}`, "error");
        void trackError("chat.saveImage", err);
        return;
      }
    }
  }

  private async onCopyMessage(uid: string) {
    const idx = this.state.uidList.indexOf(uid);
    if (idx === -1) return;
    const msg = this.state.history[idx];
    try {
      await navigator.clipboard.writeText(msg.content);
      showToast("Сообщение скопировано в буфер обмена", "success");
    } catch (err) {
      showToast(`Ошибка копирования: ${err}`, "error");
      void trackError("chat.copyMessage", err);
    }
  }

  private onEditMessage(uid: string) {
    const idx = this.state.uidList.indexOf(uid);
    if (idx === -1) return;
    const msg = this.state.history[idx];
    const msgEl = this.el.chatHistory.querySelector(`[data-msg-uid="${uid}"]`) as HTMLDivElement | null;
    if (!msgEl) return;

    const contentDiv = msgEl.querySelector(".msg-content") as HTMLDivElement | null;
    if (!contentDiv) return;
    const originalContent = msg.content || "";

    this.renderEditor(msgEl, contentDiv, originalContent, (newContent) => {
      msg.content = newContent;
      this.renderChatFromHistory();
      if (this.state.hasSession()) this.persistSession();
      this.triggerTokenCount();
      showToast("Сообщение обновлено.", "success");
    });
  }

  /// Возвращает текст пункта меню перевода для текущего целевого языка
  /// (или undefined, если язык не задан/неизвестен).
  private translatorMenuLabel(): string | undefined {
    return TRANSLATOR_LANGS[store.translatorLang]?.menu;
  }

  /// Срезает ранее вставленный блок перевода, возвращая исходный текст
  /// сообщения (чтобы повторный перевод брал за основу оригинал, а не
  /// результат предыдущего перевода).
  private stripTranslationFooter(content: string): string {
    const idx = content.search(/\n-{2,}\s*\n\*\*перевод на .*?:\*\*/s);
    if (idx === -1) return content;
    return content.slice(0, idx).trimEnd();
  }

  private async onTranslateMessage(uid: string) {
    const idx = this.state.uidList.indexOf(uid);
    if (idx === -1) return;
    const msg = this.state.history[idx];
    if (!msg || msg.type === "thought") return;

    if (!store.translatorModel) {
      showToast("Сначала выберите модель переводчика в настройках.", "error");
      return;
    }
    const lang = store.translatorLang || "ru";
    const langInfo = TRANSLATOR_LANGS[lang] ?? TRANSLATOR_LANGS.ru;
    const source = this.stripTranslationFooter(msg.content || "");

    showToast("Перевод сообщения…", "info");
    try {
      const translated: string = await invoke("translate_message", {
        modelPath: store.translatorModel,
        text: source,
        targetLang: lang,
      });
      msg.content = `${source}\n\n---\n**${langInfo.footer}**\n\n${translated.trim()}`;
      this.renderChatFromHistory();
      if (this.state.hasSession()) this.persistSession();
      showToast("Сообщение переведено.", "success");
    } catch (e) {
      showToast(`Ошибка перевода: ${e}`, "error");
      void trackError("chat.translateMessage", e);
    }
  }

  private renderEditor(msgEl: HTMLDivElement, contentDiv: HTMLDivElement, initialValue: string, onSave: (newContent: string) => void) {
    if (contentDiv.querySelector("textarea.msg-edit-input")) return;

    msgEl.classList.add("message-editing");
    contentDiv.classList.add("msg-edit-host");
    contentDiv.innerHTML = "";

    const textarea = document.createElement("textarea");
    textarea.className = "msg-edit-input";
    textarea.value = initialValue;

    const autoGrow = () => {
      textarea.style.height = "auto";
      textarea.style.height = `${Math.max(textarea.scrollHeight, 120)}px`;
    };

    const btnRow = document.createElement("div");
    btnRow.className = "msg-edit-actions";

    const saveBtn = document.createElement("button");
    saveBtn.className = "msg-edit-save";
    saveBtn.textContent = "💾 Сохранить";
    saveBtn.addEventListener("click", () => {
      onSave(textarea.value);
    });

    const cancelBtn = document.createElement("button");
    cancelBtn.className = "msg-edit-cancel";
    cancelBtn.textContent = "✖ Отмена";
    cancelBtn.addEventListener("click", () => {
      this.renderChatFromHistory();
    });

    btnRow.appendChild(saveBtn);
    btnRow.appendChild(cancelBtn);
    contentDiv.appendChild(textarea);
    contentDiv.appendChild(btnRow);

    textarea.addEventListener("input", autoGrow);
    setTimeout(() => { textarea.focus(); textarea.setSelectionRange(textarea.value.length, textarea.value.length); autoGrow(); }, 0);
  }

  private async onRunFromMessage(uid: string) {
    if (store.isProcessing || this.state.isProcessing) return;
    const idx = this.state.uidList.indexOf(uid);
    if (idx === -1) return;

    const activeAgent = this.el.agentSelect.value;
    const modelPath = this.el.modelSelect.value;
    if (!modelPath) { showToast("Выберите модель!", "error"); return; }

    logFront(`[chat] Нажата кнопка 'Отправить и запустить' для сообщения: ${uid}`);

    this.state.history = this.state.history.slice(0, idx + 1);
    this.state.uidList = this.state.uidList.slice(0, idx + 1);
    this.renderChatFromHistory();

    this.setProcessingState(true);

    this.ensureSessionId();
    if (!this.state.isChatTab) this.becomeChatTab();
    const preSendLength = this.state.history.length;
    const startTime = performance.now();
    try {
      const _p = store.currentModelParams;
      const params = { temperature: parseFloat(this.el.tempSlider.value), top_k: parseInt(this.el.topkSlider.value, 10), top_p: parseFloat(this.el.toppSlider.value), min_p: parseFloat(this.el.minpSlider.value), repetition_penalty: parseFloat(this.el.reppenSlider.value), presence_penalty: parseFloat(this.el.prespenSlider.value), dry_multiplier: _p?.dry_multiplier ?? 0.0, dry_base: _p?.dry_base ?? 1.75, dry_allowed_length: _p?.dry_allowed_length ?? 2, dry_penalty_last_n: _p?.dry_penalty_last_n ?? 0, xtc_probability: _p?.xtc_probability ?? 0.0, xtc_threshold: _p?.xtc_threshold ?? 0.1 };
      const allHistory = this.state.history.slice();

      let mmprojPath: string | null = null; // этот путь всегда текстовый (без вложений) — mmproj не нужен

      const response: any = await invoke("chat_request", {
        modelPath,
        agentId: activeAgent,
        message: "",
        history: allHistory,
        contextSize: store.contextSize,
        promptTokens: this.lastPromptTokens,
        maxGenTokens: parseInt(this.el.maxGenSlider.value, 10),
        kvQuantKeys: false,
        kvQuantValues: false,
        modelParams: params,
        attachments: [],
        mmprojPath,
        sessionId: this.state.sessionId ?? ""
      });

      if (response?.has_error) {
        void trackError("chat.send.runFrom.outcome", response.has_error);
      }

      const dur = ((performance.now() - startTime) / 1000);
      const newMessages = response.messages || [];

      if (newMessages.length > 0) {
        const oldHistory = newMessages.slice(0, preSendLength);
        const afterOldHistory = newMessages.slice(preSendLength);
        
        afterOldHistory.forEach((m: any) => {
            if (m.author && m.author !== 'user' && m.author !== 'system') {
                m.time_sec = dur;
            }
        });
        
        const oldUids = this.state.uidList.slice(0, preSendLength);
        const newUids = afterOldHistory.map(() => this.state.nextUid());

        this.state.history = [...oldHistory, ...afterOldHistory];
        this.state.uidList = [...oldUids, ...newUids];
      }

      this.renderChatFromHistory();
      await this.persistSession();
      this.triggerTokenCount();

    } catch (error) {
      if (String(error).includes("Отменено") || String(error).includes("Прервано")) {
        this.state.history.push({ type: "message", author: "system", content: "⚠️ Прервано." });
        this.state.uidList.push(this.state.nextUid());
      } else {
        showToast(`Ошибка: ${error}`, "error");
        this.state.history.push({ type: "message", author: "system", content: `⚠️ Ошибка: ${error}` });
        this.state.uidList.push(this.state.nextUid());
        void trackError("chat.send.runFrom", error);
      }
      this.renderChatFromHistory();
      await this.persistSession();
    } finally {
      this.setProcessingState(false);
    }
  }

  private async onDeleteThoughts(assistantUid: string | null, thoughtUids: string[]) {
    if (thoughtUids.length > 0) {
      const removeSet = new Set(thoughtUids);
      for (let i = this.state.history.length - 1; i >= 0; i--) {
        if (removeSet.has(this.state.uidList[i])) {
          this.state.history.splice(i, 1);
          this.state.uidList.splice(i, 1);
        }
      }
    } else if (assistantUid) {
      const idx = this.state.uidList.indexOf(assistantUid); if (idx === -1) return;
      if (this.state.history[idx].sub_calls) this.state.history[idx].sub_calls = [];
      let i = idx - 1;
      while (i >= 0 && this.state.history[i].type === 'thought') { this.state.history.splice(i, 1); this.state.uidList.splice(i, 1); i--; }
    } else {
      return;
    }
    this.renderChatFromHistory();
    await this.persistSession();
    showToast("Блок мыслей удален.", "success");
  }

  private async onCloneFromThoughts(assistantUid: string) {
    const idx = this.state.uidList.indexOf(assistantUid); if (idx === -1) return;
    let first = idx; let i = idx - 1;
    while (i >= 0 && this.state.history[i].type === 'thought') { first = i; i--; }
    const cloneIdx = first - 1; if (cloneIdx < 0) { showToast("Нельзя клонировать.", "error"); return; }
    const clonedHistory = this.state.history.slice(0, cloneIdx + 1);
    const newId = Date.now().toString();
    await saveSession(newId, clonedHistory, "", this.el.modelSelect?.value, this.el.agentSelect?.value);
    showToast("Клон сессии создан!", "success");
    bus.emit("session:changed");
    bus.emit("session:open", newId);
  }

  private appendMessageToContainer(container: HTMLDivElement, role: Role, content: string, agentName?: string, timeText?: string) {
    container.appendChild(createMessageElement(role, content, agentName, timeText));
    this.scrollToBottomIfNearEnd(container); renderMermaid();
  }

  appendMessage(role: Role, content: string, agentName?: string, timeText?: string, subCalls?: any[], skipSubcallRender = false, uid?: string, attachments?: Attachment[]) {
    this.state.activeThoughtsBlock = null;
    if (subCalls && subCalls.length > 0 && !skipSubcallRender) {
      const items = subCalls.map(call => createSubcallElement(call, (c) => this.showSubchat(c)));
      this.el.chatHistory.appendChild(createThoughtsBlock(items, uid, this.thoughtMenuCallbacks, []));
    }
    const hasMenu = uid !== undefined && (role === 'user' || role === 'agent');
    const msgEl = createMessageElement(role, content, agentName, timeText, hasMenu ? uid : undefined, hasMenu ? this.menuCallbacks : undefined, attachments, this.translatorMenuLabel());
    this.el.chatHistory.appendChild(msgEl); this.scrollToBottomIfNearEnd(this.el.chatHistory); renderMermaid();
  }

  private setEngineBadge(cls: string, text: string, title: string) {
    const b = this.el.engineBadge;
    if (!b) return;
    b.className = `engine-badge ${cls}`;
    b.innerText = text;
    b.title = title;
  }

  /** Начальное состояние бейджа по get_engine_status (до первого запроса) */
  private async refreshEngineBadgeInit() {
    let st: any;
    try {
      st = await getEngineStatus();
    } catch (_) {
      this.setEngineBadge("engine-none", "—", "Не удалось определить состояние движка");
      void trackError("chat.engineBadgeInit", _);
      return;
    }
    if (!st.installed) {
      this.setEngineBadge("engine-none", "Нет движка", "Движок llama.cpp не установлен. Откройте Настройки → «Движок запуска нейромоделей».");
      return;
    }
    const variant: string = st.cuda || "";
    if (variant.startsWith("cpu")) {
      this.setEngineBadge("engine-cpu", "CPU", `Бекенд: ${variant}. Модель работает на CPU.`);
      return;
    }
    // Физическая несовместимость (не зависит от выбора юзера — движок не запустится на GPU):
    const computeMajor = parseFloat(st.compute_cap ?? "");
    if (variant.startsWith("cuda-12") && computeMajor >= 12) {
      // Сборка cuda-12.x не содержит ядер Blackwell (sm_120, RTX 50xx)
      this.setEngineBadge("engine-error", "GPU ⚠", `Установлен движок ${variant}, но в нём нет ядер для вашей видеокарты (${st.gpu_name}, compute ${st.compute_cap}). Нужен бекенд ${st.required_variant || "cuda-13.x"}. Смените в Настройках → «Движок запуска нейромоделей».`);
      return;
    }
    if (variant.startsWith("cuda-13") && st.cuda_major > 0 && st.cuda_major < 13) {
      // Сборка cuda-13.x слинкована с cudart 13 — нужен драйвер R580+ / CUDA 13
      this.setEngineBadge("engine-error", "GPU ⚠", `Установлен движок ${variant}, но драйвер NVIDIA поддерживает только CUDA ${st.cuda_major}.${st.cuda_minor}. Обновите драйвер (нужна версия 580+ / CUDA 13) или выберите бекенд ${st.required_variant || "cuda-12.4"} в Настройках → «Движок запуска нейромоделей».`);
      return;
    }
    this.setEngineBadge("engine-idle", "GPU · …", `Бекенд: ${st.resolved_variant || variant}. Точный режим появится после первого запроса.`);
  }

  /** Обновление бейджа по событию engine_mode после каждого запроса */
  private updateEngineBadge(mode: string, tokPerSec: number, detail: string) {
    const tok = (Number.isFinite(tokPerSec) && tokPerSec > 0 && tokPerSec < 10_000)
      ? ` · ${Math.round(tokPerSec)} tok/s`
      : "";
    if (mode === "gpu") {
      this.setEngineBadge("engine-gpu", `GPU${tok}`, detail || "Модель работает в VRAM (GPU-ускорение)");
    } else {
      this.setEngineBadge("engine-cpu", `CPU${tok}`, detail || "Модель работает на процессоре (CPU)");
    }
  }

  renderChatFromHistory() {
    this.el.chatHistory.innerHTML = ''; this.state.activeThoughtsBlock = null;
    let thoughtsItems: HTMLElement[] = []; let thoughtsUids: string[] = []; let lastAssistantUid: string | undefined;
    for (let i = 0; i < this.state.history.length; i++) {
      const msg = this.state.history[i]; const uid = this.state.uidList[i];
      if (msg.type === 'thought') {
        const content = msg.content;
        const tool = this.parseToolThought(msg.content);
        if (tool) {
          thoughtsItems.push(createToolThoughtElement(msg.author || 'Система', tool.tool, tool.args, tool.result, true));
        } else {
          thoughtsItems.push(createThoughtElement(msg.author || 'Система', content, msg.time_sec));
        }
        thoughtsUids.push(uid);

        if (msg.sub_calls && msg.sub_calls.length > 0) {
          msg.sub_calls.forEach((call: any) => thoughtsItems.push(createSubcallElement(call, (c) => this.showSubchat(c))));
        }
        continue;
      }
      if (msg.type === 'message' && msg.author && msg.author !== 'user' && msg.author !== 'system' && msg.sub_calls && msg.sub_calls.length > 0) {
        lastAssistantUid = uid;
        thoughtsUids = [];
        msg.sub_calls.forEach(call => thoughtsItems.push(createSubcallElement(call, (c) => this.showSubchat(c))));
      }
      if (thoughtsItems.length > 0) {
        this.el.chatHistory.appendChild(createThoughtsBlock(thoughtsItems, lastAssistantUid, this.thoughtMenuCallbacks, thoughtsUids));
        thoughtsItems = []; thoughtsUids = []; lastAssistantUid = undefined;
      }
      const role = (msg.author && msg.author !== 'user') ? (msg.author === 'system' ? 'system' : 'agent') : 'user' as Role;
      const agentName = (msg.author && msg.author !== 'user' && msg.author !== 'system') ? msg.author : undefined;
      const hasMenu = uid && (role === 'user' || role === 'agent');
      const timeText = msg.time_sec ? `${msg.time_sec.toFixed(1)} сек` : undefined;
      // Скрываем служебные теги LLM в сохранённых ответах (defence in depth).
      const cleanContent = role === 'agent' ? stripStreamArtifacts(msg.content) : msg.content;
      this.el.chatHistory.appendChild(createMessageElement(role, cleanContent, agentName, timeText, hasMenu ? uid : undefined, hasMenu ? this.menuCallbacks : undefined, msg.attachments, this.translatorMenuLabel(), msg.model));
    }
    if (thoughtsItems.length > 0) this.el.chatHistory.appendChild(createThoughtsBlock(thoughtsItems, lastAssistantUid, this.thoughtMenuCallbacks, thoughtsUids));
    this.scrollToBottomIfNearEnd(this.el.chatHistory); renderMermaid();
  }

  /** Разбор сохранённой мысли-инструмента вида
   *  "🔧 Вызван инструмент WebSearch: {...}\nРезультат: ..." */
  private parseToolThought(content?: string): { tool: string; args: string; result: string } | null {
    if (!content || !content.startsWith("🔧 Вызван инструмент ")) return null;
    const resIdx = content.indexOf("\nРезультат: ");
    if (resIdx === -1) return null;
    const head = content.slice("🔧 Вызван инструмент ".length, resIdx);
    const sp = head.indexOf(": ");
    if (sp === -1) return null;
    return {
      tool: head.slice(0, sp),
      args: head.slice(sp + 2),
      result: content.slice(resIdx + "\nРезультат: ".length),
    };
  }

  private showSubchat(subCall: any) {
    this.el.viewChat.classList.remove('active'); this.el.viewSubchat.classList.add('active');
    this.el.subchatTitle.innerText = `Сабагент: ${subCall.agent_name}`;
    this.el.subchatHistory.innerHTML = '';
    this.appendMessageToContainer(this.el.subchatHistory, 'system', subCall.prompt, 'Отчет контекста');
    if (subCall.tool_calls) subCall.tool_calls.forEach((tc: any) => this.el.subchatHistory.appendChild(createToolCallElement(tc.tool_name, tc.arguments, tc.result)));
    if (subCall.thinking) {
      const thinkingEl = createThoughtElement(subCall.agent_name, subCall.thinking);
      this.el.subchatHistory.appendChild(thinkingEl);
    }
    this.appendMessageToContainer(this.el.subchatHistory, 'agent', subCall.response, subCall.agent_name, `${subCall.time_sec.toFixed(1)} сек`);
  }

  setProcessingState(state: boolean) {
    this.state.isProcessing = state;
    this.el.modelSelect.disabled = this.el.agentSelect.disabled = this.el.btnSend.disabled = state;
    this.el.btnStop.disabled = !state;
    if (state) {
        this.state.processingStartedAt = Date.now();
        this.state.lastActivityAt = Date.now();
        // Полный сброс стриминговых буферов перед новым прогоном.
        this.state.realtimeSubcallKeys.clear();
        this.state.activeThoughtsBlock = null;
        this.state.rtStreamUid = null;
        this.state.rtStreamBuffer = "";
        this.state.rtIsJson = false;
        this.state.rtThoughtUid = null;
        this.state.rtThoughtBuffer = "";
        this.state.rtThoughtAuthor = "";
        setActiveProcessingChat(this);
        store.isProcessing = true;
        this.armProcessingWatchdog();;
        this.el.chatFeedback.style.display = "block";
        this.el.progressBar.style.width = "0%";
        this.el.statusLabel.innerText = "Подготовка...";
    }
    else {
        // Полный сброс «рабочего» состояния: чат больше не в процессе обработки —
        // скрываем панель прогресса и возвращаем статус/прогресс к дефолту, чтобы
        // UI не оставался в застывшем виде («Загрузка модели...», прогресс 10%).
        this.disarmProcessingWatchdog();
        releaseGlobalProcessing(this);
        this.el.chatFeedback.style.display = "none";
        this.el.progressBar.style.width = "0%";
        this.el.statusLabel.innerText = "Обработка...";
    }
    bus.emit("processing:changed", state);
  }

  /** Арм таймера-сторожа: периодически проверяет, движется ли обработка. */
  private armProcessingWatchdog() {
    this.disarmProcessingWatchdog();
    this.processingWatchdog = window.setTimeout(() => this.onProcessingWatchdog(), PROCESSING_CHECK_MS);
  }

  private disarmProcessingWatchdog() {
    if (this.processingWatchdog !== null) {
      clearTimeout(this.processingWatchdog);
      this.processingWatchdog = null;
    }
  }

  /** Сторож зависшей обработки: если движок молчит дольше лимита — разблокируем UI. */
  private onProcessingWatchdog() {
    this.processingWatchdog = null;
    if (!this.state.isProcessing) return;
    if (Date.now() - this.state.lastActivityAt > PROCESSING_IDLE_LIMIT_MS) {
      logFront(`[chat] Watchdog: движок молчит ${PROCESSING_IDLE_LIMIT_MS / 1000} сек — обработка остановлена`);
      this.setProcessingState(false);
      showToast("⏱ Движок молчал более 15 минут — обработка остановлена.", "error");
      void trackError("chat.watchdog.timeout", new Error(`движок молчал ${Date.now() - this.state.lastActivityAt} мс`));
    } else {
      this.armProcessingWatchdog();
    }
  }

  /** Сохранение текущей сессии с привязкой выбранных модели и агента. */
  private persistSession(draft?: string): Promise<void> {
    const id = this.state.sessionId;
    if (!id) return Promise.resolve();
    const model = this.el.modelSelect?.value || undefined;
    const agent = this.el.agentSelect?.value || undefined;
    const d = draft !== undefined ? draft : this.el.chatInput.value;
    return saveSession(id, this.state.history, d, model, agent);
  }

  private hasOption(select: HTMLSelectElement | null, value: string): boolean {
    if (!select) return false;
    return Array.from(select.options).some(o => o.value === value);
  }

  /// Гарантирует наличие ID сессии (создаёт при первом обращении).
  private ensureSessionId(): string {
    if (!this.state.sessionId) {
      this.state.sessionId = Date.now().toString();
    }
    return this.state.sessionId;
  }

  /// Конвертация вкладки в чат-режим (убрать main-экран). Вызывается ТОЛЬКО
  /// по факту отправки сообщения либо открытия сессии, не при наборе черновика.
  private becomeChatTab() {
    this.state.isChatTab = true;
    this.hooks.onBecameChat?.(this.state.sessionId ?? "");
  }

  /// Восстановление черновика на МАIN-вкладке: подгружает сессию, но НЕ
  /// конвертирует вкладку в чат (welcome + прижатая плашка остаются).
  async openSessionDraft(id: string) {
    if (this.state.isProcessing) return;
    try {
      const session = await loadSession(id);
      this.state.sessionId = session.id ?? id;
      if (session.model && this.hasOption(this.el.modelSelect, session.model)) {
        this.el.modelSelect.value = session.model;
        bus.emit("model:changed", session.model);
      }
      if (session.agent && this.hasOption(this.el.agentSelect, session.agent)) {
        this.el.agentSelect.value = session.agent;
      }
      if (session.draft) {
        this.el.chatInput.value = session.draft;
        this.el.chatInput.style.height = "auto";
        this.el.chatInput.style.height = `${this.el.chatInput.scrollHeight}px`;
      }
      this.updateAttachButtonState();
      this.triggerTokenCount();
    } catch (e) {
      void trackError("chat.openSessionDraft", e);
    }
  }

  async handleSend() {
    const text = this.el.chatInput.value.trim(); if (!text && this.attachments.length === 0) return;
    if (store.isProcessing || this.state.isProcessing) return;
    const activeAgent = this.el.agentSelect.value; const modelPath = this.el.modelSelect.value;
    if (!modelPath) { showToast("Выберите модель!", "error"); return; }
    const userUid = this.state.nextUid(); this.state.uidList.push(userUid);
    const displayText = text || (this.attachments.length > 0 ? `[${this.attachments.length} файлов]` : '');
    this.appendMessage('user', displayText, undefined, undefined, undefined, false, userUid, this.attachments);
    this.el.chatInput.value = ""; this.el.chatInput.style.height = "auto"; clearTimeout(this.state.draftTimeout);
    this.el.filePreview.innerHTML = '';
    const attachments = [...this.attachments];
    this.attachments = [];
    this.setProcessingState(true);
    this.ensureSessionId();
    // Вкладка ещё main-режима — конвертируем в чат именно при отправке,
    // а не на первом набранном символе (чтобы плашка ввода не «прыгала»).
    if (!this.state.isChatTab) this.becomeChatTab();
    const historyBefore = this.state.history.length;
    void historyBefore;
    this.state.history.push({ type: "message", author: "user", content: displayText, attachments });
    await this.persistSession("");
    bus.emit("session:changed");
    const startTime = performance.now();
    try {
      const _p2 = store.currentModelParams;
      const params = { temperature: parseFloat(this.el.tempSlider.value), top_k: parseInt(this.el.topkSlider.value, 10), top_p: parseFloat(this.el.toppSlider.value), min_p: parseFloat(this.el.minpSlider.value), repetition_penalty: parseFloat(this.el.reppenSlider.value), presence_penalty: parseFloat(this.el.prespenSlider.value), dry_multiplier: _p2?.dry_multiplier ?? 0.0, dry_base: _p2?.dry_base ?? 1.75, dry_allowed_length: _p2?.dry_allowed_length ?? 2, dry_penalty_last_n: _p2?.dry_penalty_last_n ?? 0, xtc_probability: _p2?.xtc_probability ?? 0.0, xtc_threshold: _p2?.xtc_threshold ?? 0.1 };
      const allHistory = this.state.history.slice();
      let mmprojPath: string | null = null;
      if (attachments && attachments.length > 0) {
        try { mmprojPath = await getMmprojPath(modelPath); } catch (_) {}
      }
      const isNineRouter = modelPath.startsWith(NINE_ROUTER_MODEL_PREFIX);
      const response: any = isNineRouter
        ? await this.nineRouterTurn(modelPath.slice(NINE_ROUTER_MODEL_PREFIX.length), activeAgent, text, allHistory)
        : await invoke("chat_request", {
            modelPath,
            agentId: activeAgent,
            message: text,
            history: allHistory,
            contextSize: store.contextSize,
            promptTokens: this.lastPromptTokens,
            maxGenTokens: parseInt(this.el.maxGenSlider.value, 10),
            kvQuantKeys: false,
            kvQuantValues: false,
            modelParams: params,
            attachments,
            mmprojPath,
            sessionId: this.state.sessionId ?? ""
        });
      if (response?.has_error) {
        void trackError("chat.send.outcome", response.has_error);
      }
      const dur = ((performance.now() - startTime) / 1000);
      const newMessages = response.messages || [];
      console.log("DBG chat_request -> newMessages.len=", newMessages.length, "newMessages=", JSON.stringify(newMessages.map((m: any) => ({ t: m.type, a: m.author }))), "response.text?", !!response.text);
      // Мысли стриминга: теперь приходят в response.messages от backend.
      if (newMessages.length > 0) {
        newMessages.forEach((m: any) => {
            if (m.author && m.author !== 'user' && m.author !== 'system') {
                m.time_sec = dur;
            }
        });
        this.state.history = [...newMessages];
        this.state.uidList = this.state.history.map(() => this.state.nextUid());
      }
      const hasRT = response.sub_calls && response.sub_calls.some((c: any) => this.state.realtimeSubcallKeys.has(`${c.agent_name}:${c.time_sec.toFixed(2)}`));
      void hasRT;
      if (response.text || newMessages.length > 0) {
        this.renderChatFromHistory();
      }
      // Защита от потери ответа: ищем последнее СООБЩЕНИЕ (не мысль) от агента.
      const lastMessage = [...this.state.history].reverse().find((m: any) => m.type === 'message');
      const hasFinal = lastMessage && lastMessage.author === activeAgent;
      if (response.text && !hasFinal) {
        this.state.history.push({
          type: "message",
          author: activeAgent,
          content: response.text,
          sub_calls: (response.sub_calls && response.sub_calls.length) ? response.sub_calls : undefined,
        });
        this.state.uidList.push(this.state.nextUid());
        this.renderChatFromHistory();
      }
      await this.persistSession();
    } catch (error) {
      if (String(error).includes("Отменено") || String(error).includes("Прервано")) {
          this.state.history.push({ type: "message", author: "system", content: "⚠️ Прервано." });
          this.state.uidList.push(this.state.nextUid());
      } else {
          showToast(`Ошибка: ${error}`, "error");
          this.state.history.push({ type: "message", author: "system", content: `⚠️ Ошибка: ${error}` });
          this.state.uidList.push(this.state.nextUid());
          void trackError("chat.send", error);
      }
      this.renderChatFromHistory();
      await this.persistSession();
    } finally {
        this.setProcessingState(false);
        this.triggerTokenCount();
    }
  }

  /**
   * Облачный прогон комбо 9Router. История чата уже содержит сообщение
   * пользователя; конвертируем её в OpenAI-формат и стримим ответ через
   * плагин tauri-plugin-9router. Чанки приходят событием `9router-chunk`
   * (переиспользует общий рендер handleStreamChunk). Возвращаем результат в
   * форме, совместимой с локальным `chat_request` ({ text, messages }).
   */
  private async nineRouterTurn(
    comboName: string,
    activeAgent: string,
    text: string,
    history: any[],
  ): Promise<{ text: string; messages: any[] }> {
    const messages = history
      .filter((m: any) => m.type === "message")
      .map((m: any) => ({
        role: m.author === "user" ? "user" : m.author === "system" ? "system" : "assistant",
        content: typeof m.content === "string" ? m.content : "",
      }))
      .filter((m: any) => m.content);
    if (!messages.length || messages[messages.length - 1].content !== text) {
      messages.push({ role: "user", content: text });
    }
    bus.emit("log", `☁️ 9Router → комбо «${comboName}»`);
    const full = (await nineRouterChat({ model: comboName, messages, author: activeAgent })) ?? "";
    return { text: full, messages: [] };
  }

  private triggerDraftSave() {
    if (!this.state.sessionId && this.el.chatInput.value.trim() !== "") {
      // Черновик на пустой (main) вкладке: сессия создаётся и персистится,
      // но вкладка НЕ конвертируется в чат — welcome и прижатая к низу плашка
      // остаются до фактической отправки.
      const sid = this.ensureSessionId();
      this.hooks.onDraftSession?.(sid);
      this.persistSession().then(() => bus.emit("session:changed"));
    } else if (this.state.sessionId) {
      clearTimeout(this.state.draftTimeout);
      this.state.draftTimeout = window.setTimeout(() => { this.persistSession(); }, 500);
    }
  }

  /// Открыть существующую сессию в ЧАТ-вкладке (с конвертацией в чат-режим).
  async openSession(id: string) {
    if (this.state.isProcessing) return;
    try {
      const session = await loadSession(id);
      // Единый источник правды: ID сессии из файла (совпадает с ключом вкладки).
      this.state.sessionId = session.id ?? id;
      this.state.history = session.messages;
      this.state.uidCounter = 0; this.state.uidList = this.state.history.map(() => this.state.nextUid());
      this.el.chatInput.value = session.draft || "";
      setTimeout(() => { this.el.chatInput.style.height = "auto"; this.el.chatInput.style.height = `${this.el.chatInput.scrollHeight}px`; }, 0);
      if (session.model && this.hasOption(this.el.modelSelect, session.model)) {
        this.el.modelSelect.value = session.model;
        bus.emit("model:changed", session.model);
      }
      if (session.agent && this.hasOption(this.el.agentSelect, session.agent)) {
        this.el.agentSelect.value = session.agent;
      }
      this.renderChatFromHistory();
      bus.emit("session:changed");
      this.becomeChatTab();
      this.updateAttachButtonState();
      this.triggerTokenCount();
    } catch(e) {
      showToast(`Ошибка: ${e}`, "error");
      void trackError("chat.openSession", e);
    }
  }

  private async updateAttachButtonState() {
    const btn = this.el.btnAttach;
    const modelPath = this.el.modelSelect?.value;
    if (!modelPath) { btn.disabled = true; this.modelAudioCapable = false; btn.classList.remove('btn-attach-active'); btn.classList.add('btn-attach-inactive'); btn.title = 'Сначала выберите модель'; return; }
    // Облачные комбо 9Router не принимают локальные вложения (mmproj/аудио).
    if (modelPath.startsWith(NINE_ROUTER_MODEL_PREFIX)) {
      btn.disabled = true;
      this.modelAudioCapable = false;
      btn.classList.remove('btn-attach-active');
      btn.classList.add('btn-attach-inactive');
      btn.title = 'Облачное комбо 9Router: вложения недоступны';
      return;
    }
    let mmprojPath: string | null = null;
    try { mmprojPath = await getMmprojPath(modelPath); } catch (_) {}
    if (!mmprojPath) {
      // Модель могла быть добавлена вручную без mmproj — докачиваем по каталогу.
      btn.disabled = true;
      this.modelAudioCapable = false;
      btn.classList.remove('btn-attach-active');
      btn.classList.add('btn-attach-inactive');
      btn.title = 'Скачивание mmproj для мультимодального режима…';
      try { mmprojPath = await ensureMmproj(modelPath); } catch (_) {}
    }
    if (mmprojPath) {
      // Аудио разрешаем только моделям с реальной audio-способностью
      // (читаем из models_catalog.json живьём), иначе отправка аудио
      // ведёт к зависанию генерации (напр. Gemma-4 не поддерживает звук в llama.cpp).
      let audioCapable = false;
      try {
        const caps = await getModelCapabilities(modelPath);
        audioCapable = !!caps.audio;
      } catch (_) { audioCapable = false; }
      this.modelAudioCapable = audioCapable;
      btn.disabled = false;
      btn.classList.remove('btn-attach-inactive');
      btn.classList.add('btn-attach-active');
      btn.title = audioCapable ? 'Прикрепить файл (изображение/аудио)' : 'Прикрепить изображение';
    } else {
      this.modelAudioCapable = false;
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
      if (file.type.startsWith("audio/") && !this.modelAudioCapable) {
        showToast(`Модель не поддерживает аудио — прикрепление звука отключено`, "error");
        continue;
      }
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
    this.el.chatInput?.addEventListener("keydown", (e) => { if (e.key === "Enter" && e.ctrlKey) { e.preventDefault(); this.handleSend(); } });
    this.el.btnStop?.addEventListener("click", async () => { if (this.el.btnStop.disabled) return; this.el.btnStop.disabled = true; this.el.btnStop.innerText = "Стоп..."; this.appendMessage('system', 'Остановка...'); await invoke("stop_processing"); this.el.btnStop.innerText = "⏹ Стоп"; });
    this.el.btnBackChat?.addEventListener("click", () => { this.el.viewSubchat.classList.remove('active'); this.el.viewChat.classList.add('active'); });
    this.el.chatInput.addEventListener("input", () => {
        this.el.chatInput.style.height = "auto";
        this.el.chatInput.style.height = `${this.el.chatInput.scrollHeight}px`;
        this.triggerDraftSave();
        this.triggerTokenCount();
    });
    this.el.chatInput.addEventListener("blur", () => { if (this.state.hasSession()) { clearTimeout(this.state.draftTimeout); this.persistSession(); } });
    this.el.btnAttach?.addEventListener("click", () => { if (!this.el.btnAttach.disabled) this.el.fileInput.click(); });
    this.el.fileInput?.addEventListener("change", (e) => this.handleFileSelect((e.target as HTMLInputElement).files));
    this.el.modelSelect?.addEventListener("change", () => {
      bus.emit("model:changed", this.el.modelSelect.value);
      this.updateAttachButtonState(); this.triggerTokenCount();
      const v = this.el.modelSelect.value;
      void invoke("set_last_model", { path: v }).catch(() => {});
      if (this.state.hasSession()) this.persistSession();
    });
    this.el.agentSelect?.addEventListener("change", () => {
      this.triggerTokenCount();
      const v = this.el.agentSelect.value;
      void invoke("set_config_value", { key: "last_agent", value: v }).catch(() => {});
      if (this.state.hasSession()) this.persistSession();
    });
    this.el.btnSetWorkdir?.addEventListener("click", async () => {
      try {
        const dir = await openDialog({ directory: true });
        if (dir) {
          store.workdir = dir as string;
          this.el.currentWorkdir.textContent = dir as string;
          this.el.currentWorkdir.title = dir as string;
          await invoke("set_config_value", { key: "workdir", value: dir });
          showToast(`Рабочая директория: ${dir}`, "success");
        }
      } catch (e) { showToast(`Ошибка: ${e}`, "error"); void trackError("chat.setWorkdir", e); }
    });
  }

  private bindBusEvents() {
    bus.on("config:loaded", (config: any) => {
      // Общие каталоги уже в store (заполнил SettingsController) — перерисовываем
      // селекты этой вкладки, сохраняя текущий выбор.
      fillModelSelect(this.el.modelSelect);
      fillAgentSelect(this.el.agentSelect);
      this.updateAttachButtonState();
      this.triggerTokenCount();
      if (config.workdir) {
        store.workdir = config.workdir;
        this.el.currentWorkdir.textContent = config.workdir;
        this.el.currentWorkdir.title = config.workdir;
      }
    });
  }
}