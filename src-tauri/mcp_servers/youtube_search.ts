// YoutubeSearch (Deno, zero-dependency, БЕЗ API-ключей).
// Парсинг страницы результатов youtube.com/results (ytInitialData JSON внутри HTML):
// название, канал, просмотры, длительность, дата публикации, ссылка.
// При consent-блокировке/защите — честная ошибка с причиной.
import { createMcpServer, log } from "./mcp_base.ts";
import { request } from "./web_http.ts";

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36';

interface YoutubeArgs {
    query: string;
    max_results?: number;
}

// Найти var ytInitialData = {...}; и распарсить как JSON.
function extractInitialData(html: string): any {
    const marker = 'var ytInitialData = ';
    const start = html.indexOf(marker);
    if (start < 0) return null;
    const jsonStart = start + marker.length;
    const end = html.indexOf(';</script>', jsonStart);
    if (end < 0) return null;
    try {
        return JSON.parse(html.slice(jsonStart, end));
    } catch {
        return null;
    }
}

interface VideoInfo {
    id: string;
    title: string;
    channel: string;
    views: string;
    length: string;
    published: string;
}

// Рекурсивный обход JSON: собрать все videoRenderer объекты.
function collectVideos(node: any, out: VideoInfo[], limit: number): void {
    if (!node || typeof node !== 'object' || out.length >= limit) return;
    if (node.videoRenderer) {
        const v = node.videoRenderer;
        const id = v.videoId;
        const title = v.title && v.title.runs ? v.title.runs.map((r: any) => r.text || '').join('') : '';
        const channel = v.ownerText && v.ownerText.runs ? v.ownerText.runs.map((r: any) => r.text || '').join('') : '';
        const views = v.viewCountText && v.viewCountText.simpleText ? v.viewCountText.simpleText : '';
        const length = v.lengthText && v.lengthText.simpleText ? v.lengthText.simpleText : '';
        const published = v.publishedTimeText && v.publishedTimeText.simpleText ? v.publishedTimeText.simpleText : '';
        if (id && title) out.push({ id, title, channel, views, length, published });
        if (out.length >= limit) return;
    }
    for (const key of Object.keys(node)) {
        if (key === 'videoRenderer') continue;
        collectVideos(node[key], out, limit);
        if (out.length >= limit) return;
    }
}

function formatDuration(seconds: number): string {
    if (!seconds || seconds <= 0) return '';
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = seconds % 60;
    const pad = (n: number) => String(n).padStart(2, '0');
    return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

async function youtubeSearch(args: YoutubeArgs): Promise<string> {
    const query = (args.query || '').trim();
    if (!query) throw new Error("YoutubeSearch: укажи 'query'.");
    const maxResults = Math.max(1, Math.min(10, parseInt(String(args.max_results), 10) || 8));

    const url = `https://www.youtube.com/results?search_query=${encodeURIComponent(query)}`;
    const res = await request(url, {
        headers: { 'User-Agent': UA, 'Accept-Language': 'ru-RU,ru;q=0.9,en-US;q=0.8,en;q=0.7' },
        timeoutMs: 20000,
        maxBytes: 6 * 1024 * 1024,
    });
    if (res.status !== 200) throw new Error(`YouTube вернул HTTP ${res.status}`);

    // Consent-перехват (EU): страница с выбором региона вместо результатов.
    if (/consent\.youtube\.com/i.test(res.url) || /<form[^>]*name=["']consent/i.test(res.text)) {
        throw new Error('YouTube показал consent-страницу (региональная защита) — попробуй повторить позже или поищи это видео через WebSearch.');
    }

    const data = extractInitialData(res.text);
    if (!data) {
        throw new Error('Не удалось распарсить результаты YouTube (нет ytInitialData — вероятно, антибот-блок). Попробуй WebSearch.');
    }

    const videos: VideoInfo[] = [];
    collectVideos(data, videos, maxResults);
    if (videos.length === 0) {
        return 'По запросу ничего не найдено на YouTube. Попробуй другие ключевые слова или WebSearch.';
    }

    return videos.map((v, i) => {
        const length = v.length || formatDuration(0);
        const meta = [v.channel, v.views ? `👁 ${v.views}` : '', v.length ? `⏱ ${v.length}` : '', v.published].filter(Boolean).join(' | ');
        return `[${i + 1}] ${v.title}\n${meta}\nURL: https://www.youtube.com/watch?v=${v.id}`;
    }).join('\n\n');
}

createMcpServer({
    name: "youtube-search-mcp",
    version: "1.0.0",
    tools: [{
        name: "YoutubeSearch",
        description: "Поиск видео на YouTube БЕЗ API-ключей (парсинг страницы результатов): название, канал, просмотры, длительность, дата публикации, ссылка. Используй для: «найди видео/туториал по X», «лекции по X». Для транскрипта найденного видео используются другие инструменты (youtube_mcp).",
        inputSchema: {
            type: "object",
            properties: {
                query: { type: "string", description: "Поисковый запрос (например: 'rust tauri tutorial')" },
                max_results: { type: "number", description: "Сколько результатов (по умолчанию 8, макс 10)" }
            },
            required: ["query"]
        }
    }],
    handlers: {
        YoutubeSearch: async (args) => {
            try {
                return await youtubeSearch(args as YoutubeArgs);
            } catch (e) {
                log(`[YoutubeSearch] ошибка: ${(e as Error).message}`);
                throw e;
            }
        }
    }
});