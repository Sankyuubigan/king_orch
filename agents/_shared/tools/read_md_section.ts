/**
 * read_md_section — ОБЩИЙ тул команд агентов: читает секцию markdown-файла
 * по заголовку. Живёт в `agents/_shared/tools/` — вне бизнес-логики команд,
 * чтобы любую команда могла дёрнуть его из графа нодой `custom_tool_call`
 * (tool_file: ../../_shared/tools/read_md_section.ts).
 *
 * Вход: Deno.args[0] = JSON (значения приходят СТРОКАМИ от движка):
 *   {"file": "<путь от корня agents/>", "heading": "## 2"}
 *
 * Выход: ТОЛЬКО JSON в stdout:
 *   {"file": "...", "heading": "...", "content": "<текст секции>"}
 *
 * Секция — от строки заголовка до следующего заголовка H1/H2 или конца файла.
 * Корень agents/ выводится из import.meta.url, НЕ из CWD процесса: у one-shot
 * Deno он не обязан совпадать с папкой приложения.
 *
 * Честные отказы (stderr + exit(1), движок превращает их в ошибку ноды):
 * не-JSON вход, файл вне agents/, файл не найден, секция не найдена,
 * секция пуста. Никаких нечётких фолбэков: тихо отдать «похожую» секцию —
 * значит отдать агенту лечение не по его паттерну.
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

interface ReadInput {
  file?: unknown;
  heading?: unknown;
}

function parseInput(raw: string | undefined): ReadInput {
  if (raw === undefined || raw.trim() === "") {
    throw new Error("пустой вход: ожидался JSON-объект с полями file и heading");
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error(`вход не является валидным JSON: ${raw.slice(0, 100)}`);
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("вход обязан быть JSON-объектом с полями file и heading");
  }
  return parsed as ReadInput;
}

function requireString(value: unknown, field: string): string {
  const s = typeof value === "string" ? value : String(value ?? "");
  const trimmed = s.trim();
  if (trimmed === "") {
    throw new Error(`поле ${field} обязательно и не может быть пустым`);
  }
  return trimmed;
}

/** Корень agents/: этот файл лежит в agents/_shared/tools/. */
const AGENTS_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);

/** Путь внутри agents/, резолв от корня (не от CWD). Выход за agents/ — отказ. */
function resolveAgentsFile(rel: string): string {
  const target = path.resolve(AGENTS_ROOT, rel);
  if (target !== AGENTS_ROOT && !target.startsWith(AGENTS_ROOT + path.sep)) {
    throw new Error(`файл '${rel}' выходит за пределы agents/: ${target}`);
  }
  return target;
}

/**
 * Текст секции: от строки заголовка до следующего заголовка H1/H2 или EOF.
 * Regex заголовка — как в mcp_servers/markdown_section_reader.ts: точный
 * префикс + пробел/конец строки (без \b — не матчит кириллицу).
 */
function readSection(content: string, heading: string): string {
  const escapedHeading = heading.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const headingRe = new RegExp(`^${escapedHeading}(?=\\s|$).*$`, "m");
  const headingMatch = content.match(headingRe);
  if (!headingMatch) {
    throw new Error(`секция с заголовком "${heading}" не найдена`);
  }
  const startIdx = headingMatch.index;
  const afterHeading = startIdx + headingMatch[0].length;
  const nextRe = /^#{1,2}[ \t]/m;
  const nextMatch = content.slice(afterHeading).match(nextRe);
  const endIdx = nextMatch ? afterHeading + nextMatch.index : content.length;
  return content.slice(startIdx, endIdx).trim();
}

function main(): void {
  try {
    const input = parseInput(Deno.args[0]);
    const rel = requireString(input.file, "file");
    const heading = requireString(input.heading, "heading");

    const target = resolveAgentsFile(rel);
    if (!fs.existsSync(target)) {
      throw new Error(`файл не найден: ${target}`);
    }
    const content = fs.readFileSync(target, "utf8");
    const section = readSection(content, heading);
    if (section === "") {
      throw new Error(`секция "${heading}" в файле ${target} пуста`);
    }

    console.log(JSON.stringify({ file: rel, heading, content: section }));
  } catch (err) {
    console.error(`read_md_section: ${err instanceof Error ? err.message : String(err)}`);
    Deno.exit(1);
  }
}

main();
