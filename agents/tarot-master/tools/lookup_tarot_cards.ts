/**
 * lookup_tarot_cards — лукап-тул команды «Таролог».
 *
 * Вход: Deno.args[0] = JSON {"numbers": "[3,21,57]", "orientations": "[\"upright\",...]"}.
 * ВАЖНО: движок резолвит не-строку как JSON-массив, сериализованный в строку,
 * поэтому оба поля нормализуются одинаково: принимается и строка-JSON-массив
 * (JSON.parse), и настоящий массив.
 *
 * Читает ../database/cards.json относительно файла тула (путь от import.meta.url,
 * абсолютные пути не хардкодятся). cards.json — SSOT карт, список дублировать запрещено.
 *
 * Выход ТОЛЬКО JSON в stdout:
 *   {"count": N, "positions": [...], "cards": [{...поля карты..., "orientation": "upright"|"reversed"}]}
 *
 * Позиции — константа бизнес-логики тула (не движка):
 *   count=1 → ["совет"], count=3 → ["прошлое", "настоящее", "перспектива"].
 * Любая невалидность — stderr + exit(1) (движок превращает в ошибку ноды).
 */

interface LookupInput {
  numbers?: unknown;
  orientations?: unknown;
}

interface CardRecord {
  id: string;
  number: number;
  name_en: string;
  name_ru: string;
  arcana: string;
  rank: string;
  keywords_ru: string;
  upright_ru: string;
  reversed_ru: string;
  image_file: string;
}

const POSITIONS_ONE = ["совет"];
const POSITIONS_THREE = ["прошлое", "настоящее", "перспектива"];

function parseInput(raw: string | undefined): LookupInput {
  if (raw === undefined || raw.trim() === "") {
    throw new Error("пустой вход: ожидался JSON-объект с полями numbers и orientations");
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error(`вход не является валидным JSON: ${raw.slice(0, 100)}`);
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("вход обязан быть JSON-объектом с полями numbers и orientations");
  }
  return parsed as LookupInput;
}

/** Принимает и строку-JSON-массив (как её сериализует движок), и настоящий массив. */
function normalizeArrayField(value: unknown, fieldName: string): unknown[] {
  let candidate: unknown = value;
  if (typeof candidate === "string") {
    try {
      candidate = JSON.parse(candidate);
    } catch {
      throw new Error(`поле ${fieldName}: строка не является JSON-массивом: ${candidate.slice(0, 100)}`);
    }
  }
  if (!Array.isArray(candidate)) {
    throw new Error(`поле ${fieldName} обязано быть массивом, получено: ${typeof candidate}`);
  }
  return candidate;
}

function normalizeNumbers(raw: unknown[]): number[] {
  return raw.map((v, i) => {
    const n = typeof v === "number" ? v : Number(String(v));
    if (!Number.isInteger(n) || n < 0 || n > 77) {
      throw new Error(`numbers[${i}] должен быть целым числом 0..77, получено: ${String(v)}`);
    }
    return n;
  });
}

function normalizeOrientations(raw: unknown[]): string[] {
  return raw.map((v, i) => {
    const s = String(v);
    if (s !== "upright" && s !== "reversed") {
      throw new Error(`orientations[${i}] должен быть "upright" или "reversed", получено: "${s}"`);
    }
    return s;
  });
}

async function loadManifest(): Promise<Map<number, CardRecord>> {
  // Путь — относительно файла тула, не от cwd процесса.
  const manifestUrl = new URL("../database/cards.json", import.meta.url);
  const text = await Deno.readTextFile(manifestUrl);
  const cards = JSON.parse(text) as CardRecord[];
  if (!Array.isArray(cards) || cards.length !== 78) {
    throw new Error(`cards.json повреждён: ожидалось 78 записей, получено ${Array.isArray(cards) ? cards.length : "не массив"}`);
  }
  const byNumber = new Map<number, CardRecord>();
  for (const card of cards) {
    if (typeof card.number !== "number" || byNumber.has(card.number)) {
      throw new Error(`cards.json повреждён: некорректный или дублирующийся number: ${String(card.number)}`);
    }
    byNumber.set(card.number, card);
  }
  return byNumber;
}

function positionsFor(count: number): string[] {
  if (count === 1) return POSITIONS_ONE;
  if (count === 3) return POSITIONS_THREE;
  throw new Error(`count должен быть 1 или 3, получено: ${count}`);
}

async function main(): Promise<void> {
  try {
    const input = parseInput(Deno.args[0]);
    const numbers = normalizeNumbers(normalizeArrayField(input.numbers, "numbers"));
    const orientations = normalizeOrientations(normalizeArrayField(input.orientations, "orientations"));
    if (numbers.length !== orientations.length) {
      throw new Error(`длины numbers (${numbers.length}) и orientations (${orientations.length}) не совпадают`);
    }
    if (numbers.length < 1 || numbers.length > 3) {
      throw new Error(`count должен быть в диапазоне 1..3, получено: ${numbers.length}`);
    }
    const seen = new Set<number>();
    for (const n of numbers) {
      if (seen.has(n)) throw new Error(`дублирующийся номер карты: ${n}`);
      seen.add(n);
    }

    const manifest = await loadManifest();
    const cards = numbers.map((n) => {
      const card = manifest.get(n);
      if (!card) throw new Error(`карта с номером ${n} не найдена в manifest`);
      return { ...card };
    });
    // orientation проставляется каждой карте из параллельного массива.
    cards.forEach((card, i) => {
      (card as CardRecord & { orientation: string }).orientation = orientations[i];
    });

    const result = {
      count: numbers.length,
      positions: positionsFor(numbers.length),
      cards,
    };
    console.log(JSON.stringify(result));
  } catch (err) {
    console.error(`lookup_tarot_cards: ${err instanceof Error ? err.message : String(err)}`);
    Deno.exit(1);
  }
}

main();
