/**
 * Расчёт ведической натальной карты из файла .jhd + текущие транзиты.
 *
 * Контракт тулов графа (как у tarot-master): один argv-токен — плоский JSON
 * строк `{"jhd_path": "..."}`; на stdout — ТОЛЬКО JSON-объект; любая ошибка —
 * сообщение в stderr + exit 1 (честный отказ, workflow останавливается).
 *
 * Астрономия: astronomy-engine (вендор в ./vendor, VSOP87). Система расчёта
 * (планетная модель, айанамша, год даши) выбирается ОДНОЙ строкой в
 * vedic_system.ts (VEDIC_SYSTEM; дефолт "drik": Drik + Lahiri + 365.2425).
 * Эталон алгоритмов и констант — исходный драфт parse_and_calc.rs
 * (git-история: коммит 0434d65).
 */

import { AstroTime, e_tilt, SiderealTime, GeoVector, Ecliptic, Body } from "./vendor/astronomy.js";
import { J2000, DEG_TO_RAD, degnorm, jdUtToJdEt } from "./ephemeris_time.ts";
import { VEDIC_SYSTEM_CONFIG } from "./vedic_system.ts";
import { parseJhd, JhdInput } from "./jhd_parser.ts";
import {
    SIGNS,
    buildPointInfo,
    calculateVimshottari,
    chainToPlain,
    findActiveDashaChain,
    generateSubPeriods,
    PointInfo,
} from "./vedic_math.ts";
import { tropicalAscendant } from "./houses.ts";

/**
 * AstroTime из (jdUt, jdEt): astronomy-engine принимает UT в J2000-днях,
 * а позиции тел считает по TT (поле tt) — подставляем наш ΔT (Espenak-Meeus),
 * чтобы совпадать со швейцарской эфемеридой (swe_calc_ut = UT + ΔT).
 */
function mkTime(jdUt: number, jdEt: number): AstroTime {
    const time = new AstroTime(jdUt - J2000);
    time.tt = jdEt - J2000;
    return time;
}

/** Порядок грах как в эталоне: Солнце..Сатурн, Раху, Кету. */
const BODIES: { key: string; name: string; astro?: Body }[] = [
    { key: "Sun", name: "Солнце", astro: Body.Sun },
    { key: "Moon", name: "Луна", astro: Body.Moon },
    { key: "Mars", name: "Марс", astro: Body.Mars },
    { key: "Mercury", name: "Меркурий", astro: Body.Mercury },
    { key: "Jupiter", name: "Юпитер", astro: Body.Jupiter },
    { key: "Venus", name: "Венера", astro: Body.Venus },
    { key: "Saturn", name: "Сатурн", astro: Body.Saturn },
    { key: "Rahu", name: "Раху" },
    { key: "Ketu", name: "Кету" },
];

/** Средний лунный узел (тропический, Meeus 47.7 — эквивалент SE_MEAN_NODE). */
function meanNodeLongitude(jdEt: number): number {
    const T = (jdEt - 2451545.0) / 36525.0;
    return (
        125.0445479
        - 1934.1362891 * T
        + 0.0020754 * T * T
        + (T * T * T) / 467441.0
        - (T * T * T * T) / 60616000.0
    );
}

function r4(x: number): number {
    return Math.round(x * 10000) / 10000;
}

/** Тропическая геоцентрическая эклиптическая долгота тела на момент jdEt. */
function tropicalLongitude(body: Body, jdUt: number, jdEt: number): number {
    const time = mkTime(jdUt, jdEt);
    const vec = GeoVector(body, time, true);
    return Ecliptic(vec).elon;
}

/** Сидерическая долгота грахи + суточная скорость (град/сут). */
function siderealPosition(
    bodyDef: (typeof BODIES)[number],
    jdUt: number,
    jdEt: number,
    ayanamsa: number,
): { lon: number; speed: number } {
    if (bodyDef.key === "Rahu") {
        // Средний узел: секулярный полином, движение всегда попятное.
        const lon = degnorm(meanNodeLongitude(jdEt) - ayanamsa);
        return { lon, speed: -0.0529539 };
    }
    if (bodyDef.key === "Ketu") {
        return { lon: NaN, speed: -0.0529539 }; // заполняется после Раху
    }
    const body = bodyDef.astro!;
    const lonCenter = tropicalLongitude(body, jdUt, jdEt);
    const lonBefore = tropicalLongitude(body, jdUt - 0.5, jdEt - 0.5);
    const lonAfter = tropicalLongitude(body, jdUt + 0.5, jdEt + 0.5);
    const speed = lonAfter - lonBefore; // град/сут (через ±12ч)
    return { lon: degnorm(lonCenter - ayanamsa), speed };
}

function fail(message: string): never {
    console.error(`calc_chart: ${message}`);
    Deno.exit(1);
}

async function main() {
    // ── 1. Аргументы и чтение .jhd ──
    const rawArgs = Deno.args[0];
    if (!rawArgs) fail("не переданы аргументы (ожидается JSON с jhd_path)");
    let args: { jhd_path?: string };
    try {
        args = JSON.parse(rawArgs);
    } catch {
        fail(`аргументы не JSON: ${rawArgs.slice(0, 120)}`);
    }
    const jhdPath = (args.jhd_path ?? "").trim();
    if (!jhdPath) fail("аргумент jhd_path пуст — путь к файлу .jhd обязателен");

    let content: string;
    try {
        content = await Deno.readTextFile(jhdPath);
    } catch (e) {
        fail(`не удалось прочитать файл ${jhdPath}: ${e instanceof Error ? e.message : String(e)}`);
    }
    let input: JhdInput;
    try {
        input = parseJhd(content);
    } catch (e) {
        fail(`файл ${jhdPath} не разобран: ${e instanceof Error ? e.message : String(e)}`);
    }

    // ── 2. Система, время и айанамша ──
    // Честный отказ вместо тихо неверных чисел: SSS-планетная модель
    // (эпициклы Сурья-сиддханты) ещё не реализована — см. vedic_system.ts.
    if (VEDIC_SYSTEM_CONFIG.planetaryModel === "sss") {
        fail("планетная модель Sri Surya Siddhanta ещё не реализована (см. vedic_system.ts); переключите VEDIC_SYSTEM на \"drik\"");
    }
    const jdUt = input.julianDayUt;
    const jdEt = jdUtToJdEt(jdUt);
    const ayanamsa = VEDIC_SYSTEM_CONFIG.ayanamsa(jdUt);

    // ── 3. Лагна: RAMC (видимое звёздное время + долгота) и истинная наклонность ──
    const gastHours = SiderealTime(mkTime(jdUt, jdEt));
    const armc = degnorm(gastHours * 15.0 + input.longitudeEast);
    // e_tilt().tobl — истинная наклонность в ГРАДУСАХ (см. vendor: true_ob = mean_ob + deps/3600)
    const epsTrueRad = e_tilt(mkTime(jdUt, jdEt)).tobl * DEG_TO_RAD;
    const ascTropical = tropicalAscendant(armc, epsTrueRad, input.latitudeNorth);
    const lagnaLon = degnorm(ascTropical - ayanamsa);

    const lagnaD1Sign = Math.floor(lagnaLon / 30.0);
    const lagnaD9Sign = Math.floor(((lagnaLon * 108.0) / 360.0)) % 12;
    const lagna = buildPointInfo("Лагна (Asc)", lagnaLon, false, lagnaD1Sign, lagnaD9Sign);

    // ── 4. Грахи ──
    const positions = new Map<string, { lon: number; speed: number }>();
    for (const bodyDef of BODIES) {
        const pos = siderealPosition(bodyDef, jdUt, jdEt, ayanamsa);
        positions.set(bodyDef.key, pos);
    }
    const rahuLon = positions.get("Rahu")!.lon;
    positions.set("Ketu", { lon: degnorm(rahuLon + 180.0), speed: positions.get("Rahu")!.speed });

    const planets: PointInfo[] = BODIES.map((bodyDef) => {
        const pos = positions.get(bodyDef.key)!;
        const isRetro = bodyDef.key === "Rahu" || bodyDef.key === "Ketu" ? true : pos.speed < 0;
        return buildPointInfo(bodyDef.name, pos.lon, isRetro, lagnaD1Sign, lagnaD9Sign);
    });
    const moonLon = positions.get("Moon")!.lon;

    // ── 5. Дома D1/D9 (Whole Sign от лагны) ──
    const houseList = (fromSign: number, key: "houseD1" | "houseD9") => {
        const houses = [];
        for (let h = 1; h <= 12; h++) {
            const signIdx = (fromSign + h - 1) % 12;
            houses.push({
                house_number: h,
                sign_idx: signIdx,
                sign_name: "", // заполняется ниже из SIGNS
                planets: planets.filter((p) => p[key] === h).map((p) => p.name),
            });
        }
        return houses;
    };
    const housesD1 = houseList(lagnaD1Sign, "houseD1").map((h) => ({ ...h, sign_name: SIGNS[h.sign_idx] }));
    const housesD9 = houseList(lagnaD9Sign, "houseD9").map((h) => ({ ...h, sign_name: SIGNS[h.sign_idx] }));

    // ── 6. Вимшоттари: 9 махадаш + активная цепочка до прана-даши ──
    const nowMs = Date.now();
    const dashaTree = calculateVimshottari(moonLon, input.birthUtcMs, 3, VEDIC_SYSTEM_CONFIG.dashaYearDays);
    let activeChain = findActiveDashaChain(dashaTree, nowMs);
    // Достраиваем SD и PAD только вдоль активной ветки (всё дерево 5 уровней — 53к узлов).
    if (activeChain.length === 3) {
        const pd = activeChain[2];
        const pdDays = (pd.endMs - pd.startMs) / 86_400_000.0;
        const sds = generateSubPeriods(4, 4, pd.lordIdx, pd.startMs, pdDays);
        const activeSd = sds.find((p) => nowMs >= p.startMs && nowMs < p.endMs);
        if (activeSd) {
            const sdDays = (activeSd.endMs - activeSd.startMs) / 86_400_000.0;
            const pads = generateSubPeriods(5, 5, activeSd.lordIdx, activeSd.startMs, sdDays);
            const activePad = pads.find((p) => nowMs >= p.startMs && nowMs < p.endMs);
            activeChain = [...activeChain, activeSd, ...(activePad ? [activePad] : [])].filter(Boolean);
        } else {
            activeChain = [...activeChain, ...sds.filter((p) => nowMs >= p.startMs)].slice(0, 3);
        }
    }

    // ── 7. Транзиты: положения грах на текущий момент ──
    const jdUtNow = Date.now() / 1000.0 / 86400.0 + 2440587.5;
    const jdEtNow = jdUtToJdEt(jdUtNow);
    const ayanamsaNow = VEDIC_SYSTEM_CONFIG.ayanamsa(jdUtNow);
    const transits = BODIES.map((bodyDef) => {
        let sidLon: number;
        let retro: boolean;
        if (bodyDef.key === "Rahu" || bodyDef.key === "Ketu") {
            const base = bodyDef.key === "Rahu"
                ? meanNodeLongitude(jdEtNow)
                : meanNodeLongitude(jdEtNow) + 180.0;
            sidLon = degnorm(base - ayanamsaNow);
            retro = true;
        } else {
            const lonCenter = tropicalLongitude(bodyDef.astro!, jdUtNow, jdEtNow);
            const lonBefore = tropicalLongitude(bodyDef.astro!, jdUtNow - 0.5, jdEtNow - 0.5);
            const lonAfter = tropicalLongitude(bodyDef.astro!, jdUtNow + 0.5, jdEtNow + 0.5);
            sidLon = degnorm(lonCenter - ayanamsaNow);
            retro = lonAfter - lonBefore < 0;
        }
        const info = buildPointInfo(bodyDef.name, sidLon, retro, lagnaD1Sign, lagnaD9Sign);
        return {
            name: info.name,
            longitude: r4(info.longitude),
            sign: info.signD1Name,
            degree_in_sign: r4(info.degreeInSign),
            deg: info.deg,
            min: info.min,
            sec: Math.round(info.sec * 10) / 10,
            retro: info.isRetrograde,
            nakshatra: info.nakshatraName,
            pada: info.pada,
            /** Дом НАТАЛЬНОЙ карты, который транзитная граха проходит. */
            house_transited: info.houseD1,
        };
    });

    // ── 8. JSON на stdout ──
    const roundPoint = (p: PointInfo) => ({
        name: p.name,
        longitude: r4(p.longitude),
        retro: p.isRetrograde,
        sign: p.signD1Name,
        degree_in_sign: r4(p.degreeInSign),
        deg: p.deg,
        min: p.min,
        sec: Math.round(p.sec * 10) / 10,
        nakshatra: p.nakshatraName,
        pada: p.pada,
        house_d1: p.houseD1,
        navamsha_sign: p.signD9Name,
        house_d9: p.houseD9,
    });

    const output = {
        tool: "calc_chart",
        birth: {
            city: input.city,
            country: input.country,
            date_local: `${String(input.day).padStart(2, "0")}.${String(input.month).padStart(2, "0")}.${input.year}`,
            hour_local: input.hourLocal,
            timezone_offset: input.timezoneOffset,
            longitude_east: input.longitudeEast,
            latitude_north: input.latitudeNorth,
            utc_iso: input.birthUtcIso,
            jd_ut: r4(jdUt),
        },
        system: {
            key: VEDIC_SYSTEM_CONFIG.system,
            name: VEDIC_SYSTEM_CONFIG.name,
            planetary_model: VEDIC_SYSTEM_CONFIG.planetaryModel,
        },
        ayanamsha: {
            name: VEDIC_SYSTEM_CONFIG.ayanamshaName,
            value: r4(ayanamsa),
        },
        lagna: roundPoint(lagna),
        planets: planets.map(roundPoint),
        houses_d1: housesD1,
        houses_d9: housesD9,
        dasha: {
            year_days_setting: VEDIC_SYSTEM_CONFIG.dashaYearName,
            maha_dashas: dashaTree.map((md) => ({
                lord: md.lordName,
                start: new Date(md.startMs).toISOString(),
                end: new Date(md.endMs).toISOString(),
            })),
            active_chain: chainToPlain(activeChain),
            now_iso: new Date(nowMs).toISOString(),
        },
        transits: {
            now_iso: new Date(nowMs).toISOString(),
            jd_ut: r4(jdUtNow),
            ayanamsha: r4(ayanamsaNow),
            grahas: transits,
        },
    };
    console.log(JSON.stringify(output));
}

await main();
