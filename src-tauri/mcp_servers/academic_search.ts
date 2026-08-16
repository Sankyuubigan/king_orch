// AcademicSearch (Deno, zero-dependency, БЕЗ API-ключей).
// Поиск научных статей: OpenAlex (primary, https://api.openalex.org/works) →
// Crossref (fallback, https://api.crossref.org/works). Оба — публичные keyless API.
// OpenAlex позволяет собрать абстракт из инвертированного индекса.
import { createMcpServer, log } from "./mcp_base.ts";
import { request } from "./web_http.ts";

const OPENALEX = 'https://api.openalex.org/works';
const CROSSREF = 'https://api.crossref.org/works';
const UA = 'king-orch-research/1.0 (mailto: research@kingorch.local)';

interface AcademicArgs {
    query: string;
    per_page?: number;
    sort?: string;
}

// Инвертированный индекс абстракта OpenAlex: {"word": [0, 5], ...} → текст по позициям.
function rebuildAbstract(inverted: Record<string, number[]> | null | undefined): string {
    if (!inverted) return '';
    const words: { pos: number; word: string }[] = [];
    for (const [word, positions] of Object.entries(inverted)) {
        for (const pos of positions) words.push({ pos, word });
    }
    words.sort((a, b) => a.pos - b.pos);
    return words.map((w) => w.word).join(' ').slice(0, 600);
}

async function searchOpenAlex(query: string, perPage: number, sort: string): Promise<string> {
    const sortParam = sort === 'newest' ? 'publication_year:desc' : 'relevance_score:desc';
    const url = `${OPENALEX}?search=${encodeURIComponent(query)}&per-page=${perPage}&sort=${sortParam}`;
    const res = await request(url, { headers: { 'User-Agent': UA }, timeoutMs: 15000, maxBytes: 3 * 1024 * 1024 });
    if (res.status !== 200) throw new Error(`OpenAlex HTTP ${res.status}`);
    const json = JSON.parse(res.text);
    const items: any[] = (json.results || []).filter((w: any) => w.display_name);
    if (items.length === 0) return '';

    return items.map((w, i) => {
        const authors = (w.authorships || []).slice(0, 5).map((a: any) => (a.author || {}).display_name || '').filter(Boolean).join(', ');
        const journal = w.primary_location && w.primary_location.source ? w.primary_location.source.display_name : '—';
        const abstract = rebuildAbstract(w.abstract_inverted_index);
        const lines = [
            `[${i + 1}] ${w.display_name} (${w.publication_year || '—'})`,
            `Авторы: ${authors || '—'}`,
            `Журнал: ${journal} | Цитирований: ${w.cited_by_count || 0}`,
        ];
        if (w.doi) {
            const doi = w.doi.startsWith('http') ? w.doi.replace(/^https?:\/\/doi\.org\//, '') : w.doi;
            lines.push(`DOI: ${doi} | URL: https://doi.org/${doi}`);
        }
        if (abstract) lines.push(`Абстракт: ${abstract}${abstract.length >= 600 ? '…' : ''}`);
        return lines.join('\n');
    }).join('\n\n');
}

async function searchCrossref(query: string, perPage: number, sort: string): Promise<string> {
    const url = `${CROSSREF}?query=${encodeURIComponent(query)}&rows=${perPage}&sort=${sort === 'newest' ? 'published' : 'relevance'}`;
    const res = await request(url, { headers: { 'User-Agent': UA }, timeoutMs: 15000, maxBytes: 3 * 1024 * 1024 });
    if (res.status !== 200) throw new Error(`Crossref HTTP ${res.status}`);
    const json = JSON.parse(res.text);
    const items: any[] = (json.message && json.message.items) || [];
    if (items.length === 0) return '';

    return items.map((w, i) => {
        const title = Array.isArray(w.title) && w.title[0] ? w.title[0].slice(0, 250) : '—';
        const authors = (w.author || []).slice(0, 5).map((a: any) => [a.given, a.family].filter(Boolean).join(' ')).join(', ');
        const journal = Array.isArray(w['container-title']) && w['container-title'][0] ? w['container-title'][0] : '—';
        const year = w['published-print'] && w['published-print']['date-parts'] ? w['published-print']['date-parts'][0][0] : (w['published-online'] && w['published-online']['date-parts'] ? w['published-online']['date-parts'][0][0] : '—');
        const lines = [
            `[${i + 1}] ${title} (${year})`,
            `Авторы: ${authors || '—'}`,
            `Журнал: ${journal} | Цитирований: ${w['is-referenced-by-count'] || 0}`,
        ];
        if (w.DOI) lines.push(`DOI: ${w.DOI} | URL: https://doi.org/${w.DOI}`);
        return lines.join('\n');
    }).join('\n\n');
}

async function academicSearch(args: AcademicArgs): Promise<string> {
    const query = (args.query || '').trim();
    if (!query) throw new Error("AcademicSearch: укажи 'query'.");
    const perPage = Math.max(1, Math.min(15, parseInt(String(args.per_page), 10) || 8));
    const sort = args.sort === 'newest' ? 'newest' : 'relevance';

    // Primary: OpenAlex. Fallback: Crossref (при ошибке/пустоте).
    let lastError = '';
    try {
        const out = await searchOpenAlex(query, perPage, sort);
        if (out) return out;
        lastError = 'OpenAlex вернул пусто';
    } catch (e) {
        lastError = `OpenAlex: ${(e as Error).message}`;
        log(`[AcademicSearch] ${lastError}, пробую Crossref`);
    }
    try {
        const out = await searchCrossref(query, perPage, sort);
        if (out) return out;
        return `Статьи не найдены. (${lastError}; Crossref тоже пусто)`;
    } catch (e) {
        throw new Error(`AcademicSearch: оба источника недоступны — ${lastError}; Crossref: ${(e as Error).message}`);
    }
}

createMcpServer({
    name: "academic-search-mcp",
    version: "1.0.0",
    tools: [{
        name: "AcademicSearch",
        description: "Поиск научных статей БЕЗ API-ключей (OpenAlex + fallback Crossref): заголовок, авторы, год, журнал, DOI, число цитирований, абстракт. Используй для: «научные исследования по X», «статьи по X», «что пишут в науке о X», «актуальные работы по X». Сортировка по релевантности или свежести.",
        inputSchema: {
            type: "object",
            properties: {
                query: { type: "string", description: "Поисковый запрос (лучше на английском: 'LLM agentic workflow evaluation')" },
                per_page: { type: "number", description: "Сколько результатов (по умолчанию 8, макс 15)" },
                sort: { type: "string", description: "relevance (по умолчанию) | newest" }
            },
            required: ["query"]
        }
    }],
    handlers: {
        AcademicSearch: async (args) => {
            try {
                return await academicSearch(args as AcademicArgs);
            } catch (e) {
                log(`[AcademicSearch] ошибка: ${(e as Error).message}`);
                throw e;
            }
        }
    }
});