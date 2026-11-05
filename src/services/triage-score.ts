/**
 * Сервис триажа: чистые хелперы отображения («X из Y» + топ-лист батча) и SSOT
 * рендеринга сводки отчёта. Балл считается на бэкенде (SSOT: domain/triage.rs —
 * SCORE_WEIGHTS, compute_score_decipoints) и приходит в отчёте полями
 * score/score_max; здесь только форматирование, ранжирование и HTML.
 *
 * Рендер живёт здесь, а не в контроллере, потому что сводку показывают ДВА
 * потребителя: живые результаты (TriageTestController, вкладка «Тест LLM») и
 * просмотрщик сохранённых отчётов (AgentTestController, вкладка «Тест агентов»).
 * Контроллеры не импортируют друг друга — общим местом служит этот сервис.
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

/** Вердикт одного пробника (совпадает с Rust TriageVerdict). */
export interface TriageVerdict {
  probe_id: string;
  title: string;
  status: "pass" | "warn" | "fail" | "skip";
  detail: string;
  metrics?: unknown;
  /** Вес в итоговом балле (SSOT — Rust probe_weight); старые отчёты без поля. */
  weight?: number;
}

/** Отчёт триажа одной модели (совпадает с Rust TriageModelReport). */
export interface TriageModelReport {
  model: string;
  verdict_label: string;
  error: string | null;
  probes: TriageVerdict[];
  duration_ms: number;
  ram_peak_mb?: number | null;
  vram_peak_mb?: number | null;
  /** Балл «X из Y» (считает бэкенд); старые отчёты и ошибки запуска — без поля. */
  score?: number;
  score_max?: number;
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

/** Минимальная структура отчёта, нужная ранжированию (структурно = TriageModelReport). */
export interface TriageRankInput {
  model: string;
  verdict_label: string;
  score?: number;
  score_max?: number;
  probes: Array<{ status: string; title: string; weight?: number }>;
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

// ─── Рендеринг сводки триажа (общий для живых результатов и просмотрщика отчётов) ───

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}

function fmtMb(mb: number | null | undefined): string {
  return typeof mb === "number" && mb > 0 ? `${mb.toFixed(0)} МБ` : "—";
}

/** VRAM в сводке: Δ (сравнимая между моделями) + абсолютный пик в скобках. */
function fmtVram(delta: number | null | undefined, peak: number | null | undefined): string {
  if (typeof delta === "number" && delta > 0 && typeof peak === "number" && peak > 0) {
    return `Δ${delta.toFixed(0)} (из ${peak.toFixed(0)}) МБ`;
  }
  if (typeof peak === "number" && peak > 0) return `${peak.toFixed(0)} МБ`;
  return "—";
}

/** Иконка/цвет статуса пробника. */
export function triageStatusStyle(status: string): { icon: string; color: string } {
  switch (status) {
    case "pass": return { icon: "✅", color: "var(--green, #4caf50)" };
    case "warn": return { icon: "⚠️", color: "var(--orange, #ff9800)" };
    case "fail": return { icon: "💥", color: "var(--red, #f44336)" };
    default: return { icon: "⏭", color: "var(--border, #888)" };
  }
}

/** Цвет вердикт-лейбла модели. */
export function triageVerdictColor(label: string): string {
  if (label.includes("МУСОР")) return "var(--red, #f44336)";
  if (label.includes("Рискованная")) return "var(--orange, #ff9800)";
  if (label.includes("Годна")) return "var(--green, #4caf50)";
  return "var(--border, #888)";
}

/** Цвет бэнда итогового балла (чисто визуально; семантику вердикта не трогает). */
export function triageScoreColor(score: number, max: number): string {
  switch (triageScoreBand(score, max)) {
    case "good": return "var(--green, #4caf50)";
    case "mid": return "var(--orange, #ff9800)";
    default: return "var(--red, #f44336)";
  }
}

/** Бэйдж «Балл: X из Y» для шапки карточки модели; старые отчёты — «—». */
export function triageScoreBadge(r: TriageModelReport): string {
  if (typeof r.score !== "number" || typeof r.score_max !== "number") {
    return `<span style="opacity:0.6;">Балл: —</span>`;
  }
  return `<span style="color:${triageScoreColor(r.score, r.score_max)}; font-weight:bold;">Балл: ${escapeHtml(formatTriageScore(r.score, r.score_max))}</span>`;
}

/** Топ-лист батча (показывается при ≥2 моделях с баллом): место/модель/балл/вердикт/fail'ы. */
export function triageRankingHtml(reports: TriageModelReport[]): string {
  if (!hasTriageRanking(reports)) return "";
  const border = "1px solid var(--border, #444)";
  const rows = rankTriageModels(reports).map((row) => `
      <tr>
        <td style="padding:3px 8px; border-bottom:${border}; white-space:nowrap;">${row.place}</td>
        <td style="padding:3px 8px; border-bottom:${border};">${escapeHtml(row.model)}</td>
        <td style="padding:3px 8px; border-bottom:${border}; color:${triageScoreColor(row.score, row.scoreMax)}; white-space:nowrap;"><strong>${escapeHtml(formatTriageScore(row.score, row.scoreMax))}</strong></td>
        <td style="padding:3px 8px; border-bottom:${border}; color:${triageVerdictColor(row.verdictLabel)};">${escapeHtml(row.verdictLabel)}</td>
        <td style="padding:3px 8px; border-bottom:${border};">${escapeHtml(row.criticalFails.join("; ")) || "—"}</td>
      </tr>`).join("");
  return `<table style="border-collapse:collapse; width:100%; margin:0 0 12px 0;">
      <thead><tr>
        <th style="text-align:left; padding:3px 8px; border-bottom:${border};">#</th>
        <th style="text-align:left; padding:3px 8px; border-bottom:${border};">Модель</th>
        <th style="text-align:left; padding:3px 8px; border-bottom:${border};">Балл</th>
        <th style="text-align:left; padding:3px 8px; border-bottom:${border};">Вердикт</th>
        <th style="text-align:left; padding:3px 8px; border-bottom:${border};">Критичные fail'ы</th>
      </tr></thead>
      <tbody>${rows}</tbody></table>`;
}

/** HTML сводки триажа: общий для живых результатов и просмотрщика отчётов. */
export function triageBatchHtml(reports: TriageModelReport[], totalMs: number | null = null): string {
  const border = "1px solid var(--border, #444)";
  const totalPart = typeof totalMs === "number"
    ? ` · <strong>общее время батча:</strong> ${(totalMs / 1000).toFixed(1)}s`
    : "";
  const cards = reports.map((r, i) => {
    const short = r.model.split(/[/\\]/).pop() || r.model;
    const head = r.error
      ? `<div style="color: var(--red, #f44336); font-weight:bold;">💥 ОШИБКА ЗАПУСКА</div>
           <pre style="white-space:pre-wrap; color: var(--red, #f44336)">${escapeHtml(r.error)}</pre>`
      : `<div style="font-weight:bold; font-size:1.05em;">
             <span style="color:${triageVerdictColor(r.verdict_label)}">${escapeHtml(r.verdict_label)}</span>
             · ${triageScoreBadge(r)}
           </div>
           <div style="opacity:0.7; margin:2px 0 6px 0;">${(r.duration_ms / 1000).toFixed(1)}s · VRAM: ${fmtVram(null, r.vram_peak_mb ?? null)} · ОЗУ: ${fmtMb(r.ram_peak_mb ?? null)}</div>
           <table style="border-collapse:collapse; width:100%; margin-bottom:8px;">
             <thead><tr>
               <th style="text-align:left; padding:3px 8px; border-bottom:${border}">Пробник</th>
               <th style="text-align:left; padding:3px 8px; border-bottom:${border}">Итог</th>
               <th style="text-align:left; padding:3px 8px; border-bottom:${border}">Детали</th>
             </tr></thead>
             <tbody>${r.probes.map((p) => {
               const st = triageStatusStyle(p.status);
               return `<tr>
                 <td style="padding:3px 8px; border-bottom:${border}">${escapeHtml(p.title)}</td>
                 <td style="padding:3px 8px; border-bottom:${border}; color:${st.color}; white-space:nowrap;">${st.icon} ${escapeHtml(p.status)}</td>
                 <td style="padding:3px 8px; border-bottom:${border}">${escapeHtml(p.detail)}</td>
               </tr>`;
             }).join("")}</tbody>
           </table>`;
    return `<div style="margin-bottom:16px;"><strong>${i + 1}. ${escapeHtml(short)}</strong>${head}</div>`;
  }).join('<hr style="border:none; border-top:1px solid var(--border, #444); margin:12px 0"/>');

  return `<div style="margin-bottom:12px;"><strong>Триаж моделей</strong> · моделей: ${reports.length}${totalPart}</div>` +
    triageRankingHtml(reports) + cards;
}
