/**
 * ЕДИНСТВЕННЫЙ переключатель ведической системы расчёта астролога.
 *
 * Система меняется ОДНОЙ строкой — значением VEDIC_SYSTEM ниже.
 * Всё остальное (айанамша, год даши, планетная модель) подтягивается
 * из пресета автоматически.
 *
 *   "drik" (ДЕФОЛТ) — Drik ganita:
 *     • планетная модель: современная эфемерида (astronomy-engine/VSOP87,
 *       сверена с JPL Horizons до 1.4");
 *     • айанамша: Lahiri (Chitrapaksha) — SE_SIDM_LAHIRI;
 *     • год даши: 365.2425 сут (нормальный солнечный/григорианский год).
 *
 *   "sss" — Sri Surya Siddhanta (традиционная школа):
 *     • планетная модель: Сурья-сиддханта (эпициклы, мандоччи/шигроччи) —
 *       ЕЩЁ НЕ РЕАЛИЗОВАНА, тул честно откажется (см. calc_chart.ts);
 *     • айанамша: Sri Surya Siddhanta — SE_SIDM_SURYASIDDHANTA;
 *     • год даши: «Year with 360 tithis» (354.3670680 сут).
 *
 * Константы Lahiri/SS — из исходников Swiss Ephemeris (sweph.h, таблица
 * ayanamsa[]): JHora построена на Swiss Ephemeris, поэтому Lahiri в туле и
 * в JHora совпадает на уровне модели прецессии (проверяется гейтом).
 */

import { ayanamsaLahiri, ayanamsaSuryaSiddhanta } from "./ayanamsa.ts";

export type VedicSystem = "drik" | "sss";

/** ← ЕДИНСТВЕННАЯ СТРОКА ПЕРЕКЛЮЧЕНИЯ СИСТЕМЫ. */
export const VEDIC_SYSTEM: VedicSystem = "drik";

export interface VedicSystemConfig {
    system: VedicSystem;
    /** Человекочитаемое имя системы (идёт в вывод тула). */
    name: string;
    /** Айанамша в градусах на момент jdUt (юлианский день UT). */
    ayanamsa: (jdUt: number) => number;
    ayanamshaName: string;
    /** Длина года Вимшоттари в сутках. */
    dashaYearDays: number;
    dashaYearName: string;
    /** Планетная модель: "drik" — современная эфемерида, "sss" — Сурья-сиддханта. */
    planetaryModel: "drik" | "sss";
}

const DRIK: VedicSystemConfig = {
    system: "drik",
    name: "Drik ganita",
    ayanamsa: ayanamsaLahiri,
    ayanamshaName: "Lahiri (Chitrapaksha)",
    dashaYearDays: 365.2425,
    dashaYearName: "Normal solar year (365.2425 days, Gregorian)",
    planetaryModel: "drik",
};

const SSS: VedicSystemConfig = {
    system: "sss",
    name: "Sri Surya Siddhanta",
    ayanamsa: ayanamsaSuryaSiddhanta,
    ayanamshaName: "Sri Surya Siddhanta (SE_SIDM_SURYASIDDHANTA)",
    dashaYearDays: 354.3670680,
    dashaYearName: "Year with 360 tithis",
    planetaryModel: "sss",
};

const CONFIGS: Record<VedicSystem, VedicSystemConfig> = { drik: DRIK, sss: SSS };

/** Пресет по ключу системы (для тестов и будущих тулов). */
export function getSystemConfig(system: VedicSystem): VedicSystemConfig {
    return CONFIGS[system];
}

/** Активный пресет — единственная точка чтения конфигурации для тулов. */
export const VEDIC_SYSTEM_CONFIG: VedicSystemConfig = CONFIGS[VEDIC_SYSTEM];
