import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { showToast } from "../ui";
import { store } from "../store";
import { trackError } from "../telemetry";
import type { TestCaseDef, SingleTestResult, PipelineTestInfo, PipelineTestResult } from "../types";

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
  private selectedPipelineModel: string | null = null;
  private pipelineTestsLoaded = false;
  private pipelineModelsLoaded = false;
  private pipelineListenersSetup = false;

  constructor(el: AgentTestElements) {
    this.el = el;
    this.bindEvents();
  }

  init(): void {
    if (!this.agentsLoaded) this.loadAgents();
    if (!this.modelsLoaded) this.loadModels();
    if (!this.pipelineTestsLoaded) this.loadPipelineTests();
    if (!this.pipelineModelsLoaded) this.loadPipelineModels();
    this.setupPipelineListeners();
  }

  // ─── Mode switching ───

  private bindEvents(): void {
    this.el.btnSelectTestFile.addEventListener("click", () => this.selectFile());
    this.el.btnRunTest.addEventListener("click", () => this.runTest());
    this.el.btnSaveTestResults.addEventListener("click", () => this.saveResults());
    this.el.testModeSelect.addEventListener("change", () => this.switchMode());
    this.el.btnRunPipelineTest.addEventListener("click", () => this.runPipelineTest());
  }

  private switchMode(): void {
    const mode = this.el.testModeSelect.value;
    const isPipeline = mode === "pipeline";
    this.el.yamlTestPanel.style.display = isPipeline ? "none" : "block";
    this.el.pipelineTestPanel.style.display = isPipeline ? "block" : "none";
  }

  // ─── YAML test mode (existing) ───

  async loadAgents(): Promise<void> {
    try {
      const agents: any[] = await invoke("get_agents");
      this.el.testAgentList.innerHTML = "";
      for (const a of agents) {
        if (a.is_hidden || (a.folder !== null && !store.showFolderAgents)) continue;
        const label = document.createElement("label");
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
        label.appendChild(document.createTextNode(` ${a.name} (${a.id})`));
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
        cb.type = "radio";
        cb.name = "pipeline-model";
        cb.value = m;
        cb.addEventListener("change", () => {
          this.selectedPipelineModel = m;
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
    this.el.btnRunPipelineTest.disabled = !(this.selectedPipelineTest && this.selectedPipelineModel);
  }

  private setupPipelineListeners(): void {
    if (this.pipelineListenersSetup) return;
    this.pipelineListenersSetup = true;

    listen<string>("pipeline_status", (e) => {
      this.el.pipelineStatusLabel.textContent = e.payload;
    });
    listen<number>("pipeline_progress", (e) => {
      const pct = Math.max(0, Math.min(100, e.payload));
      this.el.pipelineProgressBar.style.width = `${pct}%`;
    });
    listen<boolean>("pipeline_done", (e) => {
      const passed = e.payload;
      this.el.pipelineStatusLabel.textContent = passed ? "✅ Тест пройден" : "❌ Тест НЕ пройден";
      this.el.pipelineProgressBar.style.width = "100%";
      showToast(passed ? "Pipeline test пройден!" : "Pipeline test НЕ пройден", passed ? "success" : "error");
    });
  }

  private async runPipelineTest(): Promise<void> {
    if (!this.selectedPipelineTest || !this.selectedPipelineModel) {
      showToast("Выберите тест и модель", "error");
      return;
    }

    this.el.btnRunPipelineTest.disabled = true;
    this.el.pipelineProgress.style.display = "block";
    this.el.pipelineResultsBox.style.display = "none";
    this.el.pipelineStatusLabel.textContent = "Подготовка...";
    this.el.pipelineProgressBar.style.width = "0%";

    try {
      const result = await invoke<PipelineTestResult>("run_pipeline_test_cmd", {
        testId: this.selectedPipelineTest,
        modelPath: this.selectedPipelineModel,
      });

      this.displayPipelineResults(result);
      this.el.pipelineResultsBox.style.display = "block";
    } catch (e) {
      this.el.pipelineStatusLabel.textContent = `Ошибка: ${e}`;
      showToast(`Ошибка pipeline test: ${e}`, "error");
      void trackError("agentTest.runPipelineTest", e);
    } finally {
      this.el.btnRunPipelineTest.disabled = false;
    }
  }

  private displayPipelineResults(result: PipelineTestResult): void {
    const levelClass = (passed: boolean) => passed ? "color: var(--green, #4caf50)" : "color: var(--red, #f44336)";
    const levelIcon = (passed: boolean) => passed ? "✅" : "❌";

    const html = `
      <div style="margin-bottom:12px;">
        <strong>Тест:</strong> ${this.escapeHtml(result.test_id)}<br/>
        <strong>Модель:</strong> ${this.escapeHtml(result.model_name)}<br/>
        <strong>Пайплайн:</strong> ${this.escapeHtml(result.workflow_name)}<br/>
        <strong>Длительность:</strong> ${(result.duration_ms / 1000).toFixed(1)}s<br/>
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
    this.el.pipelineResultsContent.innerHTML = html;
  }

  private escapeHtml(s: string): string {
    const div = document.createElement("div");
    div.textContent = s;
    return div.innerHTML;
  }
}
