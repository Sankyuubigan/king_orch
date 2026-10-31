/**
 * Математика джйотиш: знаки, накшатры, пады, навамша (D9), дома (Whole Sign)
 * и дерево Вимшоттари-даш. Порт 1:1 из исходного драфта
 * parse_and_calc.rs (git-история: коммит 0434d65, файл
 * agents/astrologist/parse_and_calc.rs — build_planet_info, calculate_vimshottari,
 * generate_sub_periods, find_active_dasha_chain).
 *
 * Константы доменной логики (длина года даши, навамша через непрерывный счёт
 * пад) зафиксированы эталоном и НЕ меняются без сверки с JHora (гейт точности).
 */

export const SIGNS: string[] = [
    "Овен", "Телец", "Близнецы", "Рак", "Лев", "Дева",
    "Весы", "Скорпион", "Стрелец", "Козерог", "Водолей", "Рыбы",
];

export const NAKSHATRAS: string[] = [
    "Ашвини", "Бхарани", "Криттика", "Рохини", "Мригашира", "Ардра",
    "Пунарвасу", "Пушья", "Ашлеша", "Магха", "Пурва Пхалгуни", "Уттара Пхалгуни",
    "Хаста", "Читра", "Свати", "Вишакха", "Анурадха", "Джйештха",
    "Мула", "Пурва Ашадха", "Уттара Ашадха", "Шравана", "Дхаништха", "Шатабхиша",
    "Пурва Бхадрапада", "Уттара Бхадрапада", "Ревати",
];

export const DASHA_LORDS: string[] = [
    "Кету", "Венера", "Солнце", "Луна", "Марс", "Раху", "Юпитер", "Сатурн", "Меркурий",
];

/** Длительности махадаш в годах (сумма = 120). */
export const DASHA_YEARS: number[] = [7.0, 20.0, 6.0, 10.0, 7.0, 18.0, 16.0, 19.0, 17.0];

/**
 * Длина года даши Sri Surya Siddhanta: «Year with 360 tithis»
 * (1 титхи = 0.9843529667 сут). Значение по умолчанию для
 * calculateVimshottari (обратная совместимость с эталоном/фикстурами);
 * боевой тул передаёт год из пресета системы (vedic_system.ts):
 * drik → 365.2425, sss → это значение.
 */
export const DASHA_YEAR_DAYS = 354.3670680;

export interface PointInfo {
    name: string;
    /** Сидерическая долгота 0..360. */
    longitude: number;
    isRetrograde: boolean;
    /** Индекс знака D1 (0..11). */
    signD1Idx: number;
    signD1Name: string;
    degreeInSign: number;
    deg: number;
    min: number;
    sec: number;
    /** Индекс накшатры (0..26). */
    nakshatraIdx: number;
    nakshatraName: string;
    /** Пада накшатры (1..4). */
    pada: number;
    /** Дом в Раши D1 (1..12, Whole Sign от лагны). */
    houseD1: number;
    /** Индекс знака в навамше D9 (0..11). */
    signD9Idx: number;
    signD9Name: string;
    /** Дом в навамше D9 (1..12). */
    houseD9: number;
}

export interface HouseInfo {
    houseNumber: number;
    signIdx: number;
    signName: string;
    planets: string[];
}

export interface DashaPeriod {
    level: number; // 1=MD, 2=AD, 3=PD, 4=SD, 5=PAD
    lordIdx: number;
    lordName: string;
    startMs: number; // epoch ms UTC
    endMs: number;
    subPeriods: DashaPeriod[];
}

/** Десятичные градусы → целые градусы/минуты/секунды внутри знака (порт to_dms). */
export function toDms(degDecimal: number): { deg: number; min: number; sec: number } {
    const d = Math.floor(degDecimal);
    const mFull = (degDecimal - d) * 60.0;
    const m = Math.floor(mFull);
    const s = (mFull - m) * 60.0;
    return { deg: d, min: m, sec: s };
}

/**
 * Сборка описания точки (планета/лагна) из сидерической долготы.
 * Порт build_planet_info: накшатра = floor(lon*27/360), пада — из
 * floor(lon*108/360), навамша = непрерывный счёт пад от 0° Овна (mod 12).
 */
export function buildPointInfo(
    name: string,
    lon: number,
    isRetro: boolean,
    lagnaD1Sign: number,
    lagnaD9Sign: number,
): PointInfo {
    const normLon = ((lon % 360.0) + 360.0) % 360.0;
    const signD1Idx = Math.floor(normLon / 30.0);
    const degreeInSign = normLon % 30.0;
    const { deg, min, sec } = toDms(degreeInSign);

    const nakshatraIdx = Math.floor((normLon * 27.0) / 360.0);
    const totalPada = Math.floor((normLon * 108.0) / 360.0);
    const pada = (totalPada % 4) + 1;

    // Навамша (D9): каждая пада накшатры = один знак D9 от 0° Овна.
    const signD9Idx = totalPada % 12;

    // Дома (Whole Sign / Раши = Дом).
    const houseD1 = ((signD1Idx + 12 - lagnaD1Sign) % 12) + 1;
    const houseD9 = ((signD9Idx + 12 - lagnaD9Sign) % 12) + 1;

    return {
        name,
        longitude: normLon,
        isRetrograde: isRetro,
        signD1Idx,
        signD1Name: SIGNS[signD1Idx],
        degreeInSign,
        deg,
        min,
        sec,
        nakshatraIdx,
        nakshatraName: NAKSHATRAS[nakshatraIdx],
        pada,
        houseD1,
        signD9Idx,
        signD9Name: SIGNS[signD9Idx],
        houseD9,
    };
}

/** Рекурсивная генерация подпериодов (AD → PD → SD → PAD), порт generate_sub_periods. */
export function generateSubPeriods(
    currentLevel: number,
    maxLevel: number,
    startLordIdx: number,
    parentStartMs: number,
    parentDurationDays: number,
): DashaPeriod[] {
    if (currentLevel > maxLevel) {
        return [];
    }
    const periods: DashaPeriod[] = [];
    let cursorMs = parentStartMs;
    for (let i = 0; i < 9; i++) {
        const lordIdx = (startLordIdx + i) % 9;
        const subDurationDays = parentDurationDays * (DASHA_YEARS[lordIdx] / 120.0);
        const durationMs = Math.round(subDurationDays * 86_400_000.0);
        const endMs = cursorMs + durationMs;
        periods.push({
            level: currentLevel,
            lordIdx,
            lordName: DASHA_LORDS[lordIdx],
            startMs: cursorMs,
            endMs,
            subPeriods: generateSubPeriods(currentLevel + 1, maxLevel, lordIdx, cursorMs, subDurationDays),
        });
        cursorMs = endMs;
    }
    return periods;
}

/**
 * Дерево Вимшоттари от сидерической долготы Луны (порт calculate_vimshottari).
 * Начало первой махадши восстанавливается назад от рождения по доле
 * пройденной накшатры — все вложенные уровни идут точно.
 *
 * maxLevel=1 строит только 9 махадаш (компактно для JSON), 5 — вплоть до
 * прана-даши (осторожно: 9^5 узлов; для вывода юзеру используй
 * buildDashaSummary или maxLevel ≤ 3).
 */
export function calculateVimshottari(
    moonLon: number,
    birthUtcMs: number,
    maxLevel: number,
    /** Длина года даши в сутках; по умолчанию — год SSS «360 tithis». */
    yearDays: number = DASHA_YEAR_DAYS,
): DashaPeriod[] {
    const normMoon = ((moonLon % 360.0) + 360.0) % 360.0;
    const nakSpan = 360.0 / 27.0;
    const nakIdx = Math.floor(normMoon / nakSpan);
    const firstLordIdx = nakIdx % 9;

    const degInNak = normMoon % nakSpan;
    const fractionPassed = degInNak / nakSpan;

    const firstMdFullDays = DASHA_YEARS[firstLordIdx] * yearDays;
    const elapsedMs = Math.round(firstMdFullDays * fractionPassed * 86_400_000.0);
    let cursorMs = birthUtcMs - elapsedMs;

    const mahadashas: DashaPeriod[] = [];
    for (let i = 0; i < 9; i++) {
        const lordIdx = (firstLordIdx + i) % 9;
        const mdDays = DASHA_YEARS[lordIdx] * yearDays;
        const mdMs = Math.round(mdDays * 86_400_000.0);
        const endMs = cursorMs + mdMs;
        mahadashas.push({
            level: 1,
            lordIdx,
            lordName: DASHA_LORDS[lordIdx],
            startMs: cursorMs,
            endMs,
            subPeriods: generateSubPeriods(2, maxLevel, lordIdx, cursorMs, mdDays),
        });
        cursorMs = endMs;
    }
    return mahadashas;
}

/**
 * Активная цепочка периодов [MD, AD, PD, SD, PAD] на момент targetMs —
 * порт find_active_dasha_chain: спуск по дереву, каждый уровень одна ветка.
 */
export function findActiveDashaChain(tree: DashaPeriod[], targetMs: number): DashaPeriod[] {
    const chain: DashaPeriod[] = [];
    let slice = tree;
    while (slice.length > 0) {
        const period = slice.find((p) => targetMs >= p.startMs && targetMs < p.endMs);
        if (!period) break;
        chain.push(period);
        slice = period.subPeriods;
    }
    return chain;
}

/** Раскрыть цепочку периодов в плоский JSON-дружелюбный список. */
export function chainToPlain(chain: DashaPeriod[]): { level: number; lord: string; startIso: string; endIso: string }[] {
    return chain.map((p) => ({
        level: p.level,
        lord: p.lordName,
        startIso: new Date(p.startMs).toISOString(),
        endIso: new Date(p.endMs).toISOString(),
    }));
}

/**
 * Братские списки каждого уровня активной ветки: для уровня k список братьев
 * — список, в котором лежит активный период уровня k. Уровни 1-3 читаются из
 * дерева; списки уровней 4-5 (сукшма/прана) дописывает вызывающий код, т.к.
 * они генерируются локально вдоль активной ветки (см. calc_chart.ts).
 */
export function levelSlicesOfChain(tree: DashaPeriod[], chain: DashaPeriod[]): DashaPeriod[][] {
    const slices: DashaPeriod[][] = [];
    let slice: DashaPeriod[] | undefined = tree;
    for (const period of chain) {
        if (!slice) break;
        slices.push(slice);
        slice = period.subPeriods;
    }
    return slices;
}

/**
 * Следующие (после активных) периоды каждого уровня: сосед активного периода
 * в его братском списке. Отсутствие соседа на уровне — уровень просто не
 * попадает в результат (нет данных — нет выдумки).
 */
export function findUpcomingChain(chain: DashaPeriod[], levelLists: DashaPeriod[][]): DashaPeriod[] {
    const upcoming: DashaPeriod[] = [];
    chain.forEach((period, i) => {
        const siblings = levelLists[i];
        if (!siblings) return;
        const idx = siblings.indexOf(period);
        if (idx >= 0 && idx + 1 < siblings.length) upcoming.push(siblings[idx + 1]);
    });
    return upcoming;
}
