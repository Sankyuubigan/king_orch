/**
 * read_pattern_sections — тул команды «Психотерапевт»: читает из базы знаний
 * ТРИ лечебных паттерна — по одному на этаж невроза — и подписывает их для
 * нейрохиллера (какой этаж каким паттерном лечится).
 *
 * Прецедент — `agents/_shared/tools/read_md_section.ts` (логика чтения секции
 * скопирована, НЕ импортирована: тот тул вызывает main() при импорте и утащил
 * бы one-shot процесс в exit(1)). Секции берутся из
 * `psychotherapist/database/inner_9_patterns_by_gifts.md` (файл приходит
 * аргументом — граф знает раскладку базы), где `## N` = больной тип N + его
 * лечение (Дыра 3, Идол 9, Дар лечебного типа по Безопасности, механика,
 * формула сдвига).
 *
 * Соответствие этаж → паттерн (лечение e4/e8/e9, решение владельца 06.11.2026):
 *   Этаж 0  (элементы 1, 4, 7) → floor0_type из сигнала валидатора
 *   Этаж -1 (элементы 2, 5, 8) → floor1_type из сигнала валидатора
 *   Этаж -2 (ядро, элементы 3, 6, 9) → core (floor2_type, вычислен нодой
 *                               compute_pattern по SAFETY-сцепке)
 *
 * Вход: Deno.args[0] = JSON (значения приходят СТРОКАМИ от движка):
 *   {
 *     "report": "<JSON-строка сигнала validator_report или сам объект>",
 *     "core":   "<цифра 1..9 из {{ nodes.compute_pattern.output.floor2_type }}>"
 *   }
 *
 * Выход: ТОЛЬКО JSON в stdout:
 *   {"content": "<трёхэтажный блок с подписями>", "floor0": "…", "floor1": "…", "core": "…"}
 *
 * Честные отказы (stderr + exit(1), движок превращает их в ошибку ноды):
 * не-JSON вход, report не объект, этажи/ядро не цифры 1..9, файл вне agents/,
 * файл не найден, секция не найдена. Никаких нечётких фолбэков: тихо отдать
 * «похожую» секцию — значит лечить не тем паттерном.
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

interface ReadInput {
  file?: unknown;
  report?: unknown;
  core?: unknown;
}

function fail(message: string): never {
  console.error(`read_pattern_sections: ${message}`);
  Deno.exit(1);
}

function parseInput(raw: string | undefined): ReadInput {
  if (raw === undefined || raw.trim() === "") {
    fail("пустой вход: ожидался JSON-объект с полями file, report, core");
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    fail(`вход не является валидным JSON: ${raw.slice(0, 100)}`);
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    fail("вход обязан быть JSON-объектом с полями file, report, core");
  }
  return parsed as ReadInput;
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

/** Корень agents/: этот файл лежит в agents/psychotherapist/tools/. */
const AGENTS_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);

/** Путь внутри agents/, резолв от корня (не от CWD). Выход за agents/ — отказ. */
function resolveAgentsFile(rel: string): string {
  const target = path.resolve(AGENTS_ROOT, rel);
  if (target !== AGENTS_ROOT && !target.startsWith(AGENTS_ROOT + path.sep)) {
    fail(`файл '${rel}' выходит за пределы agents/: ${target}`);
  }
  return target;
}

/**
 * Текст секции: от строки заголовка до следующего заголовка H1/H2 или EOF.
 * Regex заголовка — как в read_md_section.ts / mcp_servers/
 * markdown_section_reader.ts: точный префикс + пробел/конец строки.
 */
function readSection(content: string, heading: string): string {
  const escapedHeading = heading.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const headingRe = new RegExp(`^${escapedHeading}(?=\\s|$).*$`, "m");
  const headingMatch = content.match(headingRe);
  if (!headingMatch) {
    fail(`секция с заголовком "${heading}" не найдена`);
  }
  const startIdx = headingMatch.index;
  const afterHeading = startIdx + headingMatch[0].length;
  const nextRe = /^#{1,2}[ \t]/m;
  const nextMatch = content.slice(afterHeading).match(nextRe);
  const endIdx = nextMatch ? afterHeading + nextMatch.index : content.length;
  return content.slice(startIdx, endIdx).trim();
}

/** Подписанный блок одного этажа. */
function floorBlock(
  label: string,
  pattern: number,
  content: string,
): string {
  const section = readSection(content, `## ${pattern}`);
  return `=== ${label} — лечебный паттерн: Тип ${pattern} ===\n${section}`;
}

function main(): void {
  const input = parseInput(Deno.args[0]);

  const rel = typeof input.file === "string" && input.file.trim() !== ""
    ? input.file.trim()
    : fail("поле file обязательно (путь к базе от корня agents/)");
  const core = parseTypeDigit(input.core, "core");
  if (!("report" in input)) {
    fail("поле report обязательно (сигнал validator_report)");
  }
  // report приходит JSON-строкой из {{ signals.validator_report }}; объект
  // тоже принимаем (облачная ветка может отдать разобранный объект).
  let report: unknown = input.report;
  if (typeof report === "string") {
    try {
      report = JSON.parse(report);
    } catch (error) {
      fail(
        `report — не валидный JSON: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    }
  }
  const reportObj = report !== null && typeof report === "object" &&
      !Array.isArray(report)
    ? report as Record<string, unknown>
    : fail("report: ожидался JSON-объект сигнала validator_report");
  const floor0 = parseTypeDigit(reportObj["floor0_type"], "floor0_type");
  const floor1 = parseTypeDigit(reportObj["floor1_type"], "floor1_type");

  const target = resolveAgentsFile(rel);
  if (!fs.existsSync(target)) {
    fail(`файл не найден: ${target}`);
  }
  const dbContent = fs.readFileSync(target, "utf8");

  const content = [
    floorBlock("ЭТАЖ 0 (элементы 1, 4, 7)", floor0, dbContent),
    floorBlock("ЭТАЖ -1 (элементы 2, 5, 8)", floor1, dbContent),
    floorBlock(
      "ЭТАЖ -2 (ЯДРО — Триада Травмы; элементы 3, 6, 9)",
      core,
      dbContent,
    ),
  ].join("\n\n");

  console.log(JSON.stringify({ content, floor0, floor1, core }));
}

if (import.meta.main) main();
