/**
 * save_neurosis_case — тул команды «Психотерапевт»: сохранение кейса юзера.
 *
 * Заменяет агента case_packer (удалён 05.11.2026 — лишний LLM-вызов для
 * механической перекомпоновки): сборка плоского JSON из сигнала валидатора —
 * детерминированная операция кода, а не эпистемия модели (§00.2).
 *
 * Вход: Deno.args[0] = JSON (значения приходят СТРОКАМИ от движка):
 *   {
 *     "report": "<JSON-строка сигнала validator_report: e1..e9 (строка|false), floor0_type/floor1_type ("1".."9"), target>",
 *     "core_type": "<цифра 1..9 из {{ nodes.compute_pattern.output.floor2_type }}>"
 *   }
 *
 * Выход (stdout — ТОЛЬКО JSON-объект):
 *   {
 *     "saved_path": "<абсолютный путь файла>",
 *     "file_name": "<имя файла>",
 *     "is_new": true|false,          // false — идентичный кейс уже лежит (skip)
 *     "case_json": {…13 полей…},     // распарсенный кейс (для шаблонов графа)
 *     "case_json_text": "{\n  …}"    // pretty-текст (фенс ```neurosis_case в чат)
 *   }
 *
 * Формат файла — эталон владельца «3тип - ….json»: РОВНО 13 полей в порядке
 * target, core_type, floor1_type, floor0_type, e1..e9; типы этажей и ядро —
 * ЧИСЛА (сигнал отдаёт строками "1".."9" — конвертация здесь); e-поля —
 * вердикт-текст или false; target — строка. Pretty: одно поле на строку.
 *
 * Хранение: `<KO_TOOL_WRITE_DIR>/психо кейсы/` (env задаёт раннер движка,
 * прод = %APPDATA%\com.kingorch.app — NSIS-обновления appdata не трогают).
 * Имя файла: `<core>тип - <slug(target)>.json`.
 *
 * Никогда не перезаписывать кейс юзера (данные владельца): идемпотентный save
 * через writeTextFile{createNew:true} — если файл уже существует с ИДЕНТИЧНЫМ
 * содержимым → skip (is_new:false); если содержимое изменилось (юзер уточнил
 * данные) → новый файл `<stem> (2..99).json` (история ревизий); все 99 заняты
 * → честная ошибка.
 *
 * Тул НЕ повторяет SAFETY-сцепку и гейт критических элементов — это compute_
 * floor2_type; нода графа вызывается только на ветке pattern_found=true.
 * Exit(1) — за сломанную инвокацию/невалидную форму (env нет, args не JSON,
 * report не объект, target пуст, мусор в цифрах, e-поле отсутствует/не та
 * форма): форма выхода нарушена — тихий пропуск воскресил бы «кейс не
 * сохраняется» без диагностики.
 */

/** Кейс в каноническом порядке полей (эталон владельца). */
interface CaseJson {
  target: string;
  core_type: number;
  floor1_type: number;
  floor0_type: number;
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

const E_KEYS: readonly string[] = ["e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8", "e9"];
/** Суффиксы ревизий: `<stem> (2).json` … `<stem> (99).json`. */
const MAX_REVISIONS = 99;

function fail(message: string): never {
  console.error(`save_neurosis_case: ${message}`);
  Deno.exit(1);
}

function asObject(value: unknown): Record<string, unknown> | undefined {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  return value as Record<string, unknown>;
}

/** JSON.parse с честным отказом тула (uncaught SyntaxError не объясняет форму). */
function parseJsonOr(raw: string, what: string): unknown {
  try {
    return JSON.parse(raw);
  } catch (error) {
    fail(`${what} — не валидный JSON: ${error instanceof Error ? error.message : String(error)}`);
  }
}

/** Цифра типа 1..9 из строки сигнала (этажи приходят строками "1".."9"). */
function parseTypeDigit(raw: unknown, label: string): number {
  if (typeof raw !== "string") {
    fail(`${label} обязан быть строкой-цифрой 1..9, получено ${JSON.stringify(raw)}`);
  }
  const text = raw.trim();
  if (!/^\d+$/.test(text)) {
    fail(`${label}: мусор вместо цифры — ${JSON.stringify(raw)}`);
  }
  const parsed = Number(text);
  if (parsed < 1 || parsed > 9) {
    fail(`${label}: ${parsed} вне диапазона 1..9`);
  }
  return parsed;
}

/** e-поле сигнала: вердикт-текст (непустая строка) или false. Числа, null,
 * пустые/пустые-после-trim строки — форма контракта нарушена.
 * «false (…)»-строка — склеенная форма отказа, а не вердикт: канонизируется
 * в false (боевой путь нормализует сигнал на границе сохранения —
 * signals.rs::normalize_signal_value; это защита для облака/старых сессий),
 * в кейс юзера текст отказа не попадает. */
function parseElement(raw: unknown, key: string): string | false {
  if (raw === false) return false;
  if (typeof raw === "string" && raw.trim() !== "") {
    if (/^false(?![\p{L}\p{N}])/iu.test(raw.trimStart())) return false;
    return raw;
  }
  fail(`${key}: ожидался вердикт-текст или false, получено ${JSON.stringify(raw)}`);
}

/** Имя файла из target: `<core>тип - <slug>.json` (пример владельца
 * «3тип - Спазм лба и висков + ….json»). Slug: вырезаем запрещённые в
 * Windows символы и control-символы, сжимаем пробелы, обрезаем до 80,
 * снимаем хвостовые точки/пробелы, защищаемся от зарезервированных имён. */
export function caseFileName(coreType: number, target: string): string {
  const slug = target
    .replace(/[\\/:*?"<>|\u0000-\u001f\u007f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 80)
    .trim()
    .replace(/[. ]+$/, "")
    .trim();
  const safe = slug === "" || /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i.test(slug)
    ? "Без названия"
    : slug;
  return `${coreType}тип - ${safe}.json`;
}

/** Канонический порядок полей — эталон: target, core, f1, f0, e1..e9.
 * Вставка в этом порядке фиксирует и порядок в файле (JSON.stringify идёт
 * по порядку вставки строковых ключей). */
export function buildCaseJson(report: Record<string, unknown>, coreType: number): CaseJson {
  const target = typeof report["target"] === "string" ? report["target"].trim() : "";
  if (target === "") {
    fail("target: обязана быть непустая строка (мишень разбора)");
  }
  const floor1 = parseTypeDigit(report["floor1_type"], "floor1_type");
  const floor0 = parseTypeDigit(report["floor0_type"], "floor0_type");
  const elements = Object.fromEntries(
    E_KEYS.map((key) => [key, parseElement(report[key], key)]),
  ) as Record<string, string | false>;
  return {
    target,
    core_type: coreType,
    floor1_type: floor1,
    floor0_type: floor0,
    ...elements,
  } as CaseJson;
}

/** Идемпотентный never-overwrite save: createNew:true + дедуп ревизий.
 * Идентичное содержимое → skip (is_new:false, путь существующего файла);
 * изменённое → новая ревизия ` (N)`; все заняты → честная ошибка. */
export async function saveNeverOverwrite(
  dir: string,
  fileName: string,
  text: string,
): Promise<{ savedPath: string; fileName: string; isNew: boolean }> {
  await Deno.mkdir(dir, { recursive: true });
  const stem = fileName.slice(0, -".json".length);
  for (let n = 0; n <= MAX_REVISIONS; n++) {
    const name = n === 0 ? fileName : `${stem} (${n}).json`;
    const path = `${dir}${Deno.build.os === "windows" ? "\\" : "/"}${name}`;
    try {
      await Deno.writeTextFile(path, text, { createNew: true });
      return { savedPath: path, fileName: name, isNew: true };
    } catch (error) {
      if (!(error instanceof Deno.errors.AlreadyExists)) {
        fail(`запись ${path} не удалась: ${error instanceof Error ? error.message : String(error)}`);
      }
      const existing = await Deno.readTextFile(path);
      if (existing === text) {
        return { savedPath: path, fileName: name, isNew: false };
      }
      // Содержимое изменилось — пробуем следующую ревизию.
    }
  }
  fail(`все ${MAX_REVISIONS} ревизий заняты другим содержимым: ${fileName}`);
}

if (import.meta.main) {
  const dir = Deno.env.get("KO_TOOL_WRITE_DIR");
  if (dir === undefined || dir.trim() === "") {
    fail("env KO_TOOL_WRITE_DIR не задан — раннер движка обязан передать каталог записи");
  }
  const raw = Deno.args[0];
  if (raw === undefined || raw.trim() === "") {
    fail("пустой вход: ожидался JSON-объект {report, core_type}");
  }
  const args = asObject(parseJsonOr(raw, "вход")) ??
    fail("вход обязан быть JSON-объектом {report, core_type}");
  if (!("report" in args) || !("core_type" in args)) {
    fail("поля report и core_type обязательны");
  }

  // report приходит JSON-строкой из {{ signals.validator_report }}; объект
  // тоже принимаем (облачная ветка может отдать разобранный объект).
  let report: unknown = args["report"];
  if (typeof report === "string") {
    report = parseJsonOr(report, "report");
  }
  const reportObj = asObject(report) ??
    fail("report: ожидался JSON-объект сигнала validator_report");
  const coreType = parseTypeDigit(args["core_type"], "core_type");

  const caseJson = buildCaseJson(reportObj, coreType);
  const caseJsonText = JSON.stringify(caseJson, null, 2);
  const baseName = caseFileName(coreType, caseJson.target);
  const { savedPath, fileName, isNew } = await saveNeverOverwrite(dir, baseName, caseJsonText);

  console.log(JSON.stringify({
    saved_path: savedPath,
    file_name: fileName,
    is_new: isNew,
    case_json: caseJson,
    case_json_text: caseJsonText,
  }));
}
