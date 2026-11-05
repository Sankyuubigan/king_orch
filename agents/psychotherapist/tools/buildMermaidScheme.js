const PATTERN_NAMES = {
  1: "Перфекционист",
  2: "Помощник",
  3: "Достигатор",
  4: "Индивидуалист",
  5: "Наблюдатель",
  6: "Сомневающийся",
  7: "Энтузиаст",
  8: "Конфронтатор",
  9: "Миротворец"
};

// Экранирование кавычек, чтобы Mermaid никогда не ломался от текста юзера
function cleanText(str = "") {
  return str.replace(/"/g, "«").replace(/\n/g, "<br/>");
}

// УНИВЕРСАЛЬНЫЙ ГЕНЕРАТОР MERMAID (Принимает любой плоский JSON кейса)
export function buildMermaidScheme(caseData) {
  const p0 = `${caseData.floor0_type} (${PATTERN_NAMES[caseData.floor0_type] || ""})`;
  const p1 = `${caseData.floor1_type} (${PATTERN_NAMES[caseData.floor1_type] || ""})`;
  const p2 = `${caseData.core_type} (${PATTERN_NAMES[caseData.core_type] || ""})`;

  return `flowchart BT
    classDef floor0 fill:#1e2436,stroke:#5c6bc0,stroke-width:2px,color:#ffffff
    classDef floor1 fill:#1b2e2b,stroke:#26a69a,stroke-width:2px,color:#ffffff
    classDef floor2 fill:#2d1f14,stroke:#ff9800,stroke-width:2px,color:#ffffff
    classDef coreHole fill:#5c0a16,stroke:#ff1744,stroke-width:4px,color:#ffffff,font-weight:bold

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