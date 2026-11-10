/**
 * import_neurosis_case — тул ноды `import_cases` графа «Психотерапевт»:
 * распаковка прикреплённых юзером JSON-кейсов (экспорт save_neurosis_case)
 * в контекст графа.
 *
 * Детерминированная распаковка — код, а не решение LLM (§00.2): нода
 * custom_tool_call отдаёт тулу каталог `{{ json_files }}` (движок собирает
 * его из вложений текущего хода и истории сессии), тул читает файлы
 * (allow-read их уже покрывает) и возвращает блоки кейса для инжекта
 * агентам (decomposer / validator / synthesizer).
 *
 * Политика отказа (решение владельца 10.11.2026):
 * - вложений нет вообще → НЕ ошибка: {imported_count: 0} (нормальный путь
 *   большинства ходов);
 * - прикреплённый .json НЕ кейса или бит внутри → честная ошибка тула
 *   (exit 1 → ошибка ноды, workflow останавливается, текст уходит в чат).
 */

/** Кейс в каноническом порядке полей (эталон — CaseJson save_neurosis_case). */
export interface CaseJson {
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

/** Элемент каталога `{{ json_files }}` (строит RequestMedia::json_files_catalog). */
interface FileRef {
  file_name: string;
  file_path: string;
  origin: string;
}

/** Импортированный кейс с метаданными источника. */
export interface ImportedCase {
  file_name: string;
  origin: string;
  case: CaseJson;
}

const E_KEYS: readonly string[] = ["e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8", "e9"];

function fail(message: string): never {
  console.error(`import_neurosis_case: ${message}`);
  Deno.exit(1);
}

function asObject(value: unknown): Record<string, unknown> | undefined {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  return value as Record<string, unknown>;
}

/** Цифра типа 1..9 из числа (в файле кейса этажи — числа, не строки). */
function parseTypeDigit(raw: unknown, label: string, fileName: string): number {
  if (typeof raw !== "number" || !Number.isInteger(raw) || raw < 1 || raw > 9) {
    fail(
      `${fileName}: ${label} обязан быть целым числом 1..9, получено ${JSON.stringify(raw)}`,
    );
  }
  return raw;
}

/** e-поле кейса: непустой вердикт-текст или литерал false. */
function parseElement(
  raw: unknown,
  key: string,
  fileName: string,
): string | false {
  if (raw === false) return false;
  if (typeof raw === "string") {
    if (raw.trim() === "") {
      fail(`${fileName}: ${key} — пустая строка вместо вердикт-текста или false`);
    }
    return raw;
  }
  fail(
    `${fileName}: ${key} обязан быть вердикт-текстом или false, получено ${JSON.stringify(raw)}`,
  );
}

/**
 * Строгая валидация формы кейса. Несоответствие = отказ (fail): прикреплённый
 * .json обязан быть кейсом. Поля пересобираются в каноническом порядке
 * (target, core, f1, f0, e1..e9) — вывод не зависит от порядка в файле.
 */
export function parseCaseJson(raw: unknown, fileName: string): CaseJson {
  const obj = asObject(raw) ??
    fail(`${fileName}: ожидался JSON-объект кейса, получено ${JSON.stringify(raw).slice(0, 200)}`);

  const rawTarget = typeof obj["target"] === "string" ? obj["target"].trim() : "";
  if (rawTarget === "") {
    fail(`${fileName}: target обязан быть непустой строкой (мишень разбора)`);
  }

  const elements = Object.fromEntries(
    E_KEYS.map((key) => [key, parseElement(obj[key], key, fileName)]),
  ) as Record<string, string | false>;

  return {
    target: rawTarget,
    core_type: parseTypeDigit(obj["core_type"], "core_type", fileName),
    floor1_type: parseTypeDigit(obj["floor1_type"], "floor1_type", fileName),
    floor0_type: parseTypeDigit(obj["floor0_type"], "floor0_type", fileName),
    ...elements,
  } as CaseJson;
}

/** Блок одного кейса для инжекта в task-шаблоны агентов. */
function caseBlock(imported: ImportedCase): string {
  const header = `=== ИМПОРТИРОВАННЫЙ КЕЙС (из файла "${imported.file_name}") ===`;
  const body = JSON.stringify(imported.case, null, 2);
  return `${header}\n\`\`\`neurosis_case\n${body}\n\`\`\``;
}

/** Текст всех импортированных кейсов (пустая строка — импорта не было). */
export function buildImportedCasesText(cases: ImportedCase[]): string {
  return cases.map(caseBlock).join("\n\n");
}

/** Разбор каталога вложений: строка из args ("" = вложений нет). */
function parseCatalog(filesArg: string): FileRef[] {
  const text = filesArg.trim();
  if (text === "") return [];
  const parsed: unknown = JSON.parse(text);
  if (!Array.isArray(parsed)) {
    fail(`каталог вложений обязан быть JSON-массивом, получено ${text.slice(0, 200)}`);
  }
  return parsed.map((entry, index) => {
    const obj = asObject(entry) ??
      fail(`каталог[${index}]: ожидался объект {file_name, file_path, origin}`);
    const path = typeof obj["file_path"] === "string" ? obj["file_path"].trim() : "";
    if (path === "") {
      fail(`каталог[${index}]: file_path обязана быть непустой строкой`);
    }
    return {
      file_name: typeof obj["file_name"] === "string" ? obj["file_name"] : path,
      file_path: path,
      origin: typeof obj["origin"] === "string" ? obj["origin"] : "",
    };
  });
}

if (import.meta.main) {
  const raw = Deno.args[0];
  if (raw === undefined || raw.trim() === "") {
    fail("пустой вход: ожидался JSON-объект {files} (каталог {{ json_files }} из args ноды)");
  }
  let args: Record<string, unknown>;
  try {
    args = asObject(JSON.parse(raw)) ??
      fail("вход обязан быть JSON-объектом {files}");
  } catch (error) {
    fail(`вход — не валидный JSON: ${error instanceof Error ? error.message : String(error)}`);
  }
  const filesArg = typeof args["files"] === "string" ? args["files"] : "";
  const refs = parseCatalog(filesArg);

  const imported: ImportedCase[] = [];
  for (const ref of refs) {
    let text: string;
    try {
      text = await Deno.readTextFile(ref.file_path);
    } catch (error) {
      fail(
        `${ref.file_name}: файл не прочитан (${ref.file_path}): ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch (error) {
      fail(
        `${ref.file_name}: содержимое — не валидный JSON: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    }
    const caseJson = parseCaseJson(parsed, ref.file_name);
    imported.push({ file_name: ref.file_name, origin: ref.origin, case: caseJson });
    console.error(`import: ${ref.file_name} → ok (${imported.length})`);
  }

  const output = {
    imported_count: imported.length,
    imported_cases: imported,
    imported_cases_text: buildImportedCasesText(imported),
  };
  console.log(JSON.stringify(output));
}
