/**
 * compute_pattern_type — тул команды «Психотерапевт»: механическая верификация
 * сквозной SAFETY-цепи всех трёх этажей (T0 -> T-1 -> T-2) и готовности
 * критических элементов Ядра.
 *
 * Вход: Deno.args[0] = JSON (значения приходят СТРОКАМИ от движка):
 *   {"report": "<JSON-строка сигнала validator_report или сам объект>"}
 *
 * Выход: ТОЛЬКО JSON в stdout:
 *   {"floor2_type": "1".."9" | "none", "pattern_found": boolean, "surface_proven": boolean, "reason": "<объяснение>"}
 */

const SAFETY_TRANSITIONS: readonly number[] = [0, 7, 4, 6, 1, 8, 9, 5, 2, 3];

export interface PatternVerdict {
  /** Цифра типа 1..9 либо явное "none" (тип не определён → добор данных). */
  floor2_type: string;
  /** Производный булев (floor2_type != "none"): читаемое роутером состояние. */
  pattern_found: boolean;
  /** Производный булев: ВСЕ элементы поверхностного контура {e1, e2, e6, e7} подтверждены. */
  surface_proven: boolean;
  /** Честное объяснение исхода (для логов и диагностики пайплайна). */
  reason: string;
}

const NONE = "none";

/** Элементы поверхностного контура для производного surface_proven:
 * сопротивление (e1), автоматизм (e2), триггер (e6), эмоция-повторитель (e7). */
const SURFACE_ELEMENTS: readonly string[] = ["e1", "e2", "e6", "e7"];

const FALSE_TEXT_RE = /^false(?![\p{L}\p{N}])/iu;

export function is_proven(value: unknown): boolean {
  if (value === true) return true;
  if (typeof value !== "string") return false;
  return value.trim() !== "" && !FALSE_TEXT_RE.test(value.trimStart());
}

/** Критические элементы контура: тип Триады Травмы не объявляется, пока не
 * подтверждены все (e3 проверяется отдельно первым). */
const CRITICAL_ELEMENTS: readonly string[] = ["e6", "e8", "e9"];

function refusal(reason: string): PatternVerdict {
  return {
    floor2_type: NONE,
    pattern_found: false,
    surface_proven: false,
    reason,
  };
}

function asObject(value: unknown): Record<string, unknown> | undefined {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  return value as Record<string, unknown>;
}

/** Парсит цифру типа: непустая строка из десятичных цифр, значение 1..9. */
function parseTypeDigit(raw: unknown): number | undefined {
  if (typeof raw !== "string") return undefined;
  const text = raw.trim();
  if (!/^\d+$/.test(text)) return undefined;
  const parsed = Number(text);
  return parsed >= 1 && parsed <= 9 ? parsed : undefined;
}

/** Механическая проверка сцепки всех трёх этажей (T0 -> T-1 -> T-2) и
 * подтверждённости критических элементов (`e3`, `e6`, `e8`, `e9`).
 * Если Валидатор передал `floor2_type`, проверяется двойное условие:
 * `SAFETY[T0] === T-1` и `SAFETY[T-1] === T-2`. */
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
  const surface_proven = SURFACE_ELEMENTS.every((element) => is_proven(obj[element]));
  const refuse = (reason: string): PatternVerdict => ({
    floor2_type: NONE,
    pattern_found: false,
    surface_proven,
    reason,
  });
  const floor0 = typeof obj["floor0_type"] === "string" ? obj["floor0_type"].trim() : undefined;
  const floor1 = typeof obj["floor1_type"] === "string" ? obj["floor1_type"].trim() : undefined;
  const floor2 = typeof obj["floor2_type"] === "string" ? obj["floor2_type"].trim() : undefined;
  if (floor0 === undefined || floor1 === undefined) {
    return refuse("этажи не классифицированы (floor0_type/floor1_type отсутствуют)");
  }
  if (!is_proven(obj["e3"])) {
    return refuse("ядро не подтверждено (e3 — false или пустой вердикт) — тип Триады Травмы не объявляется");
  }
  for (const element of CRITICAL_ELEMENTS) {
    if (!is_proven(obj[element])) {
      return refuse(
        `критический элемент ${element} не подтверждён — тип Триады Травмы не объявляется`,
      );
    }
  }
  const t0 = parseTypeDigit(floor0);
  if (t0 === undefined) {
    return refuse(`мусор вместо цифры Этажа 0: ${floor0}`);
  }
  const t1 = parseTypeDigit(floor1);
  if (t1 === undefined) {
    return refuse(`мусор вместо цифры Этажа -1: ${floor1}`);
  }
  if (SAFETY_TRANSITIONS[t0] !== t1) {
    return refuse(
      `SAFETY-сцепка этажей 0 и -1 не сходится: Тип ${t0} -> Безопасность -> Тип ${SAFETY_TRANSITIONS[t0]}, а не Тип ${t1}`,
    );
  }
  const expectedT2 = SAFETY_TRANSITIONS[t1];
  if (floor2 !== undefined && floor2 !== "") {
    const t2 = parseTypeDigit(floor2);
    if (t2 === undefined) {
      return refuse(`мусор вместо цифры Этажа -2: ${floor2}`);
    }
    if (t2 !== expectedT2) {
      return refuse(
        `SAFETY-сцепка этажей -1 и -2 (Ядро) не сходится: Тип ${t1} -> Безопасность -> Тип ${expectedT2}, а классифицирован Тип ${t2}`,
      );
    }
  }
  return {
    floor2_type: String(expectedT2),
    pattern_found: true,
    surface_proven,
    reason: "SAFETY-сцепка этажей сходится",
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