import logoUrl from "../../logo.svg?raw";

/// DOM-конструктор страницы чат-вкладки (браузерный UI).
///
/// Вкладка содержит две под-страницы: `.chat-main-page` (история + инпут) и
/// `.chat-sub-page` (отчёт сабагента), переключаемые классом `.active`.
/// Для «главной» вкладки (`main: true`) вместо списка сообщений показывается
/// экран новой вкладки: логотип `logo.svg` над полем ввода + рейл разделов.
///
/// Селекты агента/модели и контролы ввода создаются ПО-ВКЛАДКЕ (страниц может
/// быть несколько одновременно). Слайдеры сэмплинга — общие для приложения
/// (живут в разделе «Настройки»), передаются через `shared`.

export interface SharedChatControls {
  maxGenSlider: HTMLInputElement;
  tempSlider: HTMLInputElement;
  topkSlider: HTMLInputElement;
  toppSlider: HTMLInputElement;
  minpSlider: HTMLInputElement;
  reppenSlider: HTMLInputElement;
  prespenSlider: HTMLInputElement;
}

export interface ChatPageElements extends SharedChatControls {
  pageRoot: HTMLDivElement;
  viewChat: HTMLDivElement;
  viewSubchat: HTMLDivElement;
  chatHistory: HTMLDivElement;
  chatInput: HTMLTextAreaElement;
  btnSend: HTMLButtonElement;
  btnStop: HTMLButtonElement;
  agentSelect: HTMLSelectElement;
  btnAgentSettings: HTMLButtonElement;
  modelSelect: HTMLSelectElement;
  /** Тумблер режима исполнения графа: ВКЛ = Fused, ВЫКЛ = Detailed. */
  strategyToggle: HTMLInputElement;
  subchatHistory: HTMLDivElement;
  subchatTitle: HTMLSpanElement;
  btnBackChat: HTMLButtonElement;
  btnAttach: HTMLButtonElement;
  fileInput: HTMLInputElement;
  filePreview: HTMLDivElement;
  tokenCounter: HTMLDivElement;
  engineBadge: HTMLDivElement;
  cloudCounter: HTMLDivElement;
  btnSetWorkdir: HTMLButtonElement;
  currentWorkdir: HTMLSpanElement;
  mainScreen: HTMLDivElement | null;
}

const py = (tag: string, className?: string): HTMLElement => {
  const el = document.createElement(tag);
  if (className) el.className = className;
  return el;
};

let logoInstanceSeq = 0;

function namespaceLogoIds(svg: string, prefix: string): string {
  return svg
    .replace(/\sid="([^"]+)"/g, ` id="${prefix}_$1"`)
    .replace(/url\(#([^)\s]+)\)/gi, `url(#${prefix}_$1)`)
    .replace(/\bhref="#([^"]+)"/g, ` href="#${prefix}_$1"`);
}

function inputRow(children: HTMLElement[]): HTMLDivElement {
  const row = py("div", "chat-input-controls-left") as HTMLDivElement;
  children.forEach(c => row.appendChild(c));
  return row;
}

/// Строит DOM страницы чат-вкладки.
export function buildChatPage(main: boolean, shared: SharedChatControls): ChatPageElements {
  const pageRoot = py("div", "tab-page") as HTMLDivElement;

  // ── Основная под-страница чата ──
  const viewChat = py("div", "view chat-layout chat-main-page") as HTMLDivElement;
  if (main) viewChat.classList.add("main-mode");

  let mainScreen: HTMLDivElement | null = null;
  if (main) {
    mainScreen = py("div", "chat-main-screen") as HTMLDivElement;
    const logo = document.createElement("div");
    logo.className = "main-screen-logo";
    logo.innerHTML = namespaceLogoIds(logoUrl, `lg${Date.now().toString(36)}${(logoInstanceSeq++).toString(36)}`);
    logo.setAttribute("aria-label", "King Orch");

    const stage = py("div", "main-screen-stage");
    stage.appendChild(logo);
    mainScreen.appendChild(stage);
    viewChat.appendChild(mainScreen);
  }

  const chatHistory = py("div", "chat-history") as HTMLDivElement;
  viewChat.appendChild(chatHistory);

  // Индикатор обработки (спиннер + статус + таймер) живёт В ПОТОКЕ сообщений:
  // контроллер вставляет его в chatHistory на время обработки. Отдельной
  // панели прогресса между чатом и полем ввода больше нет.

  const inputArea = py("div", "chat-input-area group-box");
  const chatInput = document.createElement("textarea");
  chatInput.className = "chat-input";
  chatInput.placeholder = "Спросите что угодно (Enter — перенос строки, Ctrl+Enter — отправить)...";
  inputArea.appendChild(chatInput);

  const filePreview = py("div", "file-preview-row") as HTMLDivElement;
  inputArea.appendChild(filePreview);

  const controls = py("div", "chat-input-controls");
  const left = inputRow([]);
  const btnAttach = document.createElement("button");
  btnAttach.className = "btn-icon";
  btnAttach.title = "Прикрепить файл (изображение/аудио)";
  btnAttach.textContent = "📎";
  const fileInput = document.createElement("input");
  fileInput.type = "file";
  fileInput.accept = "image/*,audio/*,.wav,.mp3,.flac,.jpg,.jpeg,.png,.webp";
  fileInput.multiple = true;
  fileInput.style.display = "none";
  const btnSetWorkdir = document.createElement("button");
  btnSetWorkdir.className = "btn-icon";
  btnSetWorkdir.title = "Текущий каталог";
  btnSetWorkdir.textContent = "📂";
  const currentWorkdir = py("span", "workdir-path") as HTMLSpanElement;
  currentWorkdir.textContent = "—";
  currentWorkdir.title = "Текущий каталог";
  const agentSelect = document.createElement("select");
  agentSelect.className = "agent-select";
  agentSelect.title = "Выбор агента";
  // Шестерёнка рядом со списком: выбор набора агентов живёт в Настройках,
  // и кнопка — короткий путь туда из чата.
  const btnAgentSettings = document.createElement("button");
  btnAgentSettings.className = "btn-icon";
  btnAgentSettings.title = "Настроить список агентов (Настройки)";
  btnAgentSettings.textContent = "⚙";
  btnAgentSettings.dataset.action = "open-agent-picker";
  const modelSelect = document.createElement("select");
  modelSelect.className = "model-select";
  modelSelect.title = "Выбор модели GGUF";
  const tokenCounter = py("div", "token-counter") as HTMLDivElement;
  // Стартовый placeholder до config:loaded (лимит ещё не известен) — «0 / 8192»
  // было ложью, если конфиг задаёт другой лимит или модель облачная.
  tokenCounter.textContent = "— / …";
  tokenCounter.title = "Токены: Текущие / Лимит контекста";
  const engineBadge = py("div", "engine-badge engine-idle") as HTMLDivElement;
  engineBadge.textContent = "…";
  engineBadge.title = "Режим движка (GPU/CPU)";
  const cloudCounter = py("div", "engine-badge engine-cloud") as HTMLDivElement;
  cloudCounter.textContent = "☁ 0";
  cloudCounter.title = "Количество запросов к облачной LLM за этот чат";
  cloudCounter.style.display = "none";
  // Тумблер режима исполнения графа: ВКЛ = Fused (группы одним вызовом),
  // ВЫКЛ = Detailed (подробно по нодам). У агента без схлопываемых групп
  // переключать нечего — контроллер держит тумблер выключенным и заблокированным.
  // Класс `.strategy-switch-label` не `.agent-select`: селекты агентов
  // находятся по этому классу в UI и в тестах, и новый контроллер под ним
  // маскировался бы под выбор агента.
  const strategyToggle = document.createElement("input");
  strategyToggle.type = "checkbox";
  strategyToggle.className = "strategy-switch";
  const strategyToggleLabel = document.createElement("label");
  strategyToggleLabel.className = "strategy-switch-label";
  strategyToggleLabel.title = "Режим исполнения графа: подробно по нодам или группами одним вызовом";
  const strategyToggleTrack = py("span", "strategy-switch-track");
  const strategyToggleText = py("span", "strategy-switch-text");
  strategyToggleText.textContent = "Схлопнуто ⚡";
  strategyToggleLabel.append(strategyToggle, strategyToggleTrack, strategyToggleText);

  left.append(btnAttach, fileInput, btnSetWorkdir, currentWorkdir, agentSelect, btnAgentSettings, modelSelect, strategyToggleLabel, tokenCounter, engineBadge, cloudCounter);

  const right = py("div", "chat-input-controls-right");
  const btnStop = document.createElement("button");
  btnStop.className = "btn-danger";
  btnStop.textContent = "⏹ Стоп";
  btnStop.disabled = true;
  const btnSend = document.createElement("button");
  btnSend.className = "btn-primary";
  btnSend.style.flexDirection = "column";
  btnSend.style.lineHeight = "1.2";
  btnSend.style.whiteSpace = "normal";
  btnSend.innerHTML = "Отправить<br><small>Ctrl+Enter</small>";
  right.appendChild(btnStop);
  right.appendChild(btnSend);

  controls.appendChild(left);
  controls.appendChild(right);
  inputArea.appendChild(controls);
  viewChat.appendChild(inputArea);

  // ── Под-страница отчёта сабагента ──
  const viewSubchat = py("div", "view chat-layout") as HTMLDivElement;
  const subchatHeader = py("div", "subchat-header");
  const btnBackChat = document.createElement("button");
  btnBackChat.className = "btn-secondary";
  btnBackChat.textContent = "⬅ Назад к чату";
  const subchatTitle = document.createElement("span");
  subchatTitle.className = "subchat-title";
  subchatTitle.textContent = "Отчет сабагента";
  subchatHeader.appendChild(btnBackChat);
  subchatHeader.appendChild(subchatTitle);
  const subchatHistory = py("div", "chat-history") as HTMLDivElement;
  viewSubchat.appendChild(subchatHeader);
  viewSubchat.appendChild(subchatHistory);

  pageRoot.appendChild(viewChat);
  pageRoot.appendChild(viewSubchat);

  return {
    pageRoot,
    viewChat,
    viewSubchat,
    chatHistory,
    chatInput,
    btnSend,
    btnStop,
    agentSelect,
    btnAgentSettings,
    modelSelect,
    strategyToggle,
    subchatHistory,
    subchatTitle,
    btnBackChat,
    btnAttach,
    fileInput,
    filePreview,
    tokenCounter,
    engineBadge,
    cloudCounter,
    btnSetWorkdir,
    currentWorkdir,
    mainScreen,
    maxGenSlider: shared.maxGenSlider,
    tempSlider: shared.tempSlider,
    topkSlider: shared.topkSlider,
    toppSlider: shared.toppSlider,
    minpSlider: shared.minpSlider,
    reppenSlider: shared.reppenSlider,
    prespenSlider: shared.prespenSlider,
  };
}

/// Строит DOM страницы вкладки-веб-страницы (iframe 9Router Web UI).
export function buildWebviewPage(url: string): HTMLDivElement {
  const pageRoot = py("div", "tab-page") as HTMLDivElement;
  const wrap = py("div", "webview-wrap");
  const iframe = document.createElement("iframe");
  iframe.className = "webview-frame";
  iframe.src = url;
  wrap.appendChild(iframe);
  pageRoot.appendChild(wrap);
  return pageRoot;
}