/**
 * draw_card_numbers — RNG-тул команды «Таролог».
 *
 * Вход: Deno.args[0] = JSON {"count": "3"} (значения приходят СТРОКАМИ от движка).
 * Выход: ТОЛЬКО JSON в stdout:
 *   {"count": N, "numbers": [N уникальных 0..77], "orientations": ["upright"|"reversed", ...]}
 *
 * Ориентация каждой карты — часть случайного расклада (традиция RWS): её решает
 * этот тул, а не LLM. Файлов не читает (переносимая механика).
 * Любая невалидность входа — сообщение в stderr и exit(1): движок превращает
 * ненулевой exit в честную ошибку ноды.
 */

interface DrawInput {
  count?: unknown;
}

function parseInput(raw: string | undefined): DrawInput {
  if (raw === undefined || raw.trim() === "") {
    throw new Error("пустой вход: ожидался JSON-объект с полем count");
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error(`вход не является валидным JSON: ${raw.slice(0, 100)}`);
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("вход обязан быть JSON-объектом с полем count");
  }
  return parsed as DrawInput;
}

function normalizeCount(value: unknown): number {
  // Значения из args движка приходят строками — приводим через Number.
  const n = typeof value === "number" ? value : Number(String(value ?? ""));
  if (!Number.isInteger(n)) {
    throw new Error(`count должен быть целым числом, получено: ${String(value)}`);
  }
  if (n < 1 || n > 3) {
    throw new Error(`count должен быть в диапазоне 1..3, получено: ${n}`);
  }
  return n;
}

function drawUniqueNumbers(total: number, count: number): number[] {
  const pool = Array.from({ length: total }, (_, i) => i);
  // Fisher-Yates
  for (let i = pool.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [pool[i], pool[j]] = [pool[j], pool[i]];
  }
  return pool.slice(0, count);
}

function drawOrientations(count: number): string[] {
  // Ориентация — часть RNG: перевёрнутые карты — традиция колоды RWS.
  return Array.from({ length: count }, () =>
    Math.random() < 0.5 ? "upright" : "reversed",
  );
}

function main(): void {
  try {
    const input = parseInput(Deno.args[0]);
    const count = normalizeCount(input.count);
    const numbers = drawUniqueNumbers(78, count);
    const orientations = drawOrientations(count);
    const result = { count, numbers, orientations };
    console.log(JSON.stringify(result));
  } catch (err) {
    console.error(`draw_card_numbers: ${err instanceof Error ? err.message : String(err)}`);
    Deno.exit(1);
  }
}

main();
