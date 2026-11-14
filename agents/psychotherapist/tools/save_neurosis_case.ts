/**
 * save_neurosis_case — тул команды «Психотерапевт»: сохранение кейса юзера.
 *
 * Заменяет агента case_packer (удалён 05.11.2026 — лишний LLM-вызов для
 * механической перекомпоновки). 13.11.2026 (план спеца): вход — ГОТОВЫЙ
 * чистый кейс (`case_report` ноды compute_pattern), а не сырой сигнал:
 * компоновка уже детерминирована кодом compute_pattern; здесь только
 * валидация формы, имя файла и never-overwrite запись. Ни SAFETY-таблицы,
 * ни выравнивания этажей, ни санитизации текстов — их быть не должно
 * (заглушки/химеры отсекаются upstream грамматикой и гейтами).
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

const E_KEYS: readonly string[] = [
  "e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8", "e9",
];
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

/** Цифра типа 1..9 из строки кейс-отчёта (этажи приходят строками "1".."9"). */
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

/** Проверка формы чистого кейса и пересборка в каноническом порядке полей
 * (target, core, f1, f0, e1..e9). Никаких преобразований контента: тексты
 * eN — уже житейские формулировки пользователя от compute_pattern. */
export function buildCaseJson(caseReport: Record<string, unknown>): CaseJson {
  const target = typeof caseReport["target"] === "string"
    ? caseReport["target"].trim()
    : "";
  if (target === "") {
    fail("target: обязана быть непустая строка (мишень разбора)");
  }
  const coreType = parseTypeDigit(caseReport["core_type"], "core_type");
  const floor1 = parseTypeDigit(caseReport["floor1_type"], "floor1_type");
  const floor0 = parseTypeDigit(caseReport["floor0_type"], "floor0_type");
  const elements = E_KEYS.map((key) => {
    const raw = caseReport[key];
    if (raw === false) return [key, false] as const;
    if (typeof raw === "string" && raw.trim() !== "") {
      return [key, raw] as const;
    }
    fail(`${key}: ожидался текст пользователя или false, получено ${JSON.stringify(raw)}`);
  });
  return {
    target,
    core_type: coreType,
    floor1_type: floor1,
    floor0_type: floor0,
    ...Object.fromEntries(elements),
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
    fail("пустой вход: ожидался JSON-объект {case}");
  }
  const args = asObject(parseJsonOr(raw, "вход")) ??
    fail("вход обязан быть JSON-объектом {case}");
  if (!("case" in args)) {
    fail("поле case обязательно (чистый кейс-отчёт ноды compute_pattern)");
  }

  // case приходит из {{ nodes.compute_pattern.output.case_report }}; строка
  // тоже принимается (облачная ветка может отдать её сериализованной).
  let caseReport: unknown = args["case"];
  if (typeof caseReport === "string") {
    caseReport = parseJsonOr(caseReport, "case");
  }
  const caseObj = asObject(caseReport) ??
    fail("case: ожидался JSON-объект чистого кейс-отчёта");

  const caseJson = buildCaseJson(caseObj);
  const caseJsonText = JSON.stringify(caseJson, null, 2);
  const baseName = caseFileName(caseJson.core_type, caseJson.target);
  const { savedPath, fileName, isNew } = await saveNeverOverwrite(dir, baseName, caseJsonText);

  console.log(JSON.stringify({
    saved_path: savedPath,
    file_name: fileName,
    is_new: isNew,
    case_json: caseJson,
    case_json_text: caseJsonText,
  }));
}
