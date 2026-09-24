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
  chatFeedback: HTMLDivElement;
  progressBar: HTMLDivElement;
  statusLabel: HTMLDivElement;
  agentSelect: HTMLSelectElement;
  modelSelect: HTMLSelectElement;
  subchatHistory: HTMLDivElement;
  subchatTitle: HTMLSpanElement;
  btnBackChat: HTMLButtonElement;
  btnAttach: HTMLButtonElement;
  fileInput: HTMLInputElement;
  filePreview: HTMLDivElement;
  tokenCounter: HTMLDivElement;
  engineBadge: HTMLDivElement;
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

  // ── Панель прогресса обработки ──
  const feedback = py("div", "feedback") as HTMLDivElement;
  feedback.style.display = "none";
  const resultToolbar = py("div", "result-toolbar");
  const statusLabel = py("div", "status-label") as HTMLDivElement;
  statusLabel.textContent = "Обработка...";
  resultToolbar.appendChild(statusLabel);
  const progressWrap = py("div", "progress-bar-container");
  const progressBar = py("div", "progress-bar") as HTMLDivElement;
  progressWrap.appendChild(progressBar);
  feedback.appendChild(resultToolbar);
  feedback.appendChild(progressWrap);
  viewChat.appendChild(feedback);

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
  const modelSelect = document.createElement("select");
  modelSelect.className = "model-select";
  modelSelect.title = "Выбор модели GGUF";
  const btnAddModel = document.createElement("button");
  btnAddModel.className = "btn-secondary btn-add-model";
  btnAddModel.title = "Добавить новую модель (локально)";
  btnAddModel.textContent = "+";
  const tokenCounter = py("div", "token-counter") as HTMLDivElement;
  tokenCounter.textContent = "0 / 8192";
  tokenCounter.title = "Токены: Текущие / Лимит контекста";
  const engineBadge = py("div", "engine-badge engine-idle") as HTMLDivElement;
  engineBadge.textContent = "…";
  engineBadge.title = "Режим движка (GPU/CPU)";

  left.append(btnAttach, fileInput, btnSetWorkdir, currentWorkdir, agentSelect, modelSelect, btnAddModel, tokenCounter, engineBadge);

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
    chatFeedback: feedback,
    progressBar,
    statusLabel,
    agentSelect,
    modelSelect,
    subchatHistory,
    subchatTitle,
    btnBackChat,
    btnAttach,
    fileInput,
    filePreview,
    tokenCounter,
    engineBadge,
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
  } as ChatPageElements;
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