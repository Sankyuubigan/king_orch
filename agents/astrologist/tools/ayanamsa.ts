/**
 * Айанамша Sri Surya Siddhanta (эквивалент SE_SIDM_SURYASIDDHANTA из Swiss
 * Ephemeris) + долгосрочная прецессия Vondrák/Capitaine/Wallace 2011.
 *
 * Источники:
 * - J. Vondrák, N. Capitaine, P. Wallace, "New precession expressions, valid
 *   for long time intervals", A&A 534, A22 (2011) — полиномы и периодические
 *   члены прецессии/наклонности (опубликованные данные);
 * - константы эпохи айанамши {t0 = 1903396.8128654 (UT), ayan_t0 = 0} —
 *   фактическое определение режима SE_SIDM_SURYASIDDHANTA (Swiss Ephemeris):
 *   мессой вход среднего Солнца в Овен в точке среднего равноденствия
 *   21.03.499, Уджайн (75.7684565 E), 7:30:31.57 UT;
 * - алгоритм айанамши повторяет семантику swi_get_ayanamsa_ex (без
 *   SE_SIDBIT_ECL_DATE): точка весеннего равноденствия даты прецессируется
 *   к эпохе t0, её эклиптическая долгота на эпохе t0, взятая с обратным
 *   знаком, и есть айанамша.
 *
 * Внутренняя проверка замыкания: ayanamsa(t0) === 0 по построению.
 */

import { J2000, DEG_TO_RAD, RAD_TO_DEG, degnorm, deltaT } from "./ephemeris_time.ts";

const AS2R = DEG_TO_RAD / 3600.0;
const D2PI = 2.0 * Math.PI;
const EPS0 = 84381.406 * AS2R;

/** Эпоха айанамши (JD UT) и начальное значение (0 по определению режима). */
const AYA_T0_UT = 1903396.8128654;
const AYA_T0_VALUE = 0.0;

// ── Vondrák 2011: прецессия эклиптики (p, q) ────────────────────────────────
// Полиномы (асек), степени t^0..t^3, t = века от J2000.
const PQ_POLY: number[][] = [
    [+5851.607687, -1600.886300],
    [-0.1189000, +1.1689818],
    [-0.00028913, -0.00000020],
    [+0.000000101, -0.000000437],
];
// Периодические члены: [периоды, cos-p, cos-q, sin-p, sin-q] (асек).
const PQ_PER: number[][] = [
    [708.15, 2309, 1620, 492.2, 1183, 622, 882, 547],
    [-5486.751211, -17.127623, -617.517403, 413.44294, 78.614193, -180.732815, -87.676083, 46.140315],
    // Коэффициенты по erratum A&A 541, C1 (2012):
    [-684.66156, 2446.28388, 399.671049, -356.652376, -186.387003, -316.80007, 198.296701, 101.135679],
    [667.66673, -2354.886252, -428.152441, 376.202861, 184.778874, 335.321713, -185.138669, -120.97283],
    [-5523.863691, -549.74745, -310.998056, 421.535876, -36.776172, -145.278396, -34.74445, 22.885731],
];

// ── Vondrák 2011: прецессия экватора (x, y) ─────────────────────────────────
const XY_POLY: number[][] = [
    [+5453.282155, -73750.930350],
    [+0.4252841, -0.7675452],
    [-0.00037173, -0.00018725],
    [-0.000000152, +0.000000231],
];
const XY_PER: number[][] = [
    [256.75, 708.15, 274.2, 241.45, 2309, 492.2, 396.1, 288.9, 231.1, 1610, 620, 157.87, 220.3, 1200],
    [-819.940624, -8444.676815, 2600.009459, 2755.17563, -167.659835, 871.855056, 44.769698, -512.313065, -819.415595, -538.071099, -189.793622, -402.922932, 179.516345, -9.814756],
    [75004.344875, 624.033993, 1251.136893, -1102.212834, -2660.66498, 699.291817, 153.16722, -950.865637, 499.754645, -145.18821, 558.116553, -23.923029, -165.405086, 9.344131],
    [81491.287984, 787.163481, 1251.296102, -1257.950837, -2966.79973, 639.744522, 131.600209, -445.040117, 584.522874, -89.756563, 524.42963, -13.549067, -210.157124, -44.919798],
    [1558.515853, 7774.939698, -2219.534038, -2523.969396, 247.850422, -846.485643, -1393.124055, 368.526116, 749.045012, 444.704518, 235.934465, 374.049623, -171.33018, -22.899655],
];

// ── Vondrák 2011: наклонность эклиптики (swi_ldp_peps) ─────────────────────
const EPS_POLY: number[][] = [
    [+8134.017132, +84028.206305],
    [+5043.0520035, +0.3624445],
    [-0.00710733, -0.00004039],
    [+0.000000271, -0.000000110],
];
const EPS_PER: number[][] = [
    [+409.90, +396.15, +537.22, +402.90, +417.15, +288.92, +4043.00, +306.00, +277.00, +203.00],
    [-6908.287473, -3198.706291, +1453.674527, -857.748557, +1173.231614, -156.981465, +371.836550, -216.619040, +193.691479, +11.891524],
    [+753.872780, -247.805823, +379.471484, -53.880558, -90.109153, -353.600190, -63.115353, -28.248187, +17.703387, +38.911307],
    [-2845.175469, +449.844989, -1255.915323, +886.736783, +418.887514, +997.912441, -240.979710, +76.541307, -36.788069, -170.964086],
    [-1704.720302, -862.308358, +447.832178, -889.571909, +190.402846, -56.564991, -296.222622, -75.859952, +67.473503, +3.014055],
];

/** Полином + периодика → (p, q) в радианах. */
function polyPeriodic(poly: number[][], per: number[][], jd: number): [number, number] {
    const t = (jd - J2000) / 36525.0;
    let p = 0;
    let q = 0;
    const w = D2PI * t;
    for (let i = 0; i < per[0].length; i++) {
        const a = w / per[0][i];
        const s = Math.sin(a);
        const c = Math.cos(a);
        p += c * per[1][i] + s * per[3][i];
        q += c * per[2][i] + s * per[4][i];
    }
    let mult = 1;
    for (let i = 0; i < poly.length; i++) {
        p += poly[i][0] * mult;
        q += poly[i][1] * mult;
        mult *= t;
    }
    return [p * AS2R, q * AS2R];
}

/** Средняя наклонность эклиптики (рад), модель Vondrák (swi_ldp_peps). */
export function meanObliquity(jd: number): number {
    const [, q] = polyPeriodic(EPS_POLY, EPS_PER, jd);
    return q;
}

/** Полюс эклиптики эпохи jd (вектор). */
function eclipticPole(jd: number): number[] {
    const [p, q] = polyPeriodic(PQ_POLY, PQ_PER, jd);
    let z = 1 - p * p - q * q;
    if (z < 0) z = 0;
    else z = Math.sqrt(z);
    const s = Math.sin(EPS0);
    const c = Math.cos(EPS0);
    return [p, -q * c - z * s, -q * s + z * c];
}

/** Полюс экватора эпохи jd (вектор). */
function equatorPole(jd: number): number[] {
    const [x, y] = polyPeriodic(XY_POLY, XY_PER, jd);
    const w = x * x + y * y;
    return [x, y, w < 1 ? Math.sqrt(1 - w) : 0];
}

function cross(a: number[], b: number[]): number[] {
    return [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
}

/** Матрица прецессии J2000 → эпоха jd (строки = базис эпохи). */
function precessionMatrix(jd: number): number[] {
    const peqr = equatorPole(jd);
    const pecl = eclipticPole(jd);
    const v = cross(peqr, pecl);
    const w = Math.hypot(v[0], v[1], v[2]);
    const eqx = [v[0] / w, v[1] / w, v[2] / w];
    const v2 = cross(peqr, eqx);
    return [eqx[0], eqx[1], eqx[2], v2[0], v2[1], v2[2], peqr[0], peqr[1], peqr[2]];
}

/**
 * Прецессия вектора (семантика swi_precess/precess_3).
 * direction = -1: из J2000 в эпоху jd; direction = +1: из эпохи jd в J2000.
 */
export function precess(vec: number[], jd: number, direction: number): number[] {
    if (jd === J2000) return vec.slice();
    const m = precessionMatrix(jd);
    const out = [0, 0, 0];
    if (direction === -1) {
        for (let i = 0; i < 3; i++) {
            out[i] = vec[0] * m[i * 3 + 0] + vec[1] * m[i * 3 + 1] + vec[2] * m[i * 3 + 2];
        }
    } else {
        for (let i = 0; i < 3; i++) {
            out[i] = vec[0] * m[i + 0] + vec[1] * m[i + 3] + vec[2] * m[i + 6];
        }
    }
    return out;
}

/** Поворот экваториальных координат в эклиптические эпохи eps (рад). */
function equatorialToEcliptic(vec: number[], eps: number): number[] {
    const s = Math.sin(eps);
    const c = Math.cos(eps);
    return [
        vec[0],
        vec[1] * c + vec[2] * s,
        -vec[1] * s + vec[2] * c,
    ];
}

/**
 * Айанамша Surya Siddhanta в градусах на момент jdUt (юлианский день UT).
 * Точка весеннего равноденствия даты прецессируется к эпохе t0 (в TT),
 * её эклиптическая долгота t0 с обратным знаком + ayan_t0.
 */
export function ayanamsaSuryaSiddhanta(jdUt: number): number {
    const jdEt = jdUt + deltaT(jdUt) / 86400.0;
    // Эпоха t0 задана в UT → переводим в TT (в 499 г. DeltaT ~ 5700 c;
    // пренебрежение сдвинуло бы результат на ~0.03", берём явно).
    const t0Et = AYA_T0_UT + deltaT(AYA_T0_UT) / 86400.0;

    let x = [1, 0, 0];
    if (jdEt !== J2000) x = precess(x, jdEt, +1);
    if (t0Et !== J2000) x = precess(x, t0Et, -1);

    const eps = meanObliquity(t0Et);
    const ecl = equatorialToEcliptic(x, eps);
    const lon = Math.atan2(ecl[1], ecl[0]) * RAD_TO_DEG;
    return degnorm(-lon + AYA_T0_VALUE);
}
