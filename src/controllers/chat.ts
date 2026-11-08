import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { store } from "../store";
import { bus } from "../events";
import { formatDuration } from "../utils/duration";
import { createMessageElement, createSubcallElement, createToolCallElement, createThoughtElement, createThoughtRow, updateThoughtRowSummary, completeToolRow, TOOL_ICONS, showToast, createProcessingIndicator, createAttachmentCallbacks, materializeNeurosisCases } from "../ui";
import type { Role, MessageMenuCallbacks, ImageAttachmentCallbacks, ProcessingIndicator } from "../ui";
import type { ThoughtMenuCallbacks, Attachment, AttachmentMetadata, ChatMessage, DragDropPayload } from "../types";
import { saveSession, loadSession, countTokens } from "../services";
import { getEngineStatus, getMmprojPath, ensureMmproj, getModelCapabilities, getModelsCatalog, getModelContextSize, estimatePromptMemory, type CatalogEntry } from "@my-tauri-plugins/plugin-llama-engine";
import { renderMarkdown, stripStreamArtifacts, extractChannelThought, visibleStreamText, CLOUD_ROUTER_PREFIXES, isCloudModelPath, fillModelSelect, syncModelSelectTitle, fillAgentSelect, getAgentDisplayName, getSelectedAgentName, isGraphAgent, hasFusableGroups } from "../utils";
import { logFront } from "@my-tauri-plugins/plugin-logs";
import { trackError, trackEvent } from "../telemetry";

let mermaidPromise: Promise<typeof import("mermaid")> | null = null;
let mermaidInitialized = false;

/// Watchdog зависшей обработки: если движок молчит дольше лимита, а обработка
/// формально идёт — снимаем состояние (кнопки/стоп возвращаются юзеру).
const PROCESSING_IDLE_LIMIT_MS = 15 * 60 * 1000; // 15 минут тишины движка
const PROCESSING_CHECK_MS = 30 * 1000;           // проверка раз в 30 сек

/// Троттлинг стрим-рендера (01.11.26): каждый SSE-чанк копит буфер, а полный
/// ре-рендер markdown всего накопленного текста выполняется не чаще раза в
/// этот интервал. Без троттлинга длинный ответ перерисовывался целиком на
/// КАЖДЫЙ токен — тысячи полных renderMarkdown + forced layout за ход.
const STREAM_RENDER_INTERVAL_MS = 120;

/// Окно рендера истории чата (01.11.26): в DOM живёт не больше этого числа
/// сообщений (старшие — по кнопке «Показать ещё»). `state.history` и
/// persistSession не режутся — ограничивается только DOM, переполнявший
/// память WebView2-рендерера на длинных сессиях.
const CHAT_RENDER_WINDOW = 60;

/// Названия целевых языков для переводчика сообщений.
/// `menu` — текст пункта контекстного меню, `footer` — подпись внутри сообщения.
const TRANSLATOR_LANGS: Record<string, { menu: string; footer: string }> = {
  ru: { menu: "Перевести на русский язык", footer: "перевод на русский язык:" },
  en: { menu: "Перевести на английский язык", footer: "перевод на английский язык:" },
};

async function renderMermaid() {
  try {
    if (!mermaidPromise) mermaidPromise = import("mermaid");
    const { default: mermaid } = await mermaidPromise;
    if (!mermaidInitialized) {
      mermaid.initialize({ startOnLoad: false, theme: "dark", securityLevel: "loose" });
      mermaidInitialized = true;
    }
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
// раз (initChatEventRouter, см. src/controllers/chat-router.ts), и каждое
// событие несёт sessionId в payload — роутер доставляет его в контроллер
// вкладки ЭТОЙ сессии. Обработка параллельна: облачные ходы вкладок идут
// одновременно, локальные встают в очередь на стороне бэкенда (гейт движка),
// поэтому синглтон «активной обработки» здесь больше не нужен.
// ─────────────────────────────────────────────────────────────────────────────

export interface ChatElements {
  chatHistory: HTMLDivElement;
  chatInput: HTMLTextAreaElement;
  btnSend: HTMLButtonElement;
  btnStop: HTMLButtonElement;
  agentSelect: HTMLSelectElement;
  /** Тумблер режима исполнения графа (`Fused` = checked); заблокирован без групп. */
  strategyToggle: HTMLInputElement;
  btnAgentSettings: HTMLButtonElement;
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
  cloudCounter: HTMLDivElement;
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
 * историю, черновик и стриминг. Флаг `isProcessing` — per-tab: обработка
 * разных вкладок параллельна (очередь локального движка — в бэкенде).
 */
export class ChatTabState {
  sessionId: string | null = null;
  history: ChatMessage[] = [];
  uidList: string[] = [];
  uidCounter = 0;

  /** Ключи завершённых realtime-сабколлов (дедуп при перерисовке). */
  realtimeSubcallKeys = new Set<string>();

  /** Индикатор обработки в потоке чата (спиннер + статус + таймер).
   *  Живёт только пока идёт обработка этой вкладки. */
  processingIndicator: ProcessingIndicator | null = null;

  /// Для watchdog зависшей обработки.
  processingStartedAt = 0;
  lastActivityAt = 0;

  /** Таймер автосейва черновика (персист раз в 500мс). */
  draftTimeout: number | undefined;

  /// Вкладка уже показана как чат (main-экран снят). Ставится при реальной
  /// конвертации main → chat (отправка) либо при открытии сессии.
  isChatTab = false;

  /// In-memory `history` отражает ПОЛНОЕ содержимое сессии и авторитетна для
  /// предпросмотра. Ставится при загрузке сессии (openSession) либо при
  /// конвертации в чат (becomeChatTab — история начинает строиться с нуля).
  /// ЛОЖЬ у восстановленной при старте вкладки: контроллер уже зарегистрирован
  /// в TAB_REGISTRY, но история лениво грузится при первой активации
  /// (activateSession/sessionLoadStarted) — до этого in-memory history пуста и
  /// НЕ является правдой об улице диска (фикс лжи «нет сообщений» в предпросмотре).
  historyLoaded = false;

  // Стриминг текста
  rtStreamUid: string | null = null;
  rtStreamBuffer: string = "";
  rtIsJson: boolean = false;
  /// Автор стримящегося сообщения (для честной пометки при обрыве: частичный
  /// текст сохраняется в историю от имени того, кто его писал).
  rtStreamAuthor: string = "";

  // Стриминг мыслей
  rtThoughtUid: string | null = null;
  rtThoughtBuffer: string = "";
  rtThoughtAuthor: string = "";

  /// Флаг «агент может emit'ить тул-конверт» из последнего чанка (нужен
  /// trailing-рендеру, чтобы пересчитать видимый текст из буфера).
  rtExpectTool: boolean = false;

  // Троттлинг стрим-рендера (01.11.26): отложенный trailing-рендер по
  // последнему буферу + момент последнего фактического рендера.
  streamRenderTimer: number | null = null;
  streamLastRenderAt = 0;
  streamTrailingRender: (() => void) | null = null;

  /** Дополнительные старые сообщения, раскрытые кнопкой «Показать ещё»
   *  (поверх окна CHAT_RENDER_WINDOW). Сбрасывается при новом ходе —
   *  окно снова следует за концом истории. */
  renderExpandExtra = 0;

  /** Отменяет отложенный trailing-рендер стрима (сброс таймера и замыкания). */
  cancelStreamRender() {
    if (this.streamRenderTimer !== null) {
      clearTimeout(this.streamRenderTimer);
      this.streamRenderTimer = null;
    }
    this.streamTrailingRender = null;
  }

  isRestoringSession = false;

  /**
   * Режим исполнения графа для ЭТОЙ вкладки: `detailed` | `fused`.
   *
   * Третьего состояния нет: ВЫКЛ тумблера = подробное исполнение, и это
   * отправляется в бэкенд как явный выбор, а не как «как в файле». У агента
   * без схлопываемых групп состояние принудительно `detailed` (переключать
   * нечего — значит «неопределено» равносильно «подробно»).
   * Хранится в состоянии вкладки, а не в Store: переключение относится к
   * разговору, и в соседней вкладке оно должно остаться прежним.
   */
  executionStrategy: "detailed" | "fused" = "detailed";

  /** Идёт ли обработка ИМЕННО в этой вкладке. */
  isProcessing = false;
  cloudRequests = 0;

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
    this.rtStreamUid = null;
    this.rtStreamBuffer = "";
    this.rtIsJson = false;
    this.rtStreamAuthor = "";
    this.rtThoughtUid = null;
    this.rtThoughtBuffer = "";
    this.rtThoughtAuthor = "";
    this.rtExpectTool = false;
    this.cancelStreamRender();
    this.streamLastRenderAt = 0;
    this.renderExpandExtra = 0;
    this.isRestoringSession = false;
    this.historyLoaded = false;
    this.isProcessing = false;
    // Сброс состояния в обход setProcessingState — индикатор обязан уйти
    // вместе с обработкой, иначе строка останется в чате навсегда.
    this.processingIndicator?.destroy();
    this.processingIndicator = null;
    this.cloudRequests = 0;
    this.processingStartedAt = 0;
    this.lastActivityAt = 0;
  }
}

export class ChatController {
  readonly state = new ChatTabState();
  private el: ChatElements;
  private hooks: ChatHooks;
  private menuCallbacks: MessageMenuCallbacks;
  private imageCallbacks: ImageAttachmentCallbacks;
  private thoughtMenuCallbacks: ThoughtMenuCallbacks;
  private attachments: Attachment[] = [];
  private modelAudioCapable: boolean = false;
  private countTimer: number | null = null;
  private lastPromptTokens: number = 0;
  /// Какая ветка токен-счёта рисовалась последней (облако/локальная) — для аналитики смены режима.
  private lastTokenModeCloud: boolean | null = null;
  private processingWatchdog: number | null = null;
  /// Кэш каталога моделей плагина (для tokenizer_id в счётчике токенов).
  private catalogCache: CatalogEntry[] | null = null;
  /// Кэш окна модели из GGUF (04.11.26, вариант B): лимит счётчика чата — окно
  /// модели, а не невидимый config.context_size. Ключ — путь модели.
  private modelWindowCache = new Map<string, number>();
  private thoughtDedupSet = new Set<string>();
  private pendingSessionModel: string | null = null;
  private pendingSessionAgent: string | null = null;

  /**
   * Лимит для счётчика чата: окно модели из GGUF (extract_context_length),
   * кэш по пути модели; fallback — прежний store.contextSize (битый gguf,
   * облачная модель, ошибка чтения). Фактический слот после RAM-гейта
   * доносится юзеру плашкой vram_notice (Шаг B), здесь — потолок модели.
   */
  private async resolveContextLimit(modelPath: string | undefined): Promise<number> {
    if (!modelPath || isCloudModelPath(modelPath)) return store.contextSize;
    const cached = this.modelWindowCache.get(modelPath);
    if (cached !== undefined) return cached;
    try {
      const window = await getModelContextSize(modelPath);
      if (window > 0) {
        this.modelWindowCache.set(modelPath, window);
        return window;
      }
    } catch (e) {
      logFront(`[chat] окно модели недоступно (${modelPath}): ${e}`);
    }
    return store.contextSize;
  }

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
    };
    // Операции над вложениями — общий SSOT-модуль ui/attachment-actions.ts
    // (одна копия с SessionController, whence-строки трекинга прежние).
    this.imageCallbacks = createAttachmentCallbacks();
    this.thoughtMenuCallbacks = {
      onDeleteThoughts: (uid, uids) => this.onDeleteThoughts(uid, uids),
      onCloneFromThoughts: (uid) => this.onCloneFromThoughts(uid),
    };
    this.bindDomEvents();
    this.bindStrategyToggle();
    this.bindBusEvents();
    this.refreshSendAvailability();
    setTimeout(() => {
      this.updateAttachButtonState();
      this.triggerTokenCount();
      this.refreshEngineBadgeInit();
      this.refreshSendAvailability();
    }, 100);
  }

  /** Текущее значение выбора модели (для SettingsController при загрузке параметров). */
  get modelSelectValue(): string | null {
    return this.el.modelSelect?.value || null;
  }

  // ── Публичный API для роутера событий (src/controllers/chat-router.ts) ──

  onProgress(_pct: number) {
    // Проценты больше не отображаются: полосы 10/20/60/80 были фейковыми и
    // между событиями замирали, выглядя как зависание (особенно в очереди
    // облачного провайдера). Событие остаётся heartbeat'ом сторожа —
    // status и progress всегда приходят парой (api/chat.rs, status_cb).
    this.state.lastActivityAt = Date.now();
  }

  onStatus(text: string) {
    this.state.lastActivityAt = Date.now();
    const ind = this.state.processingIndicator;
    if (ind) {
      ind.setStatus(text);
      // Строка могла быть скрыта стримом прошлого агента — новый статус
      // означает новый шаг/агента графа, показываем снова.
      ind.show(true);
    }
  }

  onStreamChunk(rawPayload: any) {
    // Бэкенд шлёт объект { kind, author, text }; для обратной совместимости
    // (старые вызовы) payload может прийти строкой — считаем его message.
    const payload = rawPayload;
    const kind: string = typeof payload === "string" ? "message" : (payload.kind || "message");
    const chunk: string = typeof payload === "string" ? payload : (payload.text || "");
    const author: string = (payload && typeof payload.author === "string") ? payload.author : "";

    // ── МЫСЛИ: компактная строка-действие в чате, живой стрим ──
    if (kind === "thought") {
      // Новый агент → новая строка
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
        // Таргетинг строки — только по уникальному data-msg-uid (как у стрима
        // сообщений). Фиксированный id ("rt-thought-stream") при нескольких
        // агентах графа дублировался, querySelector брал ПЕРВУЮ строку в DOM
        // (психосоматолог — первый думающий агент), и чужие мысли рисовались
        // в его пузыре (кейс валидатора 03.11.2026).
        const row = createThoughtRow({
          kind: "thought",
          author: label,
          summary: this.state.rtThoughtBuffer,
          detail: this.state.rtThoughtBuffer,
          uid: this.state.rtThoughtUid,
        });
        this.el.chatHistory.appendChild(row);
        this.scrollToBottomIfNearEnd(this.el.chatHistory);
      } else {
        // Обновление строки мысли — через троттлинг: каждый чанк дёргал DOM
        // (updateThoughtRowSummary + forced layout в скролле) (01.11.26).
        this.scheduleStreamRender(() => this.renderStreamThoughtDom());
      }
      return;
    }

    // ── СООБЩЕНИЕ: старая логика основного чата ──
    // Шаг 7 (план 01.11.26, D1): флаг «агент может emit'ить тул-конверт»
    // приходит из бэкенда (has_tools_for_prompt в payload stream_chunk).
    const expectTool: boolean = typeof payload === "object" && payload?.expect_tool_call === true;
    this.state.rtExpectTool = expectTool;
    this.state.rtStreamBuffer += chunk;

    // Механическая резка конверта: всё от первой ```json-оградки в ЛЮБОМ месте
    // буфера — служебное, не рендерится; преамбула («Сейчас сделаю…») остаётся.
    const visibleSource = visibleStreamText(this.state.rtStreamBuffer, expectTool);
    if (expectTool && visibleSource !== this.state.rtStreamBuffer) {
      this.state.rtIsJson = true;
    }

    // Если это внутренний JSON (вызов сабагента или тулзы) — скрываем от глаз пользователя!
    // (частный случай: конверт с самого начала; для агентов без тулов сохраняется)
    if (visibleSource.trimStart().startsWith("{") || visibleSource.trimStart().startsWith("```json")) {
      this.state.rtIsJson = true;
      return;
    }

    // Видимый текст ответа пошёл — растущий текст сам индикатор жизни,
    // строку «спиннер + статус» гасим (вернётся со следующим статусом).
    this.state.processingIndicator?.show(false);

    // Если это начало ответа, создаем пустое сообщение в UI
    if (!this.state.rtStreamUid) {
      this.state.rtStreamUid = this.state.nextUid();
      const displayName = (author && author.trim())
        ? author
        : getSelectedAgentName(this.el.agentSelect);
      this.state.rtStreamAuthor = displayName;
      this.appendMessage('agent', '', displayName, undefined, undefined, false, this.state.rtStreamUid);
    }

    // Обновляем тело сообщения ЧЕРЕЗ ТРОТЛИНГ (01.11.26): полный renderMarkdown
    // всего накопленного текста не чаще раза в STREAM_RENDER_INTERVAL_MS,
    // последний буфер добирается trailing-рендером. Раньше каждый чанк
    // перерисовывал весь ответ заново + forced layout в скролле.
    this.scheduleStreamRender(() => this.renderStreamMessageDom());
  }

  /** Полный DOM-рендер стримящегося сообщения по актуальному буферу
   *  (вызывается троттлингом: leading либо trailing). */
  private renderStreamMessageDom() {
    if (!this.state.rtStreamUid) return;
    const visibleSource = visibleStreamText(this.state.rtStreamBuffer, this.state.rtExpectTool);
    // Буфер мог вырасти в служебный JSON уже после leading-рендера —
    // пересчёт в момент рендера честно скрывает его.
    if (visibleSource.trimStart().startsWith("{") || visibleSource.trimStart().startsWith("```json")) {
      return;
    }
    // Скрываем служебные теги LLM (<|channel>...<channel|> и <|turn>) перед
    // подачей в renderMarkdown (теги могут прийти внутри чанка).
    const visibleText = stripStreamArtifacts(visibleSource);
    const msgEl = this.el.chatHistory.querySelector(`[data-msg-uid="${this.state.rtStreamUid}"]`);
    if (msgEl) {
      const contentDiv = msgEl.querySelector('div:nth-child(2)');
      if (contentDiv) {
        contentDiv.innerHTML = renderMarkdown(visibleText);
      }
    }

    // Печатаем "Мысли" агента (формат <|channel>thought>...) отдельной строкой
    // перед пузырём стримящегося ответа. Уникальный uid на сообщение стрима:
    // фиксированный id дублировался между агентами и обновления уходили в
    // первую строку DOM (кейс психосоматолога 03.11.2026).
    const channelThought = extractChannelThought(visibleSource);
    if (channelThought && msgEl) {
      const channelUid = `channel-thought-${this.state.rtStreamUid}`;
      let row = this.el.chatHistory.querySelector(`[data-msg-uid="${channelUid}"]`);
      if (!row) {
        row = createThoughtRow({ kind: "thought", author: "Агент", summary: channelThought, detail: channelThought, uid: channelUid });
        this.el.chatHistory.insertBefore(row, msgEl);
      } else {
        updateThoughtRowSummary(row as HTMLDivElement, channelThought);
      }
    }
    this.scrollToBottomIfNearEnd(this.el.chatHistory);
  }

  /** Обновление строки стримящейся мысли по актуальному буферу (троттлинг). */
  private renderStreamThoughtDom() {
    if (!this.state.rtThoughtUid) return;
    // Только по уникальному uid своей строки: фиксированный id при нескольких
    // агентах дублировался, и обновления рисовались в строке ПЕРВОГО агента.
    const row = this.el.chatHistory.querySelector(`[data-msg-uid="${this.state.rtThoughtUid}"]`);
    if (row) updateThoughtRowSummary(row as HTMLDivElement, this.state.rtThoughtBuffer);
    this.scrollToBottomIfNearEnd(this.el.chatHistory);
  }

  /** Троттлинг DOM-рендера стрима: полный renderMarkdown не чаще раза в
   *  STREAM_RENDER_INTERVAL_MS, последний буфер добирается trailing-рендером
   *  через setTimeout (rAF не годится — не тикает в скрытых окнах). */
  private scheduleStreamRender(render: () => void) {
    this.state.streamTrailingRender = render;
    const now = Date.now();
    const elapsed = now - this.state.streamLastRenderAt;
    if (elapsed >= STREAM_RENDER_INTERVAL_MS) {
      if (this.state.streamRenderTimer !== null) {
        clearTimeout(this.state.streamRenderTimer);
        this.state.streamRenderTimer = null;
      }
      this.state.streamTrailingRender = null;
      this.state.streamLastRenderAt = now;
      render();
    } else if (this.state.streamRenderTimer === null) {
      this.state.streamRenderTimer = window.setTimeout(() => {
        this.state.streamRenderTimer = null;
        this.state.streamLastRenderAt = Date.now();
        const fn = this.state.streamTrailingRender;
        this.state.streamTrailingRender = null;
        fn?.();
      }, STREAM_RENDER_INTERVAL_MS - elapsed);
    }
  }

  /**
   * Сброс стрим-буферов при повторе запроса бэкендом (событие stream_reset):
   * дельты новой попытки не должны приклеиваться к недописанному тексту
   * оборванной. Частичный пузырь убирается из UI — полный ответ придёт заново.
   */
  onStreamReset() {
    if (this.state.rtStreamUid) {
      this.el.chatHistory
        .querySelector(`[data-msg-uid="${this.state.rtStreamUid}"]`)
        ?.remove();
    }
    // Отменяем отложенный trailing-рендер: дельты новой попытки не должны
    // дорисовать в новый пузырь текст оборванной.
    this.state.cancelStreamRender();
    this.state.rtStreamUid = null;
    this.state.rtStreamBuffer = "";
    this.state.rtIsJson = false;
    this.state.rtStreamAuthor = "";
    this.state.rtExpectTool = false;
    this.state.rtThoughtUid = null;
    this.state.rtThoughtBuffer = "";
    this.state.rtThoughtAuthor = "";
  }

  onSubcallDone(call: any) {
    this.state.realtimeSubcallKeys.add(`${call.agent_name}:${call.time_sec.toFixed(2)}`);
    const row = createSubcallElement(call, (c) => this.showSubchat(c));
    this.el.chatHistory.appendChild(row);
    this.scrollToBottomIfNearEnd(this.el.chatHistory);
  }

  onAgentThought(payload: { author: string, thought: string, time_sec: number }) {
    const dedupKey = `${payload.author}:${payload.thought.substring(0, 200)}`;
    if (this.thoughtDedupSet.has(dedupKey)) return;
    this.thoughtDedupSet.add(dedupKey);
    const row = createThoughtRow({
      kind: "thought",
      author: payload.author,
      summary: payload.thought,
      detail: payload.thought,
      meta: payload.time_sec > 0 ? `⏱${formatDuration(payload.time_sec)}` : undefined,
    });
    this.el.chatHistory.appendChild(row);
    this.scrollToBottomIfNearEnd(this.el.chatHistory);
  }

  onAgentToolCall(p: { author: string, tool: string, args?: string, result?: string }) {
    const key = `${p.author}:${p.tool}`;
    if (p.result !== undefined) {
      const row = this.el.chatHistory.querySelector(`[data-tool-key="${key}"]`);
      if (row) {
        completeToolRow(row as HTMLDivElement, p.result);
        this.scrollToBottomIfNearEnd(this.el.chatHistory);
      } else {
        // Диагностика рассинхрона realtime-пары «вызов → результат»: строка
        // с таким data-tool-key не найдена (например, перерисовка чата между
        // событиями). Молчать нельзя — результат тула теряется без следа.
        logFront(`[chat] Результат тула "${key}" получен, но строка вызова не найдена в чате (рассинхрон realtime-ключей).`);
      }
      return;
    }
    const row = createThoughtRow({
      kind: "tool",
      icon: TOOL_ICONS[p.tool] ?? "🔧",
      label: `${p.author} → ${p.tool}`,
      summary: p.args,
      detail: p.args,
      toolKey: key,
      pending: true,
    });
    this.el.chatHistory.appendChild(row);
    this.scrollToBottomIfNearEnd(this.el.chatHistory);
  }

  onEngineMode(p: { mode?: string, tok_per_sec?: number, detail?: string }) {
    this.updateEngineBadge(p.mode || "cpu", p.tok_per_sec || 0, p.detail || "");
  }

  /**
   * Уведомление плагина о памяти (`vram_notice`) — в чат системным сообщением
   * вместо модалки (tasks/31.10.26: прогноз ≠ факт, окно врало оценкой).
   * Событие машинно-уровневое (локальный движок один), доставляется роутером
   * в АКТИВНУЮ вкладку; нет сессии — persistSession сам не сохранит.
   * Во время восстановления истории push потерялся бы при перерисовке —
   * туда не лезем, инфо уходит в лог.
   */
  onVramNotice(p: { note?: string }) {
    const note = (p?.note ?? "").trim();
    if (!note) return;
    if (this.state.isRestoringSession) {
      logFront(`[chat] vram_notice во время восстановления сессии пропущен: ${note}`);
      return;
    }
    this.state.history.push({ type: "message", author: "system", content: note });
    this.state.uidList.push(this.state.nextUid());
    this.renderChatFromHistory();
    this.persistSession().catch((e) => {
      logFront(`[chat] история не сохранена после vram_notice: ${e}`);
    });
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

  /**
   * Единые состояния плашки токенов для ОБЕИХ веток (вторая итерация 22.10.26:
   * tasks/22.10.26 Токены при облачных моделях.md). Раньше здесь был каскад
   * несогласованных ранних return (глюк «то 0, то сворачивается»), затем —
   * тотальное скрытие при облачной модели. Теперь контракт один — «сколько
   * токенов съедает текущий промпт»:
   *  1. нет модели/агента → «—» (облако) / «— / лимит» (локальная);
   *  2. нечего считать (нет истории, ввода и вложений) → «~0» (облако) /
   *     «0 / лимит» (локальная) — без invoke;
   *  3. иначе → токены ПОЛНОГО промпта через get_prompt_preview (system-промпт
   *     агента + ИСТОРИЯ + текущий ввод; при пустом message бэкенд не добавляет
   *     user-сообщение — api/chat.rs) → TokenizerService.countTokens:
   *     - облачная модель → «~N» (estimatePromptMemory и «/ лимит» неприменимы;
   *       «~» = оценка чужим токенизатором, точный счёт отдаёт провайдер);
   *     - локальная → прежний вид «N (~X МБ …)» + warning/danger 80%.
   *  Видимостью плашки setIndicatorMode больше НЕ управляет: валидный контент
   *  есть у обеих веток (css-скрытие на узких окнах остается).
   */
  private async updateTokenCounter() {
    const counter = this.el.tokenCounter;
    if (!counter || this.state.isProcessing) return;
    const modelPath = this.el.modelSelect?.value;
    const agentId = this.el.agentSelect?.value;
    const text = this.el.chatInput?.value || "";
    const contextSize = await this.resolveContextLimit(modelPath);
    const isCloud = !!modelPath && isCloudModelPath(modelPath);

    if (!modelPath || !agentId) {
      counter.innerText = isCloud ? "—" : `— / ${contextSize > 0 ? contextSize : "…"}`;
      counter.title = "Выберите модель и агента, чтобы увидеть заполнение контекста";
      return;
    }
    // Локальная ветка: лимит — окно модели из GGUF (fallback config.contextSize).
    if (!isCloud && contextSize <= 0) {
      counter.innerText = "— / …";
      counter.title = "Ожидание загрузки конфигурации (лимит контекста)";
      return;
    }
    if (!text.trim() && !this.state.history.some(m => m.type === "message") && this.attachments.length === 0) {
      counter.innerText = isCloud ? "~0" : `0 / ${contextSize}`;
      counter.title = isCloud
        ? "Токены промпта: истории и ввода пока нет — 0."
        : "Токены: Текущие / Лимит контекста";
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
            history: this.state.history,
            attachments: this.attachments
        });

        const tokens = await countTokens(promptText, tokenizerId);
        this.lastPromptTokens = tokens;

        // Облачная ветка: VRAM-оценка (estimatePromptMemory) и «80% лимита»
        // неприменимы — покажем только «~N». Prompt-сборщик на бэкенде от
        // модели не зависит (api/chat.rs uses model_path no-op), текст тот же,
        // что и у локальной ветки.
        if (isCloud) {
            counter.innerText = `~${tokens}`;
            counter.title = "Предварительная оценка токенов: system-промпт + история + ввод (чужой токенизатор). Точный счёт возвращает провайдер после ответа.";
            counter.classList.remove("warning", "danger");
            this.notifyTokenMode(isCloud, tokens);
            return;
        }

        const vram = await estimatePromptMemory(modelPath, contextSize, kvQuantKeys, kvQuantValues, tokens, maxGen);

        const isGraph = isGraphAgent(this.el.agentSelect.value);
        const graphHint = isGraph ? " Оценка по самому тяжёлому агенту графа." : "";
        // Формат: (расчётная потребность + уже занятая VRAM) / точный объём VRAM в MiB.
        const memStr = vram && vram.vram_used_mb > 0 && vram.vram_total_mb > 0
            ? ` (~${Math.round(vram.need_mb + vram.vram_used_mb)} МБ / ${Math.round(vram.vram_total_mb)} МБ)`
            : (isFinite(vram?.need_mb) && (vram?.need_mb ?? 0) > 0 ? ` (~${Math.round(vram.need_mb)} МБ)` : "");
        counter.innerText = `${tokens}${memStr}`;
        counter.classList.remove("warning", "danger");
        if (tokens > contextSize) {
            counter.classList.add("danger");
            counter.title = "Внимание: Лимит превышен! При отправке старые сообщения будут удалены из контекста." + graphHint;
        } else if (tokens > contextSize * 0.8) {
            counter.classList.add("warning");
            counter.title = "Контекст заполняется. Близко к лимиту." + graphHint;
        } else {
            counter.title = "Токены: текущие / Лимит контекста. Память: оценка (потребность + занято) / объём VRAM." + graphHint;
        }
        this.notifyTokenMode(isCloud, tokens);
    } catch (e) {
        console.error("Ошибка обновления счетчика токенов:", e);
        void trackError("chat.updateTokenCounter", e);
    }
  }

  /**
   * Аналитика смены ветки токен-счёта (облако ↔ локальная). Событие шлём
   * один раз при смене режима, не на каждый пересчёт (debounce-тип работы).
   */
  private notifyTokenMode(cloud: boolean, tokens: number) {
    if (this.lastTokenModeCloud === cloud) return;
    this.lastTokenModeCloud = cloud;
    void trackEvent("token_mode", { cloud: cloud ? 1 : 0, tokens });
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
    if (this.state.isProcessing) return;
    const readiness = this.sendReadiness();
    if (!readiness.ready) {
      showToast(readiness.reason || "Дождитесь готовности интерфейса", "error");
      return;
    }
    const idx = this.state.uidList.indexOf(uid);
    if (idx === -1) return;

    const activeAgent = this.el.agentSelect.value;
    const modelPath = this.el.modelSelect.value;

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
        sessionId: this.state.sessionId ?? "",
        executionStrategy: this.state.executionStrategy
      });

      if (response?.has_error) {
        void trackError("chat.send.runFrom.outcome", response.has_error);
      }
      this.applyCloudRequestsFromResponse(response);

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

      const lastMessage = [...this.state.history].reverse().find(message => message.type === "message");
      if (response.text && lastMessage?.author !== activeAgent) {
        this.state.history.push({
          type: "message",
          author: activeAgent,
          content: response.text,
        });
        this.state.uidList.push(this.state.nextUid());
      }

      this.renderChatFromHistory();
      await this.persistSession();
      this.triggerTokenCount();

    } catch (error) {
      await this.handleSendFailure(error, "chat.send.runFrom");
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
    showToast("Мысль удалена.", "success");
  }

  /** Клон сессии до указанного хода. uid может быть как uid'ом строки-мысли
   *  (меню строки), так и — для обратной совместимости — uid'ом ответа
   *  ассистента (прежде меню висело на блоке мыслей с его uid). Оба случая
   *  сводятся к одному проходу: подняться до начала серии мыслей/сигналов
   *  перед ходом ассистента и клонировать всё, что выше неё. */
  private async onCloneFromThoughts(uid: string) {
    const idx = this.state.uidList.indexOf(uid); if (idx === -1) return;
    const isThoughtish = (t?: string) => t === 'thought' || t === 'signal';
    let first = idx; let i = idx - 1;
    while (i >= 0 && isThoughtish(this.state.history[i].type)) { first = i; i--; }
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
    this.scrollToBottomIfNearEnd(container); renderMermaid(); materializeNeurosisCases(container, renderMermaid);
  }

  appendMessage(role: Role, content: string, agentName?: string, timeText?: string, subCalls?: any[], skipSubcallRender = false, uid?: string, attachments?: Attachment[]) {
    if (subCalls && subCalls.length > 0 && !skipSubcallRender) {
      for (const call of subCalls) {
        this.el.chatHistory.appendChild(createSubcallElement(call, (c) => this.showSubchat(c)));
      }
    }
    const hasMenu = uid !== undefined && (role === 'user' || role === 'agent');
    const msgEl = createMessageElement(role, content, agentName, timeText, hasMenu ? uid : undefined, hasMenu ? this.menuCallbacks : undefined, attachments, this.translatorMenuLabel(), undefined, this.imageCallbacks);
    this.el.chatHistory.appendChild(msgEl); this.scrollToBottomIfNearEnd(this.el.chatHistory); renderMermaid(); materializeNeurosisCases(this.el.chatHistory, renderMermaid);
  }

  private isCloud(): boolean {
    return isCloudModelPath(this.el.modelSelect?.value);
  }

  /** Единственная точка обновления счётчика из ответа `chat_request`.
   *  Бэкенд присылает СУММУ за сессию, поэтому это не «+N», а присваивание.
   *  Ответ без поля (локальная модель) накопленное значение не трогает. */
  private applyCloudRequestsFromResponse(response: any) {
    const total = response?.cloud_requests;
    if (typeof total !== "number" || !isFinite(total)) return;
    this.state.cloudRequests = total;
    this.renderCloudCounter();
  }

  private renderCloudCounter() {
    if (!this.el.cloudCounter) return;
    this.el.cloudCounter.textContent = `☁ ${this.state.cloudRequests} запр.`;
  }

  /** Единая точка выбора индикатора под полем ввода.
   *  cloud=true → облако: счётчик запросов вместо бейджа движка.
   *  Плашку токенов этот метод больше НЕ прячет: облачная ветка
   *  updateTokenCounter рисует свою оценку «~N», поэтому у плашки есть
   *  валидный контент для обоих режимов (css-скрытие на узких окнах
   *  в styles.css остаётся как было). */
  private setIndicatorMode(cloud: boolean) {
    if (this.el.engineBadge) {
      this.el.engineBadge.style.display = cloud ? "none" : "";
    }
    if (this.el.cloudCounter) {
      this.el.cloudCounter.style.display = cloud ? "" : "none";
      if (cloud) this.renderCloudCounter();
    }
  }

  /** Видимость по ВЫБРАННОЙ модели: переключение модели, открытие сессии, конфиг. */
  private applyIndicatorVisibility() {
    this.setIndicatorMode(this.isCloud());
  }

  private setEngineBadge(cls: string, text: string, title: string) {
    if (this.isCloud()) {
      this.setIndicatorMode(true);
      return;
    }
    const b = this.el.engineBadge;
    if (!b) return;
    b.className = `engine-badge ${cls}`;
    b.innerText = text;
    b.title = title;
  }

  /** Начальное состояние бейджа по get_engine_status (до первого запроса) */
  private async refreshEngineBadgeInit() {
    this.applyIndicatorVisibility();
    if (this.isCloud()) return;
    let st: any;
    try {
      st = await getEngineStatus();
    } catch (_) {
      this.setEngineBadge("engine-none", "—", "Не удалось определить состояние движка");
      void trackError("chat.engineBadgeInit", _);
      return;
    }
    // Форма ответа проверяется до доступа к полям: нет объекта — нет свойства.
    if (!st) {
      this.setEngineBadge("engine-none", "—", "Не удалось определить состояние движка");
      void trackError("chat.engineBadgeInit", new Error("getEngineStatus вернул пустой ответ"));
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

  /** Обновление бейджа по событию engine_mode после каждого запроса.
   *  `mode === "cloud"` — авторитетный факт о том, как реально шёл прогон:
   *  бейдж движка (GPU/CPU, tok/s) к облаку отношения не имеет и обязан уйти,
   *  даже если модель в селекте ещё не успела переключиться. */
  private updateEngineBadge(mode: string, tokPerSec: number, detail: string) {
    if (mode === "cloud" || this.isCloud()) {
      this.setIndicatorMode(true);
      return;
    }
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
    this.el.chatHistory.innerHTML = '';
    // Окно рендера (01.11.26): в DOM живёт не больше CHAT_RENDER_WINDOW +
    // renderExpandExtra ПОСЛЕДНИХ сообщений — безлимитная история переполняла
    // память WebView2-рендерера (краш «На этой странице возникла проблема»).
    // state.history и persistSession НЕ режутся — ограничивается только DOM.
    // Правка/регенерация старого сообщения кликом вне окна структурно
    // невозможна: кнопки меню живут на самих узлах, неокрашенный узел не кликается.
    const total = this.state.history.length;
    const windowSize = CHAT_RENDER_WINDOW + this.state.renderExpandExtra;
    const windowStart = Math.max(0, total - windowSize);
    if (windowStart > 0) {
      const btn = document.createElement('button');
      btn.className = 'chat-load-more';
      btn.type = 'button';
      btn.textContent = `Показать ещё ${Math.min(CHAT_RENDER_WINDOW, windowStart)}`;
      btn.addEventListener('click', () => {
        this.state.renderExpandExtra += CHAT_RENDER_WINDOW;
        this.renderChatFromHistory();
      });
      this.el.chatHistory.appendChild(btn);
    }
    for (let i = windowStart; i < total; i++) {
      const msg = this.state.history[i]; const uid = this.state.uidList[i];
      if (msg.type === 'thought' || msg.type === 'signal') {
        const content = msg.content;
        const tool = msg.type === 'thought' ? this.parseToolThought(msg.content) : null;
        // Имя автора: как у message-ветки ниже — готовое author_name имеет
        // приоритет над сырым машинным id (мысли-дубля в soma_агента из-за этого
        // и выглядели как «мысли другого агента»).
        const thoughtAuthor = (msg.author_name ?? getAgentDisplayName(msg.author)) || 'Система';
        if (tool) {
          const row = createThoughtRow({
            kind: "tool",
            icon: TOOL_ICONS[tool.tool] ?? "🔧",
            label: `${thoughtAuthor} → ${tool.tool}`,
            summary: tool.args,
            detail: tool.args,
            uid,
            menuCallbacks: this.thoughtMenuCallbacks,
          });
          completeToolRow(row, tool.result);
          this.el.chatHistory.appendChild(row);
        } else {
          const row = createThoughtRow({
            kind: msg.type === 'signal' ? 'signal' : 'thought',
            author: thoughtAuthor,
            summary: content,
            detail: content,
            meta: msg.time_sec && msg.time_sec > 0 ? `⏱${formatDuration(msg.time_sec)}` : undefined,
            uid,
            menuCallbacks: this.thoughtMenuCallbacks,
          });
          this.el.chatHistory.appendChild(row);
        }

        if (msg.sub_calls && msg.sub_calls.length > 0) {
          msg.sub_calls.forEach((call: any) => this.el.chatHistory.appendChild(createSubcallElement(call, (c) => this.showSubchat(c))));
        }
        continue;
      }
      if (msg.type !== 'message') {
        logFront(`[chat] Неизвестный тип сообщения "${String(msg.type)}" по индексу ${i}; сообщение пропущено.`);
        continue;
      }
      if (msg.author && msg.author !== 'user' && msg.author !== 'system' && msg.sub_calls && msg.sub_calls.length > 0) {
        msg.sub_calls.forEach(call => this.el.chatHistory.appendChild(createSubcallElement(call, (c) => this.showSubchat(c))));
      }
      const role = (msg.author && msg.author !== 'user') ? (msg.author === 'system' ? 'system' : 'agent') : 'user' as Role;
      // Имя автора: готовое `author_name` имеет приоритет (машиный ключ
      // `author` может не резолвиться через каталог, напр. у fused-группы).
      const agentName = (msg.author && msg.author !== 'user' && msg.author !== 'system')
        ? (msg.author_name ?? getAgentDisplayName(msg.author))
        : undefined;
      const hasMenu = uid && (role === 'user' || role === 'agent');
      const timeText = msg.time_sec ? formatDuration(msg.time_sec) : undefined;
      // Скрываем служебные теги LLM в сохранённых ответах (defence in depth).
      const cleanContent = role === 'agent' ? stripStreamArtifacts(msg.content) : msg.content;
      this.el.chatHistory.appendChild(createMessageElement(role, cleanContent, agentName, timeText, hasMenu ? uid : undefined, hasMenu ? this.menuCallbacks : undefined, msg.attachments, this.translatorMenuLabel(), msg.model, this.imageCallbacks));
    }
    // Полная перерисовка стирает chatHistory — если обработка ещё идёт,
    // возвращаем строку-индикатор в DOM (инвариант: она живёт в чате,
    // пока isProcessing; убрать её может только setProcessingState(false)).
    if (this.state.processingIndicator) this.el.chatHistory.appendChild(this.state.processingIndicator.el);
    this.scrollToBottomIfNearEnd(this.el.chatHistory); renderMermaid(); materializeNeurosisCases(this.el.chatHistory, renderMermaid);
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
    this.appendMessageToContainer(this.el.subchatHistory, 'agent', subCall.response, subCall.agent_name, formatDuration(subCall.time_sec));
  }

  private sendReadiness(): { ready: boolean; reason: string } {
    const model = this.el.modelSelect.value;
    const cloudModelAvailable = (value: string) => {
      const router = Object.entries(CLOUD_ROUTER_PREFIXES).find(([_, p]) => value.startsWith(p));
      if (!router) return false;
      return store.cloudRouterCombos[router[0]]?.some(combo => `${router[1]}${combo.name}` === value) ?? false;
    };
    const savedCloudPending = this.el.modelSelect.dataset.modelExplicit !== "true"
      && !!store.lastModel && isCloudModelPath(store.lastModel)
      && !cloudModelAvailable(store.lastModel);
    const modelReady = !savedCloudPending && (isCloudModelPath(model)
      ? cloudModelAvailable(model)
      : store.models.includes(model));
    let reason = "";
    if (this.state.isRestoringSession) reason = "Восстановление сессии…";
    else if (!store.agentsReady) reason = "Загрузка каталога агентов…";
    else if (this.pendingSessionModel || this.pendingSessionAgent) reason = "Восстановление модели или агента сессии…";
    else if (savedCloudPending) reason = "Загрузка сохранённой облачной модели…";
    else if (!modelReady) reason = "Выберите установленную модель";
    else if (!this.el.agentSelect.value) reason = "Выберите агента";
    const ready = !reason
      && !this.state.isProcessing
      && !this.state.isRestoringSession;
    return { ready, reason };
  }

  private refreshSendAvailability() {
    const readiness = this.sendReadiness();
    this.el.btnSend.disabled = !readiness.ready;
    // Неготовность — тултип объясняет причину; готовность — подсказка хоткеев
    // (отправить / добавить в чат без запуска LLM).
    this.el.btnSend.title = readiness.reason || "Отправить (Ctrl+Enter)\nДобавить (Alt+Enter)";
  }

  private setSessionRestoreState(restoring: boolean) {
    this.state.isRestoringSession = restoring;
    this.el.chatInput.disabled = restoring;
    this.refreshSendAvailability();
  }

  setProcessingState(state: boolean) {
    this.state.isProcessing = state;
    this.el.modelSelect.disabled = this.el.agentSelect.disabled = state;
    this.el.btnStop.disabled = !state;
    if (state) {
        this.state.processingStartedAt = Date.now();
        this.state.lastActivityAt = Date.now();
        // Полный сброс стриминговых буферов перед новым прогоном.
        this.state.realtimeSubcallKeys.clear();
        this.state.rtStreamUid = null;
        this.state.rtStreamBuffer = "";
        this.state.rtIsJson = false;
        this.state.rtStreamAuthor = "";
        this.state.rtExpectTool = false;
        this.state.rtThoughtUid = null;
        this.state.rtThoughtBuffer = "";
        this.state.rtThoughtAuthor = "";
        this.state.cancelStreamRender();
        this.state.streamLastRenderAt = 0;
        // Окно рендера снова следует за концом истории: раскрытие «Показать
        // ещё» было разовым просмотром прошлого контекста.
        this.state.renderExpandExtra = 0;
        this.armProcessingWatchdog();
        // Индикатор обработки — в поток сообщений: после сообщения юзера,
        // визуально всегда последний (CSS order). Живёт до конца обработки.
        if (!this.state.processingIndicator) this.state.processingIndicator = createProcessingIndicator();
        const indicator = this.state.processingIndicator;
        this.el.chatHistory.appendChild(indicator.el);
        indicator.show(true);
        indicator.setStatus("Подготовка...");
        indicator.startTimer(this.state.processingStartedAt);
        this.scrollToBottomIfNearEnd(this.el.chatHistory);
    }
    else {
        // Полный сброс «рабочего» состояния: чат больше не в процессе обработки —
        // строка-индикатор уходит из чата, таймер гаснет. Это единственная
        // точка освобождения: finally отправки, run-from-message, стоп и
        // сторож сходятся сюда, поэтому «залипшая» строка структурно невозможна.
        // Троттлинг стрима тоже отменяется: полный рендер истории ниже рисует
        // финальный текст, trailing по стрим-буферу больше не нужен.
        this.disarmProcessingWatchdog();
        this.state.cancelStreamRender();
        this.state.processingIndicator?.destroy();
        this.state.processingIndicator = null;
    }
    this.refreshSendAvailability();
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

  private restoreSessionSelection(model?: string, agent?: string) {
    this.pendingSessionModel = null;
    this.pendingSessionAgent = null;
    if (model) {
      if (this.hasOption(this.el.modelSelect, model)) {
        this.el.modelSelect.value = model;
        this.el.modelSelect.dataset.modelExplicit = "true";
        syncModelSelectTitle(this.el.modelSelect);
        bus.emit("model:changed", model);
      } else {
        this.pendingSessionModel = model;
      }
    }
    if (agent) {
      if (this.hasOption(this.el.agentSelect, agent)) {
        this.el.agentSelect.value = agent;
      } else {
        this.pendingSessionAgent = agent;
      }
    }
  }

  private applyPendingSessionSelection(): boolean {
    const model = this.pendingSessionModel;
    const agent = this.pendingSessionAgent;
    let modelRestored = false;
    if (model && this.hasOption(this.el.modelSelect, model)) {
      this.el.modelSelect.value = model;
      this.el.modelSelect.dataset.modelExplicit = "true";
      syncModelSelectTitle(this.el.modelSelect);
      this.pendingSessionModel = null;
      modelRestored = true;
    }
    if (agent && this.hasOption(this.el.agentSelect, agent)) {
      this.el.agentSelect.value = agent;
      this.pendingSessionAgent = null;
    }
    return modelRestored;
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
    // История вкладки с этого момента строится с нуля (первый ход / повтор
    // с обрезкой) и авторитетна — см. ChatTabState.historyLoaded.
    this.state.historyLoaded = true;
    this.hooks.onBecameChat?.(this.state.sessionId ?? "");
  }

  /// Восстановление черновика на МАIN-вкладке: подгружает сессию, но НЕ
  /// конвертирует вкладку в чат (welcome + прижатая плашка остаются).
  async openSessionDraft(id: string) {
    if (this.state.isProcessing) return;
    this.setSessionRestoreState(true);
    try {
      const session = await loadSession(id);
      this.state.sessionId = session.id ?? id;
      this.state.cloudRequests = session.cloud_requests || 0;
      this.restoreSessionSelection(session.model, session.agent);
      // ПОСЛЕ восстановления выбора модели: индикатор выбирается по модели.
      this.applyIndicatorVisibility();
      if (session.draft) {
        this.el.chatInput.value = session.draft;
        this.el.chatInput.style.height = "auto";
        this.el.chatInput.style.height = `${this.el.chatInput.scrollHeight}px`;
      }
      this.updateAttachButtonState();
      this.triggerTokenCount();
    } catch (e) {
      void trackError("chat.openSessionDraft", e);
    } finally {
      this.setSessionRestoreState(false);
    }
  }

  /**
   * Фиксирует сообщение пользователя: DOM, очистка поля ввода, создание ID сессии
   * и запись в историю. Вызывается внутри `try` в handleSend и из handleAppend
   * («добавить в чат» без запуска LLM) — отказ персистентности не должен пройти
   * мимо обработки ошибки и оставить рассинхрон DOM/истории.
   */
  private commitUserMessage(text: string): Attachment[] {
    const userUid = this.state.nextUid(); this.state.uidList.push(userUid);
    const displayText = text || (this.attachments.length > 0 ? `[${this.attachments.length} файлов]` : '');
    this.appendMessage('user', displayText, undefined, undefined, undefined, false, userUid, this.attachments);
    this.el.chatInput.value = ""; this.el.chatInput.style.height = "auto"; clearTimeout(this.state.draftTimeout);
    this.el.filePreview.innerHTML = '';
    const attachments = [...this.attachments];
    this.attachments = [];
    this.ensureSessionId();
    // Вкладка ещё main-режима — конвертируем в чат именно при отправке,
    // а не на первом набранном символе (чтобы плашка ввода не «прыгала»).
    if (!this.state.isChatTab) this.becomeChatTab();
    this.state.history.push({ type: "message", author: "user", content: displayText, attachments });
    return attachments;
  }

  /**
   * Единая обработка отказа отправки (общая для handleSend и onRunFromMessage):
   * видимая запись в чат + перерисовка + попытка сохранить её. Отказ самой
   * персистентности обязан быть залогирован, а не проглочен (core rules §2.2).
   *
   * Частичный текст оборванного стрима НЕ проглатывается (инцидент 25.10.2026):
   * юзер видел, как ответ писался в чат, — при ошибке он обязан остаться в
   * истории с честной пометкой обрыва, а не молча исчезнуть при перерисовке.
   * Служебные JSON-конверты (сигналы/вызовы тулов) не сохраняются: юзер их
   * не видел.
   */
  private async handleSendFailure(error: unknown, scope: string) {
    const errText = String(error);
    const wasCancelled = errText.includes("Отменено") || errText.includes("Прервано");
    const partial = stripStreamArtifacts(this.state.rtStreamBuffer ?? "").trim();
    if (partial && !this.state.rtIsJson) {
      const reason = wasCancelled
        ? "прервано пользователем"
        : errText.split("\n")[0].trim() || "причина неизвестна";
      const author = this.state.rtStreamAuthor || getSelectedAgentName(this.el.agentSelect);
      this.state.history.push({
        type: "message",
        author,
        content: `${partial}\n\n---\n⚠️ **Ответ оборван:** ${reason}`,
      });
      this.state.uidList.push(this.state.nextUid());
    }
    if (wasCancelled) {
      this.state.history.push({ type: "message", author: "system", content: "⚠️ Прервано." });
      this.state.uidList.push(this.state.nextUid());
    } else {
      showToast(`Ошибка: ${error}`, "error");
      this.state.history.push({ type: "message", author: "system", content: `⚠️ Ошибка: ${error}` });
      this.state.uidList.push(this.state.nextUid());
      void trackError(scope, error);
    }
    this.renderChatFromHistory();
    try {
      await this.persistSession();
    } catch (persistError) {
      logFront(`[chat] история не сохранена после отказа отправки: ${persistError}`);
      void trackError(`${scope}.persistFailure`, persistError);
    }
  }

  async handleSend() {
    const text = this.el.chatInput.value.trim();     if (!text && this.attachments.length === 0) return;
    // Per-tab guard: вкладка B шлёт свободно, пока вкладка A обрабатывает —
    // параллельность решает бэкенд (облако сразу, локал — в очередь).
    if (this.state.isProcessing || this.state.isRestoringSession) return;
    const readiness = this.sendReadiness();
    if (!readiness.ready) {
      showToast(readiness.reason || "Дождитесь готовности интерфейса", "error");
      return;
    }
    const activeAgent = this.el.agentSelect.value; const modelPath = this.el.modelSelect.value;
    if (!modelPath) { showToast("Выберите модель!", "error"); return; }

    // setProcessingState(true) — ДО первого await и ДО входа в try. Только так
    // инвариант «UI не остаётся в состоянии Обработка» держится структурно: любой
    // await ниже, включая персистентность, обязан дойти до finally.
    this.setProcessingState(true);
    try {
      const attachments = this.commitUserMessage(text);
      bus.emit("session:changed");
      await this.persistSession("");
      const startTime = performance.now();
      const _p2 = store.currentModelParams;
      const params = { temperature: parseFloat(this.el.tempSlider.value), top_k: parseInt(this.el.topkSlider.value, 10), top_p: parseFloat(this.el.toppSlider.value), min_p: parseFloat(this.el.minpSlider.value), repetition_penalty: parseFloat(this.el.reppenSlider.value), presence_penalty: parseFloat(this.el.prespenSlider.value), dry_multiplier: _p2?.dry_multiplier ?? 0.0, dry_base: _p2?.dry_base ?? 1.75, dry_allowed_length: _p2?.dry_allowed_length ?? 2, dry_penalty_last_n: _p2?.dry_penalty_last_n ?? 0, xtc_probability: _p2?.xtc_probability ?? 0.0, xtc_threshold: _p2?.xtc_threshold ?? 0.1 };
      const allHistory = this.state.history.slice();
      const isCloudRouter = isCloudModelPath(modelPath);
      let mmprojPath: string | null = null;
      if (!isCloudRouter && attachments.length > 0) {
        try { mmprojPath = await getMmprojPath(modelPath); } catch (_) {}
      }
      const response: any = await invoke("chat_request", {
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
        sessionId: this.state.sessionId ?? "",
        // null = «решает файл графа». Переопределение уходит только когда
        // пользователь его сделал — иначе мы бы молча уводили разговор в другой
        // режим молча.
        executionStrategy: this.state.executionStrategy
      });
      if (response?.has_error) {
        void trackError("chat.send.outcome", response.has_error);
      }
      this.applyCloudRequestsFromResponse(response);
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
      await this.handleSendFailure(error, "chat.send");
    } finally {
      this.setProcessingState(false);
      this.triggerTokenCount();
    }
  }

  /**
   * «Добавить в чат» (Alt+Enter): фиксирует сообщение пользователя в истории
   * сессии БЕЗ вызова chat_request — LLM не запускается, вкладка не уходит в
   * «Обработку». Дописанные сообщения едут в history следующего реального хода:
   * run_chat берёт текст хода из параметра message, а вся история инжектится
   * агентам автоматически.
   *
   * Гард isProcessing обязателен: ответ бэкенда заменяет history целиком
   * (handleSend), добавленное во время обработки сообщение было бы затёрто.
   * Готовность модели/агента не требуется — LLM здесь не участвует.
   */
  private async handleAppend() {
    const text = this.el.chatInput.value.trim();
    if (!text && this.attachments.length === 0) return;
    if (this.state.isProcessing || this.state.isRestoringSession) return;
    try {
      this.commitUserMessage(text);
      bus.emit("session:changed");
      await this.persistSession("");
      this.triggerTokenCount();
    } catch (error) {
      logFront(`[chat] не удалось дописать сообщение в чат: ${error}`);
      void trackError("chat.append", error);
      showToast(`Ошибка: ${error}`, "error");
    }
  }

  /// Подпись зелёной кнопки отражает действие при зажатом Alt (keydown/keyup
  /// на chatInput): «Отправить / Ctrl+Enter» ↔ «Добавить в чат / Alt+Enter».
  /// Обе подписи всегда в DOM (.btn-send-labels, неактивная скрыта visibility) —
  /// ширина кнопки постоянна, переключение не двигает layout.
  private setAppendButtonMode(on: boolean) {
    if (!this.el.btnSendLabelSend || !this.el.btnSendLabelAppend) return;
    this.el.btnSendLabelSend.classList.toggle("is-hidden", on);
    this.el.btnSendLabelAppend.classList.toggle("is-hidden", !on);
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
    this.setSessionRestoreState(true);
    try {
      const session = await loadSession(id);
      // Единый источник правды: ID сессии из файла (совпадает с ключом вкладки).
      this.state.sessionId = session.id ?? id;
      this.state.history = session.messages;
      this.state.historyLoaded = true;
      // Счётчик облачных запросов приходит уже поднятым бэкендом (load_session).
      this.state.cloudRequests = session.cloud_requests || 0;
      this.state.uidCounter = 0; this.state.uidList = this.state.history.map(() => this.state.nextUid());
      this.el.chatInput.value = session.draft || "";
      setTimeout(() => { this.el.chatInput.style.height = "auto"; this.el.chatInput.style.height = `${this.el.chatInput.scrollHeight}px`; }, 0);
      this.restoreSessionSelection(session.model, session.agent);
      // ПОСЛЕ восстановления выбора модели: индикатор выбирается по модели.
      this.applyIndicatorVisibility();
      this.renderChatFromHistory();
      bus.emit("session:changed");
      this.becomeChatTab();
      this.updateAttachButtonState();
      this.triggerTokenCount();
    } catch(e) {
      showToast(`Ошибка: ${e}`, "error");
      void trackError("chat.openSession", e);
    } finally {
      this.setSessionRestoreState(false);
    }
  }

  private async updateAttachButtonState() {
    const btn = this.el.btnAttach;
    const modelPath = this.el.modelSelect?.value;
    if (!modelPath) { btn.disabled = true; this.modelAudioCapable = false; btn.classList.remove('btn-attach-active'); btn.classList.add('btn-attach-inactive'); btn.title = 'Сначала выберите модель'; return; }
    if (isCloudModelPath(modelPath)) {
      btn.disabled = false;
      this.modelAudioCapable = true;
      btn.classList.remove('btn-attach-inactive');
      btn.classList.add('btn-attach-active');
      btn.title = 'Прикрепить файл, папку или изображение';
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
    if (!files || this.state.isProcessing) return;
    for (let i = 0; i < files.length; i++) {
      const file = files[i];
      if (file.size > 20 * 1024 * 1024) { showToast(`Файл ${file.name} слишком большой (>20MB)`, "error"); continue; }
      if (file.type.startsWith("audio/") && !this.modelAudioCapable) {
        showToast(`Модель не поддерживает аудио — прикрепление звука отключено`, "error");
        continue;
      }
      const dataBase64 = await this.fileToBase64(file);
      const attachment: Attachment = { file_name: file.name, mime_type: file.type, data_base64: dataBase64 };
      if (this.attachments.some((item) => this.attachmentKey(item) === this.attachmentKey(attachment))) continue;
      this.attachments.push(attachment);
      this.addFilePreview(attachment);
    }
    this.el.fileInput.value = '';
    this.triggerTokenCount();
  }

  private async handlePaths(paths: string[]) {
    if (paths.length === 0 || this.state.isProcessing) return;
    try {
      const metadata = await invoke<AttachmentMetadata[]>("get_attachment_metadata", { paths });
      for (const item of metadata) {
        const attachment: Attachment = {
          file_name: item.file_name,
          mime_type: item.mime_type,
          file_path: item.file_path,
          is_dir: item.is_dir,
        };
        if (this.attachments.some((existing) => this.attachmentKey(existing) === this.attachmentKey(attachment))) continue;
        this.attachments.push(attachment);
        this.addFilePreview(attachment);
      }
      this.triggerTokenCount();
    } catch (error) {
      showToast(`Не удалось прикрепить путь: ${error}`, "error");
      void trackError("chat.handlePaths", error);
    }
  }

  onFileDragEvent(payload: DragDropPayload) {
    if (this.state.isProcessing) return;
    const isDragging = payload.type === "enter" || payload.type === "over";
    this.el.viewChat.classList.toggle("drag-over", isDragging);
    if (payload.type === "drop") {
      this.el.viewChat.classList.remove("drag-over");
      void this.handlePaths(payload.paths || []);
    } else if (payload.type === "leave") {
      this.el.viewChat.classList.remove("drag-over");
    }
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

  private attachmentKey(attachment: Attachment) {
    return attachment.file_path ? `path:${attachment.file_path}` : `data:${attachment.file_name}:${attachment.mime_type}`;
  }

  private addFilePreview(attachment: Attachment) {
    const div = document.createElement('div');
    div.className = 'file-preview-item';
    const isImage = attachment.mime_type.startsWith('image/');
    if (isImage) {
      const img = document.createElement('img');
      img.alt = attachment.file_name;
      if (attachment.data_base64) {
        img.src = `data:${attachment.mime_type};base64,${attachment.data_base64}`;
      } else if (attachment.file_path) {
        void this.imageCallbacks.loadPreview?.(attachment, img);
      }
      div.appendChild(img);
    }
    const nameSpan = document.createElement('span');
    nameSpan.className = 'file-preview-name';
    nameSpan.textContent = attachment.file_name;
    if (attachment.file_path) nameSpan.title = attachment.file_path;
    div.appendChild(nameSpan);
    const remove = document.createElement('span');
    remove.className = 'file-preview-remove';
    remove.textContent = '✕';
    remove.addEventListener('click', () => {
      const idx = this.attachments.findIndex((item) => this.attachmentKey(item) === this.attachmentKey(attachment));
      if (idx !== -1) this.attachments.splice(idx, 1);
      div.remove();
      this.triggerTokenCount();
    });
    div.appendChild(remove);
    this.el.filePreview.appendChild(div);
  }

  private bindDomEvents() {
    this.el.btnSend?.addEventListener("click", (e) => { if (e.altKey) { void this.handleAppend(); return; } this.handleSend(); });
    this.el.chatInput?.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && e.ctrlKey) { e.preventDefault(); this.handleSend(); }
      // Alt+Enter: добавить в чат без запуска LLM (preventDefault — без переноса строки).
      else if (e.key === "Enter" && e.altKey) { e.preventDefault(); void this.handleAppend(); }
      // Зажатый Alt — подпись кнопки показывает действие «Добавить в чат».
      else if (e.key === "Alt") this.setAppendButtonMode(true);
    });
    this.el.chatInput?.addEventListener("keyup", (e) => { if (e.key === "Alt") this.setAppendButtonMode(false); });
    this.el.btnStop?.addEventListener("click", async () => { if (this.el.btnStop.disabled) return; this.el.btnStop.disabled = true; this.el.btnStop.innerText = "Стоп..."; this.appendMessage('system', 'Остановка...'); // Стоп только своего хода: бэкенд отменяет ход ЭТОЙ сессии (в т.ч. выводит из очереди движка мгновенно), соседние вкладки не трогает.
      await invoke("stop_processing", { sessionId: this.state.sessionId ?? "" }); this.el.btnStop.innerText = "⏹ Стоп"; });
    this.el.btnBackChat?.addEventListener("click", () => { this.el.viewSubchat.classList.remove('active'); this.el.viewChat.classList.add('active'); });
    this.el.chatInput.addEventListener("input", () => {
        this.el.chatInput.style.height = "auto";
        this.el.chatInput.style.height = `${this.el.chatInput.scrollHeight}px`;
        this.triggerDraftSave();
        this.triggerTokenCount();
    });
    this.el.chatInput.addEventListener("blur", () => { this.setAppendButtonMode(false); if (this.state.hasSession()) { clearTimeout(this.state.draftTimeout); this.persistSession(); } });
    this.el.btnAttach?.addEventListener("click", async () => {
      if (this.el.btnAttach.disabled) return;
      try {
        const selected = await openDialog({ multiple: true });
        if (!selected) return;
        const paths = Array.isArray(selected) ? selected : [selected];
        await this.handlePaths(paths);
      } catch (error) {
        showToast(`Не удалось открыть выбор файлов: ${error}`, "error");
        void trackError("chat.openAttachDialog", error);
      }
    });
    this.el.fileInput?.addEventListener("change", (e) => this.handleFileSelect((e.target as HTMLInputElement).files));
    this.el.modelSelect?.addEventListener("change", () => {
      this.el.modelSelect.dataset.modelExplicit = "true";
      this.pendingSessionModel = null;
      syncModelSelectTitle(this.el.modelSelect);
      bus.emit("model:changed", this.el.modelSelect.value);
      this.applyIndicatorVisibility();
      this.updateAttachButtonState(); this.triggerTokenCount(); this.refreshSendAvailability();
      const v = this.el.modelSelect.value;
      void invoke("set_last_model", { path: v }).catch(() => {});
      if (this.state.hasSession()) this.persistSession();
    });
    this.el.agentSelect?.addEventListener("change", () => {
      this.pendingSessionAgent = null;
      this.triggerTokenCount();
      this.refreshSendAvailability();
      // Смена агента меняет ответ на «есть ли что схлопывать» — тумблер должен
      // появиться или исчезнуть сразу, а не после перезагрузки вкладки.
      this.applyStrategyToggleState();
      const v = this.el.agentSelect.value;
      void invoke("set_config_value", { key: "last_agent", value: v }).catch(() => {});
      if (this.state.hasSession()) this.persistSession();
    });
    this.el.btnAgentSettings?.addEventListener("click", () => {
      bus.emit("agents:pick-requested");
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

  /**
   * Синхронизировать тумблер режима с выбранным агентом.
   *
   * Проверка по `fusable_groups` из каталога, а не «у графа есть секция groups»:
   * группа может быть нефьюзируемой (роутер внутри, нечем склеивать), и тумблер
   * над ней обещал бы режим, который движок отвергнёт.
   *
   * У агента без схлопываемых групп тумблер виден, но неактивен и выключен,
   * а состояние принудительно `detailed`: «мы могли бы, но не смешиваем» —
   * честный ответ агента без групп, а не молчаливое «как в файле».
   */
  private applyStrategyToggleState() {
    // `?.` — как у `agentSelect`: элемент строит страница чата всегда, но набор
    // элементов в тестовых харнессах собирается вручную, и падать из-за
    // отсутствующего декоративного контрола не должен весь контроллер.
    const toggle = this.el.strategyToggle;
    if (!toggle) return;
    const fusable = hasFusableGroups(this.el.agentSelect?.value);
    toggle.disabled = !fusable;
    toggle.checked = fusable && this.state.executionStrategy === "fused";
    if (!fusable) {
      // Лог — ТОЛЬКО при реальном сбросе: applyStrategyToggleState вызывается и на
      // перерисовках каталога (model-catalog-changed), где состояние уже detailed,
      // и порция одинаковых строк каждое событие скрывала бы настоящий смысл.
      if (this.state.executionStrategy === "fused") {
        logFront("[chat] у агента без схлопываемых групп режим сброшен на «подробно (Detailed)»");
      }
      this.state.executionStrategy = "detailed";
    }
  }

  private bindStrategyToggle() {
    this.el.strategyToggle?.addEventListener("change", () => {
      this.state.executionStrategy = this.el.strategyToggle?.checked ? "fused" : "detailed";
      logFront(`[chat] режим исполнения: ${this.state.executionStrategy === "fused" ? "Fused Mode" : "подробно (Detailed)"}`);
    });
    this.applyStrategyToggleState();
  }

  private bindBusEvents() {
    bus.on("model-catalog-changed", () => {
      fillModelSelect(this.el.modelSelect);
      fillAgentSelect(this.el.agentSelect);
      if (this.applyPendingSessionSelection()) bus.emit("model:changed", this.el.modelSelect.value);
      // Каталог мог привести модель к облачной (или вернуть отложенную сессионную).
      this.applyIndicatorVisibility();
      this.applyStrategyToggleState();
      this.refreshSendAvailability();
    });
    bus.on("config:loaded", (config: any) => {
      // Общие каталоги уже в store (заполнил SettingsController) — перерисовываем
      // селекты этой вкладки, сохраняя текущий выбор.
      fillModelSelect(this.el.modelSelect);
      fillAgentSelect(this.el.agentSelect);
      if (this.applyPendingSessionSelection()) bus.emit("model:changed", this.el.modelSelect.value);
      this.applyIndicatorVisibility();
      this.applyStrategyToggleState();
      this.refreshSendAvailability();
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
