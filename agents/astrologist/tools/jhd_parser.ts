/**
 * Парсер файлов натальных данных Jagannatha Hora (.jhd).
 *
 * Порт 1:1 из исходного драфта parse_and_calc.rs (git-история: коммит 0434d65,
 * JhdInput::from_jhd_str):
 * - первые строки: месяц, день, год (каждый на своей строке);
 * - время и координаты в фирменном десятичном формате JHora «ГГ.ММСС»:
 *   8.3 → 8ч 03м (или 8°03'); 34.1530 → 34°15'30";
 * - в JHora ВОСТОК пишется со знаком минус (и часовой пояс, и долгота) —
 *   поэтому знаки инвертируются;
 * - город/страна: '^' заменяет пробелы;
 * - строки 8..11 (поля JHora о давлении и пр.) игнорируются.
 */

import { julday } from "./ephemeris_time.ts";

export interface JhdInput {
    year: number;
    month: number;
    day: number;
    /** Десятичные часы локального времени рождения. */
    hourLocal: number;
    /** Часовой пояс, часы относительно UTC (восток — положительный). */
    timezoneOffset: number;
    /** Географическая долгота в градусах (восток — положительный). */
    longitudeEast: number;
    /** Географическая широта в градусах (север — положительный). */
    latitudeNorth: number;
    altitude: number;
    city: string;
    country: string;
    /** Момент рождения в UTC (ISO 8601). */
    birthUtcIso: string;
    /** Момент рождения в UTC, миллисекунды эпохи Unix. */
    birthUtcMs: number;
    /** Юлианский день рождения (UT). */
    julianDayUt: number;
}

/** Формат JHora «ГГ.ММСС» → десятичные градусы/часы (порт parse_jhora_dms). */
export function parseJhoraDms(val: number): number {
    const sign = val < 0.0 ? -1.0 : 1.0;
    const absVal = Math.abs(val);
    const deg = Math.floor(absVal);
    const minSec = (absVal - deg) * 100.0;
    const min = Math.floor(minSec + 1e-9);
    const sec = (minSec - min) * 100.0;
    return sign * (deg + min / 60.0 + sec / 3600.0);
}

function parseFinite(raw: string, what: string): number {
    const v = Number(raw);
    if (!Number.isFinite(v)) {
        throw new Error(`Нечисловое значение в .jhd для ${what}: "${raw}"`);
    }
    return v;
}

/** Разбор содержимого .jhd. Бросает Error с понятным сообщением (честный отказ). */
export function parseJhd(content: string): JhdInput {
    const lines = content.split(/\r?\n/).map((l) => l.trim());
    if (lines.length < 14) {
        throw new Error(`Файл .jhd повреждён или содержит меньше 14 строк (найдено ${lines.length})`);
    }

    const month = Math.trunc(parseFinite(lines[0], "месяц"));
    const day = Math.trunc(parseFinite(lines[1], "день"));
    const year = Math.trunc(parseFinite(lines[2], "год"));
    if (!(month >= 1 && month <= 12) || !(day >= 1 && day <= 31) || !(year >= -4800 && year <= 9999)) {
        throw new Error(`Неправдоподобная дата в .jhd: ${day}.${month}.${year}`);
    }

    const rawTime = parseFinite(lines[3], "время");
    const rawTz = parseFinite(lines[4], "часовой пояс");
    const rawLon = parseFinite(lines[5], "долгота");
    const rawLat = parseFinite(lines[6], "широта");
    const altitude = lines[7] !== "" && Number.isFinite(Number(lines[7])) ? Number(lines[7]) : 0.0;

    const hourLocal = parseJhoraDms(rawTime);
    const timezoneOffset = -parseJhoraDms(rawTz); // В JHora восток со знаком минус
    const longitudeEast = -parseJhoraDms(rawLon); // В JHora восток со знаком минус
    const latitudeNorth = parseJhoraDms(rawLat);

    const city = lines[12].replace(/\^/g, " ");
    const country = lines[13].replace(/\^/g, " ");

    // Точное время UTC: из юлианского дня напрямую (точнее эталонного
    // округления до секунд; JD 2440587.5 = эпоха Unix 1970-01-01T00:00Z).
    const hourUt = hourLocal - timezoneOffset;
    const julianDayUt = julday(year, month, day, hourUt);
    const birthUtcMs = Math.round((julianDayUt - 2440587.5) * 86400.0 * 1000.0);
    const birthUtcIso = new Date(birthUtcMs).toISOString();

    return {
        year,
        month,
        day,
        hourLocal,
        timezoneOffset,
        longitudeEast,
        latitudeNorth,
        altitude,
        city,
        country,
        birthUtcIso,
        birthUtcMs,
        julianDayUt,
    };
}
