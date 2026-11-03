/**
 * compute_pattern_type — тул команды «Психотерапевт»: механическое вычисление
 * типа Триады Травмы (Этаж -2) из классификации этажей Валидатора.
 *
 * Живёт в папке команды (не в движке): движок остаётся агент-агностиком,
 * SAFETY-механика валидатора — бизнес-логика команды (§00.2). Вызывается из
 * графа нодой `custom_tool_call` (tool_file: ../tools/compute_pattern_type.ts),
 * прецедент — `agents/_shared/tools/read_md_section.ts`.
 *
 * Вход: Deno.args[0] = JSON (значения приходят СТРОКАМИ от движка):
 *   {"report": "<JSON-строка сигнала validator_report или сам объект>"}
 *
 * Выход: ТОЛЬКО JSON в stdout:
 *   {"pattern_type": "1".."9" | "none", "pattern_found": boolean, "reason": "<объяснение>"}
 *
 * Функция ТОТАЛЬНА по полю: успех — цифра 1..9; любой отказ (критический
 * элемент e3/e6/e8/e9 не подтверждён, этажи не классифицированы, сцепка не
 * сошлась, сигнал не объект) — явное `pattern_type = "none"` (презумпция
 * дефицита: нет типа лучше неверного). Гейт критических элементов {e3,e6,e8,e9}
 * живёт ЗДЕСЬ (04.11.2026: нода check_critical_elements удалена из графа — её
 * проверка e3 дублировала тул, остаток переехал следом; §00.2 — механический
 * гейт в тотальной функции, а не отдельной нодой). Маршрутизация графа
 * (check_pattern, type condition_check) читает производный булев
 * `pattern_found` — состояние, которое роутер способен прочитать: у
 * condition_check нет доступа к шине, а condition_router не читает output
 * тула (мёртвое условие 03.11.2026). Exit(1) — только за сломанную инвокацию
 * (не-JSON вход, нет поля report): это ошибка узла движка, а не доменный отказ.
 *
 * Разделение труда (§00.2): классификация «какой тип проявляется на этаже» —
 * эпистемия, её делает модель по эталонам базы; переход по таблице — табличная
 * операция, её делает код (прогоны 02.11.2026: модельному переходу доверять
 * нельзя — «Тип 1 → Безопасность → Тип 8» при SAFETY[1] = 7).
 */

/** SAFETY-переходы этажей (SSOT: `conversions_patterns.md`, таблица
 * «переходы типов»). Индекс = тип источника, значение = тип на один шаг
 * Безопасности вниз (Спуск причины: T0 → Безопасность → T-1 → T-2).
 * Нулевая ячейка служебная (типы 1..9).
 *
 * Внимание (разбор 02.11.2026): множества `{1,4,7}/{2,5,8}/{3,6,9}` в SSOT —
 * это множества НОМЕРОВ ЭЛЕМЕНТОВ по этажам (1,4,7 — соматика/Я-концепция/
 * страсть; 3,6,9 — Дыра/Триггер/Идол), а НЕ множества типов: у каждого типа
 * в `patterns_9_types_detailed.md` есть секции всех трёх этажей, тип 1..9
 * может проявиться на любом этаже. Проверка принадлежности типа «множеству
 * этажа» невозможна ни для одного реального сигнала (циклы SAFETY (1 7 5 8 2 4)
 * и (3 6 9) не пересекаются) — она бы отбраковала всё и заставляла модель
 * галлюцинировать сходимость (кейс «1 → Безопасность → 8»). Поэтому
 * механически проверяется только сходимость цепи. */
const SAFETY_TRANSITIONS: readonly number[] = [0, 7, 4, 6, 1, 8, 9, 5, 2, 3];

export interface PatternVerdict {
  /** Цифра типа 1..9 либо явное "none" (тип не определён → добор данных). */
  pattern_type: string;
  /** Производный булев (pattern_type != "none"): читаемое роутером состояние.
   * check_pattern (condition_check) не имеет доступа к шине и не умеет
   * сравнивать строки — только boolean (nodes.rs: condition_check as_bool). */
  pattern_found: boolean;
  /** Честное объяснение исхода (для логов и диагностики пайплайна). */
  reason: string;
}

const NONE = "none";

/** Критические элементы контура: тип Триады Травмы не объявляется, пока не
 * подтверждены все (SSOT — бывшая нода check_critical_elements, 04.11.2026
 * перенесена сюда). e3 проверяется отдельной причиной (ядро). */
const CRITICAL_ELEMENTS: readonly string[] = ["e6", "e8", "e9"];

function refusal(reason: string): PatternVerdict {
  return { pattern_type: NONE, pattern_found: false, reason };
}

function asObject(value: unknown): Record<string, unknown> | undefined {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  return value as Record<string, unknown>;
}

/** Парсит цифру типа: непустая строка из десятичных цифр, значение 1..9.
 * Возвращает undefined на мусоре/вне диапазона (виновен при отказе). */
function parseTypeDigit(raw: unknown): number | undefined {
  if (typeof raw !== "string") return undefined;
  const text = raw.trim();
  if (!/^\d+$/.test(text)) return undefined;
  const parsed = Number(text);
  return parsed >= 1 && parsed <= 9 ? parsed : undefined;
}

/** Механическая проверка сцепки этажей и вычисление типа ядра Триады Травмы
 * (`pattern_type`, Этаж -2). Принимает сигнал validator_report (объект либо
 * JSON-строка). Условия выдачи: критические элементы подтверждены
 * (`e3`, `e6`, `e8`, `e9 == true`), этажи классифицированы и SAFETY-сцепка
 * сходится (`SAFETY[T0] == T-1`). Тип 1..9 может проявиться на любом этаже
 * (см. док SAFETY_TRANSITIONS), принадлежность «множеству этажа» не проверяется. */
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
  const floor0 = typeof obj["floor0_type"] === "string" ? obj["floor0_type"].trim() : undefined;
  const floor1 = typeof obj["floor1_type"] === "string" ? obj["floor1_type"].trim() : undefined;
  if (floor0 === undefined || floor1 === undefined) {
    return refusal("этажи не классифицированы (floor0_type/floor1_type отсутствуют)");
  }
  if (obj["e3"] !== true) {
    return refusal("ядро не подтверждено (e3 != true) — тип Триады Травмы не объявляется");
  }
  for (const element of CRITICAL_ELEMENTS) {
    if (obj[element] !== true) {
      return refusal(
        `критический элемент ${element} не подтверждён — тип Триады Травмы не объявляется`,
      );
    }
  }
  const t0 = parseTypeDigit(floor0);
  if (t0 === undefined) {
    return refusal(`мусор вместо цифры Этажа 0: ${floor0}`);
  }
  const t1 = parseTypeDigit(floor1);
  if (t1 === undefined) {
    return refusal(`мусор вместо цифры Этажа -1: ${floor1}`);
  }
  if (SAFETY_TRANSITIONS[t0] !== t1) {
    return refusal(
      `SAFETY-сцепка этажей не сходится: Тип ${t0} -> Безопасность -> не Тип ${t1}`,
    );
  }
  return {
    pattern_type: String(SAFETY_TRANSITIONS[t1]),
    pattern_found: true,
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
