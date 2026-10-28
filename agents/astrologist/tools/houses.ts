/**
 * Асцендент для системы домов Whole Sign ('W') — геометрия из Swiss Ephemeris
 * swehouse.c (Asc1/Asc2/fix_asc_polar, spherical trig по справочнику
 * spherical trigonometry; тривиальные угловые формулы, не защищаемая логика).
 *
 * Входные углы — ТРОПИЧЕСКИЕ: armc (прямое восхождение MC = звёздное время
 * в градусах), eps (истинная наклонность, рад), geolat (широта, градусы).
 * Сидерическую лагну получает вызывающий: asc_sidereal = asc_tropical − айанамша.
 */

const VERY_SMALL = 1e-10;

function sind(deg: number): number {
    return Math.sin(deg * (Math.PI / 180));
}
function cosd(deg: number): number {
    return Math.cos(deg * (Math.PI / 180));
}
function tand(deg: number): number {
    return Math.tan(deg * (Math.PI / 180));
}
function atand(x: number): number {
    return Math.atan(x) * (180 / Math.PI);
}
export function degnormH(x: number): number {
    let y = x % 360.0;
    if (Math.abs(y) < 1e-13) y = 0;
    if (y < 0.0) y += 360.0;
    return y;
}

/** Asc2 — базовая ветка сферической тригонометрии (порт swehouse.c). */
function asc2(x: number, f: number, sine: number, cose: number): number {
    let ass = -tand(f) * sine + cose * cosd(x);
    if (Math.abs(ass) < VERY_SMALL) ass = 0;
    let sinx = sind(x);
    if (Math.abs(sinx) < VERY_SMALL) sinx = 0;
    if (sinx === 0) {
        ass = ass < 0 ? -VERY_SMALL : VERY_SMALL;
        return ass < 0 ? 180 + ass : ass;
    }
    if (ass === 0) {
        return sinx < 0 ? -90 : 90;
    }
    ass = atand(sinx / ass);
    if (ass < 0) ass = 180 + ass;
    return ass;
}

/** Asc1 — асцендент для точки x1 (порт swehouse.c с квадрантами). */
function asc1(x1: number, f: number, sine: number, cose: number): number {
    const x = degnormH(x1);
    const n = Math.floor(x / 90) + 1; // квадрант 1..4
    if (Math.abs(90 - f) < VERY_SMALL) return 180; // близко к северному полюсу
    if (Math.abs(90 + f) < VERY_SMALL) return 0; // близко к южному полюсу
    let ass: number;
    if (n === 1) {
        ass = asc2(x, f, sine, cose);
    } else if (n === 2) {
        ass = 180 - asc2(180 - x, -f, sine, cose);
    } else if (n === 3) {
        ass = 180 + asc2(x - 180, -f, sine, cose);
    } else {
        ass = 360 - asc2(360 - x, f, sine, cose);
    }
    ass = degnormH(ass);
    // защита от накопленной погрешности на кардинальных точках (порт)
    if (Math.abs(ass - 90) < VERY_SMALL) ass = 90;
    if (Math.abs(ass - 180) < VERY_SMALL) ass = 180;
    if (Math.abs(ass - 270) < VERY_SMALL) ass = 270;
    if (Math.abs(ass - 360) < VERY_SMALL) ass = 0;
    return ass;
}

/** Коррекция полярных широт (порт fix_asc_polar). */
function fixAscPolar(asc: number, armc: number, eps: number, geolat: number): number {
    const demc = atand(sind(armc) * tand((eps * 180) / Math.PI));
    if (geolat >= 0 && 90 - geolat + demc < 0) asc = degnormH(asc + 180);
    if (geolat < 0 && -90 - geolat + demc > 0) asc = degnormH(asc + 180);
    return asc;
}

/**
 * Тропический асцендент в градусах.
 * armc — звёздное время в градусах (GAST*15 + восточная долгота),
 * epsRad — истинная наклонность в радианах, geolat — широта в градусах.
 * Семантика swehouse.c: asc = Asc1(armc + 90, geolat, sin(eps), cos(eps)).
 */
export function tropicalAscendant(armc: number, epsRad: number, geolat: number): number {
    const sine = Math.sin(epsRad);
    const cose = Math.cos(epsRad);
    const asc = asc1(degnormH(armc + 90), geolat, sine, cose);
    return fixAscPolar(asc, degnormH(armc), epsRad, geolat);
}
