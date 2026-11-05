/**
 * neurosis-view — визуальный отчёт кейса психотерапевта в чате (05.11.2026).
 *
 * Порт сырых билдеров владельца (buildNeurosisSVG.js / buildMermaidScheme.js,
 * сырой коммит 45b5469): foreignObject-карточки, SMIL-анимации, flowchart BT.
 * Билдеры — чистые функции; состояние переключателя SVG/Схема — локальное
 * замыкание карточки.
 *
 * Данные приходят из system_message ноды save_neurosis_case: фенс
 * neurosis_case с плоским JSON (13 полей) renderMarkdown извлекает в
 * `<pre class="neurosis-case">`, карточку собирает materializeNeurosisCases
 * (хук chat.ts рядом с renderMermaid). Дефолт — SVG (решение владельца).
 */

export interface NeurosisCase {
  target: string;
  core_type: number | string;
  floor1_type: number | string;
  floor0_type: number | string;
  e1: string | false;
  e2: string | false;
  e3: string | false;
  e4: string | false;
  e5: string | false;
  e6: string | false;
  e7: string | false;
  e8: string | false;
  e9: string | false;
}

const PATTERN_NAMES: Record<string, string> = {
  "1": "Перфекционист / Реформатор",
  "2": "Помощник / Спаситель",
  "3": "Достигатор / Исполнитель",
  "4": "Индивидуалист / Романтик",
  "5": "Наблюдатель / Исследователь",
  "6": "Сомневающийся / Лоялист",
  "7": "Энтузиаст / Эпикуреец",
  "8": "Конфронтатор / Босс",
  "9": "Миротворец / Сливающийся",
};

const ELEMENT_ROLES: Record<string, string> = {
  e1: "ПРОБЛЕМА / СОПРОТИВЛЕНИЕ (РЕЗУЛЬТАТ)",
  e2: "АВТОМАТИЧЕСКОЕ ПОВЕДЕНИЕ",
  e3: "КАТАСТРОФА / ДЫРА (ЯДРО)",
  e4: "САМООЦЕНКА (Я-КОНЦЕПЦИЯ)",
  e5: "ПРАВИЛО-КНУТ (ЗАПРЕТ)",
  e6: "ТРИГГЕР (ЗАБОР / ГРАНИЦА ЗОНЫ e3)",
  e7: "ЭМОЦИЯ-ПОВТОРИТЕЛЬ (ГРЕХ)",
  e8: "ИЛЛЮЗИЯ (КАРТИНА МИРА)",
  e9: "КОРНЕВОЙ ИДОЛ (МАЯК-ОРИЕНТИР)",
};

/** Разбор JSON кейса: объект обязателен (массив/скаляр/битый — null). */
export function parseCaseJson(raw: string): NeurosisCase | null {
  try {
    const value: unknown = JSON.parse(raw);
    if (value === null || typeof value !== "object" || Array.isArray(value)) return null;
    return value as NeurosisCase;
  } catch {
    return null;
  }
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function textOf(v: string | false): string {
  return typeof v === "string" ? v : "";
}

/** Универсальный генератор анимированного SVG (плоский JSON любого кейса). */
export function buildNeurosisSVG(caseData: NeurosisCase): string {
  const p0Name = PATTERN_NAMES[String(caseData.floor0_type)] || "";
  const p1Name = PATTERN_NAMES[String(caseData.floor1_type)] || "";
  const p2Name = PATTERN_NAMES[String(caseData.core_type)] || "";

  const renderCard = (
    x: number,
    y: number,
    w: number,
    h: number,
    code: string,
    text: string | false,
    themeClass: string,
    badgeBg: string,
  ): string =>
    `<foreignObject x="${x}" y="${y}" width="${w}" height="${h}">` +
    `<div xmlns="http://www.w3.org/1999/xhtml" class="ncard ${themeClass}">` +
    `<div class="nhead"><span>${code.toUpperCase()} — ${ELEMENT_ROLES[code]}</span>` +
    `<span class="nbadge" style="background:${badgeBg};">${code.toUpperCase()}</span></div>` +
    `<div class="ntext">${escapeHtml(textOf(text))}</div></div></foreignObject>`;

  return `<svg viewBox="0 0 900 1380" width="100%" xmlns="http://www.w3.org/2000/svg" style="max-width:900px;height:auto;display:block;">
<style>
.ncard { width: 100%; height: 100%; box-sizing: border-box; border-radius: 10px; padding: 10px 14px; display: flex; flex-direction: column; justify-content: center; color: #f8fafc; font-family: 'Segoe UI', Roboto, sans-serif; font-size: 13px; line-height: 1.35; }
.nhead { display: flex; justify-content: space-between; align-items: center; margin-bottom: 4px; font-weight: 800; font-size: 13px; color: #ffffff; }
.nbadge { padding: 2px 7px; border-radius: 5px; font-size: 11px; font-weight: 800; color: #ffffff; }
.ntext { color: #cbd5e1; font-size: 12.5px; }
.t-f0 { background: linear-gradient(135deg, #171c2d, #1e2640); border: 1.5px solid #5c6bc0; }
.t-f1 { background: linear-gradient(135deg, #132423, #183330); border: 1.5px solid #26a69a; }
.t-f2 { background: linear-gradient(135deg, #271b12, #362314); border: 1.5px solid #ff9800; }
.t-e6 { background: linear-gradient(135deg, #2b1810, #3d2012); border: 2px dashed #ff9100; }
.t-e3 { background: linear-gradient(135deg, #4a0913, #6e0d1c); border: 2.5px solid #ff1744; box-shadow: 0 0 25px rgba(255, 23, 68, 0.45); }
.t-e3 .ntext { color: #ffe4e6; font-weight: 600; font-size: 13px; }
</style>
<defs>
<radialGradient id="coreGlow" cx="50%" cy="50%" r="50%">
<stop offset="0%" stop-color="#ff1744" stop-opacity="0.55"/>
<stop offset="65%" stop-color="#d50000" stop-opacity="0.18"/>
<stop offset="100%" stop-color="#ff1744" stop-opacity="0"/>
</radialGradient>
<marker id="arr-f2" viewBox="0 0 10 10" refX="6" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M 0 1 L 10 5 L 0 9 z" fill="#ff9800"/></marker>
<marker id="arr-f1" viewBox="0 0 10 10" refX="6" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M 0 1 L 10 5 L 0 9 z" fill="#26a69a"/></marker>
<marker id="arr-f0" viewBox="0 0 10 10" refX="6" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M 0 1 L 10 5 L 0 9 z" fill="#7986cb"/></marker>
</defs>

<rect x="25" y="18" width="850" height="72" rx="12" fill="#141a26" stroke="#2d3748" stroke-width="1.5"/>
<text x="45" y="47" fill="#f8fafc" font-family="Segoe UI, sans-serif" font-size="16.5" font-weight="800">🎯 МИШЕНЬ: ${escapeHtml(caseData.target || "")}</text>
<text x="45" y="72" fill="#94a3b8" font-family="Segoe UI, sans-serif" font-size="13.5" font-weight="600">🔥 ЯДРО: ПАТТЕРН ${caseData.core_type} (${p2Name}) | Рост энергии: Тип ${caseData.core_type} ➔ Тип ${caseData.floor1_type} ➔ Тип ${caseData.floor0_type}</text>

<rect x="25" y="110" width="850" height="365" rx="14" fill="#111624" stroke="#5c6bc0" stroke-width="2"/>
<rect x="25" y="110" width="850" height="36" rx="14" fill="#283593"/>
<text x="45" y="133" fill="#ffffff" font-family="Segoe UI, sans-serif" font-size="14" font-weight="800">🧠 ЭТАЖ 0: ГОМЕОСТАЗ И СИМПТОМ — ПАТТЕРН ${caseData.floor0_type} (${p0Name})</text>
${renderCard(55, 160, 790, 86, "e1", caseData.e1, "t-f0", "#3949ab")}
${renderCard(55, 270, 790, 86, "e7", caseData.e7, "t-f0", "#3949ab")}
${renderCard(55, 378, 790, 82, "e4", caseData.e4, "t-f0", "#3949ab")}
<path d="M 450 376 L 450 360" stroke="#7986cb" stroke-width="3" marker-end="url(#arr-f0)"/>
<path d="M 450 268 L 450 250" stroke="#7986cb" stroke-width="3" marker-end="url(#arr-f0)"/>

<rect x="25" y="510" width="850" height="365" rx="14" fill="#0e1c1c" stroke="#26a69a" stroke-width="2"/>
<rect x="25" y="510" width="850" height="36" rx="14" fill="#00695c"/>
<text x="45" y="533" fill="#ffffff" font-family="Segoe UI, sans-serif" font-size="14" font-weight="800">🛡 ЭТАЖ -1: ЗАЩИТНЫЙ ПЕРИМЕТР — ПАТТЕРН ${caseData.floor1_type} (${p1Name})</text>
${renderCard(55, 560, 790, 86, "e2", caseData.e2, "t-f1", "#00897b")}
${renderCard(55, 670, 790, 86, "e5", caseData.e5, "t-f1", "#00897b")}
${renderCard(55, 778, 790, 82, "e8", caseData.e8, "t-f1", "#00897b")}
<path d="M 450 776 L 450 760" stroke="#26a69a" stroke-width="3" marker-end="url(#arr-f1)"/>
<path d="M 450 668 L 450 650" stroke="#26a69a" stroke-width="3" marker-end="url(#arr-f1)"/>

<rect x="25" y="910" width="850" height="445" rx="14" fill="#1c100b" stroke="#ff1744" stroke-width="3"/>
<rect x="25" y="910" width="850" height="40" rx="14" fill="#b71c1c"/>
<text x="45" y="935" fill="#ffffff" font-family="Segoe UI, sans-serif" font-size="15" font-weight="900">🔥 ЭТАЖ -2: ЯДРО ТРАВМЫ — ПАТТЕРН ${caseData.core_type} (${p2Name})</text>

<ellipse cx="450" cy="1265" rx="390" ry="75" fill="url(#coreGlow)">
<animate attributeName="rx" values="360;415;360" dur="3.2s" repeatCount="indefinite"/>
<animate attributeName="ry" values="65;92;65" dur="3.2s" repeatCount="indefinite"/>
</ellipse>

<path d="M 55 1205 Q 450 1175 845 1205" fill="none" stroke="#ff9100" stroke-width="2.5" stroke-dasharray="10,6">
<animate attributeName="stroke-dashoffset" from="0" to="32" dur="1.5s" repeatCount="indefinite"/>
</path>
<text x="450" y="1192" text-anchor="middle" fill="#ffab40" font-family="Segoe UI, sans-serif" font-size="11" font-weight="800">▲ ГРАНИЦА ЗОНЫ ЯДРА (ЗАБОР e6 — ЧЕЛОВЕК БЕЖИТ ОТСЮДА К МАЯКУ e9) ▲</text>

${renderCard(55, 968, 790, 86, "e9", caseData.e9, "t-f2", "#e65100")}
${renderCard(55, 1082, 790, 88, "e6", caseData.e6, "t-e6", "#ff6d00")}
${renderCard(55, 1215, 790, 115, "e3", caseData.e3, "t-e3", "#d50000")}

<path d="M 450 1213 L 450 1174" stroke="#ff1744" stroke-width="4" marker-end="url(#arr-f2)"/>
<path d="M 450 1080 L 450 1058" stroke="#ff9800" stroke-width="3.5" marker-end="url(#arr-f2)"/>

<path d="M 450 966 L 450 868" stroke="#ff9800" stroke-width="4" marker-end="url(#arr-f1)"/>
<path d="M 450 558 L 450 478" stroke="#26a69a" stroke-width="4" marker-end="url(#arr-f0)"/>

<circle r="5" fill="#ff1744"><animateMotion dur="1.4s" repeatCount="indefinite" path="M 450 1213 L 450 1060"/></circle>
<circle r="5" fill="#ffab00"><animateMotion dur="1.2s" repeatCount="indefinite" path="M 450 966 L 450 870"/></circle>
<circle r="5" fill="#1de9b6"><animateMotion dur="1.2s" repeatCount="indefinite" path="M 450 558 L 450 480"/></circle>
</svg>`;
}

/** Экранирование кавычек и переводов строк, чтобы Mermaid не ломался от текста. */
function cleanText(v: string | false): string {
  const str = typeof v === "string" ? v : "";
  return str.replace(/"/g, "«").replace(/\n/g, "<br/>");
}

/** Универсальный генератор Mermaid-схемы (плоский JSON любого кейса). */
export function buildMermaidScheme(caseData: NeurosisCase): string {
  const p0 = `${caseData.floor0_type} (${PATTERN_NAMES[String(caseData.floor0_type)] || ""})`;
  const p1 = `${caseData.floor1_type} (${PATTERN_NAMES[String(caseData.floor1_type)] || ""})`;
  const p2 = `${caseData.core_type} (${PATTERN_NAMES[String(caseData.core_type)] || ""})`;

  return `flowchart BT
    classDef floor0 fill:#1e2436,stroke:#5c6bc0,stroke-width:2px,color:#ffffff
    classDef floor1 fill:#1b2e2b,stroke:#26a69a,stroke-width:2px,color:#ffffff
    classDef floor2 fill:#2d1f14,stroke:#ff9800,stroke-width:2px,color:#ffffff
    classDef coreHole fill:#5c0a16,stroke:#ff1744,stroke-width:4px,color:#ffffff

    subgraph F2 ["🔥 ЭТАЖ -2: ЯДРО ТРАВМЫ — ПАТТЕРН ${p2}"]
        direction BT
        E3["💀 <b>e3: КАТАСТРОФА / ДЫРА (Ядро)</b><br/>${cleanText(caseData.e3)}"]:::coreHole
        E6["⚡ <b>e6: Триггер (Забор вокруг Дыры)</b><br/>${cleanText(caseData.e6)}"]:::floor2
        E9["🏆 <b>e9: Корневой Идол (Маяк-ориентир)</b><br/>${cleanText(caseData.e9)}"]:::floor2

        E3 --> E6
        E6 --> E9
    end

    subgraph F1 ["🛡 ЭТАЖ -1: ЗАЩИТНЫЙ ПЕРИМЕТР — ПАТТЕРН ${p1}"]
        direction BT
        E8["🌍 <b>e8: Иллюзия (Картина мира)</b><br/>${cleanText(caseData.e8)}"]:::floor1
        E5["⛓ <b>e5: Правило-Кнут (Запрет)</b><br/>${cleanText(caseData.e5)}"]:::floor1
        E2["🔄 <b>e2: Автоматическое поведение</b><br/>${cleanText(caseData.e2)}"]:::floor1

        E8 --> E5
        E5 --> E2
    end

    subgraph F0 ["🧠 ЭТАЖ 0: ГОМЕОСТАЗ И СИМПТОМ — ПАТТЕРН ${p0}"]
        direction BT
        E4["👤 <b>e4: Самооценка (Я-концепция)</b><br/>${cleanText(caseData.e4)}"]:::floor0
        E7["🔋 <b>e7: Эмоция-повторитель (Грех)</b><br/>${cleanText(caseData.e7)}"]:::floor0
        E1["🧱 <b>e1: Проблема / Сопротивление (Результат)</b><br/>${cleanText(caseData.e1)}"]:::floor0

        E4 --> E7
        E7 --> E1
    end

    E9 ==> E8
    E2 ==> E4`;
}

/** Карточка визуального отчёта на месте фенса neurosis_case.
 * Дефолт — SVG; кнопка «Схема»/«SVG» переключает режимы, схема рендерится
 * существующим mermaid-механизмом чата (переданный renderMermaid). */
export function materializeNeurosisCases(
  container: HTMLElement,
  renderMermaid?: () => Promise<void>,
): void {
  const blocks = container.querySelectorAll<HTMLElement>("pre.neurosis-case");
  blocks.forEach((pre) => {
    if (pre.dataset.materialized === "1") return; // идемпотентность
    pre.dataset.materialized = "1";

    const raw = (pre.textContent || "").trim();
    const caseData = parseCaseJson(raw);
    const card = document.createElement("div");
    card.className = "neurosis-card";

    if (!caseData) {
      // Битый JSON — честная ошибка вместо тихого пропуска (§00.3).
      const err = document.createElement("div");
      err.className = "neurosis-card-error";
      err.textContent = "⚠️ Кейс повреждён: не удалось разобрать JSON визуального отчёта";
      err.style.cssText =
        "padding:8px 12px;color:#fca5a5;background:#2a1215;border:1px solid #7f1d1d;border-radius:8px;font-size:13px;";
      card.appendChild(err);
      pre.replaceWith(card);
      return;
    }

    card.style.cssText =
      "border:1px solid #2d3748;border-radius:12px;padding:10px;margin:6px 0;background:#0d1117;";

    let mode: "svg" | "scheme" = "svg";
    const view = document.createElement("div");
    view.className = "neurosis-card-view";
    view.innerHTML = buildNeurosisSVG(caseData);

    const btn = document.createElement("button");
    btn.className = "neurosis-card-toggle";
    btn.type = "button";
    btn.textContent = "Схема";
    btn.style.cssText =
      "display:block;margin-left:auto;margin-bottom:6px;background:#283593;color:#ffffff;border:none;border-radius:6px;padding:4px 12px;font-size:12px;cursor:pointer;";

    btn.addEventListener("click", () => {
      if (mode === "svg") {
        mode = "scheme";
        // Экранируем разметку, чтобы textContent pre.mermaid содержал исходник
        // (mermaid.run читает textContent; htmlLabels отрисует <b>/<br/>).
        const scheme = buildMermaidScheme(caseData)
          .replace(/&/g, "&amp;")
          .replace(/</g, "&lt;");
        view.innerHTML = `<pre class="mermaid">${scheme}</pre>`;
        btn.textContent = "SVG";
        if (renderMermaid) void renderMermaid();
      } else {
        mode = "svg";
        view.innerHTML = buildNeurosisSVG(caseData);
        btn.textContent = "Схема";
      }
    });

    card.appendChild(btn);
    card.appendChild(view);
    pre.replaceWith(card);
  });
}
