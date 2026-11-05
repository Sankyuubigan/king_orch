const PATTERN_NAMES = {
  1: "Перфекционист / Реформатор",
  2: "Помощник / Спаситель",
  3: "Достигатор / Исполнитель",
  4: "Индивидуалист / Романтик",
  5: "Наблюдатель / Исследователь",
  6: "Сомневающийся / Лоялист",
  7: "Энтузиаст / Эпикуреец",
  8: "Конфронтатор / Босс",
  9: "Миротворец / Сливающийся"
};

const ELEMENT_ROLES = {
  e1: "ПРОБЛЕМА / СОПРОТИВЛЕНИЕ (РЕЗУЛЬТАТ)",
  e7: "ЭМОЦИЯ-ПОВТОРИТЕЛЬ (ГРЕХ)",
  e4: "САМООЦЕНКА (Я-КОНЦЕПЦИЯ)",
  e2: "АВТОМАТИЧЕСКОЕ ПОВЕДЕНИЕ",
  e5: "ПРАВИЛО-КНУТ (ЗАПРЕТ)",
  e8: "ИЛЛЮЗИЯ (КАРТИНА МИРА)",
  e9: "КОРНЕВОЙ ИДОЛ (МАЯК-ОРИЕНТИР)",
  e6: "ТРИГГЕР (ЗАБОР / ГРАНИЦА ЗОНЫ e3)",
  e3: "КАТАСТРОФА / ДЫРА (ЯДРО)"
};

// УНИВЕРСАЛЬНЫЙ ГЕНЕРАТОР АНИМИРОВАННОГО SVG (Принимает плоский JSON любого кейса)
export function buildNeurosisSVG(caseData) {
  const p0Name = PATTERN_NAMES[caseData.floor0_type] || "";
  const p1Name = PATTERN_NAMES[caseData.floor1_type] || "";
  const p2Name = PATTERN_NAMES[caseData.core_type] || "";

  const renderCard = (x, y, w, h, code, text, themeClass, badgeBg) => `
    <foreignObject x="${x}" y="${y}" width="${w}" height="${h}">
      <div xmlns="http://www.w3.org/1999/xhtml" class="ncard ${themeClass}">
        <div class="nhead">
          <span>${code.toUpperCase()} — ${ELEMENT_ROLES[code]}</span>
          <span class="nbadge" style="background:${badgeBg};">${code.toUpperCase()}</span>
        </div>
        <div class="ntext">${text || ""}</div>
      </div>
    </foreignObject>
  `;

  return `
  <svg viewBox="0 0 900 1380" width="100%" height="100%" xmlns="http://www.w3.org/2000/svg">
    <style>
      .ncard {
        width: 100%; height: 100%; box-sizing: border-box;
        border-radius: 10px; padding: 10px 14px;
        display: flex; flex-direction: column; justify-content: center;
        color: #f8fafc; font-family: 'Segoe UI', Roboto, sans-serif;
        font-size: 13px; line-height: 1.35;
      }
      .nhead {
        display: flex; justify-content: space-between; align-items: center;
        margin-bottom: 4px; font-weight: 800; font-size: 13px; color: #ffffff;
      }
      .nbadge {
        padding: 2px 7px; border-radius: 5px; font-size: 11px;
        font-weight: 800; color: #ffffff;
      }
      .ntext { color: #cbd5e1; font-size: 12.5px; }
      .t-f0 { background: linear-gradient(135deg, #171c2d, #1e2640); border: 1.5px solid #5c6bc0; }
      .t-f1 { background: linear-gradient(135deg, #132423, #183330); border: 1.5px solid #26a69a; }
      .t-f2 { background: linear-gradient(135deg, #271b12, #362314); border: 1.5px solid #ff9800; }
      .t-e6 { background: linear-gradient(135deg, #2b1810, #3d2012); border: 2px dashed #ff9100; }
      .t-e3 {
        background: linear-gradient(135deg, #4a0913, #6e0d1c);
        border: 2.5px solid #ff1744;
        box-shadow: 0 0 25px rgba(255, 23, 68, 0.45);
      }
      .t-e3 .ntext { color: #ffe4e6; font-weight: 600; font-size: 13px; }
    </style>

    <defs>
      <radialGradient id="coreGlow" cx="50%" cy="50%" r="50%">
        <stop offset="0%" stop-color="#ff1744" stop-opacity="0.55"/>
        <stop offset="65%" stop-color="#d50000" stop-opacity="0.18"/>
        <stop offset="100%" stop-color="#ff1744" stop-opacity="0"/>
      </radialGradient>
      <marker id="arr-f2" viewBox="0 0 10 10" refX="6" refY="5" markerWidth="7" markerHeight="7" orient="auto">
        <path d="M 0 1 L 10 5 L 0 9 z" fill="#ff9800"/>
      </marker>
      <marker id="arr-f1" viewBox="0 0 10 10" refX="6" refY="5" markerWidth="7" markerHeight="7" orient="auto">
        <path d="M 0 1 L 10 5 L 0 9 z" fill="#26a69a"/>
      </marker>
      <marker id="arr-f0" viewBox="0 0 10 10" refX="6" refY="5" markerWidth="7" markerHeight="7" orient="auto">
        <path d="M 0 1 L 10 5 L 0 9 z" fill="#7986cb"/>
      </marker>
    </defs>

    <!-- ШАПКА -->
    <rect x="25" y="18" width="850" height="72" rx="12" fill="#141a26" stroke="#2d3748" stroke-width="1.5"/>
    <text x="45" y="47" fill="#f8fafc" font-family="Segoe UI, sans-serif" font-size="16.5" font-weight="800">
      🎯 МИШЕНЬ: ${caseData.target || ""}
    </text>
    <text x="45" y="72" fill="#94a3b8" font-family="Segoe UI, sans-serif" font-size="13.5" font-weight="600">
      🔥 ЯДРО: ПАТТЕРН ${caseData.core_type} (${p2Name})  |  Рост энергии: Тип ${caseData.core_type} ➔ Тип ${caseData.floor1_type} ➔ Тип ${caseData.floor0_type}
    </text>

    <!-- ЭТАЖ 0 (НАВЕРХУ): e4 -> e7 -> e1 -->
    <rect x="25" y="110" width="850" height="365" rx="14" fill="#111624" stroke="#5c6bc0" stroke-width="2"/>
    <rect x="25" y="110" width="850" height="36" rx="14" fill="#283593"/>
    <text x="45" y="133" fill="#ffffff" font-family="Segoe UI, sans-serif" font-size="14" font-weight="800">
      🧠 ЭТАЖ 0: ГОМЕОСТАЗ И СИМПТОМ — ПАТТЕРН ${caseData.floor0_type} (${p0Name})
    </text>
    ${renderCard(55, 160, 790, 86, "e1", caseData.e1, "t-f0", "#3949ab")}
    ${renderCard(55, 270, 790, 86, "e7", caseData.e7, "t-f0", "#3949ab")}
    ${renderCard(55, 378, 790, 82, "e4", caseData.e4, "t-f0", "#3949ab")}
    <path d="M 450 378 L 450 360" stroke="#7986cb" stroke-width="3" marker-end="url(#arr-f0)"/>
    <path d="M 450 270 L 450 250" stroke="#7986cb" stroke-width="3" marker-end="url(#arr-f0)"/>

    <!-- ЭТАЖ -1 (В СЕРЕДИНЕ): e8 -> e5 -> e2 -->
    <rect x="25" y="510" width="850" height="365" rx="14" fill="#0e1c1c" stroke="#26a69a" stroke-width="2"/>
    <rect x="25" y="510" width="850" height="36" rx="14" fill="#00695c"/>
    <text x="45" y="533" fill="#ffffff" font-family="Segoe UI, sans-serif" font-size="14" font-weight="800">
      🛡 ЭТАЖ -1: ЗАЩИТНЫЙ ПЕРИМЕТР — ПАТТЕРН ${caseData.floor1_type} (${p1Name})
    </text>
    ${renderCard(55, 560, 790, 86, "e2", caseData.e2, "t-f1", "#00897b")}
    ${renderCard(55, 670, 790, 86, "e5", caseData.e5, "t-f1", "#00897b")}
    ${renderCard(55, 778, 790, 82, "e8", caseData.e8, "t-f1", "#00897b")}
    <path d="M 450 778 L 450 760" stroke="#26a69a" stroke-width="3" marker-end="url(#arr-f1)"/>
    <path d="M 450 670 L 450 650" stroke="#26a69a" stroke-width="3" marker-end="url(#arr-f1)"/>

    <!-- ЭТАЖ -2 (ВНИЗУ): ЯДРО e3 -> ЗАБОР e6 -> МАЯК e9 -->
    <rect x="25" y="910" width="850" height="445" rx="14" fill="#1c100b" stroke="#ff1744" stroke-width="3"/>
    <rect x="25" y="910" width="850" height="40" rx="14" fill="#b71c1c"/>
    <text x="45" y="935" fill="#ffffff" font-family="Segoe UI, sans-serif" font-size="15" font-weight="900">
      🔥 ЭТАЖ -2: ЯДРО ТРАВМЫ — ПАТТЕРН ${caseData.core_type} (${p2Name})
    </text>

    <!-- Пульсирующая область действия e3 -->
    <ellipse cx="450" cy="1265" rx="390" ry="75" fill="url(#coreGlow)">
      <animate attributeName="rx" values="360;415;360" dur="3.2s" repeatCount="indefinite"/>
      <animate attributeName="ry" values="65;92;65" dur="3.2s" repeatCount="indefinite"/>
    </ellipse>

    <!-- Линия забора e6 (граница зоны e3) -->
    <path d="M 55 1205 Q 450 1175 845 1205" fill="none" stroke="#ff9100" stroke-width="2.5" stroke-dasharray="10,6">
      <animate attributeName="stroke-dashoffset" from="0" to="32" dur="1.5s" repeatCount="indefinite"/>
    </path>
    <text x="450" y="1192" text-anchor="middle" fill="#ffab40" font-family="Segoe UI, sans-serif" font-size="11" font-weight="800">
      ▲ ГРАНИЦА ЗОНЫ ЯДРА (ЗАБОР e6 — ЧЕЛОВЕК БЕЖИТ ОТСЮДА К МАЯКУ e9) ▲
    </text>

    ${renderCard(55, 968, 790, 86, "e9", caseData.e9, "t-f2", "#e65100")}
    ${renderCard(55, 1082, 790, 88, "e6", caseData.e6, "t-e6", "#ff6d00")}
    ${renderCard(55, 1215, 790, 115, "e3", caseData.e3, "t-e3", "#d50000")}

    <path d="M 450 1215 L 450 1174" stroke="#ff1744" stroke-width="4" marker-end="url(#arr-f2)"/>
    <path d="M 450 1082 L 450 1058" stroke="#ff9800" stroke-width="3.5" marker-end="url(#arr-f2)"/>

    <!-- Межэтажные магистрали роста дерева снизу вверх -->
    <path d="M 450 968 L 450 864" stroke="#ff9800" stroke-width="4" marker-end="url(#arr-f1)"/>
    <path d="M 450 560 L 450 464" stroke="#26a69a" stroke-width="4" marker-end="url(#arr-f0)"/>

    <!-- Анимированные импульсы энергии снизу вверх -->
    <circle r="5" fill="#ff1744"><animateMotion dur="1.4s" repeatCount="indefinite" path="M 450 1215 L 450 1058"/></circle>
    <circle r="5" fill="#ffab00"><animateMotion dur="1.2s" repeatCount="indefinite" path="M 450 968 L 450 860"/></circle>
    <circle r="5" fill="#1de9b6"><animateMotion dur="1.2s" repeatCount="indefinite" path="M 450 560 L 450 460"/></circle>
  </svg>`;
}