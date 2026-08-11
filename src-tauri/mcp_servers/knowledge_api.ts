// MCP-сервер знаний (Deno, без API-ключей): Википедия + погода (wttr.in).
import https from "node:https";
import { createMcpServer } from "./mcp_base.ts";

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36';
const TIMEOUT_MS = 20000;

function httpsGetJson(url: string, timeoutMs: number = TIMEOUT_MS): Promise<any> {
    return new Promise((resolve, reject) => {
        const req = https.get(url, { headers: { 'User-Agent': UA, 'Accept': 'application/json' }, timeout: timeoutMs }, (res) => {
            if (res.statusCode && res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
                res.resume();
                reject(new Error(`Редирект запрещён: HTTP ${res.statusCode} -> ${res.headers.location}`));
                return;
            }
            let data = '';
            res.on('data', (chunk) => { data += chunk; });
            res.on('end', () => {
                if (res.statusCode !== 200) {
                    reject(new Error(`HTTP ${res.statusCode} от ${url}`));
                    return;
                }
                try {
                    resolve(JSON.parse(data));
                } catch (e) {
                    reject(new Error(`Невалидный JSON от ${url}: ${(e as Error).message}`));
                }
            });
        });
        req.on('timeout', () => req.destroy(new Error('Таймаут запроса')));
        req.on('error', (err) => reject(err));
    });
}

const WIKI_API = 'https://{lang}.wikipedia.org/w/api.php';

async function wikipediaSearch(query: string, lang: string): Promise<string> {
    const url = WIKI_API.replace('{lang}', encodeURIComponent(lang)) +
        '?action=query&list=search&srsearch=' + encodeURIComponent(query) +
        '&srlimit=5&format=json&utf8=1&origin=*';
    const data = await httpsGetJson(url);
    const hits = (data.query && data.query.search) || [];
    if (hits.length === 0) {
        return 'Ничего не найдено в Википедии по запросу "' + query + '".';
    }

    const titles = hits.map((h: any) => h.title).join('|');
    const extractsUrl = WIKI_API.replace('{lang}', encodeURIComponent(lang)) +
        '?action=query&prop=extracts&exintro&explaintext&redirects=1&format=json&utf8=1&origin=*&titles=' + encodeURIComponent(titles);
    const extData = await httpsGetJson(extractsUrl);
    const pages = (extData.query && extData.query.pages) || {};
    const result: string[] = [];

    for (const hit of hits) {
        const page = Object.values(pages).find((p: any) => p.title === hit.title);
        const extract = page && page.extract ? page.extract.trim() : (hit.snippet ? hit.snippet.replace(/<[^>]+>/g, '') : '');
        const url = `https://${lang}.wikipedia.org/wiki/${encodeURIComponent(hit.title.replace(/ /g, '_'))}`;
        const snippet = extract.length > 600 ? extract.slice(0, 600) + '…' : extract;
        result.push(`[${hit.title}]\nИсточник: ${url}\n${snippet}`);
    }

    return result.join('\n\n');
}

async function weather(city: string): Promise<string> {
    const url = 'https://wttr.in/' + encodeURIComponent(city) + '?format=j1&lang=ru';
    const data = await httpsGetJson(url);

    if (!data.current_condition || data.current_condition.length === 0) {
        throw new Error(`Не удалось получить погоду для "${city}" (нет данных от wttr.in). Проверь название города.`);
    }
    const cur = data.current_condition[0];
    const desc = (cur.weatherDesc && cur.weatherDesc[0] && cur.weatherDesc[0].value) || 'без описания';
    const lines = [
        `Погода в ${city}:`,
        `Температура: ${cur.temp_C}°C (ощущается как ${cur.FeelsLikeC}°C)`,
        `Описание: ${desc}`,
        `Влажность: ${cur.humidity}%`,
        `Ветер: ${cur.windspeedKmph} км/ч (${cur.winddir16Point})`,
        `Облачность: ${cur.cloudcover}%`,
    ];
    const today = data.weather && data.weather[0];
    if (today) {
        lines.push(`Сегодня: мин ${today.mintempC}°C, макс ${today.maxtempC}°C`);
    }
    lines.push('Источник: https://wttr.in/' + encodeURIComponent(city));
    return lines.join('\n');
}

createMcpServer({
    name: "knowledge-api-mcp",
    version: "1.0.0",
    tools: [
        {
            name: "WikipediaSearch",
            description: "Поиск фактов в Википедии (без API-ключа). Возвращает до 5 статей с кратким вступлением и ссылками. Для быстрых фактических справок.",
            inputSchema: {
                type: "object",
                properties: {
                    query: { type: "string", description: "Поисковый запрос, например 'столица Австралии'" },
                    lang: { type: "string", description: "Язык раздела Википедии (ru/en/de и т.д.), по умолчанию ru" }
                },
                required: ["query"]
            }
        },
        {
            name: "Weather",
            description: "Текущая погода в городе (через wttr.in, без API-ключа). Вводить название города (можно на русском).",
            inputSchema: {
                type: "object",
                properties: {
                    city: { type: "string", description: "Название города, например 'Москва' или 'London'" }
                },
                required: ["city"]
            }
        }
    ],
    handlers: {
        WikipediaSearch: async (args) => {
            const query = (args.query || '').trim();
            if (!query) { throw new Error("WikipediaSearch: укажи 'query'."); }
            const lang = /^[a-z]{2,3}$/.test(args.lang || '') ? args.lang : 'ru';
            return await wikipediaSearch(query, lang);
        },
        Weather: async (args) => {
            const city = (args.city || '').trim();
            if (!city) { throw new Error("Weather: укажи 'city'."); }
            return await weather(city);
        }
    }
});
