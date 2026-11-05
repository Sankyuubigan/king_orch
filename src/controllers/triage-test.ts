import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { showToast } from "../ui";
import { trackError } from "../telemetry";
import { triageBatchHtml, type TriageModelReport } from "../services/triage-score";

export interface TriageTestElements {
  triageTestPanel: HTMLDivElement;
  btnRunTriageTest: HTMLButtonElement;
  btnStopTriageTest: HTMLButtonElement;
  triageProgress: HTMLDivElement;
  triageStatusLabel: HTMLDivElement;
  triageProgressBar: HTMLDivElement;
  triageResultsBox: HTMLDivElement;
  triageResultsContent: HTMLDivElement;
}

/**
 * Контроллер триажа моделей (14 пробников классов отказов).
 *
 * Живёт на вкладке «Тест LLM» (перенос 05.11.2026: триаж — это тест LLM,
 * а не тест агентов). Панель не владеет списком моделей: выбор читается из
 * общего пула вкладки (`modelListContainer` — чекбоксы с value = путь модели)
 * в момент запуска. Один источник правды о выборе, без копий стейта.
 */
export class TriageTestController {
  private el: TriageTestElements;
  /** Контейнер чекбоксов моделей вкладки (владелец — CodingTestController). */
  modelListContainer: HTMLDivElement;
  private triageBatch: { total: number; done: number } | null = null;
  private triageBatchCancelled = false;
  private destroyed = false;
  private unsubscribers: (() => void)[] = [];

  /**
   * Отписка от Tauri-события `triage_status` (пробник N/M текущей модели).
   * `listen` возвращает промис: отписка может прийти после destroy() —
   * такой обработчик отписывается сразу (утечка закрытой вкладки).
   */
  private trackListen(promise: Promise<() => void>): void {
    void promise.then((unlisten) => {
      if (this.destroyed) unlisten();
      else this.unsubscribers.push(unlisten);
    });
  }

  constructor(el: TriageTestElements, modelListContainer: HTMLDivElement) {
    this.el = el;
    this.modelListContainer = modelListContainer;
    this.bindEvents();
    // Прогресс внутри одной модели триажа (backend эмитит triage_status на каждый пробник).
    this.trackListen(
      listen<{ model: string; probe: number; total: number; title: string }>(
        "triage_status",
        (ev) => this.onTriageStatus(ev.payload),
      ),
    );
  }

  /** Панель не нуждается в отложенной загрузке: слушатели ставятся в конструкторе. */
  init(): void {}

  destroy(): void {
    this.destroyed = true;
    for (const unsubscribe of this.unsubscribers.splice(0)) unsubscribe();
  }

  private bindEvents(): void {
    this.el.btnRunTriageTest.addEventListener("click", () => this.runTriageTest());
    this.el.btnStopTriageTest.addEventListener("click", () => this.stopTriageBatch());
    // Выбор моделей общий с бенчмарком: слушаем контейнер делегированием,
    // чтобы не зависеть от перерисовок списка владельцем.
    this.modelListContainer.addEventListener("change", () => this.updateTriageRunButton());
  }

  /** Выбор моделей — из DOM в момент вызова (SSOT выбора — чекбоксы пула вкладки). */
  private selectedModelPaths(): string[] {
    const boxes = this.modelListContainer.querySelectorAll<HTMLInputElement>(
      "input[type=checkbox]:checked",
    );
    return Array.from(boxes).map((cb) => cb.value);
  }

  private updateTriageRunButton(): void {
    this.el.btnRunTriageTest.disabled = this.selectedModelPaths().length === 0;
  }

  private stopTriageBatch(): void {
    if (!this.triageBatch) return;
    this.triageBatchCancelled = true;
    void invoke("cancel_test_run").catch(() => {});
    showToast("Останавливаю текущую модель...", "info");
  }

  /** Прогресс батча триажа: доля завершённых моделей + доля текущей модели. */
  private setTriageBatchProgress(currentModelPct: number): void {
    if (!this.triageBatch) return;
    const current = Math.max(0, Math.min(100, currentModelPct)) / 100;
    const overall = ((this.triageBatch.done + current) / this.triageBatch.total) * 100;
    this.el.triageProgressBar.style.width = `${Math.max(0, Math.min(100, overall))}%`;
  }

  /** Событие triage_status: пробник N/M текущей модели (эмитит backend). */
  private onTriageStatus(p: { model: string; probe: number; total: number; title: string }): void {
    if (!this.triageBatch) return;
    this.el.triageStatusLabel.textContent =
      `Модель ${this.triageBatch.done + 1}/${this.triageBatch.total} (${p.model}) — пробник ${p.probe}/${p.total}: ${p.title}`;
    this.setTriageBatchProgress(((p.probe - 1) / p.total) * 100);
  }

  /**
   * Батч-триаж всех отмеченных моделей — последовательно, одна модель за раз
   * (VRAM §6.5). Ошибка запуска модели не роняет батч: попадает в отчёт 💥-записью.
   */
  private async runTriageTest(): Promise<void> {
    const modelPaths = this.selectedModelPaths();
    if (modelPaths.length === 0) {
      showToast("Выберите хотя бы одну модель", "error");
      return;
    }

    this.el.btnRunTriageTest.disabled = true;
    this.el.triageProgress.style.display = "block";
    this.el.triageResultsBox.style.display = "none";
    this.el.triageStatusLabel.textContent = "Подготовка...";
    this.el.triageProgressBar.style.width = "0%";
    this.el.btnStopTriageTest.style.display = "inline-block";

    this.triageBatch = { total: modelPaths.length, done: 0 };
    this.triageBatchCancelled = false;
    const reports: TriageModelReport[] = [];
    const batchStart = Date.now();

    try {
      for (let i = 0; i < modelPaths.length; i++) {
        if (this.triageBatchCancelled) break;
        const modelPath = modelPaths[i];
        const shortName = modelPath.split(/[/\\]/).pop() || modelPath;
        this.el.triageStatusLabel.textContent = `Модель ${i + 1}/${modelPaths.length} (${shortName}) — запуск...`;
        try {
          const report = await invoke<TriageModelReport>("run_triage_test_cmd", { modelPath });
          reports.push(report);
        } catch (e) {
          reports.push({
            model: modelPath,
            verdict_label: "",
            error: String(e),
            probes: [],
            duration_ms: 0,
          });
          void trackError("triageTest.runTriageBatch", e);
        }
        this.triageBatch.done = i + 1;
        this.setTriageBatchProgress(0);
      }

      const totalMs = Date.now() - batchStart;
      this.el.triageProgressBar.style.width = "100%";
      const doneCount = reports.filter((r) => !r.error).length;
      this.el.triageStatusLabel.textContent = this.triageBatchCancelled
        ? `Остановлено пользователем: выполнено ${reports.length} из ${modelPaths.length} · общее время: ${(totalMs / 1000).toFixed(1)}s`
        : `Готово: ${doneCount}/${reports.length} моделей отбрано · общее время: ${(totalMs / 1000).toFixed(1)}s`;
      this.el.triageResultsContent.innerHTML = triageBatchHtml(reports, totalMs);
      this.el.triageResultsBox.style.display = "block";

      try {
        const reportPath = await invoke<string>("write_triage_report", {
          reports,
          totalDurationMs: totalMs,
        });
        showToast(`Сводный отчёт: ${reportPath}`, "success");
      } catch (e) {
        showToast(`Не удалось сохранить сводный отчёт: ${e}`, "error");
        void trackError("triageTest.saveTriageReport", e);
      }
    } finally {
      this.triageBatch = null;
      this.updateTriageRunButton();
      this.el.btnStopTriageTest.style.display = "none";
    }
  }
}
