import { logFront } from "@my-tauri-plugins/plugin-logs";
import type { GraphElements, GraphController } from "./graph";
import { AgentTestController } from "./agent-test";
import type { AgentTestElements } from "./agent-test";
import { CodingTestController } from "./coding-test";
import type { CodingTestElements } from "./coding-test";

/**
 * Экземпляр страницы «Студия агентов».
 *
 * Разметка НЕ статична в `index.html`: она лежит в `<template id="template-agent-studio">`,
 * и каждая вкладка студии создаёт из неё СВОЙ DOM-узел. Раньше студия была синглтоном
 * (один узел `#view-agent-studio` + один `GraphController` на приложение), поэтому вторую
 * вкладку нельзя было даже открыть: `TabController` дедуплицировал секцию, а если бы
 * дедупликацию снять — обе вкладки делили бы один холст, один открытый файл и один undo.
 *
 * Ключевая идея развязки: у каждого экземпляра свой префикс id (`s-<p>-...`), и все
 * поиски элементов идут В ПРЕДЕЛАХ корня экземпляра, а не по `document`. Иначе две студии
 * находили бы элементы друг друга по одинаковым id.
 *
 * Файл лежит в `controllers/`, а НЕ в `ui/`: дверь `ui/index.ts` реэкспортирует всё
 * содержимое папки, поэтому файл там создал бы цикл `ui -> studio-page -> ui`.
 */
export interface AgentStudioPage {
  /** Корень страницы: то, что вкладка вставляет себе в слот. */
  root: HTMLElement;
  /** Кнопки суб-вкладок. */
  subtabs: {
    graph: HTMLButtonElement;
    aiTest: HTMLButtonElement;
    codingTest: HTMLButtonElement;
  };
  /** Панели суб-вкладок. */
  views: {
    graph: HTMLElement;
    aiTest: HTMLElement;
    codingTest: HTMLElement;
  };
  /**
   * Элементы редактора графа для `GraphController`.
   *
   * `root` входит в набор намеренно: он задаёт границу поиска для полей сайдбара
   * (`ge-*`) и нод холста (`node-*`), которые раньше искались по всему документу.
   */
  graph: GraphElements & { root: HTMLElement };
  /** Элементы панели «Тест агентов ИИ». */
  agentTest: AgentTestElements;
  /** Элементы панели «Кодинг тест LLM». */
  codingTest: CodingTestElements;
  /** Показать одну из суб-вкладок. */
  switchSubTab(tab: StudioSubTab): void;
}

export type StudioSubTab = "graph" | "ai-test" | "coding-test";

/** Префикс id внутри конкретного экземпляра студии. */
export function studioPrefix(instanceKey: string): string {
  return `s-${instanceKey}-`;
}

/**
 * Найти элемент экземпляра по «короткому» id, как он записан в шаблоне.
 *
 * Без префикса (`id="subtab-graph"`) искать нельзя: два экземпляра получили бы один и
 * тот же элемент — это ровно тот класс ошибки, ради которого заведён префикс.
 */
function pick<T extends Element>(root: HTMLElement, prefix: string, id: string): T {
  const el = root.querySelector<T>(`#${prefix}${id}`);
  if (!el) {
    // Молча вернуть null нельзя: без элемента контроллер упал бы позже и в другом месте,
    // а причина («шаблон и код разъехались по id») осталась бы невидимой.
    const message = `[studio] В шаблоне нет элемента #${prefix}${id} — разметка и код разошлись по id`;
    logFront(message);
    throw new Error(message);
  }
  return el;
}

/**
 * Создать экземпляр студии агентов из шаблона.
 *
 * `instanceKey` делает id уникальными в пределах документа. Для единственного
 * «исторического» экземпляра допустим пустой ключ — тогда id имеют вид `s--graph-container`,
 * а внешний код, который искал студию по старым id, продолжает работать через `root`.
 */
export function buildAgentStudioPage(instanceKey = ""): AgentStudioPage {
  const template = document.getElementById("template-agent-studio") as HTMLTemplateElement | null;
  if (!template) {
    const message = "Не найден шаблон студии агентов (#template-agent-studio)";
    logFront(`[studio] ${message}`);
    throw new Error(message);
  }

  const prefix = studioPrefix(instanceKey);
  const frag = template.content.cloneNode(true) as DocumentFragment;
  // `{p}` в шаблоне — метка места ключа; подстановка идёт на КЛОНЕ, поэтому сам шаблон
  // остаётся неизменным и годится для следующей вкладки.
  frag.querySelectorAll<HTMLElement>("[id]").forEach((el) => {
    el.id = el.id.replace("{p}", instanceKey);
  });

  const root = document.createElement("div");
  root.className = "view";
  root.id = `${prefix}view-agent-studio`;
  root.appendChild(frag);

  // Страховка от расхождения шаблона и кода: неразвёрнутый `{p}` (или новый id без метки)
  // должен падать здесь с понятной причиной, а не «элемент не найден» где-то позже.
  const unexpanded = root.querySelector<HTMLElement>('[id*="{p}"]');
  if (unexpanded) {
    const message = `[studio] id "${unexpanded.id}" не развёрнут: в шаблоне нет подстановки {p}`;
    logFront(message);
    throw new Error(message);
  }

  const subtabs = {
    graph: pick<HTMLButtonElement>(root, prefix, "subtab-graph"),
    aiTest: pick<HTMLButtonElement>(root, prefix, "subtab-ai-test"),
    codingTest: pick<HTMLButtonElement>(root, prefix, "subtab-coding-test"),
  };
  const views = {
    graph: pick<HTMLElement>(root, prefix, "view-sub-graph"),
    aiTest: pick<HTMLElement>(root, prefix, "view-sub-ai-test"),
    codingTest: pick<HTMLElement>(root, prefix, "view-sub-coding-test"),
  };

  const graph: GraphElements & { root: HTMLElement } = {
    root,
    graphContainer: pick<HTMLDivElement>(root, prefix, "graph-container"),
    graphSidebar: pick<HTMLDivElement>(root, prefix, "graph-sidebar"),
    graphDetailTitle: pick<HTMLSpanElement>(root, prefix, "graph-detail-title"),
    graphDetailContent: pick<HTMLDivElement>(root, prefix, "graph-detail-content"),
    graphSidebarClose: pick<HTMLButtonElement>(root, prefix, "graph-sidebar-close"),
    btnOpenWorkflow: pick<HTMLButtonElement>(root, prefix, "btn-open-workflow"),
    btnSaveWorkflow: pick<HTMLButtonElement>(root, prefix, "btn-save-workflow"),
    currentWorkflowName: pick<HTMLSpanElement>(root, prefix, "current-workflow-name"),
    yamlWarning: pick<HTMLDetailsElement>(root, prefix, "graph-yaml-warning"),
    yamlWarningList: pick<HTMLUListElement>(root, prefix, "graph-yaml-warning-list"),
    btnUndo: pick<HTMLButtonElement>(root, prefix, "btn-undo"),
    btnRedo: pick<HTMLButtonElement>(root, prefix, "btn-redo"),
    dirtyIndicator: pick<HTMLSpanElement>(root, prefix, "dirty-indicator"),
  };

  const agentTest: AgentTestElements = {
    testFilePath: pick<HTMLInputElement>(root, prefix, "test-file-path"),
    btnSelectTestFile: pick<HTMLButtonElement>(root, prefix, "btn-select-test-file"),
    testAgentList: pick<HTMLDivElement>(root, prefix, "test-agent-list"),
    testModelList: pick<HTMLDivElement>(root, prefix, "test-model-list"),
    btnRunTest: pick<HTMLButtonElement>(root, prefix, "btn-run-test"),
    testProgress: pick<HTMLDivElement>(root, prefix, "test-progress"),
    testStatusLabel: pick<HTMLDivElement>(root, prefix, "test-status-label"),
    testProgressBar: pick<HTMLDivElement>(root, prefix, "test-progress-bar"),
    testResultsBox: pick<HTMLDivElement>(root, prefix, "test-results-box"),
    testResultsContent: pick<HTMLDivElement>(root, prefix, "test-results-content"),
    btnSaveTestResults: pick<HTMLButtonElement>(root, prefix, "btn-save-test-results"),
    testModeSelect: pick<HTMLSelectElement>(root, prefix, "test-mode-select"),
    yamlTestPanel: pick<HTMLDivElement>(root, prefix, "yaml-test-panel"),
    pipelineTestPanel: pick<HTMLDivElement>(root, prefix, "pipeline-test-panel"),
    pipelineTestList: pick<HTMLDivElement>(root, prefix, "pipeline-test-list"),
    pipelineModelList: pick<HTMLDivElement>(root, prefix, "pipeline-model-list"),
    btnRunPipelineTest: pick<HTMLButtonElement>(root, prefix, "btn-run-pipeline-test"),
    pipelineProgress: pick<HTMLDivElement>(root, prefix, "pipeline-progress"),
    pipelineStatusLabel: pick<HTMLDivElement>(root, prefix, "pipeline-status-label"),
    pipelineProgressBar: pick<HTMLDivElement>(root, prefix, "pipeline-progress-bar"),
    pipelineResultsBox: pick<HTMLDivElement>(root, prefix, "pipeline-results-box"),
    pipelineResultsContent: pick<HTMLDivElement>(root, prefix, "pipeline-results-content"),
    btnStopPipelineTest: pick<HTMLButtonElement>(root, prefix, "btn-stop-pipeline-test"),
    customTestPanel: pick<HTMLDivElement>(root, prefix, "custom-test-panel"),
    customPromptInput: pick<HTMLTextAreaElement>(root, prefix, "custom-prompt-input"),
    customAgentList: pick<HTMLDivElement>(root, prefix, "custom-agent-list"),
    customModelList: pick<HTMLDivElement>(root, prefix, "custom-model-list"),
    btnRunCustomTest: pick<HTMLButtonElement>(root, prefix, "btn-run-custom-test"),
    btnStopCustomTest: pick<HTMLButtonElement>(root, prefix, "btn-stop-custom-test"),
    customProgress: pick<HTMLDivElement>(root, prefix, "custom-progress"),
    customStatusLabel: pick<HTMLDivElement>(root, prefix, "custom-status-label"),
    customProgressBar: pick<HTMLDivElement>(root, prefix, "custom-progress-bar"),
    customResultsBox: pick<HTMLDivElement>(root, prefix, "custom-results-box"),
    customResultsContent: pick<HTMLDivElement>(root, prefix, "custom-results-content"),
    reportSelect: pick<HTMLSelectElement>(root, prefix, "report-select"),
    btnRefreshReports: pick<HTMLButtonElement>(root, prefix, "btn-refresh-reports"),
    reportView: pick<HTMLDivElement>(root, prefix, "report-view"),
  };

  const codingTest: CodingTestElements = {
    codingModelList: pick<HTMLDivElement>(root, prefix, "coding-model-list"),
    codingSuiteList: pick<HTMLDivElement>(root, prefix, "coding-suite-list"),
    codingQuickCount: pick<HTMLInputElement>(root, prefix, "coding-quick-count"),
    codingVrBudget: pick<HTMLInputElement>(root, prefix, "coding-vr-budget"),
    btnRunCodingTest: pick<HTMLButtonElement>(root, prefix, "btn-run-coding-test"),
    btnStopCodingTest: pick<HTMLButtonElement>(root, prefix, "btn-stop-coding-test"),
    codingProgress: pick<HTMLDivElement>(root, prefix, "coding-progress"),
    codingStatusLabel: pick<HTMLDivElement>(root, prefix, "coding-status-label"),
    codingProgressBar: pick<HTMLDivElement>(root, prefix, "coding-progress-bar"),
    codingLog: pick<HTMLDivElement>(root, prefix, "coding-log"),
    codingResultsBox: pick<HTMLDivElement>(root, prefix, "coding-results-box"),
    codingResultsContent: pick<HTMLDivElement>(root, prefix, "coding-results-content"),
    codingReportLink: pick<HTMLAnchorElement>(root, prefix, "coding-report-link"),
  };

  /**
   * Переключение суб-вкладок живёт в экземпляре, а не в глобальной переменной:
   * у каждой вкладки студии своя активная суб-вкладка (в одной открыт граф,
   * в другой — тесты), и переключение в одной не должно дёргать другую.
   */
  const switchSubTab = (tab: StudioSubTab): void => {
    subtabs.graph.classList.toggle("active", tab === "graph");
    subtabs.aiTest.classList.toggle("active", tab === "ai-test");
    subtabs.codingTest.classList.toggle("active", tab === "coding-test");
    views.graph.classList.toggle("active", tab === "graph");
    views.aiTest.classList.toggle("active", tab === "ai-test");
    views.codingTest.classList.toggle("active", tab === "coding-test");
  };

  return { root, subtabs, views, graph, agentTest, codingTest, switchSubTab };
}

/**
 * Готовая к работе вкладка студии: страница + её контроллеры.
 *
 * Именно это должно быть «одной строкой»: `createAgentStudioTab(id)` отдаёт вкладку,
 * которая сама владеет своим графом, своими тестовыми панелями и своим активным
 * суб-табом. Знание о том, как собрать студию, живёт здесь, а не растекается по
 * `TabController` и `main.ts` (иначе вторая вкладка снова потребовала бы правок в трёх местах).
 */
export interface AgentStudioTab {
  /** Корень страницы для вставки в слот вкладки. */
  root: HTMLElement;
  /** Сама страница (её суб-вкладки и панели). */
  page: AgentStudioPage;
  /** Вкладка стала активной: поднять холст графа (ленивая загрузка кода редактора). */
  activate(): void;
  /** Вкладка закрывается: освободить ресурсы экземпляра. */
  dispose(): void;
}

/**
 * Создать рабочий экземпляр студии агентов.
 *
 * `loadGraphController` передаётся снаружи, а не импортируется из двери `./index`:
 * иначе получился бы цикл `index -> studio-page -> index`. Побочная польза — фабрику
 * можно проверить тестом, подсунув подставной загрузчик редактора.
 *
 * Редактор графа грузится динамически (тяжёлый drawflow не должен попадать в стартовый
 * бандл), поэтому вкладка создаётся СИНХРОННО, а холст поднимается при первом `activate()`.
 * Контроллеры тестов создаются сразу: они лёгкие.
 */
export function createAgentStudioTab(
  instanceKey: string,
  loadGraphController: () => Promise<new (el: GraphElements) => GraphController>,
): AgentStudioTab {
  const page = buildAgentStudioPage(instanceKey);

  const agentTest = new AgentTestController(page.agentTest);
  const codingTest = new CodingTestController(page.codingTest);

  let activeSubTab: StudioSubTab = "graph";
  let graphCtrl: GraphController | null = null;
  let graphPromise: Promise<GraphController | null> | null = null;
  let disposed = false;

  /**
   * Ленивая загрузка редактора графа (только для ЭТОГО экземпляра).
   *
   * Отдаёт `null`, если вкладку уже закрыли, пока модуль редактора грузился: иначе
   * контроллер создался бы после `dispose()` и остался бы жить без вкладки (с холстом
   * в отсоединённом DOM и слушателем на `document`).
   */
  const ensureGraph = (): Promise<GraphController | null> => {
    if (graphCtrl) return Promise.resolve(graphCtrl);
    if (!graphPromise) {
      graphPromise = loadGraphController().then((GraphControllerClass) => {
        if (disposed) return null;
        graphCtrl = new GraphControllerClass(page.graph);
        return graphCtrl;
      });
    }
    return graphPromise;
  };
  const switchSubTab = (tab: StudioSubTab): void => {
    activeSubTab = tab;
    page.switchSubTab(tab);
    if (tab === "graph") {
      void ensureGraph().then((ctrl) => {
        if (ctrl) requestAnimationFrame(() => ctrl.onTabActivated());
      });
    }
  };

  page.subtabs.graph.addEventListener("click", () => switchSubTab("graph"));
  page.subtabs.aiTest.addEventListener("click", () => { switchSubTab("ai-test"); agentTest.init(); });
  page.subtabs.codingTest.addEventListener("click", () => { switchSubTab("coding-test"); codingTest.init(); });

  return {
    root: page.root,
    page,
    activate(): void {
      // Возвращаемся на ту суб-вкладку, которая была активна в ЭТОЙ вкладке: у каждой
      // студии своё состояние, и переключение в соседней не должно его сбрасывать.
      switchSubTab(activeSubTab);
    },
    dispose(): void {
      disposed = true;
      // Контроллеры отпускают то, что переживает DOM: подписку на шину, Tauri-`listen`
      // и слушатели на `document` (горячие клавиши, контекстное меню в body). Пока они
      // висели, закрытая вкладка продолжала держать память и реагировать на события,
      // а её контекстное меню оставалось в интерфейсе.
      graphCtrl?.destroy();
      graphCtrl = null;
      graphPromise = null;
      agentTest.destroy();
      codingTest.destroy();
      // Снимаем корень из DOM: слушатели внутри него собираются вместе с ним, а
      // `GraphController` больше не найдёт свои элементы (его поиски идут от `root`).
      page.root.remove();
    },
  };
}
