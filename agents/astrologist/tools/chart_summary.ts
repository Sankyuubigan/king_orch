/**
 * Текстовая сводка карты человеческим языком для промпта Астролога.
 *
 * Единственная работа модуля — превратить ДАННЫЕ calc_chart в читаемые
 * строки. Никаких расчётов по небу здесь НЕТ и быть не должно: всё, чего
 * нет во входных данных, честно помечается «данные не переданы», а не
 * восполняется.
 *
 * Модуль чистый (нет Deno-API) — проверяется vitest'ом из test/.
 */

/** Значения поля dignity в выводе calc_chart (chart_analysis.ts). */
const DIGNITY_RU: Record<string, string> = {
    exalted: "экзальтация",
    moolatrikona: "муллатрикона",
    own_sign: "свой знак",
    debilitated: "падение",
    neutral: "нейтральна",
};

const LEVEL_RU: Record<number, string> = {
    1: "Махадаша",
    2: "Антардаша",
    3: "Пратьянтардаша",
    4: "Сукшма-даша",
    5: "Прана-даша",
};

export interface SummaryPointLike {
    name?: unknown;
    sign?: unknown;
    deg?: unknown;
    min?: unknown;
    retro?: unknown;
    nakshatra?: unknown;
    house_d1?: unknown;
    ruled_houses_d1?: unknown;
    dignity?: unknown;
    combust?: unknown;
    is_vargottama?: unknown;
}

export interface SummaryTransitLike {
    name?: unknown;
    sign?: unknown;
    deg?: unknown;
    min?: unknown;
    retro?: unknown;
    house_transited?: unknown;
}

export interface SummaryDashaEntry {
    level?: unknown;
    lord?: unknown;
    startIso?: unknown;
    endIso?: unknown;
}

export interface ChartSummaryInput {
    lagna?: SummaryPointLike | null;
    planets?: SummaryPointLike[] | null;
    dasha?: {
        now_iso?: unknown;
        active_chain?: SummaryDashaEntry[] | null;
        upcoming?: SummaryDashaEntry[] | null;
    } | null;
    transits?: { grahas?: SummaryTransitLike[] | null } | null;
    /** Сдвиг зоны отображения дат относительно UTC, минут (UTC+2 → +120). */
    tzOffsetMinutes?: number;
    /** Момент «сейчас»; по умолчанию берётся из dasha.now_iso. */
    nowIso?: unknown;
}

/** «31.10.2026 22:17» — детерминированный формат без Intl из разных рантаймов. */
export function fmtDateTime(iso: string, tzOffsetMinutes: number): string {
    const time = new Date(iso);
    if (Number.isNaN(time.getTime())) return "";
    const shifted = new Date(time.getTime() + tzOffsetMinutes * 60_000);
    const dd = String(shifted.getUTCDate()).padStart(2, "0");
    const mm = String(shifted.getUTCMonth() + 1).padStart(2, "0");
    const yyyy = shifted.getUTCFullYear();
    const hh = String(shifted.getUTCHours()).padStart(2, "0");
    const mi = String(shifted.getUTCMinutes()).padStart(2, "0");
    return `${dd}.${mm}.${yyyy} ${hh}:${mi}`;
}

/** «31.10.2026» — только дата. */
export function fmtDateShort(iso: string, tzOffsetMinutes: number): string {
    const full = fmtDateTime(iso, tzOffsetMinutes);
    return full.slice(0, 10);
}

function fmtDegMin(deg: unknown, min: unknown): string {
    if (typeof deg !== "number") return "";
    const realMin = typeof min === "number" ? Math.round(min) : 0;
    return `${deg}°${String(realMin).padStart(2, "0")}'`;
}

/** Безопасное чтение: строка или пусто. */
function asString(value: unknown): string {
    return typeof value === "string" ? value : "";
}

function asNumber(value: unknown): number | null {
    return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/** Лагна: «Лагна: Стрелец (14°00', накшатра Мула).» */
function lagnaLine(lagna: SummaryPointLike | null | undefined, tzOffsetMinutes: number): string {
    if (!lagna) return "Лагна: данные не переданы.";
    const sign = asString(lagna.sign);
    if (!sign) return "Лагна: данные не переданы.";
    const degree = fmtDegMin(lagna.deg, lagna.min);
    const nakshatra = asString(lagna.nakshatra);
    const degreePart = degree ? ` (${degree}${nakshatra ? `, накшатра ${nakshatra}` : ""})` : (nakshatra ? ` (накшатра ${nakshatra})` : "");
    return `Лагна: ${sign}${degreePart}.`;
}

/** Строка одной грахи рождения. */
export function planetLine(p: SummaryPointLike): string {
    const name = asString(p.name) || "неизвестная граха";
    const parts: string[] = [];
    const house = asNumber(p.house_d1);
    if (house !== null) parts.push(`дом ${house}`);
    const degree = fmtDegMin(p.deg, p.min);
    const sign = asString(p.sign);
    const place = [sign, degree].filter(Boolean).join(" ");
    if (place) parts.push(`${place}`);
    const nakshatra = asString(p.nakshatra);
    if (nakshatra) parts.push(`накшатра ${nakshatra}`);
    if (p.retro === true) parts.push("ретро");
    if (p.combust === true) parts.push("сожжён Солнцем");
    if (p.is_vargottama === true) parts.push("варготтама");
    const ruled = Array.isArray(p.ruled_houses_d1) ? p.ruled_houses_d1.filter((h): h is number => typeof h === "number") : [];
    if (ruled.length === 1) parts.push(`управитель дома ${ruled[0]}`);
    if (ruled.length > 1) parts.push(`управитель домов ${ruled.join(", ")}`);
    const dignity = asString(p.dignity);
    const dignityRu = dignity ? DIGNITY_RU[dignity] : undefined;
    if (dignityRu) parts.push(dignityRu);
    return `- ${name}: ${parts.join(", ")}.`;
}

/** Строка одного транзитного положения (дом — от лагны). */
export function transitLine(t: SummaryTransitLike): string {
    const name = asString(t.name) || "неизвестная граха";
    const parts: string[] = [];
    const sign = asString(t.sign);
    const degree = fmtDegMin(t.deg, t.min);
    if (sign) parts.push(degree ? `${sign} (${degree})` : sign);
    const house = asNumber(t.house_transited);
    if (house !== null) parts.push(`проходит дом ${house} от лагны`);
    if (t.retro === true) parts.push("ретро");
    return `- ${name}: ${parts.join(", ")}.`;
}

function dashaPeriodText(entry: SummaryDashaEntry, tzOffsetMinutes: number): string {
    const level = asNumber(entry.level) ?? 0;
    const lord = asString(entry.lord) || "?";
    const levelName = LEVEL_RU[level] ?? `уровень ${level}`;
    const startIso = asString(entry.startIso);
    const endIso = asString(entry.endIso);
    const start = startIso ? fmtDateTime(startIso, tzOffsetMinutes) : "?";
    const end = endIso ? fmtDateTime(endIso, tzOffsetMinutes) : "?";
    return `${levelName} ${lord} (${start} — ${end})`;
}

function dashaUpcomingText(entry: SummaryDashaEntry, tzOffsetMinutes: number): string {
    const level = asNumber(entry.level) ?? 0;
    const lord = asString(entry.lord) || "?";
    const levelName = LEVEL_RU[level] ?? `уровень ${level}`;
    const startIso = asString(entry.startIso);
    const endIso = asString(entry.endIso);
    const start = startIso ? fmtDateTime(startIso, tzOffsetMinutes) : "?";
    const end = endIso ? fmtDateTime(endIso, tzOffsetMinutes) : "?";
    return `${levelName} ${lord} (с ${start} до ${end})`;
}

/** Читаемая сводка-истина; падает только если сломаны сами аргументы. */
export function buildChartSummary(input: ChartSummaryInput): string {
    const nowOffset = typeof input.tzOffsetMinutes === "number" ? input.tzOffsetMinutes : 0;
    const lines: string[] = [];
    lines.push("=== СВОДКА ИСТИННОЙ КАРТЫ НА СЕЙЧАС (рассчитано кодом; это истина, не менять и не додумывать) ===");

    lines.push(lagnaLine(input.lagna, nowOffset));

    const planets = input.planets;
    if (Array.isArray(planets) && planets.length > 0) {
        lines.push("Грахи рождения (дом D1 от лагны):");
        for (const p of planets) lines.push(planetLine(p));
    } else {
        lines.push("Грахи: данные не переданы.");
    }

    const nowIsoRaw = input.nowIso ?? input.dasha?.now_iso;
    const nowIso = asString(nowIsoRaw);
    const chain = input.dasha?.active_chain;
    const hasChain = Array.isArray(chain) && chain.length > 0;
    if (hasChain) {
        const nowPrefix = nowIso ? ` на ${fmtDateShort(nowIso, nowOffset)}` : "";
        const chainText = chain!.map((e) => dashaPeriodText(e, nowOffset)).join(", ");
        lines.push(`Текущий период (Даши)${nowPrefix}: ${chainText}.`);
    } else {
        lines.push("Текущий период: данные не переданы.");
    }

    const upcoming = input.dasha?.upcoming;
    if (Array.isArray(upcoming) && upcoming.length > 0) {
        const upcomingText = upcoming.map((e) => dashaUpcomingText(e, nowOffset)).join("; ");
        lines.push(`Следующие периоды после текущих: ${upcomingText}.`);
    }

    const grahas = input.transits?.grahas;
    if (Array.isArray(grahas) && grahas.length > 0) {
        lines.push("Транзиты сейчас (дома от лагны):");
        for (const t of grahas) lines.push(transitLine(t));
    } else {
        lines.push("Транзиты: данные не переданы.");
    }

    return lines.join("\n");
}
