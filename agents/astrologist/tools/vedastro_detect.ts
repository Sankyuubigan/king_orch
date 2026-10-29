/**
 * Детекция применимости записей VedAstro (MIT, см. ../database/source/vedastro/)
 * к конкретной карте. Реестр computable-йог и даш-правил — единый источник
 * правды: классификатор (vedastro_classes.ts) берёт отсюда список id, селектор
 * (build_knowledge.ts) — сами предикаты.
 *
 * Все чтения формулировок зафиксированы ДО реализации в
 * tasks/29.10.26 Этап 2 — VedAstro обогащение базы знаний.md
 * («Решения, зафиксированные ДО реализации»); здесь они не переизобретаются:
 *   - соединение = один знак (одинаковый house_d1);
 *   - благодетели = Меркурий, Юпитер, Венера + растущая Луна (шукла-пакша:
 *     Луна во 2..7 доме от Солнца; без Луны Васуматхи была бы пустым правилом);
 *   - узлы считаются планетами, где запись не делает исключений;
 *   - «свой знак или экзальтация» = dignity ∈ {exalted, moolatrikona, own_sign};
 *   - семейства «одна сторона / обе стороны» (Sunapha/Dhurdhura, Vesi/Obhayachari)
 *     взаимоисключающие по классике;
 *   - самбандха = соединение ИЛИ взаимная дришти ИЛИ обмен знаками.
 *
 * Функции чистые: вход — минимальный срез карты (дом D1, достоинство, владения),
 * эфемерид и времени не касаются. Недетектируемые записи (Аштакаварга, Гулика,
 * пол, мухурта и т.п.) сюда не входят — они получают честный флаг в классификаторе
 * и в промпт не попадают (правило 00.2: нельзя инжектить правило, которое модель
 * применит вслепую).
 */

import { Dignity, drishtiHouses, signLordOf } from "./chart_analysis.ts";

/** Русское имя грахи из calc_chart (совпадает с BODIES там) → латинский ключ. */
export const RU_NAME_TO_KEY: Record<string, string> = {
    "Солнце": "Sun",
    "Луна": "Moon",
    "Марс": "Mars",
    "Меркурий": "Mercury",
    "Юпитер": "Jupiter",
    "Венера": "Venus",
    "Сатурн": "Saturn",
    "Раху": "Rahu",
    "Кету": "Ketu",
};

/** Минимальный срез грахи, достаточный для всех детекций этапа. */
export interface DetectPlanet {
    key: string;
    houseD1: number;
    dignity: Dignity | null;
    ruledHousesD1: number[] | null;
}

const KENDRA = [1, 4, 7, 10];
const UPACHAYA = [3, 6, 10, 11];
const BENEFIC_KEYS = ["Mercury", "Jupiter", "Venus"];
const MALEFICS = new Set(["Mars", "Saturn", "Rahu", "Ketu"]);
/** Планеты «второй/двенадцатой стороны» — все, кроме Солнца; Луна исключается там, где запись это делает. */
const NON_SUN_PLANETS = ["Mars", "Mercury", "Jupiter", "Venus", "Saturn", "Rahu", "Ketu"];
const NON_SUN_MOON_PLANETS = NON_SUN_PLANETS.filter((k) => k !== "Moon");
const SEVEN_GRAHAS = ["Sun", "Moon", "Mars", "Mercury", "Jupiter", "Venus", "Saturn"];
const DUSTHANA = [6, 8, 12];

function requirePlanet(byKey: Record<string, DetectPlanet>, key: string): DetectPlanet {
    const p = byKey[key];
    if (!p) throw new Error(`detect: в карте нет грахи ${key} — детекция невозможна`);
    return p;
}

/** Дом-от-точки: номер дома toHouse, если точка отсчёта стоит в fromHouse. От лагны (fromHouse=1) совпадает с номером дома. */
function houseFrom(fromHouse: number, toHouse: number): number {
    return ((toHouse - fromHouse + 12) % 12) + 1;
}

function ownOrExalt(dignity: Dignity | null): boolean {
    return dignity === "exalted" || dignity === "moolatrikona" || dignity === "own_sign";
}

/**
 * Благодетели: Меркурий, Юпитер, Венера и растущая Луна (шукла-пакша — Луна
 * во 2..7 доме от Солнца). Без Луны правило «все упачайи заняты благодетелями»
 * (Васуматхи) было бы пустым: 3 благодетеля на 4 дома. Луна от Луны не
 * отсчитывается, так что для правил от Луны это расширение ничего не меняет.
 */
function isBenefic(byKey: Record<string, DetectPlanet>, key: string): boolean {
    if (BENEFIC_KEYS.includes(key)) return true;
    if (key === "Moon") {
        const n = houseFrom(byKey.Sun.houseD1, byKey.Moon.houseD1);
        return n >= 2 && n <= 7;
    }
    return false;
}

/** Занят ли n-й дом от точки отсчёта благодетелем (isBenefic). */
function occupiedByBenefic(byKey: Record<string, DetectPlanet>, fromHouse: number, n: number): boolean {
    return Object.values(byKey).some((p) => isBenefic(byKey, p.key) && houseFrom(fromHouse, p.houseD1) === n);
}

export interface YogaContext {
    planets: Record<string, DetectPlanet>;
    lagnaSignIdx: number;
    /** Знак (0..11), в котором стоит граха. */
    signIdxOf: (key: string) => number;
    /** Знак (0..11) дома D1 от лагны. */
    signIdxOfHouse: (houseD1: number) => number;
    houseFromMoon: (key: string) => number;
    houseFromSun: (key: string) => number;
    /** Управитель дома N (1..12) — граха, владеющая этим домом от лагны. */
    lordOfHouse: (house: number) => DetectPlanet;
    /** Самбандха: соединение ИЛИ взаимная дришти ИЛИ обмен знаками. */
    sambandha: (a: string, b: string) => boolean;
    /** Занят ли n-й дом от точки отсчёта (fromHouse) хотя бы одной грахой из набора. */
    occupied: (fromHouse: number, n: number, keys: string[]) => boolean;
}

/** Маліка: 7 грах занимают 7 подряд идущих домов от startHouse (с закольцовкой через 12-й). */
function malika(ctx: YogaContext, startHouse: number): boolean {
    const houses = SEVEN_GRAHAS.map((k) => requirePlanet(ctx.planets, k).houseD1).sort((a, b) => a - b);
    const expected = Array.from({ length: 7 }, (_, i) => ((startHouse - 1 + i) % 12) + 1).sort((a, b) => a - b);
    return houses.length === 7 && houses.every((h, i) => h === expected[i]);
}

/** Панча-махапуруша: граха в кендре от лагны в своём знаке/экзальтации/муллатриконе. */
function mahapurusha(ctx: YogaContext, key: string): boolean {
    const p = requirePlanet(ctx.planets, key);
    return KENDRA.includes(p.houseD1) && ownOrExalt(p.dignity);
}

/**
 * Реестр computable-йог: id записи VedAstro → предикат карты. Порядок ключей —
 * порядок секций в yogas.md.
 */
export const COMPUTABLE_YOGAS: Record<string, (ctx: YogaContext) => boolean> = {
    // Управитель знака Меркурия в дустхане (6/8/12).
    MercuryAshtakavargaYoga12B: (c) => {
        const mercury = requirePlanet(c.planets, "Mercury");
        const lordKey = signLordOf(c.signIdxOfHouse(mercury.houseD1));
        return DUSTHANA.includes(requirePlanet(c.planets, lordKey).houseD1);
    },
    GajakesariYoga: (c) => KENDRA.includes(c.houseFromMoon("Jupiter")),
    SunaphaYoga: (c) => c.occupied(c.planets.Moon.houseD1, 2, NON_SUN_PLANETS) && !c.occupied(c.planets.Moon.houseD1, 12, NON_SUN_PLANETS),
    AnaphaYoga: (c) => c.occupied(c.planets.Moon.houseD1, 12, NON_SUN_PLANETS) && !c.occupied(c.planets.Moon.houseD1, 2, NON_SUN_PLANETS),
    DhurdhuraYoga: (c) => c.occupied(c.planets.Moon.houseD1, 2, NON_SUN_PLANETS) && c.occupied(c.planets.Moon.houseD1, 12, NON_SUN_PLANETS),
    KemadrumaYoga: (c) => !c.occupied(c.planets.Moon.houseD1, 2, NON_SUN_PLANETS) && !c.occupied(c.planets.Moon.houseD1, 12, NON_SUN_PLANETS),
    ChandraMangalaYoga: (c) => c.planets.Mars.houseD1 === c.planets.Moon.houseD1,
    AdhiYoga: (c) => [6, 7, 8].every((n) => c.occupied(c.planets.Moon.houseD1, n, BENEFIC_KEYS)),
    ChatussagaraYoga: (c) => KENDRA.every((h) => Object.values(c.planets).some((p) => p.houseD1 === h)),
    // Все четыре упачайи заняты благодетелями — от лагны ИЛИ от Луны.
    VasumathiYoga: (c) =>
        UPACHAYA.every((n) => occupiedByBenefic(c.planets, 1, n)) ||
        UPACHAYA.every((n) => occupiedByBenefic(c.planets, c.planets.Moon.houseD1, n)),
    RajalakshanaYoga: (c) => ["Jupiter", "Venus", "Mercury", "Moon"].every((k) => KENDRA.includes(c.planets[k].houseD1)),
    SakataYoga: (c) => [6, 8, 12].includes(houseFrom(c.planets.Jupiter.houseD1, c.planets.Moon.houseD1)),
    AmalaYoga: (c) => occupiedByBenefic(c.planets, 1, 10) || occupiedByBenefic(c.planets, c.planets.Moon.houseD1, 10),
    VesiYoga: (c) => c.occupied(c.planets.Sun.houseD1, 2, NON_SUN_MOON_PLANETS) && !c.occupied(c.planets.Sun.houseD1, 12, NON_SUN_MOON_PLANETS),
    VasiYoga: (c) => c.occupied(c.planets.Sun.houseD1, 12, NON_SUN_MOON_PLANETS) && !c.occupied(c.planets.Sun.houseD1, 2, NON_SUN_MOON_PLANETS),
    ObhayachariYoga: (c) => c.occupied(c.planets.Sun.houseD1, 2, NON_SUN_MOON_PLANETS) && c.occupied(c.planets.Sun.houseD1, 12, NON_SUN_MOON_PLANETS),
    HamsaYoga: (c) => mahapurusha(c, "Jupiter"),
    MalavyaYoga: (c) => mahapurusha(c, "Venus"),
    SasaYoga: (c) => mahapurusha(c, "Saturn"),
    RuchakaYoga: (c) => mahapurusha(c, "Mars"),
    BhadraYoga: (c) => mahapurusha(c, "Mercury"),
    BudhaAdityaYoga: (c) => c.planets.Mercury.houseD1 === c.planets.Sun.houseD1,
    // Лагна-управитель экзальтирован, 4-й и 10-й управители обменялись домами.
    ChapaYoga: (c) =>
        c.planets[signLordOf(c.lagnaSignIdx)].dignity === "exalted" &&
        c.lordOfHouse(4).houseD1 === 10 && c.lordOfHouse(10).houseD1 === 4,
    // Экзальтированный управитель 7-го в 10-м, управитель 10-го вместе с управителем 9-го.
    SreenathaYoga: (c) => {
        const lord7 = c.lordOfHouse(7);
        return lord7.dignity === "exalted" && lord7.houseD1 === 10 &&
            c.lordOfHouse(10).houseD1 === c.lordOfHouse(9).houseD1;
    },
    LagnaMalikaYoga: (c) => malika(c, 1),
    DhanaMalikaYoga: (c) => malika(c, 2),
    VikramaMalikaYoga: (c) => malika(c, 3),
    SukhaMalikaYoga: (c) => malika(c, 4),
    PutraMalikaYoga: (c) => malika(c, 5),
    SatruMalikaYoga: (c) => malika(c, 6),
    KalatraMalikaYoga: (c) => malika(c, 7),
    RandhraMalikaYoga: (c) => malika(c, 8),
    BhagyaMalikaYoga: (c) => malika(c, 9),
};

function validateLagna(lagnaSignIdx: number, fn: string): void {
    if (!Number.isInteger(lagnaSignIdx) || lagnaSignIdx < 0 || lagnaSignIdx > 11) {
        throw new Error(`${fn}: лагна вне 0..11 (${String(lagnaSignIdx)})`);
    }
}

/**
 * Детекция йог. planets — ровно 9 грах (Sun..Ketu); недостающая граха, битый
 * дом или повторный ключ — честная ошибка (fail-fast, никаких тихих пропусков).
 */
export function detectYogas(planets: DetectPlanet[], lagnaSignIdx: number): string[] {
    validateLagna(lagnaSignIdx, "detectYogas");
    const byKey = validatePlanets(planets, "detectYogas");
    const ctx = buildYogaContext(byKey, lagnaSignIdx);
    return Object.keys(COMPUTABLE_YOGAS).filter((id) => COMPUTABLE_YOGAS[id](ctx));
}

function validatePlanets(planets: DetectPlanet[], fn: string): Record<string, DetectPlanet> {
    const byKey: Record<string, DetectPlanet> = {};
    for (const p of planets) {
        if (byKey[p.key]) throw new Error(`${fn}: граха ${p.key} встречается повторно`);
        if (!Number.isInteger(p.houseD1) || p.houseD1 < 1 || p.houseD1 > 12) {
            throw new Error(`${fn}: дом грахи ${p.key} вне 1..12 (${String(p.houseD1)})`);
        }
        if (p.ruledHousesD1 !== null && !Array.isArray(p.ruledHousesD1)) {
            throw new Error(`${fn}: ruledHousesD1 грахи ${p.key} не массив и не null`);
        }
        byKey[p.key] = p;
    }
    for (const key of Object.values(RU_NAME_TO_KEY)) {
        if (!byKey[key]) throw new Error(`${fn}: в карте нет грахи ${key}`);
    }
    return byKey;
}

function buildYogaContext(byKey: Record<string, DetectPlanet>, lagnaSignIdx: number): YogaContext {
    const signIdxOfHouse = (houseD1: number) => (lagnaSignIdx + houseD1 - 1) % 12;
    const aspectsOf = (key: string): number[] => drishtiHouses(key, byKey[key].houseD1);
    return {
        planets: byKey,
        lagnaSignIdx,
        signIdxOf: (key) => signIdxOfHouse(byKey[key].houseD1),
        signIdxOfHouse,
        houseFromMoon: (key) => houseFrom(byKey.Moon.houseD1, byKey[key].houseD1),
        houseFromSun: (key) => houseFrom(byKey.Sun.houseD1, byKey[key].houseD1),
        lordOfHouse: (house) => {
            const lord = Object.values(byKey).find((p) => p.ruledHousesD1?.includes(house));
            if (!lord) throw new Error(`detect: управитель дома ${house} не найден в карте`);
            return lord;
        },
        sambandha: (a, b) => {
            const pa = requirePlanet(byKey, a);
            const pb = requirePlanet(byKey, b);
            if (pa.houseD1 === pb.houseD1) return true; // соединение = один знак
            if (aspectsOf(a).includes(pb.houseD1) && aspectsOf(b).includes(pa.houseD1)) return true;
            // обмен знаками: a стоит в знаке b, b — в знаке a
            return signLordOf(signIdxOfHouse(pa.houseD1)) === b && signLordOf(signIdxOfHouse(pb.houseD1)) === a;
        },
        occupied: (fromHouse, n, keys) =>
            keys.some((k) => houseFrom(fromHouse, byKey[k].houseD1) === n),
    };
}

// ── Даш-правила (DasaSpecialRules) ───────────────────────────────────────────

/**
 * Универсальные правила периодов: применимы к любой карте (условие в записи
 * пустое, содержание — как трактовать даши управителей конкретных домов).
 */
export const UNIVERSAL_DASHA_RULES: string[] = [
    "Lord6And8Dasa",
    "Lord5And9Dasa",
    "Lord5And9DasaBhukti",
    "BhuktiDasaLordInBadHouses",
    "Lord2Dasa",
    "Lord3Dasa",
    "LagnaLordDasa",
    "Saturn4thDasa",
    "Jupiter6thDasa",
];

/** Солнечные правила: применимы, только если состояние Солнца в карте совпадает. */
export const SUN_DASHA_RULES: Record<string, (ctx: YogaContext) => boolean> = {
    ElevatedSunDasa: (c) => c.planets.Sun.dignity === "exalted",
    // «Sun in good position, in own house or joined with lord of 9 or 10».
    SunWithLord9Or10Dasa: (c) => {
        const sun = c.planets.Sun;
        if (sun.dignity === "own_sign" || sun.dignity === "moolatrikona") return true;
        return [9, 10].some((n) => {
            const lord = c.lordOfHouse(n);
            return lord.key !== "Sun" && lord.houseD1 === sun.houseD1;
        });
    },
    // «С управителем дома» = соединение с ДРУГОЙ планетой; если Солнце само
    // управитель дома (5-й для Овна/Льва, 2-й для Рака) — соединение с самим
    // собой тривиально и правило не считается выполненным.
    SunWithLord5Dasa: (c) => {
        const lord5 = c.lordOfHouse(5);
        return lord5.key !== "Sun" && lord5.houseD1 === c.planets.Sun.houseD1;
    },
    SunWithLord2Dasa: (c) => {
        const lord2 = c.lordOfHouse(2);
        return lord2.key !== "Sun" && c.sambandha("Sun", lord2.key);
    },
    SunBadPositionDasa: (c) => {
        const sun = c.planets.Sun;
        if (sun.dignity === "debilitated" || [6, 8].includes(sun.houseD1)) return true;
        return Object.values(c.planets).some((p) =>
            p.key !== "Sun" && p.houseD1 === sun.houseD1 && MALEFICS.has(p.key)
        );
    },
};

/** Детекция даш-правил: универсальные (всегда) + солнечные (по состоянию Солнца). */
export function detectDashaRules(planets: DetectPlanet[], lagnaSignIdx: number): string[] {
    validateLagna(lagnaSignIdx, "detectDashaRules");
    const byKey = validatePlanets(planets, "detectDashaRules");
    const ctx = buildYogaContext(byKey, lagnaSignIdx);
    return [
        ...UNIVERSAL_DASHA_RULES,
        ...Object.keys(SUN_DASHA_RULES).filter((id) => SUN_DASHA_RULES[id](ctx)),
    ];
}

// ── Gochara (транзиты от Луны) ───────────────────────────────────────────────

/** Срез транзитной грахи из calc_chart.output.transits.grahas. */
export interface DetectTransit {
    name: string;
    houseTransited: number;
}

/**
 * Транзитные записи Gochara считаются от знака рождения Луны (подтверждено
 * текстами записей). Дом-от-Луны транзитной грахи = ((house_transited −
 * house_Луны + 12) mod 12) + 1, где house_transited — дом НАТАЛЬНОЙ карты от
 * лагны, который граха проходит (поле house_transited в transits.grahas).
 */
export function detectGochara(planets: DetectPlanet[], transits: DetectTransit[]): string[] {
    const moonHouse = planets.find((p) => p.key === "Moon")?.houseD1;
    if (!Number.isInteger(moonHouse)) {
        throw new Error("detectGochara: в карте нет Луны — гочара не определяется");
    }
    return transits.map((t) => {
        const key = RU_NAME_TO_KEY[t.name];
        if (!key) {
            throw new Error(`detectGochara: неизвестная транзитная граха "${t.name}"`);
        }
        if (!Number.isInteger(t.houseTransited) || t.houseTransited < 1 || t.houseTransited > 12) {
            throw new Error(`detectGochara: дом транзита "${t.name}" вне 1..12 (${String(t.houseTransited)})`);
        }
        return `${key}GocharaInHouse${houseFrom(moonHouse!, t.houseTransited)}`;
    });
}
