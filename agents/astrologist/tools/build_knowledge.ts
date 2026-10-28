/**
 * build_knowledge — сборка «досье» из базы знаний под конкретную карту и вопрос.
 *
 * Вход: один argv-токен — плоский JSON строк вида
 *   {"lagna": "<вывод calc_chart.output.lagna>", "planets": "<...output.planets>",
 *    "topic": "<...output.question_topic>"}
 * Движок сериализует не-строки в JSON-строку (как у tarot lookup), поэтому поля
 * принимаются и строкой-JSON, и готовым объектом/массивом.
 *
 * Читает ../database/ относительно файла тула (import.meta.url) — прецедент
 * lookup_tarot_cards.ts; allow-read Deno покрывает папку команды (custom_tool.rs).
 *
 * Выход ТОЛЬКО JSON в stdout:
 *   {"text": "...", "files_used": [...], "sections": N}
 * Состав досье: файл лагны + все 12 связок «управитель дома в доме» этой карты
 * (каждый дом имеет ровно одного управителя среди 7 грах; Раху/Кету владений
 * не имеют) + файл темы вопроса (неизвестная/пустая тема → general).
 *
 * Любая недостающая часть БЗ или битый вход — stderr + exit 1: честный отказ,
 * workflow останавливается, тихой деградации нет.
 */

/** Русское имя знака → файл лагны в database/lagnas/. */
const SIGN_FILES: Record<string, string> = {
    "Овен": "aries.md",
    "Телец": "taurus.md",
    "Близнецы": "gemini.md",
    "Рак": "cancer.md",
    "Лев": "leo.md",
    "Дева": "virgo.md",
    "Весы": "libra.md",
    "Скорпион": "scorpio.md",
    "Стрелец": "sagittarius.md",
    "Козерог": "capricorn.md",
    "Водолей": "aquarius.md",
    "Рыбы": "pisces.md",
};

const TOPIC_FILES: Record<string, string> = {
    career_finance: "career_finance.md",
    relationships: "relationships.md",
    health: "health.md",
    spirituality: "spirituality.md",
    general: "general.md",
};

interface PlanetIn {
    name?: unknown;
    house_d1?: unknown;
    ruled_houses_d1?: unknown;
}

export interface SelectedSection {
    heading: string;
    text: string;
}

/** Принимает и JSON-строку (как сериализует движок), и готовое значение. */
function normalizeField<T>(value: unknown, fieldName: string): T {
    let candidate: unknown = value;
    if (typeof candidate === "string") {
        try {
            candidate = JSON.parse(candidate);
        } catch {
            throw new Error(`поле ${fieldName}: строка не является JSON: ${candidate.slice(0, 100)}`);
        }
    }
    if (candidate === null || typeof candidate !== "object") {
        throw new Error(`поле ${fieldName} обязано быть объектом/массивом, получено: ${typeof candidate}`);
    }
    return candidate as T;
}

/** Знак лагны → имя файла; неизвестный знак — честная ошибка. */
export function lagnaFileFor(signRu: string): string {
    const file = SIGN_FILES[signRu];
    if (!file) {
        throw new Error(`неизвестный знак лагны "${signRu}" — файла в database/lagnas/ нет`);
    }
    return file;
}

/** Тема → имя файла; пустая/неизвестная/не-строка → general (тема — совет, не контракт). */
export function topicFileFor(topic: unknown): string {
    if (typeof topic === "string" && TOPIC_FILES[topic]) return TOPIC_FILES[topic];
    return TOPIC_FILES.general;
}

/**
 * Валидация грах и пары «управляемый дом → дом, где стоит управитель».
 * Контракт карты: у 7 грах есть ruled_houses_d1 (массив 1..12), у Раху/Кету — null;
 * суммарно покрыты ровно 12 домов без дублей.
 */
export function lordPlacements(planets: PlanetIn[]): { lord: string; ruled: number; placed: number }[] {
    const seen = new Set<number>();
    const placements: { lord: string; ruled: number; placed: number }[] = [];
    let lordsCount = 0;
    for (const p of planets) {
        const name = typeof p?.name === "string" ? p.name : "";
        const house = typeof p?.house_d1 === "number" ? p.house_d1 : NaN;
        const ruled = p?.ruled_houses_d1;
        if (ruled === null || ruled === undefined) continue; // Раху/Кету
        if (!Array.isArray(ruled)) {
            throw new Error(`планета "${name}": ruled_houses_d1 не массив — данные calc_chart повреждены`);
        }
        if (!Number.isInteger(house) || house < 1 || house > 12) {
            throw new Error(`планета "${name}": дом house_d1 вне 1..12 (${String(p?.house_d1)})`);
        }
        lordsCount += 1;
        for (const h of ruled) {
            if (typeof h !== "number" || !Number.isInteger(h) || h < 1 || h > 12) {
                throw new Error(`планета "${name}": дом владения вне 1..12 (${String(h)})`);
            }
            if (seen.has(h)) {
                throw new Error(`дом ${h} заявлен как владение повторно (планета "${name}") — карта повреждена`);
            }
            seen.add(h);
            placements.push({ lord: name, ruled: h, placed: house });
        }
    }
    if (lordsCount !== 7) {
        throw new Error(`ожидалось 7 грах-управителей (без Раху/Кету), получено ${lordsCount}`);
    }
    if (seen.size !== 12) {
        throw new Error(`покрыто ${seen.size} домов из 12 — владения грах в карте неполны`);
    }
    return placements;
}

/** Заголовки секций строго в формате «## Управитель N-го дома в M-м доме». */
function parseSections(md: string, sourceName: string): Map<string, SelectedSection> {
    const lines = md.split(/\r?\n/);
    const map = new Map<string, SelectedSection>();
    let current: { key: string; heading: string; body: string[] } | null = null;
    const flush = () => {
        if (current) {
            if (map.has(current.key)) {
                throw new Error(`${sourceName}: дублирующаяся секция «${current.heading}»`);
            }
            map.set(current.key, { heading: current.heading, text: current.body.join("\n").trim() });
        }
    };
    const re = /^## Управитель (\d+)-го дома в (\d+)-м доме\s*$/;
    for (const line of lines) {
        const m = re.exec(line);
        if (m) {
            flush();
            current = { key: `${m[1]}-${m[2]}`, heading: line.trim(), body: [] };
        } else if (current) {
            current.body.push(line);
        }
    }
    flush();
    return map;
}

/**
 * Выборка связок этой карты из содержимого обоих файлов bhava_lords.
 * Отсутствующая секция — ошибка с полным списком недостающего (fail-fast).
 */
export function selectBhavaSections(
    mdLow: string,
    mdHigh: string,
    placements: { lord: string; ruled: number; placed: number }[],
): SelectedSection[] {
    const low = parseSections(mdLow, "bhava_lords_1_6.md");
    const high = parseSections(mdHigh, "bhava_lords_7_12.md");
    const missing: string[] = [];
    const selected: SelectedSection[] = [];
    for (const { lord, ruled, placed } of placements) {
        const key = `${ruled}-${placed}`;
        const section = (ruled <= 6 ? low : high).get(key);
        if (!section) {
            missing.push(`управитель ${ruled}-го дома (${lord}) в ${placed}-м`);
            continue;
        }
        selected.push(section);
    }
    if (missing.length > 0) {
        throw new Error(`в базе знаний нет секций: ${missing.join("; ")}`);
    }
    // Порядок секций — по номеру управляемого дома (1..12), читается логичнее.
    return selected.sort((a, b) => Number(a.heading.match(/\d+/)![0]) - Number(b.heading.match(/\d+/)![0]));
}

export interface KnowledgeInput {
    lagnaSign: string;
    placements: { lord: string; ruled: number; placed: number }[];
    topicFile: string;
    lagnaContent: string;
    lowContent: string;
    highContent: string;
    topicContent: string;
}

/** Сборка итогового текста досье. */
export function assembleText(input: KnowledgeInput): string {
    const sections = selectBhavaSections(input.lowContent, input.highContent, input.placements);
    const parts = [
        `=== ФУНКЦИОНАЛЬНАЯ ПРИРОДА ГРАХ ДЛЯ ЛАГНЫ ${input.lagnaSign.toUpperCase()} ===`,
        input.lagnaContent.trim(),
        `=== УПРАВИТЕЛИ ДОМОВ В ДОМАХ (связки этой карты) ===`,
        sections.map((s) => `${s.heading}\n${s.text}`).join("\n\n"),
        `=== ПРАВИЛА ТЕМЫ ВОПРОСА: ${input.topicFile.replace(".md", "")} ===`,
        input.topicContent.trim(),
    ];
    return parts.join("\n\n");
}

async function main() {
    const raw = Deno.args[0];
    if (!raw) throw new Error("не переданы аргументы (ожидался JSON с lagna, planets, topic)");
    const args = normalizeField<Record<string, unknown>>(raw, "args");

    const lagna = normalizeField<Record<string, unknown>>(args.lagna, "lagna");
    const planets = normalizeField<PlanetIn[]>(args.planets, "planets");
    const sign = typeof lagna.sign === "string" ? lagna.sign : "";
    if (!sign) throw new Error("в lagna нет строкового поля sign — данные calc_chart повреждены");

    const placements = lordPlacements(planets);
    const lagnaFile = lagnaFileFor(sign);
    const topicFile = topicFileFor(args.topic);

    const read = async (rel: string): Promise<string> => {
        const url = new URL(`../database/${rel}`, import.meta.url);
        try {
            return await Deno.readTextFile(url);
        } catch (e) {
            throw new Error(
                `файл базы знаний database/${rel} не прочитан: ${e instanceof Error ? e.message : String(e)}`,
            );
        }
    };

    const lagnaContent = await read(`lagnas/${lagnaFile}`);
    const lowContent = await read("bhava_lords_1_6.md");
    const highContent = await read("bhava_lords_7_12.md");
    const topicContent = await read(`topics/${topicFile}`);

    const text = assembleText({
        lagnaSign: sign,
        placements,
        topicFile,
        lagnaContent,
        lowContent,
        highContent,
        topicContent,
    });

    console.log(JSON.stringify({
        text,
        files_used: [`lagnas/${lagnaFile}`, "bhava_lords_1_6.md", "bhava_lords_7_12.md", `topics/${topicFile}`],
        sections: placements.length,
    }));
}

if (import.meta.main) {
    main().catch((e) => {
        console.error(`build_knowledge: ${e instanceof Error ? e.message : String(e)}`);
        Deno.exit(1);
    });
}
