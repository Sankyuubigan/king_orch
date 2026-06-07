import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { showToast } from "../ui";
import type { TestCaseDef, SingleTestResult } from "../types";

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
}

export class AgentTestController {
  private el: AgentTestElements;
  private testCases: TestCaseDef[] = [];
  private selectedAgents: Set<string> = new Set();
  private selectedModels: Set<string> = new Set();
  private results: SingleTestResult[] | null = null;
  private agentsLoaded = false;
  private modelsLoaded = false;

  constructor(el: AgentTestElements) {
    this.el = el;
    this.bindEvents();
  }

  init(): void {
    if (!this.agentsLoaded) this.loadAgents();
    if (!this.modelsLoaded) this.loadModels();
  }

  async loadAgents(): Promise<void> {
    try {
      const agents: any[] = await invoke("get_agents");
      this.el.testAgentList.innerHTML = "";
      for (const a of agents) {
        if (a.is_hidden) continue;
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
    }
  }

  private bindEvents(): void {
    this.el.btnSelectTestFile.addEventListener("click", () => this.selectFile());
    this.el.btnRunTest.addEventListener("click", () => this.runTest());
    this.el.btnSaveTestResults.addEventListener("click", () => this.saveResults());
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
    }
  }

  private async parseTestFile(path: string): Promise<void> {
    try {
      this.testCases = await invoke<TestCaseDef[]>("read_test_file", { path });
      showToast(`Загружено тест-кейсов: ${this.testCases.length}`, "success");
    } catch (e) {
      showToast(`Ошибка чтения файла: ${e}`, "error");
      this.testCases = [];
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
    let done = 0;

    try {
      this.el.testStatusLabel.textContent = `Запуск тестов (0/${total})...`;
      this.el.testProgressBar.style.width = "0%";

      this.results = await invoke<SingleTestResult[]>("run_iterative_test", {
        testCases: this.testCases,
        agentIds,
        modelPaths,
      });

      done = total;
      this.el.testStatusLabel.textContent = `Готово! Обработано ${done}/${total}`;
      this.el.testProgressBar.style.width = "100%";
      this.displayResults(this.results);
      this.el.testResultsBox.style.display = "block";
      showToast("Тестирование завершено!", "success");
    } catch (e) {
      this.el.testStatusLabel.textContent = `Ошибка: ${e}`;
      showToast(`Ошибка тестирования: ${e}`, "error");
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
    }
  }

  private escapeHtml(s: string): string {
    const div = document.createElement("div");
    div.textContent = s;
    return div.innerHTML;
  }
}
