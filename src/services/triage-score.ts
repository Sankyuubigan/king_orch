/**
 * Чистые хелперы отображения рейтинга триажа («X из Y» + топ-лист батча).
 * Балл считается на бэкенде (SSOT: domain/triage.rs — SCORE_WEIGHTS, compute_score_decipoints)
 * и приходит в отчёте полями score/score_max; здесь только форматирование и ранжирование.
 * Обоснование весов: tasks/261104_1822-cdabe7fa/05.11.2026 Рейтинг триажа (баллы и топ-лист).md
 */

/** Цветовой бэнд процента набранного балла: ≥85 — good, ≥60 — mid, ниже — bad. */
export type TriageScoreBand = "good" | "mid" | "bad";

export function triageScoreBand(score: number, max: number): TriageScoreBand {
  const ratio = max > 0 ? (score / max) * 100 : 0;
  if (ratio >= 85) return "good";
  if (ratio >= 60) return "mid";
  return "bad";
}

/** «94.2 из 100»: у Y дробная часть не показывается, если она нулевая. */
export function formatTriageScore(score: number, max: number): string {
  const maxPart = Number.isInteger(max) ? String(max) : max.toFixed(1);
  return `${score.toFixed(1)} из ${maxPart}`;
}

/** Минимальная структура отчёта, нужная ранжированию (структурно = TriageModelReport). */
export interface TriageRankInput {
  model: string;
  verdict_label: string;
  score?: number;
  score_max?: number;
  probes: Array<{ status: string; title: string; weight?: number }>;
}

export interface TriageRankedRow {
  /** Короткое имя модели (basename пути). */
  model: string;
  /** Competition ranking: равный балл = одинаковое место, далее с пропуском (1,1,3). */
  place: number;
  score: number;
  scoreMax: number;
  verdictLabel: string;
  /** Критичные fail'ы (до 3 title'ов, по весу убыв; вес неизвестен — порядок отчёта). */
  criticalFails: string[];
}

/** Топ-лист показываем только когда моделей ≥2 и хотя бы у двух есть балл. */
export function hasTriageRanking(reports: TriageRankInput[]): boolean {
  if (reports.length < 2) return false;
  return reports.filter((r) => typeof r.score === "number").length >= 2;
}

/** Ранжирование scored-отчётов: балл убыв, при равенстве имя asc. */
export function rankTriageModels(reports: TriageRankInput[]): TriageRankedRow[] {
  const rows = reports
    .filter((r) => typeof r.score === "number" && typeof r.score_max === "number")
    .map((r) => ({
      model: r.model.split(/[/\\]/).pop() || r.model,
      score: r.score as number,
      scoreMax: r.score_max as number,
      verdictLabel: r.verdict_label,
      criticalFails: r.probes
        .filter((p) => p.status === "fail")
        .sort((a, b) => (b.weight ?? 0) - (a.weight ?? 0))
        .slice(0, 3)
        .map((p) => p.title),
    }));
  rows.sort((a, b) => b.score - a.score || a.model.localeCompare(b.model));
  let place = 0;
  return rows.map((row, i) => {
    if (i === 0 || row.score !== rows[i - 1].score) place = i + 1;
    return { ...row, place };
  });
}
