import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { showToast } from "../ui";
import { trackError } from "../telemetry";

export interface CodingTestElements {
  codingModelList: HTMLDivElement;
  codingSuiteList: HTMLDivElement;
  codingQuickCount: HTMLInputElement;
  codingVrBudget: HTMLInputElement;
  btnRunCodingTest: HTMLButtonElement;
  btnStopCodingTest: HTMLButtonElement;
  codingProgress: HTMLDivElement;
  codingStatusLabel: HTMLDivElement;
  codingProgressBar: HTMLDivElement;
  codingLog: HTMLDivElement;
  codingResultsBox: HTMLDivElement;
  codingResultsContent: HTMLDivElement;
  codingReportLink: HTMLAnchorElement;
}

interface ModelSelection {
  path: string;
  name: string;
}

export class CodingTestController {
  private el: CodingTestElements;
  private selectedModels: Map<string, ModelSelection> = new Map();
  private selectedSuites: Set<string> = new Set();
  private modelsLoaded = false;
  private suitesLoaded = false;
  private listenersReady = false;
  private running = false;

  constructor(el: CodingTestElements) {
    this.el = el;
    this.bindEvents();
  }

  init(): void {
    if (!this.modelsLoaded) void this.loadModels();
    if (!this.suitesLoaded) void this.loadSuites();
    if (!this.listenersReady) {
      this.setupListeners();
      this.listenersReady = true;
    }
  }

  private bindEvents(): void {
    this.el.btnRunCodingTest.addEventListener("click", () => void this.run());
    this.el.btnStopCodingTest.addEventListener("click", () => void this.stop());
  }

  private async loadModels(): Promise<void> {
    try {
      const config: any = await invoke("get_config");
      this.el.codingModelList.innerHTML = "";
      const models: string[] = config.models || [];
      if (models.length === 0) {
        this.el.codingModelList.innerHTML = `<span class="test-hint">Модели не найдены</span>`;
        return;
      }
      for (const m of models) {
        const name = m.split(/[/\\]/).pop() || m;
        const sel: ModelSelection = { path: m, name };
        const label = document.createElement("label");
        const cb = document.createElement("input");
        cb.type = "checkbox";
        cb.value = m;
        if (this.selectedModels.has(m)) cb.checked = true;
        cb.addEventListener("change", () => {
          if (cb.checked) this.selectedModels.set(m, sel);
          else this.selectedModels.delete(m);
          this.updateRunButton();
        });
        label.appendChild(cb);
        label.appendChild(document.createTextNode(` ${name}`));
        this.el.codingModelList.appendChild(label);
      }
      this.modelsLoaded = true;
    } catch (e) {
      this.el.codingModelList.innerHTML = `<span class="test-hint">Ошибка загрузки: ${e}</span>`;
      void trackError("codingTest.loadModels", e);
    }
  }

  private async loadSuites(): Promise<void> {
    try {
      const suites: any[] = await invoke("get_coding_bench_info");
      this.el.codingSuiteList.innerHTML = "";
      if (suites.length === 0) {
        this.el.codingSuiteList.innerHTML = `<span class="test-hint">Наборы не найдены</span>`;
        return;
      }
      for (const s of suites) {
        const label = document.createElement("label");
        const cb = document.createElement("input");
        cb.type = "checkbox";
        cb.value = s.id;
        if (this.selectedSuites.has(s.id)) cb.checked = true;
        const catInfo = Object.entries(s.categories || {})
          .map(([k, v]) => `${k}:${v}`)
          .join(", ");
        cb.addEventListener("change", () => {
          if (cb.checked) this.selectedSuites.add(s.id);
          else this.selectedSuites.delete(s.id);
          this.updateRunButton();
        });
        label.appendChild(cb);
        label.appendChild(
          document.createTextNode(` ${s.id} (${s.language}, ${s.runnable}/${s.total} запускаемых) — ${catInfo}`),
        );
        this.el.codingSuiteList.appendChild(label);
      }
      this.suitesLoaded = true;
    } catch (e) {
      this.el.codingSuiteList.innerHTML = `<span class="test-hint">Ошибка загрузки: ${e}</span>`;
      void trackError("codingTest.loadSuites", e);
    }
  }

  private updateRunButton(): void {
    const canRun =
      !this.running && this.selectedModels.size > 0 && this.selectedSuites.size > 0;
    this.el.btnRunCodingTest.disabled = !canRun;
  }

  private setupListeners(): void {
    void listen<string>("coding_status", (e) => {
      this.el.codingStatusLabel.textContent = e.payload;
    });
    void listen<number>("coding_progress", (e) => {
      const pct = Math.max(0, Math.min(100, e.payload));
      this.el.codingProgressBar.style.width = `${pct}%`;
    });
    void listen<any>("coding_done", (e) => {
      this.onDone(e.payload);
    });
  }

  private async run(): Promise<void> {
    const models = Array.from(this.selectedModels.values());
    const suites = Array.from(this.selectedSuites);
    if (models.length === 0 || suites.length === 0) {
      showToast("Выберите модели и наборы задач", "error");
      return;
    }

    const quick = parseInt(this.el.codingQuickCount.value || "0", 10);
    const vrBudget = parseInt(this.el.codingVrBudget.value || "14336", 10);

    this.running = true;
    this.updateRunButton();
    this.el.btnStopCodingTest.disabled = false;
    this.el.codingProgress.style.display = "block";
    this.el.codingResultsBox.style.display = "none";
    this.el.codingReportLink.style.display = "none";
    this.el.codingLog.innerHTML = "";
    this.el.codingStatusLabel.textContent = "Запуск бенчмарка...";
    this.el.codingProgressBar.style.width = "0%";

    try {
      const config: any = await invoke("get_config");
      await invoke("run_coding_bench", {
        config,
        models,
        suites,
        quickPerSuite: quick > 0 ? quick : null,
        vrBudgetMb: vrBudget,
      });
    } catch (e) {
      this.el.codingStatusLabel.textContent = `Ошибка: ${e}`;
      showToast(`Ошибка запуска бенчмарка: ${e}`, "error");
      void trackError("codingTest.run", e);
      this.finish();
    }
  }

  private async stop(): Promise<void> {
    try {
      await invoke("stop_processing");
      this.el.codingStatusLabel.textContent = "Остановка...";
    } catch (e) {
      showToast(`Ошибка остановки: ${e}`, "error");
    }
  }

  private async onDone(payload: any): Promise<void> {
    const reportFile: string | undefined = payload?.report_file;
    const artifactsDir: string | undefined = payload?.artifacts_dir;
    this.appendLog(`Бенчмарк завершён. Отчёт: ${reportFile ?? "—"}`);
    if (artifactsDir) this.appendLog(`Артефакты: ${artifactsDir}`);
    if (reportFile) {
      try {
        const json: string = await invoke("read_text_file", { path: reportFile });
        this.renderReport(JSON.parse(json));
      } catch (e) {
        showToast(`Не удалось прочитать отчёт: ${e}`, "error");
        void trackError("codingTest.readReport", e);
      }
    }
    this.finish();
  }

  private finish(): void {
    this.running = false;
    this.el.btnStopCodingTest.disabled = true;
    this.updateRunButton();
  }

  private appendLog(msg: string): void {
    const line = document.createElement("div");
    line.textContent = msg;
    this.el.codingLog.appendChild(line);
  }

  private renderReport(report: any): void {
    const models: any[] = report.models || [];
    const rows = models
      .map((m) => {
        const kv = m.kv_probe
          ? `KV(f16) ctx: ${m.kv_probe.max_ctx}, VRAM: ${m.kv_probe.vram_mb?.toFixed(0)} МБ`
          : "KV: —";
        const tasks = (m.tasks || [])
          .map(
            (t: any) =>
              `<li class="${t.passed ? "coding-pass" : "coding-fail"}">${this.escape(t.task_id)} — ${
                t.passed ? "✓" : "✗"
              } (${t.gen_tok_per_sec?.toFixed(1)} tok/s, TTFT ${t.ttft_sec?.toFixed(2)}s${
                t.timed_out ? ", timeout" : ""
              })</li>`,
          )
          .join("");
        return `<div class="coding-model-result">
          <div class="coding-model-name">${this.escape(m.model_name)}</div>
          <div class="coding-metrics">pass@1: <strong>${((m.pass_rate || 0) * 100).toFixed(
            1,
          )}%</strong> (${m.passed}/${m.total}) · gen: ${m.avg_gen_tok_per_sec?.toFixed(
            1,
          )} tok/s · prompt: ${m.avg_prompt_tok_per_sec?.toFixed(1)} tok/s · TTFT: ${m.avg_ttft_sec?.toFixed(
            2,
          )}s · ${kv}</div>
          <ul class="coding-task-list">${tasks}</ul>
        </div>`;
      })
      .join("");

    const budget = report.budget_vram_mb ? `Бюджет VRAM: ${report.budget_vram_mb} МБ` : "";
    this.el.codingResultsContent.innerHTML = `<div class="coding-report-meta">${budget} · ${this.escape(
      report.timestamp || "",
    )}</div>${rows}`;
    this.el.codingResultsBox.style.display = "block";
    this.el.codingReportLink.style.display = "inline-block";
  }

  private escape(s: string): string {
    const div = document.createElement("div");
    div.textContent = s ?? "";
    return div.innerHTML;
  }
}
