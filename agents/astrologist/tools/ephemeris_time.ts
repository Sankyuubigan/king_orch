/**
 * Календарно-временные функции, совместимые со Swiss Ephemeris.
 *
 * Эталон алгоритмов — исходный драфт parse_and_calc.rs (git-история: коммит
 * 0434d65, swe_julday)
 * и опубликованные формулы:
 * - swe_julday — алгоритм Swiss Ephemeris (swedate.c, Meeus-совместимый);
 * - DeltaT — Espenak & Meeus 2006 (NASA eclipse.gsfc.nasa.gov), кусочные
 *   полиномы 1900-2150 + Morrison-Stephenson вне диапазона. Расхождение
 *   с моделью Swiss Ephemeris ≤ ~2 c, что даёт ≤ ~1" по Луне — на три
 *   порядка ниже гейта точности (1').
 */

export const J2000 = 2451545.0;
export const DEG_TO_RAD = Math.PI / 180;
export const RAD_TO_DEG = 180 / Math.PI;

/** swe_degnorm: нормализация угла в 0..360 (fmod + защита от отрицательных). */
export function degnorm(x: number): number {
    let y = x % 360.0;
    if (Math.abs(y) < 1e-13) y = 0;
    if (y < 0.0) y += 360.0;
    return y;
}

/**
 * swe_julday (Swiss Ephemeris, swedate.c): юлианский день для григорианской
 * даты; hour — десятичные часы UT.
 */
export function julday(year: number, month: number, day: number, hour: number): number {
    let u = year;
    if (month < 3) u -= 1;
    const u0 = u + 4712.0;
    let u1 = month + 1.0;
    if (u1 < 4) u1 += 12.0;
    let jd = Math.floor(u0 * 365.25)
        + Math.floor(30.6 * u1 + 0.000001)
        + day + hour / 24.0 - 63.5;
    // SE_GREG_CAL: поправка григорианского календаря
    let u2 = Math.floor(Math.abs(u) / 100) - Math.floor(Math.abs(u) / 400);
    if (u < 0.0) u2 = -u2;
    jd = jd - u2 + 2;
    if (u < 0.0 && u / 100 === Math.floor(u / 100) && u / 400 !== Math.floor(u / 400)) {
        jd -= 1;
    }
    return jd;
}

/** Десятичный год из юлианского дня (для выбора сегмента DeltaT). */
export function decimalYear(jd: number): number {
    return 2000.0 + (jd - J2000) / 365.2425;
}

/**
 * DeltaT = TT - UT в секундах. Espenak & Meeus (2006), кусочные полиномы.
 * Диапазон 1900-2150 покрывает все боевые случаи (натальные карты и даты
 * транзитов); вне диапазона — зеркальная экстраполяция полиномов.
 */
export function deltaT(jd: number): number {
    const y = decimalYear(jd);
    if (y >= 1986.0 && y < 2005.0) {
        const t = y - 2000.0;
        return 63.86 + 0.3345 * t - 0.060374 * t * t
            + 0.0017275 * t * t * t
            + 0.000651814 * t * t * t * t
            + 0.00002373599 * t * t * t * t * t;
    }
    if (y >= 2005.0 && y < 2050.0) {
        const t = y - 2000.0;
        return 62.92 + 0.32217 * t + 0.005589 * t * t;
    }
    if (y >= 2050.0 && y <= 2150.0) {
        const u = (y - 1820.0) / 100.0;
        return -20.0 + 32.0 * u * u - 0.5628 * (2150.0 - y);
    }
    if (y >= 1920.0 && y < 1941.0) {
        const t = y - 1920.0;
        return 21.20 + 0.84493 * t - 0.076100 * t * t + 0.0020936 * t * t * t;
    }
    if (y >= 1941.0 && y < 1961.0) {
        const t = y - 1950.0;
        return 29.07 + 0.407 * t - (t * t) / 233.0 + (t * t * t) / 2547.0;
    }
    if (y >= 1961.0 && y < 1986.0) {
        const t = y - 1975.0;
        return 45.45 + 1.067 * t - (t * t) / 260.0 - (t * t * t) / 718.0;
    }
    if (y >= 1900.0 && y < 1920.0) {
        const t = y - 1900.0;
        return -2.79 + 1.494119 * t - 0.0598939 * t * t
            + 0.0061966 * t * t * t - 0.000197 * t * t * t * t;
    }
    if (y >= 1860.0 && y < 1900.0) {
        const t = y - 1860.0;
        return 7.62 + 0.5737 * t - 0.251754 * t * t + 0.01680668 * t * t * t
            - 0.0004473624 * t * t * t * t + (t * t * t * t * t) / 233174.0;
    }
    if (y >= 1800.0 && y < 1860.0) {
        const t = y - 1800.0;
        return 13.72 - 0.332447 * t + 0.0068612 * t * t + 0.0041116 * t * t * t
            - 0.00037436 * t * t * t * t + 0.0000121272 * t * t * t * t * t
            - 0.0000001699 * t * t * t * t * t * t + 0.000000000875 * t * t * t * t * t * t * t;
    }
    if (y >= 1700.0 && y < 1800.0) {
        const t = y - 1700.0;
        return 8.83 + 0.1603 * t - 0.0059285 * t * t + 0.00013336 * t * t * t
            - (t * t * t * t) / 1174000.0;
    }
    if (y >= 1600.0 && y < 1700.0) {
        const t = y - 1600.0;
        return 120.0 - 0.9808 * t - 0.01532 * t * t + (t * t * t) / 7129.0;
    }
    if (y > 500.0 && y < 1600.0) {
        const u = (y - 1000.0) / 100.0;
        return 1574.2 - 556.01 * u + 71.23472 * u * u + 0.319781 * u * u * u
            - 0.8503463 * u * u * u * u - 0.00505099 * u * u * u * u * u
            + 0.0083572073 * u * u * u * u * u * u;
    }
    if (y > -500.0 && y <= 500.0) {
        const u = y / 100.0;
        return 10583.6 - 101.3 * u + 2.42 * u * u + 0.0184 * u * u * u
            - 0.00011 * u * u * u * u - 0.0000139 * u * u * u * u * u;
    }
    const u = (y - 1820.0) / 100.0;
    return -20.0 + 32.0 * u * u;
}

/** JD(UT) → JD(TT/ET): Swiss Ephemeris считает эфемериду в TT. */
export function jdUtToJdEt(jdUt: number): number {
    return jdUt + deltaT(jdUt) / 86400.0;
}
