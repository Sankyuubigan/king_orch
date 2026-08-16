// GithubSearch (Deno, zero-dependency, БЕЗ API-ключей).
// GitHub REST search API: api.github.com/search/{repositories|issues|commits}.
// Лимит без токена: 10 поисковых запросов/мин (заголовок X-RateLimit-Remaining).
// Обязателен User-Agent, иначе 403. При превышении лимита — честная ошибка.
import { createMcpServer, log } from "./mcp_base.ts";
import { request } from "./web_http.ts";

const API = 'https://api.github.com';
const UA = 'king-orch-research/1.0 (keyless search)';

interface SearchArgs {
    query: string;
    type?: string;
    per_page?: number;
    sort?: string;
}

async function githubSearch(args: SearchArgs): Promise<string> {
    const query = (args.query || '').trim();
    if (!query) throw new Error("GithubSearch: укажи 'query'.");
    const type = ['repositories', 'issues', 'commits'].includes(args.type || '') ? args.type! : 'repositories';
    const perPage = Math.max(1, Math.min(15, parseInt(String(args.per_page), 10) || 8));
    const sort = args.sort === 'stars' || args.sort === 'updated' ? args.sort : 'best_match';

    const url = `${API}/search/${type}?q=${encodeURIComponent(query)}&per_page=${perPage}&sort=${sort}`;
    const res = await request(url, {
        headers: { 'User-Agent': UA, 'Accept': 'application/vnd.github+json' },
        timeoutMs: 15000,
        maxBytes: 2 * 1024 * 1024,
    });

    if (res.status === 403) {
        const limit = res.headers['x-ratelimit-remaining'];
        if (limit === '0') {
            throw new Error('GithubSearch: превышен keyless-лимит GitHub (10 поисковых запросов/мин без токена) — подожди ~1 минуту и повтори.');
        }
        throw new Error(`GithubSearch: GitHub вернул HTTP 403 (${res.text.slice(0, 200)})`);
    }
    if (res.status !== 200) {
        throw new Error(`GithubSearch: GitHub вернул HTTP ${res.status}: ${res.text.slice(0, 200)}`);
    }

    let json: any;
    try { json = JSON.parse(res.text); } catch { throw new Error('GithubSearch: GitHub вернул не JSON.'); }
    const items: any[] = json.items || [];
    if (items.length === 0) return 'По запросу ничего не найдено. Попробуй другие ключевые слова или сократи запрос.';

    if (type === 'repositories') {
        return items.map((r, i) => {
            const desc = (r.description || '—').slice(0, 300);
            return `[${i + 1}] ${r.full_name} ⭐${r.stargazers_count}\n` +
                `Язык: ${r.language || '—'} | Лицензия: ${(r.license && r.license.spdx_id) || '—'} | Обновлён: ${(r.pushed_at || '').slice(0, 10)}\n` +
                `Описание: ${desc}\n` +
                `URL: ${r.html_url}`;
        }).join('\n\n');
    }

    if (type === 'issues') {
        return items.map((r, i) => {
            const labels = (r.labels || []).map((l: any) => l.name).join(', ') || '—';
            return `[${i + 1}] [${r.state}] ${r.title}\n` +
                `Репозиторий: ${r.repository_url.replace('https://api.github.com/repos/', '')} #${r.number}\n` +
                `Комментариев: ${r.comments} | Обновлён: ${(r.updated_at || '').slice(0, 10)} | Метки: ${labels}\n` +
                `URL: ${r.html_url}`;
        }).join('\n\n');
    }

    // commits
    return items.map((r, i) => {
        const c = r.commit || {};
        return `[${i + 1}] ${c.message ? c.message.split('\n')[0].slice(0, 120) : '—'}\n` +
            `Автор: ${(c.author && c.author.name) || '—'} | Дата: ${(c.author && c.author.date || '').slice(0, 10)} | SHA: ${r.sha.slice(0, 8)}\n` +
            `URL: ${r.html_url}`;
    }).join('\n\n');
}

createMcpServer({
    name: "github-search-mcp",
    version: "1.0.0",
    tools: [{
        name: "GithubSearch",
        description: "Поиск по GitHub БЕЗ API-ключей (REST search API, лимит 10 запросов/мин): репозитории (звёзды, язык, лицензия, активность), issues (баги/обсуждения) или commits. Используй для: «найди библиотеку/репозиторий для X», «чем заменить X», «кто поддерживает X», «есть ли баг X в проекте Y». После поиска репозитория детали читай через FetchGithubReadme.",
        inputSchema: {
            type: "object",
            properties: {
                query: { type: "string", description: "Поисковый запрос (например: 'rust tauri webview', 'tesseract ocr wrapper')" },
                type: { type: "string", description: "Что искать: repositories (по умолчанию) | issues | commits" },
                per_page: { type: "number", description: "Сколько результатов (по умолчанию 8, макс 15)" },
                sort: { type: "string", description: "Сортировка: best_match (по умолчанию) | stars | updated" }
            },
            required: ["query"]
        }
    }],
    handlers: {
        GithubSearch: async (args) => {
            try {
                return await githubSearch(args as SearchArgs);
            } catch (e) {
                log(`[GithubSearch] ошибка: ${(e as Error).message}`);
                throw e;
            }
        }
    }
});