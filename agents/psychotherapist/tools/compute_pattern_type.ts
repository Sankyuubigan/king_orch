/**
 * compute_pattern_type — тул команды «Психотерапевт»: механическая проверка
 * ПАРАМИ eN/eN_text и ОДНОГО сквозного вектора (`pattern_vector`), выбранного
 * Валидатором, + сборка ЧИСТОГО кейс-отчёта (план спеца 13.11.2026).
 *
 * Роли разведены: канал решения — ЛИТЕРАЛ bool (`eN`), текст — `eN_text`
 * (виден юзеру на схеме). Заглушка-строка в канале решения невозможна
 * грамматикой контракта; химеры (согласование этажей по отдельности)
 * невозможны выбором РОВНО одного готового вектора из каталога.
 * Здесь НЕТ авто-выравнивания и НЕТ эвристик «доказанности по тексту»:
 * неполные данные — честный отказ (§00.2), маршрутизируемый графом
 * (check_pattern → добор данных).
 *
 * Вход: Deno.args[0] = JSON (значения приходят СТРОКАМИ от движка):
 *   {"report": "<JSON-строка сигнала validator_report или сам объект>"}
 *
 * Выход: ТОЛЬКО JSON в stdout:
 *   {
 *     "pattern_found":  boolean,
 *     "surface_proven": boolean,
 *     "reason":         "<честное объяснение исхода>",
 *     "pattern_vector": "<литерал из сигнала, 'none' — как пришло>",
 *     "core_type":      "1".."9" | "none",
 *     "floor1_type":    "1".."9" | "none",
 *     "floor0_type":    "1".."9" | "none",
 *     "case_report": {
 *       "target": "<житейская мишень>",
 *       "core_type": "…", "floor1_type": "…", "floor0_type": "…",
 *       "e1": "<суть словами пользователя>" | false, …, "e9": …
 *     }
 *   }
 *
 * case_report — ЕДИНСТВЕННЫЙ downstream-формат (save_neurosis_case,
 * read_pattern_sections, call_healer читают его, а не сырой сигнал):
 * текст eN_text без подтверждения bool-каналом сюда не попадает никогда.
 */

/** Каталог сквозных векторов — SSOT (конверсии Безопасности,
 * `database/conversions_patterns.md`; порядок = каталог №1..№9).
 * Литерал контракта (формат владельца 13.11.2026) → [Ядро, Защита, Симптом]. */
const PATTERN_VECTORS: Readonly<Record<string, readonly [number, number, number]>> = {
  "Core 5 | Protection 7 | Symptom 1": [5, 7, 1],
  "Core 1 | Protection 4 | Symptom 2": [1, 4, 2],
  "Core 9 | Protection 6 | Symptom 3": [9, 6, 3],
  "Core 7 | Protection 1 | Symptom 4": [7, 1, 4],
  "Core 2 | Protection 8 | Symptom 5": [2, 8, 5],
  "Core 3 | Protection 9 | Symptom 6": [3, 9, 6],
  "Core 8 | Protection 5 | Symptom 7": [8, 5, 7],
  "Core 4 | Protection 2 | Symptom 8": [4, 2, 8],
  "Core 6 | Protection 3 | Symptom 9": [6, 3, 9],
};

const NONE = "none";

/** Элементы поверхностного контура для производного surface_proven:
 * сопротивление (e1), автоматизм (e2), триггер (e6), эмоция-повторитель (e7). */
const SURFACE_ELEMENTS: readonly string[] = ["e1", "e2", "e6", "e7"];

/** Гейты принятия вектора: элемент считается доказанным, только если
 * пользователь САМ подтвердил его (eN=true) и дал свою формулировку
 * (непустой eN_text). e4 (самооценка) НЕ гейтит — опорный контекст:
 * нейрохиллер строит шаги лечения на e1/e2/e9 (проверено по его md). */
const GATES: readonly string[] = ["e1", "e2", "e3", "e5", "e6", "e7", "e8", "e9"];

const E_KEYS: readonly string[] = [
  "e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8", "e9",
];

/** trim + заглавная первая буква. НИКАКОЙ хирургии текста регэкспами:
 * валидатор обязан писать житейскую формулировку словами пользователя,
 * а не канцелярит (правило — в его md); чистая строка downstream. */
function capFirst(text: string): string {
  const trimmed = text.trim();
  if (trimmed.length === 0) return trimmed;
  return trimmed[0].toUpperCase() + trimmed.slice(1);
}

export interface CaseReport {
  target: string;
  core_type: string;
  floor1_type: string;
  floor0_type: string;
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

export interface PatternVerdict {
  /** Вектор принят (все гейты подтверждены парами eN/eN_text). */
  pattern_found: boolean;
  /** Производный булев: ВСЕ элементы поверхностного контура {e1, e2, e6, e7} подтверждены. */
  surface_proven: boolean;
  /** Честное объяснение исхода (для логов и диагностики пайплайна). */
  reason: string;
  /** Литерал из сигнала как пришло ("none" — ранний ход без вскрытого Ядра). */
  pattern_vector: string;
  /** Цифры выбранного вектора строками ("none" при отказе) — читаемо роутером. */
  core_type: string;
  floor1_type: string;
  floor0_type: string;
  /** Чистый кейс-отчёт для save_neurosis_case / call_healer. */
  case_report: CaseReport;
}

/** Элемент доказан = пользователь сам подтвердил (bool-канал решения)
 * И дал свою формулировку (непустой eN_text). Одно без другого — не доказано:
 * bool без текста не даёт материала кейса, текст без bool — не решение. */
function proven(obj: Record<string, unknown>, key: string): boolean {
  if (obj[key] !== true) return false;
  const text = obj[`${key}_text`];
  return typeof text === "string" && text.trim() !== "";
}

/** Чистый элемент кейса: формулировка пользователя или честное false. */
function element(obj: Record<string, unknown>, key: string): string | false {
  if (!proven(obj, key)) return false;
  const text = obj[`${key}_text`];
  return typeof text === "string" ? capFirst(text) : false;
}

function asObject(value: unknown): Record<string, unknown> | undefined {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  return value as Record<string, unknown>;
}

function buildCaseReport(
  obj: Record<string, unknown>,
  core: string,
  floor1: string,
  floor0: string,
): CaseReport {
  const rawTarget = typeof obj["target"] === "string" ? obj["target"] : "";
  const elements = Object.fromEntries(
    E_KEYS.map((key) => [key, element(obj, key)]),
  ) as Record<string, string | false>;
  return {
    target: capFirst(rawTarget),
    core_type: core,
    floor1_type: floor1,
    floor0_type: floor0,
    ...elements,
  } as CaseReport;
}

/** Механическая проверка пар и вектора + сборка чистого кейс-отчёта.
 * Авто-выравнивания нет: вектор выбирает МОДЕЛЬ одним выбором из каталога
 * (химеры невозможны), здесь только проверка гейтов. Неполные данные —
 * отказ с именованными элементами. */
export function computePatternType(report: unknown): PatternVerdict {
  let value: unknown = report;
  if (typeof value === "string") {
    try {
      value = JSON.parse(value);
    } catch {
      return refusal("отчёт валидатора не является валидным JSON");
    }
  }
  const obj = asObject(value);
  if (!obj) {
    return refusal("значение сигнала не объект");
  }
  const surface_proven = SURFACE_ELEMENTS.every((e) => proven(obj, e));
  const unproven = GATES.filter((e) => !proven(obj, e));
  const refuse = (reason: string, vector: string): PatternVerdict => ({
    pattern_found: false,
    surface_proven,
    reason,
    pattern_vector: vector,
    core_type: NONE,
    floor1_type: NONE,
    floor0_type: NONE,
    case_report: buildCaseReport(obj, NONE, NONE, NONE),
  });
  const vectorRaw = typeof obj["pattern_vector"] === "string"
    ? obj["pattern_vector"].trim()
    : "";
  if (vectorRaw === "") {
    return refuse("pattern_vector отсутствует или не строка", NONE);
  }
  if (vectorRaw === NONE) {
    return refuse(
      unproven.length === 0
        ? "все факты подтверждены, но валидатор не выбрал вектор (pattern_vector='none') — нужен добор данных"
        : `вектор не выбран (pattern_vector='none'): не подтверждены самим пользователем: ${unproven.join(", ")} — нужен добор данных`,
      NONE,
    );
  }
  const vector = PATTERN_VECTORS[vectorRaw];
  if (vector === undefined) {
    return refuse(
      `мусор вместо вектора: "${vectorRaw}" — допустимы 'none' и 9 векторов каталога`,
      vectorRaw,
    );
  }
  if (unproven.length > 0) {
    return refuse(
      `вектор "${vectorRaw}" не принимается: не подтверждены самим пользователем: ${unproven.join(", ")}`,
      vectorRaw,
    );
  }
  const [core, floor1, floor0] = vector;
  return {
    pattern_found: true,
    surface_proven,
    reason: `сквозной вектор принят: все гейты подтверждены парами eN/eN_text (${vectorRaw})`,
    pattern_vector: vectorRaw,
    core_type: String(core),
    floor1_type: String(floor1),
    floor0_type: String(floor0),
    case_report: buildCaseReport(obj, String(core), String(floor1), String(floor0)),
  };
}

/** refusal ДО объявления — вынесен наверх hoisting'ом function declaration. */
function refusal(reason: string): PatternVerdict {
  return {
    pattern_found: false,
    surface_proven: false,
    reason,
    pattern_vector: NONE,
    core_type: NONE,
    floor1_type: NONE,
    floor0_type: NONE,
    case_report: {
      target: "",
      core_type: NONE,
      floor1_type: NONE,
      floor0_type: NONE,
      e1: false,
      e2: false,
      e3: false,
      e4: false,
      e5: false,
      e6: false,
      e7: false,
      e8: false,
      e9: false,
    },
  };
}

if (import.meta.main) {
  try {
    const raw = Deno.args[0];
    if (raw === undefined || raw.trim() === "") {
      throw new Error("пустой вход: ожидался JSON-объект с полем report");
    }
    const args = asObject(JSON.parse(raw));
    if (!args) {
      throw new Error("вход обязан быть JSON-объектом с полем report");
    }
    if (!("report" in args)) {
      throw new Error("поле report обязательно");
    }
    console.log(JSON.stringify(computePatternType(args["report"])));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    Deno.exit(1);
  }
}
