/**
 * build_knowledge — сборка «досье» из базы знаний под конкретную карту и вопрос.
 *
 * Вход: один argv-токен — плоский JSON строк вида
 *   {"lagna": "<вывод calc_chart.output.lagna>", "planets": "<...output.planets>",
 *    "topic": "<...output.question_topic>", "transits": "<...output.transits>"}
 * Движок сериализует не-строки в JSON-строку (как у tarot lookup), поэтому поля
 * принимаются и строкой-JSON, и готовым объектом/массивом.
 *
 * Читает ../database/ относительно файла тула (import.meta.url) — прецедент
 * lookup_tarot_cards.ts; allow-read Deno покрывает папку команды (custom_tool.rs).
 *
 * Выход ТОЛЬКО JSON в stdout:
 *   {"text": "...", "files_used": [...], "sections": N}
 * Состав досье:
 *   - файл лагны;
 *   - все 12 связок «управитель дома в доме» этой карты (7 грах-управителей,
 *     Раху/Кету владений не имеют);
 *   - йоги, детектированные в карте (vedastro_detect.ts) — только они, секции
 *     из yogas.md; недетектированные йоги в досье не попадают (00.2);
 *   - гочара: ровно по одной транзитной секции на граху (дома от Луны), из gochara.md;
 *   - правила периодов даш: универсальные + солнечные по состоянию Солнца,
 *     из dasha_rules.md;
 *   - файл темы вопроса (неизвестная/пустая тема → general).
 *
 * Любая недостающая часть БЗ, битый вход или детектированный id без секции в
 * базе — stderr + exit 1: честный отказ, workflow останавливается, тихой
 * деградации нет.
 */

import { SIGNS } from "./vedic_math.ts";
import {
    DetectPlanet,
    DetectTransit,
    RU_NAME_TO_KEY,
    detectDashaRules,
    detectGochara,
    detectYogas,
} from "./vedastro_detect.ts";
import { Dignity } from "./chart_analysis.ts";

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
    dignity?: unknown;
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

/** Русское имя знака → индекс 0..11 (источник — SIGNS vedic_math.ts). */
export function signIndexFor(signRu: string): number {
    const idx = SIGNS.indexOf(signRu);
    if (idx < 0) throw new Error(`неизвестный знак "${signRu}" — нет в SIGNS (vedic_math.ts)`);
    return idx;
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

const DIGNITIES: Dignity[] = ["exalted", "moolatrikona", "own_sign", "debilitated", "neutral"];

/**
 * JSON-грахи calc_chart → срез детекции. Русское имя должно быть известно
 * (RU_NAME_TO_KEY), достоинство — из словаря или null (узлы).
 */
export function toDetectPlanets(planets: PlanetIn[]): DetectPlanet[] {
    const mapped = planets.map((p) => {
        const name = typeof p?.name === "string" ? p.name : "";
        const key = RU_NAME_TO_KEY[name];
        if (!key) throw new Error(`планета с именем "${name}" не известна детекции (RU_NAME_TO_KEY)`);
        if (!Number.isInteger(p?.house_d1) || (p.house_d1 as number) < 1 || (p.house_d1 as number) > 12) {
            throw new Error(`планета "${name}": дом house_d1 вне 1..12 (${String(p?.house_d1)})`);
        }
        let dignity: Dignity | null = null;
        if (p.dignity !== null && p.dignity !== undefined) {
            if (typeof p.dignity !== "string" || !DIGNITIES.includes(p.dignity as Dignity)) {
                throw new Error(`планета "${name}": некорректное dignity (${String(p.dignity)})`);
            }
            dignity = p.dignity as Dignity;
        }
        const ruled = p.ruled_houses_d1;
        if (ruled !== null && ruled !== undefined && !Array.isArray(ruled)) {
            throw new Error(`планета "${name}": ruled_houses_d1 не массив — данные calc_chart повреждены`);
        }
        return {
            key,
            houseD1: p.house_d1 as number,
            dignity,
            ruledHousesD1: ruled === null || ruled === undefined ? null : (ruled as number[]),
        };
    });
    if (mapped.length !== 9) {
        throw new Error(`ожидалось 9 грах, получено ${mapped.length}`);
    }
    return mapped;
}

/** Транзиты calc_chart → срез детекции (имя + проходимый дом натальной карты). */
export function toDetectTransits(raw: unknown): DetectTransit[] {
    const transits = normalizeField<Record<string, unknown>>(raw, "transits");
    if (!Array.isArray(transits.grahas)) {
        throw new Error("transits.grahas не массив — данные calc_chart повреждены");
    }
    return (transits.grahas as Record<string, unknown>[]).map((g) => {
        const name = typeof g?.name === "string" ? g.name : "";
        if (!RU_NAME_TO_KEY[name]) throw new Error(`транзитная граха "${name}" не известна детекции`);
        if (!Number.isInteger(g?.house_transited)) {
            throw new Error(`транзит "${name}": house_transited не число (${String(g?.house_transited)})`);
        }
        return { name, houseTransited: g.house_transited as number };
    });
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

/** Заголовки секций-идентификаторов: «## <LatinId>» (yogas.md / gochara.md / dasha_rules.md). */
function parseIdSections(md: string, sourceName: string): Map<string, SelectedSection> {
    const lines = md.split(/\r?\n/);
    const map = new Map<string, SelectedSection>();
    let current: { key: string; body: string[] } | null = null;
    const flush = () => {
        if (current) {
            if (map.has(current.key)) {
                throw new Error(`${sourceName}: дублирующаяся секция «${current.key}»`);
            }
            map.set(current.key, { heading: `## ${current.key}`, text: current.body.join("\n").trim() });
        }
    };
    const re = /^## ([A-Za-z][A-Za-z0-9]*)\s*$/;
    for (const line of lines) {
        const m = re.exec(line);
        if (m) {
            flush();
            current = { key: m[1], body: [] };
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

/**
 * Секции-идентификаторы по списку id (порядок списка сохраняется).
 * Детектированный id без секции в базе — fail-fast (битая БЗ, а не тихий пропуск).
 */
export function selectIdSections(
    md: string,
    sourceName: string,
    ids: string[],
): SelectedSection[] {
    const map = parseIdSections(md, sourceName);
    const missing = ids.filter((id) => !map.has(id));
    if (missing.length > 0) {
        throw new Error(`${sourceName}: нет секций для детектированных записей: ${missing.join(", ")}`);
    }
    return ids.map((id) => map.get(id)!);
}

export interface KnowledgeInput {
    lagnaSign: string;
    placements: { lord: string; ruled: number; placed: number }[];
    topicFile: string;
    lagnaContent: string;
    lowContent: string;
    highContent: string;
    topicContent: string;
    yogasContent: string;
    gocharaContent: string;
    dashaRulesContent: string;
    detectedYogaIds: string[];
    detectedGocharaIds: string[];
    detectedDashaRuleIds: string[];
}

/** Секции «id-файла» в текст досье; пустой набор — честная строка вместо пустоты. */
function idSectionsBlock(
    md: string,
    sourceName: string,
    ids: string[],
    emptyNote: string,
): string {
    if (ids.length === 0) return emptyNote;
    return selectIdSections(md, sourceName, ids)
        .map((s) => `${s.heading}\n${s.text}`)
        .join("\n\n");
}

/** Сборка итогового текста досье. */
export function assembleText(input: KnowledgeInput): string {
    const sections = selectBhavaSections(input.lowContent, input.highContent, input.placements);
    const yogasBlock = idSectionsBlock(
        input.yogasContent,
        "yogas.md",
        input.detectedYogaIds,
        "Детектируемых йог этого набора в карте не обнаружено.",
    );
    const gocharaBlock = idSectionsBlock(
        input.gocharaContent,
        "gochara.md",
        input.detectedGocharaIds,
        "Транзитные секции гочары не выбраны (список транзитных грах пуст).",
    );
    const dashaBlock = idSectionsBlock(
        input.dashaRulesContent,
        "dasha_rules.md",
        input.detectedDashaRuleIds,
        "Правил периодов даш не выбрано.",
    );
    const parts = [
        `=== ФУНКЦИОНАЛЬНАЯ ПРИРОДА ГРАХ ДЛЯ ЛАГНЫ ${input.lagnaSign.toUpperCase()} ===`,
        input.lagnaContent.trim(),
        `=== УПРАВИТЕЛИ ДОМОВ В ДОМАХ (связки этой карты) ===`,
        sections.map((s) => `${s.heading}\n${s.text}`).join("\n\n"),
        `=== ЙОГИ ЭТОЙ КАРТЫ (детектированы автоматически; применять только этот список) ===`,
        yogasBlock,
        `=== ТРАНЗИТЫ GOCHARA: ЭФФЕКТ ТЕКУЩЕГО ТРАНЗИТА КАЖДОЙ ГРАХИ (номер дома — от Луны) ===`,
        gocharaBlock,
        `=== ПРАВИЛА ПЕРИОДОВ ДАШ (универсальные и по состоянию Солнца) ===`,
        dashaBlock,
        `=== ПРАВИЛА ТЕМЫ ВОПРОСА: ${input.topicFile.replace(".md", "")} ===`,
        input.topicContent.trim(),
    ];
    return parts.join("\n\n");
}

async function main() {
    const raw = Deno.args[0];
    if (!raw) {
        throw new Error("не переданы аргументы (ожидался JSON с lagna, planets, topic, transits)");
    }
    const args = normalizeField<Record<string, unknown>>(raw, "args");

    const lagna = normalizeField<Record<string, unknown>>(args.lagna, "lagna");
    const planets = normalizeField<PlanetIn[]>(args.planets, "planets");
    const sign = typeof lagna.sign === "string" ? lagna.sign : "";
    if (!sign) throw new Error("в lagna нет строкового поля sign — данные calc_chart повреждены");

    const placements = lordPlacements(planets);
    const detectPlanets = toDetectPlanets(planets);
    const detectTransits = toDetectTransits(args.transits);
    const lagnaIdx = signIndexFor(sign);

    const detectedYogaIds = detectYogas(detectPlanets, lagnaIdx);
    const detectedGocharaIds = detectGochara(detectPlanets, detectTransits);
    const detectedDashaRuleIds = detectDashaRules(detectPlanets, lagnaIdx);

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
    const yogasContent = await read("yogas.md");
    const gocharaContent = await read("gochara.md");
    const dashaRulesContent = await read("dasha_rules.md");

    const text = assembleText({
        lagnaSign: sign,
        placements,
        topicFile,
        lagnaContent,
        lowContent,
        highContent,
        topicContent,
        yogasContent,
        gocharaContent,
        dashaRulesContent,
        detectedYogaIds,
        detectedGocharaIds,
        detectedDashaRuleIds,
    });

    console.log(JSON.stringify({
        text,
        files_used: [
            `lagnas/${lagnaFile}`,
            "bhava_lords_1_6.md",
            "bhava_lords_7_12.md",
            "yogas.md",
            "gochara.md",
            "dasha_rules.md",
            `topics/${topicFile}`,
        ],
        sections: placements.length + detectedYogaIds.length + detectedGocharaIds.length +
            detectedDashaRuleIds.length,
        detected: {
            yogas: detectedYogaIds,
            gochara: detectedGocharaIds,
            dasha_rules: detectedDashaRuleIds,
        },
    }));
}

if (import.meta.main) {
    main().catch((e) => {
        console.error(`build_knowledge: ${e instanceof Error ? e.message : String(e)}`);
        Deno.exit(1);
    });
}
