// MCP-сервер времени (zero-dependency, без API-ключей).
// Инструмент GetCurrentTime — текущая дата/время в системной таймзоне или произвольной IANA-таймзоне.
// Аналог официального MCP Time server (modelcontextprotocol/servers), реализация на Deno без зависимостей.
import { createMcpServer } from "./mcp_base.ts";

const WEEKDAYS = ['понедельник', 'вторник', 'среда', 'четверг', 'пятница', 'суббота', 'воскресенье'];
const MONTHS = ['января', 'февраля', 'марта', 'апреля', 'мая', 'июня', 'июля', 'августа', 'сентября', 'октября', 'ноября', 'декабря'];

function toUtcIso(date: Date): string {
    return date.toISOString();
}

function getLocalParts(date: Date, timeZone: string): Record<string, string> {
    const fmt = new Intl.DateTimeFormat('ru-RU', {
        timeZone,
        year: 'numeric', month: '2-digit', day: '2-digit',
        hour: '2-digit', minute: '2-digit', second: '2-digit',
        weekday: 'long', hour12: false,
    });
    const parts: Record<string, string> = {};
    for (const p of fmt.formatToParts(date)) {
        if (p.type !== 'literal') parts[p.type] = p.value;
    }
    return parts;
}

function getCurrentTime(args: { timezone?: string }): string {
    const requested = (args.timezone || '').trim();
    const timeZone = requested || Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';

    // Проверяем валидность таймзоны заранее (Intl бросит RangeError на неизвестную).
    try {
        new Intl.DateTimeFormat('en-US', { timeZone });
    } catch (e) {
        throw new Error(`GetCurrentTime: неизвестная таймзона "${requested}". Используй IANA-имя (например, "Europe/Moscow", "America/New_York") или оставь пустым для системной.`);
    }

    const now = new Date();
    const parts = getLocalParts(now, timeZone);
    const weekdayIdx = WEEKDAYS.indexOf(parts.weekday);
    const iso = toUtcIso(now);

    const year = parts.year;
    const month = parts.month;
    const day = parts.day;

    return JSON.stringify({
        timezone: timeZone,
        datetime: `${year}-${month}-${day}T${parts.hour}:${parts.minute}:${parts.second}`,
        date_iso: `${year}-${month}-${day}`,
        date_ru: `${day}.${month}.${year}`,
        date_ru_human: `${parts.weekday}, ${day} ${MONTHS[parseInt(month, 10) - 1]} ${year}`,
        time: `${parts.hour}:${parts.minute}:${parts.second}`,
        weekday: parts.weekday,
        weekday_index: weekdayIdx,
        utc_iso: iso,
    }, null, 0);
}

createMcpServer({
    name: "time-mcp",
    version: "1.0.0",
    tools: [{
        name: "GetCurrentTime",
        description: "Текущая дата и время (без API-ключей). По умолчанию — системная таймзона; можно указать IANA-таймзону (например \"Europe/Moscow\"). Возвращает дату (ISO и по-русски), время, день недели и UTC.",
        inputSchema: {
            type: "object",
            properties: {
                timezone: { type: "string", description: "Опционально: IANA-имя таймзоны (например 'Europe/Moscow'). Без него — системная таймзона." }
            }
        }
    }],
    handlers: {
        GetCurrentTime: async (args) => getCurrentTime(args || {})
    }
});
