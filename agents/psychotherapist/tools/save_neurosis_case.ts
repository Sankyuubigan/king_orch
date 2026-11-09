/**
 * save_neurosis_case — тул команды «Психотерапевт»: сохранение кейса юзера.
 *
 * Заменяет агента case_packer (удалён 05.11.2026 — лишний LLM-вызов для
 * механической перекомпоновки): сборка плоского JSON из сигнала валидатора —
 * детерминированная операция кода, а не эпистемия модели (§00.2).
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

/** JSON.parse с честным отказом тула. */
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

/** Санитизация текста вердикта на случай, если LLM всё же вставила канцелярский
 * оборот 3-го лица («пользователь фиксирует…», «из контекста вытекает…») или
 * академический термин («онтологическая», «императив»). */
export function sanitizeVerdictText(text: string): string {
  let cleaned = text.trim();

  const metaPrefixes: readonly RegExp[] = [
    /^из\s+контекста\s+(?:вытекает|следует|считывается|видно)\s*(?:убеждение|требование|императив|правило|страх|боязнь|ощущение)?\s*(?:,\s*что|:\s*|\s+)/iu,
    /^(?:пользователь|клиент|человек)\s+(?:указывает\s+на\s+(?:конкретный\s+)?(?:пусковой\s+момент|триггер|стимул|ситуацию)\s*:\s*)/iu,
    /^(?:пользователь|клиент|человек)\s+(?:описывает\s+(?:хроническую\s+|повторяющуюся\s+)?эмоцию\s*:?\s*)/iu,
    /^(?:пользователь|клиент|человек)\s+(?:фиксирует|описывает|указывает|отмечает|испытывает|ощущает|чувствует|проявляет)\s*(?:,\s*что\s+|\s+)/iu,
    /^(?:пользователь|клиент|человек)\s+оценивает\s+себя\s+как\s+/iu,
  ];
  for (const re of metaPrefixes) {
    cleaned = cleaned.replace(re, "");
  }

  cleaned = cleaned
    .replace(/онтологическ(?:ая|ой|ую|ий|ого|ому|им|ом|ие|их)\s+/giu, "")
    .replace(/экзистенциальн(?:ая|ой|ую|ый|ого|ому|ым|ом|ые|ых)\s+/giu, "")
    .replace(/категорический\s+императив\s*:?\s*/giu, "жёсткое правило: ")
    .replace(/\bимператив\s*:?\s*/giu, "правило: ")
    .replace(/конкретный\s+пусковой\s+(?:момент|стимул)\s*:\s*/giu, "")
    .replace(/пусковой\s+(?:момент|стимул)\s*:\s*/giu, "")
    .replace(/\s+/g, " ")
    .trim();

  if (cleaned.length > 0) {
    cleaned = cleaned[0].toUpperCase() + cleaned.slice(1);
  }
  return cleaned;
}

/** e-поле сигнала: вердикт-текст (непустая строка) или false. */
function parseElement(raw: unknown, key: string): string | false {
  if (raw === false) return false;
  if (typeof raw === "string" && raw.trim() !== "") {
    if (/^false(?![\p{L}\p{N}])/iu.test(raw.trimStart())) return false;
    return sanitizeVerdictText(raw);
  }
  fail(`${key}: ожидался вердикт-текст или false, получено ${JSON.stringify(raw)}`);
}

/** Имя файла из target: `<core>тип - <slug>.json`. */
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

/** Канонический порядок полей — эталон: target, core, f1, f0, e1..e9. */
export function buildCaseJson(report: Record<string, unknown>, coreType: number): CaseJson {
  const rawTarget = typeof report["target"] === "string" ? report["target"].trim() : "";
  if (rawTarget === "") {
    fail("target: обязана быть непустая строка (мишень разбора)");
  }
  const target = sanitizeVerdictText(rawTarget);
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

/** Идемпотентный never-overwrite save: createNew:true + дедуп ревизий. */
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