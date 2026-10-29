/**
 * Аналитический слой джйотиш поверх позиций грах: функциональные управители
 * домов (от лагны), граха-дришти (включая спец-аспекты Марса/Юпитера/Сатурна),
 * достоинства (exaltation/moolatrikona/own/debilitation), сожжение, варготтама
 * и чаракараки (Джаймини).
 *
 * Назначение — предрасчёт ДО отправки в LLM (см. AGENTS.md, правило 00.2):
 * модель не обязана помнить, какими домами владеет планета для конкретной
 * лагны и куда падают спец-аспекты. Все функции чистые и детерминированные,
 * эфемерид не касаются — тестируются на фиксированных числах.
 *
 * Источники таблиц (классическая доктрина Парашары, BPHS):
 *  - владения знаками, экзальтация/падение, муллатрикона (градусные диапазоны),
 *  - дришти: все грахи смотрят в 7-й дом от себя; Марс — 4/7/8, Юпитер — 5/7/9,
 *    Сатурн — 3/7/10 (спец-аспекты ЗАМЕЩАЮТ общий 7-й, не добавляются к нему);
 *  - орбисы сожжения: Луна 12°, Марс 17°, Меркурий 14° (ретро 12°), Юпитер 11°,
 *    Венера 10° (ретро 8°), Сатурн 15° — расстояние по долготе до Солнца.
 * Раху/Кету: владений и достоинств не имеют (школы расходятся — честный null,
 * а не выбранное мнение); сожжение узлов не определяется.
 */

export type Dignity = "exalted" | "moolatrikona" | "own_sign" | "debilitated" | "neutral";

export type KarakaScheme = "seven" | "eight";

/** Канонический порядок грах — тай-брейк чаракарак при равных градусах. */
const CANONICAL_ORDER = ["Sun", "Moon", "Mars", "Mercury", "Jupiter", "Venus", "Saturn", "Rahu"];

/** Знаки-владения семи грах (индексы 0..11 от Овна). */
const OWN_SIGNS: Record<string, number[]> = {
    Sun: [4],            // Лев
    Moon: [3],           // Рак
    Mars: [0, 7],        // Овен, Скорпион
    Mercury: [2, 5],     // Близнецы, Дева
    Jupiter: [8, 11],    // Стрелец, Рыбы
    Venus: [1, 6],       // Телец, Весы
    Saturn: [9, 10],     // Козерог, Водолей
};

/** Знак экзальтации; падение = (экзальтация + 6) % 12. */
const EXALT_SIGN: Record<string, number> = {
    Sun: 0, Moon: 1, Mars: 9, Mercury: 5, Jupiter: 3, Venus: 11, Saturn: 6,
};

/** Муллатрикона: знак + диапазон градусов в знаке [from, to). */
const MOOLATRIKONA: Record<string, { sign: number; from: number; to: number }> = {
    Sun: { sign: 4, from: 0, to: 20 },
    Moon: { sign: 1, from: 3, to: 30 },
    Mars: { sign: 0, from: 0, to: 12 },
    Mercury: { sign: 5, from: 15, to: 20 },
    Jupiter: { sign: 8, from: 0, to: 10 },
    Venus: { sign: 6, from: 0, to: 15 },
    Saturn: { sign: 10, from: 0, to: 20 },
};

/** Орбисы сожжения в градусах дуги до Солнца (ретро-орбис — для Меркурия/Венеры). */
const COMBUST_ORB: Record<string, { direct: number; retro: number }> = {
    Moon: { direct: 12, retro: 12 },
    Mars: { direct: 17, retro: 17 },
    Mercury: { direct: 14, retro: 12 },
    Jupiter: { direct: 11, retro: 11 },
    Venus: { direct: 10, retro: 8 },
    Saturn: { direct: 15, retro: 15 },
};

/** Спец-аспекты (смещения домов от своего); остальные грахи — только 7-й. */
const SPECIAL_ASPECTS: Record<string, number[]> = {
    Mars: [4, 7, 8],
    Jupiter: [5, 7, 9],
    Saturn: [3, 7, 10],
};

const KARAKA_SEQUENCE: Record<KarakaScheme, string[]> = {
    seven: ["AK", "AmK", "BK", "MK", "PiK", "PK", "DK"],
    eight: ["AK", "AmK", "BK", "MK", "PiK", "PK", "GK", "DK"],
};

/**
 * Дома, которыми планета владеет при данной лагне (1..12). Раху/Кету владений
 * не имеют → null. Формула: дом знака-владения s от лагны L = ((s−L+12) mod 12)+1.
 */
export function housesOwned(lagnaSignIdx: number, planetKey: string): number[] | null {
    const owned = OWN_SIGNS[planetKey];
    if (!owned) return null;
    return owned
        .map((s) => ((s - lagnaSignIdx + 12) % 12) + 1)
        .sort((a, b) => a - b);
}

/** Управитель знака — инверсия OWN_SIGNS (таблица существует ровно в одном месте). */
const SIGN_LORDS: string[] = (() => {
    const lords = new Array<string>(12).fill("");
    for (const [planet, signs] of Object.entries(OWN_SIGNS)) {
        for (const s of signs) lords[s] = planet;
    }
    return lords;
})();

/** Управитель знака (индекс 0..11 от Овна) — ключ грахи. */
export function signLordOf(signIdx: number): string {
    const lord = SIGN_LORDS[signIdx];
    if (!lord) {
        throw new Error(`signLordOf: для знака ${signIdx} нет управителя — таблица OWN_SIGNS неполна`);
    }
    return lord;
}

/**
 * Граха-дришти: номера домов (1..12), на которые смотрит граха из дома houseD1.
 * k-й дом от h = ((h+k−2) mod 12)+1 (k=1 — тот же дом; 7-й — общий аспект).
 */
export function drishtiHouses(planetKey: string, houseD1: number): number[] {
    const offsets = SPECIAL_ASPECTS[planetKey] ?? [7];
    return offsets
        .map((k) => ((houseD1 - 1 + (k - 1)) % 12) + 1)
        .sort((a, b) => a - b);
}

/** Имена грах, стоящих в аспектируемых домах (порядок — порядок входного массива). */
export function aspectedPlanetNames(
    houses: number[],
    planets: { name: string; houseD1: number }[],
    selfName: string,
): string[] {
    return planets
        .filter((p) => p.name !== selfName && houses.includes(p.houseD1))
        .map((p) => p.name);
}

/**
 * Достоинство грахи в знаке. Порядок проверки: муллатрикона (градусный диапазон,
 * частный случай своего знака) → экзальтация → падение → свой знак → нейтрально.
 * Раху/Кету → null (школы расходятся — мнение кода не навязывается).
 */
export function dignityOf(planetKey: string, signIdx: number, degInSign: number): Dignity | null {
    const exalt = EXALT_SIGN[planetKey];
    if (exalt === undefined) return null;
    const mt = MOOLATRIKONA[planetKey];
    if (signIdx === mt.sign && degInSign >= mt.from && degInSign < mt.to) return "moolatrikona";
    if (signIdx === exalt) return "exalted";
    if (signIdx === (exalt + 6) % 12) return "debilitated";
    if (OWN_SIGNS[planetKey].includes(signIdx)) return "own_sign";
    return "neutral";
}

/**
 * Сожжение: дуга до Солнца меньше орбиса (для Меркурия/Венеры — ретро-орбис).
 * Солнце само по определению не сожжено (false); Раху/Кету — null.
 */
export function isCombust(
    planetKey: string,
    lon: number,
    sunLon: number,
    isRetro: boolean,
): boolean | null {
    if (planetKey === "Sun") return false;
    const orb = COMBUST_ORB[planetKey];
    if (!orb) return null;
    const diff = Math.abs(lon - sunLon);
    const arc = Math.min(diff, 360 - diff);
    return arc < (isRetro ? orb.retro : orb.direct);
}

/** Варготтама: знак D1 совпадает со знаком D9. */
export function isVargottama(signD1Idx: number, signD9Idx: number): boolean {
    return signD1Idx === signD9Idx;
}

export interface CharaKarakaInput {
    /** Латинский ключ грахи ("Sun".."Saturn", "Rahu", "Ketu"). */
    key: string;
    /** Отображаемое имя (русское — идёт в JSON карты). */
    name: string;
    /** Градус в знаке 0..30 (для Раху инверсируется внутри). */
    degreeInSign: number;
}

export interface CharaKarakasResult {
    scheme: KarakaScheme;
    /** karaka → имя грахи: AK, AmK, BK, MK, PiK, PK, GK (только eight), DK. */
    karakas: Record<string, string>;
}

/**
 * Чаракараки (Джаймини) по убыванию градуса в знаке: AK (Атмакарака) …
 * DK (Даракарака). Схема "eight" (дефолт, 1-в-1 с Jagannatha Hora) включает
 * Раху с перевёрнутым градусом (30° − положение в знаке); "seven" (школа
 * К.Н. Рао) — без Раху. Кету не участвует ни в одной схеме. Тай-брейк при
 * равных градусах — канонический порядок грах.
 */
export function charaKarakas(
    inputs: CharaKarakaInput[],
    scheme: KarakaScheme,
): CharaKarakasResult {
    const sequence = KARAKA_SEQUENCE[scheme];
    const eligible = inputs
        .filter((p) => p.key !== "Ketu" && (scheme === "seven" ? p.key !== "Rahu" : true))
        .map((p) => ({
            key: p.key,
            name: p.name,
            degree: p.key === "Rahu" ? 30 - p.degreeInSign : p.degreeInSign,
        }));
    if (eligible.length !== sequence.length) {
        throw new Error(
            `charaKarakas: ожидалось ${sequence.length} грах для схемы "${scheme}", получено ${eligible.length}`,
        );
    }
    eligible.sort((a, b) => {
        const byDegree = b.degree - a.degree;
        if (byDegree !== 0) return byDegree;
        return CANONICAL_ORDER.indexOf(a.key) - CANONICAL_ORDER.indexOf(b.key);
    });
    const karakas: Record<string, string> = {};
    sequence.forEach((karaka, i) => {
        karakas[karaka] = eligible[i].name;
    });
    return { scheme, karakas };
}

export interface PlanetInput {
    key: string;
    name: string;
    /** Сидерическая долгота 0..360 (для сожжения). */
    lon: number;
    isRetro: boolean;
    signD1Idx: number;
    signD9Idx: number;
    houseD1: number;
    degreeInSign: number;
}

/** Аналитические поля, добавляемые к каждому объекту planets[] в выводе calc_chart. */
export interface PlanetAnalysis {
    ruled_houses_d1: number[] | null;
    aspects_houses_d1: number[];
    aspects_planets: string[];
    dignity: Dignity | null;
    combust: boolean | null;
    is_vargottama: boolean;
}

export interface ChartAnalysis {
    /** key грахи → анализ. */
    planets: Record<string, PlanetAnalysis>;
    chara_karakas: CharaKarakasResult;
}

/**
 * Полный анализ карты: управители домов, дришти, достоинства, сожжение,
 * варготтама для каждой грахи + чаракараки карты.
 */
export function buildChartAnalysis(
    planets: PlanetInput[],
    lagnaSignIdx: number,
    scheme: KarakaScheme,
): ChartAnalysis {
    const sun = planets.find((p) => p.key === "Sun");
    if (!sun) throw new Error("buildChartAnalysis: в карте нет Солнца — сожжение не определяется");
    const perPlanet: Record<string, PlanetAnalysis> = {};
    for (const p of planets) {
        const houses = drishtiHouses(p.key, p.houseD1);
        perPlanet[p.key] = {
            ruled_houses_d1: housesOwned(lagnaSignIdx, p.key),
            aspects_houses_d1: houses,
            aspects_planets: aspectedPlanetNames(houses, planets, p.name),
            dignity: dignityOf(p.key, p.signD1Idx, p.degreeInSign),
            combust: isCombust(p.key, p.lon, sun.lon, p.isRetro),
            is_vargottama: isVargottama(p.signD1Idx, p.signD9Idx),
        };
    }
    return {
        planets: perPlanet,
        chara_karakas: charaKarakas(
            planets.map((p) => ({ key: p.key, name: p.name, degreeInSign: p.degreeInSign })),
            scheme,
        ),
    };
}
