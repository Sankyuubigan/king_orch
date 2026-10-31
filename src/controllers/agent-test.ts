import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { showToast } from "../ui";
import { store } from "../store";
import { bus } from "../events";
import { trackError } from "../telemetry";
import type { AgentEntry, TestCaseDef, SingleTestResult, PipelineTestInfo, PipelineTestResult, CustomPromptResult, TestReportMeta } from "../types";

/** Одна строка батч-прогона: результат харнесса либо ошибка запуска модели. */
interface PipelineBatchEntry {
  model: string;
  error: string | null;
  result: PipelineTestResult | null;
}

/**
 * Одна строка батча кастомного промпта (совпадает с Rust `CustomPromptEntry`
 * и с entries в JSON-отчёте — просмотрщик читает отчёты той же структурой).
 */
interface CustomPromptEntry {
  model: string;
  error: string | null;
  result: CustomPromptResult | null;
}

/** Сообщение цепочки переходов (совпадает с Rust AgentMessage в отчётах). */
interface ChainMessage {
  author: string;
  content_preview: string;
  msg_type: string;
}

export interface AgentTestElements {
  testFilePath: HTMLInputElement;
  btnSelectTestFile: HTMLButtonElement;
  testAgentList: HTMLDivElement;
  testModelList: HTMLDivElement;
  btnRunTest: HTMLButtonElement;
  testProgress: HTMLDivElement;
  testStatusLabel: HTMLDivElement;
  testProgressBar: HTMLDivElement;
  testResultsBox: HTMLDivElement;
  testResultsContent: HTMLDivElement;
  btnSaveTestResults: HTMLButtonElement;
  testModeSelect: HTMLSelectElement;
  yamlTestPanel: HTMLDivElement;
  pipelineTestPanel: HTMLDivElement;
  pipelineTestList: HTMLDivElement;
  pipelineModelList: HTMLDivElement;
  btnRunPipelineTest: HTMLButtonElement;
  pipelineProgress: HTMLDivElement;
  pipelineStatusLabel: HTMLDivElement;
  pipelineProgressBar: HTMLDivElement;
  pipelineResultsBox: HTMLDivElement;
  pipelineResultsContent: HTMLDivElement;
  btnStopPipelineTest: HTMLButtonElement;
  customTestPanel: HTMLDivElement;
  customPromptInput: HTMLTextAreaElement;
  customAgentList: HTMLDivElement;
  customModelList: HTMLDivElement;
  btnRunCustomTest: HTMLButtonElement;
  btnStopCustomTest: HTMLButtonElement;
  customProgress: HTMLDivElement;
  customStatusLabel: HTMLDivElement;
  customProgressBar: HTMLDivElement;
  customResultsBox: HTMLDivElement;
  customResultsContent: HTMLDivElement;
  reportSelect: HTMLSelectElement;
  btnRefreshReports: HTMLButtonElement;
  reportView: HTMLDivElement;
}

export class AgentTestController {
  private el: AgentTestElements;
  private testCases: TestCaseDef[] = [];
  private selectedAgents: Set<string> = new Set();
  private selectedModels: Set<string> = new Set();
  private results: SingleTestResult[] | null = null;
  private agentsLoaded = false;
  private modelsLoaded = false;

  // Pipeline test state
  private pipelineTests: PipelineTestInfo[] = [];
  private selectedPipelineTest: string | null = null;
  /** Мультивыбор: фикстуру можно прогнать сразу на нескольких моделях (по очереди — VRAM §6.5). */
  private selectedPipelineModels: Set<string> = new Set();
  /** Активный батч: сколько моделей всего / завершено. null = батча нет (одиночный режим событий). */
  private pipelineBatch: { total: number; done: number } | null = null;
  /** Подпись текущей модели батча — события статуса движка приклеиваются к ней. */
  private pipelineBatchLabel = "";
  /** true = юзер нажал «Остановить»: текущая модель доиграет/прервётся, следующие не стартуют. */
  private pipelineBatchCancelled = false;

  // Custom prompt test state
  private customAgentId: string | null = null;
  private selectedCustomModels: Set<string> = new Set();
  private customBatch: { total: number; done: number } | null = null;
  private customBatchLabel = "";
  private customBatchCancelled = false;
  private pipelineTestsLoaded = false;
  private pipelineModelsLoaded = false;
  private pipelineListenersSetup = false;
  /**
   * Отписки от подписок, которые переживают закрытие вкладки студии.
   *
   * Подписка на шине и `listen()` Tauri живут дольше DOM панели: без отписки закрытая
   * вкладка продолжала бы получать `model-catalog-changed` и события pipeline-теста и
   * писать в отсоединённые элементы — а на 20 вкладках это ещё и 20 живых обработчиков.
   */
  private unsubscribers: (() => void)[] = [];
  private destroyed = false;

  constructor(el: AgentTestElements) {
    this.el = el;
    this.bindEvents();
    this.unsubscribers.push(
      bus.on("model-catalog-changed", () => {
        this.agentsLoaded = false;
        void this.loadAgents();
      }),
    );
  }

  /**
   * Запомнить отписку от Tauri-события.
   *
   * `listen` возвращает промис, поэтому отписка может прийти ПОСЛЕ `destroy()` (вкладку
   * закрыли, пока шла регистрация). Такой обработчик отписывается сразу, иначе он
   * остался бы жить навсегда — ровно та утечка, ради которой `destroy()` и написан.
   */
  private trackListen(promise: Promise<() => void>): void {
    void promise.then((unlisten) => {
      if (this.destroyed) unlisten();
      else this.unsubscribers.push(unlisten);
    });
  }

  /**
   * Освободить панель: снять подписку на шину и Tauri-`listen`.
   *
   * Вызывается при закрытии вкладки студии (`AgentStudioTab.dispose`). Слушатели кнопок
   * отдельно снимать не нужно — их цели лежат внутри корня экземпляра и уходят с DOM.
   */
  destroy(): void {
    this.destroyed = true;
    for (const unsubscribe of this.unsubscribers.splice(0)) unsubscribe();
  }

  init(): void {
    if (!this.agentsLoaded) this.loadAgents();
    if (!this.modelsLoaded) this.loadModels();
    if (!this.pipelineTestsLoaded) this.loadPipelineTests();
    if (!this.pipelineModelsLoaded) this.loadPipelineModels();
    this.loadCustomAgents();
    this.loadCustomModels();
    this.loadReportList();
    this.setupPipelineListeners();
  }

  // ─── Mode switching ───

  private bindEvents(): void {
    this.el.btnSelectTestFile.addEventListener("click", () => this.selectFile());
    this.el.btnRunTest.addEventListener("click", () => this.runTest());
    this.el.btnSaveTestResults.addEventListener("click", () => this.saveResults());
    this.el.testModeSelect.addEventListener("change", () => this.switchMode());
    this.el.btnRunPipelineTest.addEventListener("click", () => this.runPipelineTest());
    this.el.btnStopPipelineTest.addEventListener("click", () => this.stopPipelineBatch());
    this.el.btnRunCustomTest.addEventListener("click", () => this.runCustomTest());
    this.el.btnStopCustomTest.addEventListener("click", () => this.stopCustomBatch());
    this.el.customPromptInput.addEventListener("input", () => this.updateCustomRunButton());
    this.el.btnRefreshReports.addEventListener("click", () => this.loadReportList());
    this.el.reportSelect.addEventListener("change", () => this.viewSelectedReport());
  }

  private switchMode(): void {
    const mode = this.el.testModeSelect.value;
    const isPipeline = mode === "pipeline";
    const isCustom = mode === "custom";
    this.el.yamlTestPanel.style.display = isPipeline || isCustom ? "none" : "block";
    this.el.pipelineTestPanel.style.display = isPipeline ? "block" : "none";
    this.el.customTestPanel.style.display = isCustom ? "block" : "none";
  }

  // ─── YAML test mode (existing) ───

  /**
   * Список для тестов НЕ ограничен выбором пользователя в чате: это
   * диагностический инструмент, и проверять надо всё, что лежит в `agents/`.
   * Раньше здесь стоял предикат `is_hidden`/`showFolderAgents`, продублированный
   * с дропдауном чата — теперь единственный источник правды (`store.agents`)
   * отдаёт весь каталог, и фильтровать его нечем и незачем.
   */
  async loadAgents(): Promise<void> {
    try {
      const agents: AgentEntry[] = await invoke("get_agents");
      store.agents = agents;
      this.el.testAgentList.innerHTML = "";
      for (const a of agents) {
        const label = document.createElement("label");
        label.className = "test-agent-row";
        const cb = document.createElement("input");
        cb.type = "checkbox";
        cb.value = a.id;
        if (this.selectedAgents.has(a.id)) cb.checked = true;
        cb.addEventListener("change", () => {
          if (cb.checked) this.selectedAgents.add(a.id);
          else this.selectedAgents.delete(a.id);
          this.updateRunButton();
        });
        label.appendChild(cb);
        const icon = a.entry_type === "workflow" ? "📁" : "📊";
        label.appendChild(document.createTextNode(` ${icon} ${a.name} (${a.id})`));
        this.el.testAgentList.appendChild(label);
      }
      this.agentsLoaded = true;
    } catch (e) {
      this.el.testAgentList.innerHTML = `<span class="test-hint">Ошибка загрузки: ${e}</span>`;
      void trackError("agentTest.loadAgents", e);
    }
  }

  async loadModels(): Promise<void> {
    try {
      const config: any = await invoke("get_config");
      this.el.testModelList.innerHTML = "";
      for (const m of config.models || []) {
        const label = document.createElement("label");
        const cb = document.createElement("input");
        cb.type = "checkbox";
        cb.value = m;
        if (this.selectedModels.has(m)) cb.checked = true;
        cb.addEventListener("change", () => {
          if (cb.checked) this.selectedModels.add(m);
          else this.selectedModels.delete(m);
          this.updateRunButton();
        });
        label.appendChild(cb);
        label.appendChild(document.createTextNode(` ${m.split(/[/\\]/).pop() || m}`));
        this.el.testModelList.appendChild(label);
      }
      this.modelsLoaded = true;
    } catch (e) {
      this.el.testModelList.innerHTML = `<span class="test-hint">Ошибка загрузки: ${e}</span>`;
      void trackError("agentTest.loadModels", e);
    }
  }

  private updateRunButton(): void {
    const hasCases = this.testCases.length > 0;
    const hasAgents = this.selectedAgents.size > 0;
    const hasModels = this.selectedModels.size > 0;
    this.el.btnRunTest.disabled = !(hasCases && hasAgents && hasModels);
  }

  private async selectFile(): Promise<void> {
    try {
      const sel = await open({
        filters: [{ name: "YAML Test Cases", extensions: ["yaml", "yml"] }],
        multiple: false,
      });
      if (!sel) return;
      const path = sel as string;
      this.el.testFilePath.value = path;
      await this.parseTestFile(path);
    } catch (e) {
      showToast(`Ошибка выбора файла: ${e}`, "error");
      void trackError("agentTest.selectFile", e);
    }
  }

  private async parseTestFile(path: string): Promise<void> {
    try {
      this.testCases = await invoke<TestCaseDef[]>("read_test_file", { path });
      showToast(`Загружено тест-кейсов: ${this.testCases.length}`, "success");
    } catch (e) {
      showToast(`Ошибка чтения файла: ${e}`, "error");
      this.testCases = [];
      void trackError("agentTest.parseTestFile", e);
    }
    this.updateRunButton();
  }

  private async runTest(): Promise<void> {
    if (this.testCases.length === 0 || this.selectedAgents.size === 0 || this.selectedModels.size === 0) {
      showToast("Выберите файл, агентов и модели", "error");
      return;
    }

    this.el.btnRunTest.disabled = true;
    this.el.testProgress.style.display = "block";
    this.el.testResultsBox.style.display = "none";
    this.results = null;

    const agentIds = Array.from(this.selectedAgents);
    const modelPaths = Array.from(this.selectedModels);
    const total = this.testCases.length * agentIds.length * modelPaths.length;

    try {
      this.el.testStatusLabel.textContent = `Запуск тестов (0/${total})...`;
      this.el.testProgressBar.style.width = "0%";

      this.results = await invoke<SingleTestResult[]>("run_iterative_test", {
        testCases: this.testCases,
        agentIds,
        modelPaths,
      });

      this.el.testStatusLabel.textContent = `Готово! Обработано ${total}/${total}`;
      this.el.testProgressBar.style.width = "100%";
      this.displayResults(this.results);
      this.el.testResultsBox.style.display = "block";
      showToast("Тестирование завершено!", "success");
    } catch (e) {
      this.el.testStatusLabel.textContent = `Ошибка: ${e}`;
      showToast(`Ошибка тестирования: ${e}`, "error");
      void trackError("agentTest.run", e);
    } finally {
      this.el.btnRunTest.disabled = false;
    }
  }

  private displayResults(results: SingleTestResult[]): void {
    const html = results.map((r, idx) => {
      const responsesHtml = Object.entries(r.responses).map(([key, val]) => {
        const isError = val.startsWith("ERROR:");
        return `<div class="result-response ${isError ? 'result-error' : ''}"><strong>${this.escapeHtml(key)}:</strong> ${this.escapeHtml(val.substring(0, 500))}</div>`;
      }).join("");
      return `<div class="test-result-item">
        <div class="result-label">Кейс #${idx + 1}</div>
        <div class="result-value"><strong>Вход:</strong> ${this.escapeHtml(r.input_data.substring(0, 200))}</div>
        <div class="result-value"><strong>Эталон:</strong> ${this.escapeHtml(r.right_answer_context.substring(0, 200))}</div>
        ${responsesHtml}
      </div>`;
    }).join("");
    this.el.testResultsContent.innerHTML = html;
  }

  private async saveResults(): Promise<void> {
    if (!this.results) return;
    try {
      const now = new Date();
      const ts = `${now.getFullYear()}-${String(now.getMonth()+1).padStart(2,'0')}-${String(now.getDate()).padStart(2,'0')}_${String(now.getHours()).padStart(2,'0')}${String(now.getMinutes()).padStart(2,'0')}${String(now.getSeconds()).padStart(2,'0')}`;
      const savePath = await save({
        defaultPath: `test_results_${ts}.yaml`,
        filters: [{ name: "YAML", extensions: ["yaml"] }],
      });
      if (!savePath) return;
      await invoke("write_test_results", { results: this.results, path: savePath });
      showToast(`Результаты сохранены: ${savePath}`, "success");
    } catch (e) {
      showToast(`Ошибка сохранения: ${e}`, "error");
      void trackError("agentTest.saveResults", e);
    }
  }

  // ─── Pipeline test mode ───

  async loadPipelineTests(): Promise<void> {
    try {
      this.pipelineTests = await invoke<PipelineTestInfo[]>("get_pipeline_test_list");
      this.el.pipelineTestList.innerHTML = "";
      if (this.pipelineTests.length === 0) {
        this.el.pipelineTestList.innerHTML = '<span class="test-hint">Нет доступных тестов</span>';
        return;
      }
      for (const t of this.pipelineTests) {
        const label = document.createElement("label");
        const cb = document.createElement("input");
        cb.type = "radio";
        cb.name = "pipeline-test";
        cb.value = t.id;
        cb.addEventListener("change", () => {
          this.selectedPipelineTest = t.id;
          this.updatePipelineRunButton();
        });
        label.appendChild(cb);
        label.appendChild(document.createTextNode(` ${t.id} (${t.workflow_name})`));
        this.el.pipelineTestList.appendChild(label);
      }
      this.pipelineTestsLoaded = true;
    } catch (e) {
      this.el.pipelineTestList.innerHTML = `<span class="test-hint">Ошибка загрузки: ${e}</span>`;
      void trackError("agentTest.loadPipelineTests", e);
    }
  }

  async loadPipelineModels(): Promise<void> {
    try {
      const config: any = await invoke("get_config");
      this.el.pipelineModelList.innerHTML = "";
      for (const m of config.models || []) {
        const label = document.createElement("label");
        const cb = document.createElement("input");
        cb.type = "checkbox";
        cb.value = m;
        if (this.selectedPipelineModels.has(m)) cb.checked = true;
        cb.addEventListener("change", () => {
          if (cb.checked) this.selectedPipelineModels.add(m);
          else this.selectedPipelineModels.delete(m);
          this.updatePipelineRunButton();
        });
        label.appendChild(cb);
        label.appendChild(document.createTextNode(` ${m.split(/[/\\]/).pop() || m}`));
        this.el.pipelineModelList.appendChild(label);
      }
      this.pipelineModelsLoaded = true;
    } catch (e) {
      this.el.pipelineModelList.innerHTML = `<span class="test-hint">Ошибка загрузки: ${e}</span>`;
      void trackError("agentTest.loadPipelineModels", e);
    }
  }

  private updatePipelineRunButton(): void {
    this.el.btnRunPipelineTest.disabled = !(this.selectedPipelineTest && this.selectedPipelineModels.size > 0);
  }

  private setupPipelineListeners(): void {
    if (this.pipelineListenersSetup) return;
    this.pipelineListenersSetup = true;

    this.trackListen(
      listen<number>("pipeline_progress", (e) => {
        if (this.pipelineBatch) {
          // Прогресс текущей модели ребейсится на весь батч:
          // (завершено моделей + доля текущей) / всего моделей.
          this.setPipelineBatchProgress(e.payload);
          return;
        }
        const pct = Math.max(0, Math.min(100, e.payload));
        this.el.pipelineProgressBar.style.width = `${pct}%`;
      }),
    );
    this.trackListen(
      listen<string>("pipeline_status", (e) => {
        if (this.pipelineBatch) {
          this.el.pipelineStatusLabel.textContent = `${this.pipelineBatchLabel} — ${e.payload}`;
          return;
        }
        this.el.pipelineStatusLabel.textContent = e.payload;
      }),
    );
    this.trackListen(
      listen<boolean>("pipeline_done", (e) => {
        if (this.pipelineBatch) {
          // Финал отдельной модели: полосу и тосты ведёт батч-цикл,
          // иначе 100% и тост середины батча перекрыли бы следующую модель.
          return;
        }
        const passed = e.payload;
        this.el.pipelineStatusLabel.textContent = passed ? "✅ Тест пройден" : "❌ Тест НЕ пройден";
        this.el.pipelineProgressBar.style.width = "100%";
        showToast(passed ? "Pipeline test пройден!" : "Pipeline test НЕ пройден", passed ? "success" : "error");
      }),
    );
    this.trackListen(
      listen<string>("custom_test_status", (e) => {
        if (this.customBatch) {
          this.el.customStatusLabel.textContent = `${this.customBatchLabel} — ${e.payload}`;
          return;
        }
        this.el.customStatusLabel.textContent = e.payload;
      }),
    );
    this.trackListen(
      listen<number>("custom_test_progress", (e) => {
        if (this.customBatch) {
          this.setCustomBatchProgress(e.payload);
          return;
        }
        const pct = Math.max(0, Math.min(100, e.payload));
        this.el.customProgressBar.style.width = `${pct}%`;
      }),
    );
  }

  /**
   * Батч-прогон фикстуры на всех отмеченных моделях.
   *
   * Модели гоняются строго последовательно: каждая — через свой `run_pipeline_test_cmd`,
   * где backend берёт permit гейта локального движка (VRAM §6.5, модели не делят VRAM).
   * Ошибка одной модели не роняет батч — она попадает строкой в сводку и в отчёт.
   */
  private async runPipelineTest(): Promise<void> {
    if (!this.selectedPipelineTest || this.selectedPipelineModels.size === 0) {
      showToast("Выберите тест и хотя бы одну модель", "error");
      return;
    }

    const testId = this.selectedPipelineTest;
    const modelPaths = Array.from(this.selectedPipelineModels);

    this.el.btnRunPipelineTest.disabled = true;
    this.el.pipelineProgress.style.display = "block";
    this.el.pipelineResultsBox.style.display = "none";
    this.el.pipelineStatusLabel.textContent = "Подготовка...";
    this.el.pipelineProgressBar.style.width = "0%";

    this.pipelineBatch = { total: modelPaths.length, done: 0 };
    this.pipelineBatchCancelled = false;
    const entries: PipelineBatchEntry[] = [];
    const batchStart = Date.now();

    this.el.btnStopPipelineTest.style.display = "inline-block";

    try {
      for (let i = 0; i < modelPaths.length; i++) {
        if (this.pipelineBatchCancelled) break;
        const modelPath = modelPaths[i];
        const shortName = modelPath.split(/[/\\]/).pop() || modelPath;
        this.pipelineBatchLabel = `Модель ${i + 1}/${modelPaths.length} (${shortName})`;
        this.el.pipelineStatusLabel.textContent = `${this.pipelineBatchLabel} — запуск...`;
        try {
          const result = await invoke<PipelineTestResult>("run_pipeline_test_cmd", {
            testId,
            modelPath,
          });
          entries.push({ model: modelPath, error: null, result });
        } catch (e) {
          entries.push({ model: modelPath, error: String(e), result: null });
          void trackError("agentTest.runPipelineBatch", e);
        }
        this.pipelineBatch.done = i + 1;
        this.setPipelineBatchProgress(0);
      }

      const totalMs = Date.now() - batchStart;
      const passedCount = entries.filter((e) => e.result?.overall_passed).length;
      const crashedCount = entries.filter((e) => e.error).length;

      this.el.pipelineStatusLabel.textContent = this.pipelineBatchCancelled
        ? `Остановлено пользователем: выполнено ${entries.length} из ${modelPaths.length} · общее время: ${(totalMs / 1000).toFixed(1)}s`
        : `Готово: ✅ ${passedCount} · ❌ ${entries.length - passedCount - crashedCount} · 💥 ${crashedCount} из ${entries.length} · общее время: ${(totalMs / 1000).toFixed(1)}s`;
      this.el.pipelineProgressBar.style.width = "100%";
      this.displayPipelineBatchResults(testId, entries, totalMs);
      this.el.pipelineResultsBox.style.display = "block";

      // Сводный отчёт батча — на диск в папку фикстуры (JSON для просмотрщика + md).
      try {
        const reportPath = await invoke<string>("write_pipeline_batch_report", {
          testId,
          entries,
          totalDurationMs: totalMs,
        });
        showToast(`Сводный отчёт: ${reportPath}`, "success");
      } catch (e) {
        showToast(`Не удалось сохранить сводный отчёт: ${e}`, "error");
        void trackError("agentTest.saveBatchReport", e);
      }

      showToast(
        crashedCount > 0
          ? `Батч завершён: пройдено ${passedCount}, ошибок запуска ${crashedCount}`
          : passedCount === entries.length
            ? "Все модели пройдены!"
            : `Пройдено ${passedCount} из ${entries.length}`,
        passedCount === entries.length && crashedCount === 0 ? "success" : "error",
      );
    } finally {
      this.pipelineBatch = null;
      this.pipelineBatchLabel = "";
      this.el.btnRunPipelineTest.disabled = false;
      this.el.btnStopPipelineTest.style.display = "none";
    }
  }

  /**
   * «Остановить» в pipeline-режиме: текущая модель прерывается через
   * `cancel_test_run` (убийство llama-server → workflow падает за секунды),
   * а батч-цикл по флагу не стартует следующие модели.
   */
  private stopPipelineBatch(): void {
    if (!this.pipelineBatch) return;
    this.pipelineBatchCancelled = true;
    void invoke("cancel_test_run").catch(() => {});
    showToast("Останавливаю текущую модель...", "info");
  }

  /** Прогресс батча: доля завершённых моделей + доля текущей (из событий движка). */
  private setPipelineBatchProgress(currentModelPct: number): void {
    if (!this.pipelineBatch) return;
    const current = Math.max(0, Math.min(100, currentModelPct)) / 100;
    const overall = ((this.pipelineBatch.done + current) / this.pipelineBatch.total) * 100;
    this.el.pipelineProgressBar.style.width = `${Math.max(0, Math.min(100, overall))}%`;
  }

  private pipelineResultHtml(result: PipelineTestResult): string {
    const levelClass = (passed: boolean) => passed ? "color: var(--green, #4caf50)" : "color: var(--red, #f44336)";
    const levelIcon = (passed: boolean) => passed ? "✅" : "❌";

    return `
      <div style="margin-bottom:12px;">
        <strong>Тест:</strong> ${this.escapeHtml(result.test_id)}<br/>
        <strong>Модель:</strong> ${this.escapeHtml(result.model_name)}<br/>
        <strong>Пайплайн:</strong> ${this.escapeHtml(result.workflow_name)}<br/>
        <strong>Длительность:</strong> ${(result.duration_ms / 1000).toFixed(1)}s<br/>
        <strong>VRAM:</strong> ${this.fmtVram(result.vram_delta_mb, result.vram_peak_mb)}<br/>
        <strong>ОЗУ llama-server (пик):</strong> ${this.fmtMb(result.ram_peak_mb)}<br/>
        <strong>Итог:</strong> <span style="${levelClass(result.overall_passed)}; font-weight:bold">
          ${levelIcon(result.overall_passed)} ${result.overall_passed ? "ПРОЙДЕН" : "НЕ ПРОЙДЕН"}
        </span>
      </div>

      <div style="margin-bottom:8px;">
        <strong>Уровень 1: Структура</strong>
        <span style="${levelClass(result.level1_structure.passed)}">${levelIcon(result.level1_structure.passed)}</span>
        <ul style="margin:4px 0 0 20px; padding:0;">
          ${result.level1_structure.details.map(d => `<li>${this.escapeHtml(d)}</li>`).join("")}
        </ul>
      </div>

      <div style="margin-bottom:8px;">
        <strong>Уровень 2: Файл</strong>
        <span style="${levelClass(result.level2_file.passed)}">${levelIcon(result.level2_file.passed)}</span>
        <ul style="margin:4px 0 0 20px; padding:0;">
          ${result.level2_file.details.map(d => `<li>${this.escapeHtml(d)}</li>`).join("")}
        </ul>
      </div>

      <div style="margin-bottom:8px;">
        <strong>Уровень 3: Functional</strong>
        <span style="${levelClass(result.level3_functional.passed)}">${levelIcon(result.level3_functional.passed)}</span>
        <ul style="margin:4px 0 0 20px; padding:0;">
          ${result.level3_functional.details.map(d => `<li>${this.escapeHtml(d)}</li>`).join("")}
        </ul>
      </div>

      ${result.report_path ? `<div style="margin-top:8px;"><strong>Отчёт:</strong> <code>${this.escapeHtml(result.report_path)}</code></div>` : ""}
    `;
  }

  /** Сводка батча: таблица моделей + детали каждой (как раньше для одиночного прогона). */
  private displayPipelineBatchResults(testId: string, entries: PipelineBatchEntry[], totalMs: number | null = null): void {
    this.el.pipelineResultsContent.innerHTML = this.pipelineBatchHtml(testId, entries, totalMs);
  }

  /** HTML сводки pipeline-батча: общий для живых результатов и просмотрщика отчётов. */
  private pipelineBatchHtml(testId: string, entries: PipelineBatchEntry[], totalMs: number | null = null): string {
    const border = "1px solid var(--border, #444)";
    const rows = entries.map((e, i) => {
      const short = e.model.split(/[/\\]/).pop() || e.model;
      if (e.error) {
        return `<tr>
          <td style="padding:4px 8px; border-bottom:${border}">${i + 1}</td>
          <td style="padding:4px 8px; border-bottom:${border}">${this.escapeHtml(short)}</td>
          <td colspan="3" style="padding:4px 8px; border-bottom:${border}; color: var(--red, #f44336)">💥 Ошибка запуска: ${this.escapeHtml(e.error.substring(0, 300))}</td>
        </tr>`;
      }
      const r = e.result!;
      const color = r.overall_passed ? "var(--green, #4caf50)" : "var(--red, #f44336)";
      return `<tr>
        <td style="padding:4px 8px; border-bottom:${border}">${i + 1}</td>
        <td style="padding:4px 8px; border-bottom:${border}">${this.escapeHtml(short)}</td>
        <td style="padding:4px 8px; border-bottom:${border}; color:${color}; font-weight:bold">${r.overall_passed ? "✅ ПРОЙДЕН" : "❌ НЕ ПРОЙДЕН"}</td>
        <td style="padding:4px 8px; border-bottom:${border}">${(r.duration_ms / 1000).toFixed(1)}s</td>
        <td style="padding:4px 8px; border-bottom:${border}">${this.fmtVram(r.vram_delta_mb, r.vram_peak_mb)}</td>
        <td style="padding:4px 8px; border-bottom:${border}">${this.fmtMb(r.ram_peak_mb)}</td>
        <td style="padding:4px 8px; border-bottom:${border}">${r.report_path ? `<code>${this.escapeHtml(r.report_path)}</code>` : ""}</td>
      </tr>`;
    }).join("");

    const totalPart = typeof totalMs === "number"
      ? ` · <strong>общее время батча:</strong> ${(totalMs / 1000).toFixed(1)}s`
      : "";
    const table = `
      <div style="margin-bottom:12px;"><strong>Тест:</strong> ${this.escapeHtml(testId)} · моделей: ${entries.length}${totalPart}</div>
      <table style="border-collapse:collapse; width:100%; margin-bottom:16px;">
        <thead><tr>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">#</th>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">Модель</th>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">Итог</th>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">Время</th>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">VRAM</th>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">ОЗУ</th>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">Отчёт</th>
        </tr></thead>
        <tbody>${rows}</tbody>
      </table>`;

    const details = entries
      .filter((e) => e.result)
      .map((e) => {
        const r = e.result!;
        return (
          this.pipelineResultHtml(r) +
          this.finalAnswerHtml(this.finalAuthorOf(r.messages), this.finalTextOf(r.messages)) +
          this.chainHtml(r.messages)
        );
      })
      .join('<hr style="border:none; border-top:1px solid var(--border, #444); margin:12px 0"/>');

    return table + details;
  }

  // ─── Custom prompt test mode (третий режим) ───

  async loadCustomAgents(): Promise<void> {
    try {
      const agents: AgentEntry[] = await invoke("get_agents");
      this.el.customAgentList.innerHTML = "";
      for (const a of agents) {
        const label = document.createElement("label");
        const rb = document.createElement("input");
        rb.type = "radio";
        rb.name = "custom-agent";
        rb.value = a.id;
        rb.addEventListener("change", () => {
          this.customAgentId = a.id;
          this.updateCustomRunButton();
        });
        label.appendChild(rb);
        const icon = a.entry_type === "workflow" ? "📁" : "📊";
        label.appendChild(document.createTextNode(` ${icon} ${a.name} (${a.id})`));
        this.el.customAgentList.appendChild(label);
      }
    } catch (e) {
      this.el.customAgentList.innerHTML = `<span class="test-hint">Ошибка загрузки: ${e}</span>`;
      void trackError("agentTest.loadCustomAgents", e);
    }
  }

  async loadCustomModels(): Promise<void> {
    try {
      const config: any = await invoke("get_config");
      this.el.customModelList.innerHTML = "";
      for (const m of config.models || []) {
        const label = document.createElement("label");
        const cb = document.createElement("input");
        cb.type = "checkbox";
        cb.value = m;
        if (this.selectedCustomModels.has(m)) cb.checked = true;
        cb.addEventListener("change", () => {
          if (cb.checked) this.selectedCustomModels.add(m);
          else this.selectedCustomModels.delete(m);
          this.updateCustomRunButton();
        });
        label.appendChild(cb);
        label.appendChild(document.createTextNode(` ${m.split(/[/\\]/).pop() || m}`));
        this.el.customModelList.appendChild(label);
      }
    } catch (e) {
      this.el.customModelList.innerHTML = `<span class="test-hint">Ошибка загрузки: ${e}</span>`;
      void trackError("agentTest.loadCustomModels", e);
    }
  }

  private updateCustomRunButton(): void {
    const hasPrompt = this.el.customPromptInput.value.trim().length > 0;
    this.el.btnRunCustomTest.disabled = !(hasPrompt && this.customAgentId && this.selectedCustomModels.size > 0);
  }

  private stopCustomBatch(): void {
    if (!this.customBatch) return;
    this.customBatchCancelled = true;
    void invoke("cancel_test_run").catch(() => {});
    showToast("Останавливаю текущую модель...", "info");
  }

  /**
   * Батч-прогон своего промпта через выбранного агента/пайплайн на всех
   * отмеченных моделях — последовательно, как pipeline-батч (VRAM §6.5).
   */
  private async runCustomTest(): Promise<void> {
    const prompt = this.el.customPromptInput.value.trim();
    if (!prompt || !this.customAgentId || this.selectedCustomModels.size === 0) {
      showToast("Введите промпт, выберите агента и хотя бы одну модель", "error");
      return;
    }

    const agentId = this.customAgentId;
    const modelPaths = Array.from(this.selectedCustomModels);

    this.el.btnRunCustomTest.disabled = true;
    this.el.customProgress.style.display = "block";
    this.el.customResultsBox.style.display = "none";
    this.el.customStatusLabel.textContent = "Подготовка...";
    this.el.customProgressBar.style.width = "0%";
    this.el.btnStopCustomTest.style.display = "inline-block";

    this.customBatch = { total: modelPaths.length, done: 0 };
    this.customBatchCancelled = false;
    const entries: CustomPromptEntry[] = [];
    const batchStart = Date.now();

    try {
      for (let i = 0; i < modelPaths.length; i++) {
        if (this.customBatchCancelled) break;
        const modelPath = modelPaths[i];
        const shortName = modelPath.split(/[/\\]/).pop() || modelPath;
        this.customBatchLabel = `Модель ${i + 1}/${modelPaths.length} (${shortName})`;
        this.el.customStatusLabel.textContent = `${this.customBatchLabel} — запуск...`;
        try {
          const result = await invoke<CustomPromptResult>("run_custom_prompt_test", {
            prompt,
            agentId,
            modelPath,
          });
          entries.push({ model: modelPath, error: null, result });
        } catch (e) {
          entries.push({ model: modelPath, error: String(e), result: null });
          void trackError("agentTest.runCustomBatch", e);
        }
        this.customBatch.done = i + 1;
        this.setCustomBatchProgress(0);
      }

      const totalMs = Date.now() - batchStart;
      this.el.customProgressBar.style.width = "100%";
      this.el.customStatusLabel.textContent = this.customBatchCancelled
        ? `Остановлено пользователем: выполнено ${entries.length} из ${modelPaths.length} · общее время: ${(totalMs / 1000).toFixed(1)}s`
        : `Готово: ${entries.filter((e) => e.result && !e.result.error).length}/${entries.length} ответов получено · общее время: ${(totalMs / 1000).toFixed(1)}s`;
      this.displayCustomBatchResults(agentId, prompt, entries, totalMs);
      this.el.customResultsBox.style.display = "block";

      try {
        const reportPath = await invoke<string>("write_custom_prompt_report", {
          agentId,
          prompt,
          entries,
          totalDurationMs: totalMs,
        });
        showToast(`Сводный отчёт: ${reportPath}`, "success");
      } catch (e) {
        showToast(`Не удалось сохранить сводный отчёт: ${e}`, "error");
        void trackError("agentTest.saveCustomReport", e);
      }
    } finally {
      this.customBatch = null;
      this.customBatchLabel = "";
      this.el.btnRunCustomTest.disabled = false;
      this.el.btnStopCustomTest.style.display = "none";
    }
  }

  private setCustomBatchProgress(currentModelPct: number): void {
    if (!this.customBatch) return;
    const current = Math.max(0, Math.min(100, currentModelPct)) / 100;
    const overall = ((this.customBatch.done + current) / this.customBatch.total) * 100;
    this.el.customProgressBar.style.width = `${Math.max(0, Math.min(100, overall))}%`;
  }

  private displayCustomBatchResults(
    agentId: string,
    prompt: string,
    entries: CustomPromptEntry[],
    totalMs: number | null = null,
  ): void {
    this.el.customResultsContent.innerHTML = this.customBatchHtml(agentId, prompt, entries, totalMs);
  }

  /** HTML сводки кастом-батча: общий для живых результатов и просмотрщика отчётов. */
  private customBatchHtml(
    agentId: string,
    prompt: string,
    entries: CustomPromptEntry[],
    totalMs: number | null = null,
  ): string {
    const border = "1px solid var(--border, #444)";
    const rows = entries.map((e, i) => {
      const short = e.model.split(/[/\\]/).pop() || e.model;
      if (e.error) {
        return `<tr>
          <td style="padding:4px 8px; border-bottom:${border}">${i + 1}</td>
          <td style="padding:4px 8px; border-bottom:${border}">${this.escapeHtml(short)}</td>
          <td colspan="4" style="padding:4px 8px; border-bottom:${border}; color: var(--red, #f44336)">💥 Ошибка запуска: ${this.escapeHtml(e.error.substring(0, 300))}</td>
        </tr>`;
      }
      const r = e.result!;
      const failed = !!r.error;
      const color = failed ? "var(--red, #f44336)" : "var(--green, #4caf50)";
      return `<tr>
        <td style="padding:4px 8px; border-bottom:${border}">${i + 1}</td>
        <td style="padding:4px 8px; border-bottom:${border}">${this.escapeHtml(short)}</td>
        <td style="padding:4px 8px; border-bottom:${border}; color:${color}; font-weight:bold">${failed ? "💥 ОШИБКА" : "✅ ОТВЕТ"}</td>
        <td style="padding:4px 8px; border-bottom:${border}">${(r.duration_ms / 1000).toFixed(1)}s</td>
        <td style="padding:4px 8px; border-bottom:${border}">${this.fmtVram(r.vram_delta_mb, r.vram_peak_mb)}</td>
        <td style="padding:4px 8px; border-bottom:${border}">${this.fmtMb(r.ram_peak_mb)}</td>
      </tr>`;
    }).join("");

    const totalPart = typeof totalMs === "number"
      ? ` · <strong>общее время батча:</strong> ${(totalMs / 1000).toFixed(1)}s`
      : "";
    const table = `
      <div style="margin-bottom:12px;">
        <strong>Агент/пайплайн:</strong> ${this.escapeHtml(agentId)} · моделей: ${entries.length}${totalPart}<br/>
        <strong>Промпт:</strong> ${this.escapeHtml(prompt.substring(0, 300))}
      </div>
      <table style="border-collapse:collapse; width:100%; margin-bottom:16px;">
        <thead><tr>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">#</th>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">Модель</th>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">Итог</th>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">Время</th>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">VRAM</th>
          <th style="text-align:left; padding:4px 8px; border-bottom:${border}">ОЗУ</th>
        </tr></thead>
        <tbody>${rows}</tbody>
      </table>`;

    const details = entries.map((e, i) => {
      const short = e.model.split(/[/\\]/).pop() || e.model;
      let html = `<div style="margin-bottom:16px;"><strong>${i + 1}. ${this.escapeHtml(short)}</strong>`;
      if (e.error) {
        html += `<pre style="white-space:pre-wrap; color: var(--red, #f44336)">${this.escapeHtml(e.error)}</pre>`;
      }
      if (e.result) {
        if (e.result.error) {
          html += `<pre style="white-space:pre-wrap; color: var(--red, #f44336)">${this.escapeHtml(e.result.error)}</pre>`;
        } else {
          html += this.finalAnswerHtml(this.finalAuthorOf(e.result.messages), e.result.response);
        }
        html += this.chainHtml(e.result.messages);
      }
      return html + "</div>";
    }).join('<hr style="border:none; border-top:1px solid var(--border, #444); margin:12px 0"/>');

    return table + details;
  }

  // ─── Плашки итога и цепочки переходов (общие для живых результатов и отчётов) ───

  /** Автор итогового ответа: последнее message-сообщение от агента (зеркало backend-логики). */
  private finalAuthorOf(messages: ChainMessage[]): string {
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i];
      if (m.msg_type === "message" && m.author && m.author !== "user") return m.author;
    }
    for (let i = messages.length - 1; i >= 0; i--) {
      if (messages[i].author) return messages[i].author;
    }
    return "неизвестно";
  }

  /** Текст итогового ответа из цепочки (для pipeline-отчётов, где response нет). */
  private finalTextOf(messages: ChainMessage[]): string {
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i];
      if (m.msg_type === "message" && m.author && m.author !== "user") return m.content_preview;
    }
    return "";
  }

  /** Плашка «Итоговый ответ»: выделена рамкой, подписана именем агента. */
  private finalAnswerHtml(author: string, text: string): string {
    if (!text) return "";
    return `
      <div style="margin:8px 0; padding:8px 10px; border:1px solid var(--border, #444); border-left:4px solid var(--accent, #4a9eff); border-radius:4px; background:rgba(74,158,255,0.06);">
        <div style="margin-bottom:4px;">🏁 <strong>Итоговый ответ</strong> <span style="opacity:0.75;">— агент: ${this.escapeHtml(author)}</span></div>
        <pre style="white-space:pre-wrap; margin:4px 0 0 0;">${this.escapeHtml(text)}</pre>
      </div>`;
  }

  /** Блок «Цепочка переходов»: нумерованные шаги — кто отвечал, что выдал. */
  private chainHtml(messages: ChainMessage[]): string {
    const steps = messages.filter((m) => m.author);
    if (steps.length === 0) return "";
    const typeIcon = (t: string) => (t === "signal" ? "📡" : t === "thought" ? "💭" : "💬");
    const items = steps.map((m) =>
      `<li style="margin-bottom:2px;">${typeIcon(m.msg_type)} <strong>${this.escapeHtml(m.author)}</strong> <span style="opacity:0.6;">[${this.escapeHtml(m.msg_type)}]</span> — ${this.escapeHtml(m.content_preview)}</li>`,
    ).join("");
    return `
      <div style="margin:8px 0;">
        <div style="margin-bottom:4px;">🔗 <strong>Цепочка переходов</strong> <span style="opacity:0.6;">(${steps.length} шагов)</span></div>
        <ol style="margin:4px 0 0 20px; padding:0;">${items}</ol>
      </div>`;
  }

  // ─── Просмотрщик сохранённых отчётов (загрузка из JSON на диске) ───

  /**
   * Заполнить список сохранённых отчётов (list_test_reports).
   *
   * Список — метки из JSON-файлов на диске; выбранную позицию стараемся
   * сохранить при обновлении (кейс: юзер смотрит отчёт и жмёт «Обновить»).
   */
  private async loadReportList(): Promise<void> {
    const select = this.el.reportSelect;
    const previous = select.value;
    try {
      const reports = await invoke<TestReportMeta[]>("list_test_reports");
      if (reports.length === 0) {
        select.innerHTML = `<option value="">— отчётов пока нет —</option>`;
        this.el.reportView.style.display = "none";
        return;
      }
      const options = reports.map((r) => {
        const kind = r.kind === "custom" ? "промпт" : "фикстура";
        const date = new Date(r.created_ms).toLocaleString();
        const label = `${kind}: ${r.label} · ${r.file_name} · ${date}`;
        const selected = r.path === previous ? " selected" : "";
        return `<option value="${this.escapeHtml(r.path)}"${selected}>${this.escapeHtml(label)}</option>`;
      }).join("");
      select.innerHTML = options;
      if (previous && select.value !== previous) {
        // Прежний выбор пропал (файл удалён) — спрятать его вид.
        this.el.reportView.style.display = "none";
      }
    } catch (e) {
      showToast(`Не удалось получить список отчётов: ${String(e)}`);
    }
  }

  /**
   * Показать выбранный отчёт: читаем JSON с диска (read_test_report) и рендерим
   * теми же билдерами, что и живые результаты — отображение из файла идентично
   * отображению на странице сразу после прогона.
   */
  private async viewSelectedReport(): Promise<void> {
    const path = this.el.reportSelect.value;
    const view = this.el.reportView;
    if (!path) {
      view.style.display = "none";
      view.innerHTML = "";
      return;
    }
    try {
      const json = await invoke<Record<string, unknown>>("read_test_report", { path });
      const type = typeof json.type === "string" ? json.type : "";
      const entries = Array.isArray(json.entries) ? json.entries : [];
      const totalMs = typeof json.total_duration_ms === "number" ? json.total_duration_ms : null;
      const sourceNote = `<div style="opacity:0.6; margin-bottom:8px;">Источник: ${this.escapeHtml(path)}</div>`;
      let html: string;
      if (type === "pipeline_batch_report") {
        html = this.pipelineBatchHtml(
          typeof json.test_id === "string" ? json.test_id : "—",
          entries as PipelineBatchEntry[],
          totalMs,
        );
      } else if (type === "custom_prompt_report") {
        html = this.customBatchHtml(
          typeof json.agent_id === "string" ? json.agent_id : "—",
          typeof json.prompt === "string" ? json.prompt : "",
          entries as CustomPromptEntry[],
          totalMs,
        );
      } else {
        html = `<div style="color: var(--red, #f44336)">Неизвестный формат отчёта (type: ${this.escapeHtml(type || "—")})</div>`;
      }
      view.innerHTML = sourceNote + html;
      view.style.display = "";
    } catch (e) {
      view.innerHTML = `<div style="color: var(--red, #f44336)">Ошибка чтения отчёта: ${this.escapeHtml(String(e))}</div>`;
      view.style.display = "";
    }
  }

  private escapeHtml(s: string): string {
    const div = document.createElement("div");
    div.textContent = s;
    return div.innerHTML;
  }

  private fmtMb(mb: number | null | undefined): string {
    return typeof mb === "number" && mb > 0 ? `${mb.toFixed(0)} МБ` : "—";
  }

  /** VRAM в сводке: Δ (сравнимая между моделями) + абсолютный пик в скобках. */
  private fmtVram(delta: number | null | undefined, peak: number | null | undefined): string {
    if (typeof delta === "number" && delta > 0 && typeof peak === "number" && peak > 0) {
      return `Δ${delta.toFixed(0)} (из ${peak.toFixed(0)}) МБ`;
    }
    if (typeof peak === "number" && peak > 0) return `${peak.toFixed(0)} МБ`;
    return "—";
  }
}
