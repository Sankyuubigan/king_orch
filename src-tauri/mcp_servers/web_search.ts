// Мульти-движковый веб-поиск (Deno, zero-dependency, без API-ключей).
// Движки: wiby, duckduckgo (primary), marginalia, brave, startpage, sogou, baidu, bing, juejin, csdn, exa.
// Логирование статусов движков — в stderr (попадает в лог приложения как [MCP Stderr]).
import { Buffer } from "node:buffer";
import { createMcpServer } from "./mcp_base.ts";
import {
    recordEngineCall,
} from "./search_stats.ts";
import {
    cacheGet, cachePut, cacheKey,
} from "./search_cache.ts";
import {
    request, stripTags, parseBlocksByClass, firstHref, normalizeHttpUrl,
} from "./web_http.ts";

const MAX_RESULTS = 8;
const CHAIN_PAUSE_MS = 800;
const ENGINE_DEADLINE_MS = 15000;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// Защита от зависших движков: жёсткий дедлайн на каждый движок.
function withDeadline<T>(promise: Promise<T>, ms: number, name: string): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error('движок завис (дедлайн)')), ms);
    });
    return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

const UA_CHROME_112 = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/112.0.0.0 Safari/537.36';

// Человекочитаемое описание HTTP-ошибок движков (пишется в лог [ENGINE]).
function httpStatusError(status: number): string {
    if (status === 429) return 'лимит запросов (rate-limit) — попробуй позже';
    if (status === 521) return 'антибот-защита сайта (521)';
    if (status === 403) return 'доступ запрещён (403) — вероятна блокировка сервисом';
    if (status === 404) return 'HTTP 404 (страница/эндпоинт не найден)';
    if (status === 503) return 'сервис временно недоступен (503)';
    return 'HTTP ' + status;
}

// ─────────────────────────── DuckDuckGo (primary) ───────────────────────────

// DDG при автоматизированных запросах отдаёт антибот-страницу "anomaly" (202, JS-challenge).
function isAnomaly(status: number, body: string): boolean {
    return status >= 200 && status < 300 && (body.includes('anomaly') || body.includes('execDeep'));
}

function ddgNormalizeUrl(raw: string): string {
    let url = normalizeHttpUrl(raw, 'https://duckduckgo.com/');
    if (!url) return '';
    // Развернуть внутренние ссылки-редиректы duckduckgo.com/l/?uddg=...
    try {
        const u = new URL(url);
        if (u.hostname === 'duckduckgo.com' && u.pathname.startsWith('/l/') && u.searchParams.get('uddg')) {
            const decoded = decodeURIComponent(u.searchParams.get('uddg')!);
            url = normalizeHttpUrl(decoded, 'https://duckduckgo.com/');
        }
    } catch { /* оставляем как есть */ }
    return url;
}

interface SearchResult {
    title: string;
    url: string;
    snippet: string;
    source?: string;
}

async function searchDuckDuckGo(query: string, limit: number): Promise<SearchResult[]> {
    const all: SearchResult[] = [];
    const errors: string[] = [];

    // 1. Instant Answer API — мгновенный, но для многих запросов пуст.
    try {
        const res = await request(
            `https://api.duckduckgo.com/?q=${encodeURIComponent(query)}&format=json&no_html=1&skip_disambig=1`,
            { headers: { 'Accept': 'application/json' }, timeoutMs: 4000 }
        );
        if (res.status === 200) {
            let json: any;
            try { json = JSON.parse(res.text); } catch { json = null; }
            if (json) {
                const pushTopic = (t: any) => {
                    const url = ddgNormalizeUrl(t.FirstURL || t.Url || '');
                    const text = stripTags(t.Text || t.Result || '');
                    if (url && text) all.push({ title: (t.Text || url).slice(0, 120), url, snippet: text.slice(0, 400) });
                };
                if (json.AbstractText) {
                    all.push({ title: json.Heading || 'Краткий ответ', url: json.AbstractURL || '', snippet: json.AbstractText });
                }
                const topics = Array.isArray(json.RelatedTopics) ? json.RelatedTopics : [];
                for (const t of topics) {
                    if (t.Topics && Array.isArray(t.Topics)) { t.Topics.forEach(pushTopic); } else { pushTopic(t); }
                }
            }
        }
    } catch (e) { errors.push('Instant Answer: ' + (e as Error).message); }

    if (all.length < 3) {
        await sleep(700);
        // 2. html POST с ретраем при антибот-странице (anomaly).
        for (let i = 0; i < 2; i++) {
            try {
                const res = await request('https://html.duckduckgo.com/html/', {
                    method: 'POST',
                    body: 'q=' + encodeURIComponent(query),
                    headers: { 'Content-Type': 'application/x-www-form-urlencoded', 'Referer': 'https://html.duckduckgo.com/' },
                    timeoutMs: 6000,
                });
                if (res.status !== 200) throw new Error(httpStatusError(res.status));
                if (isAnomaly(res.status, res.text)) { throw new Error('аномальная страница (anomaly)'); }
                for (const block of parseBlocksByClass(res.text, 'result')) {
                    const a = block.match(/class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/i);
                    if (!a) continue;
                    const url = ddgNormalizeUrl(a[1]);
                    const title = stripTags(a[2]);
                    if (!url || !title) continue;
                    const snip = block.match(/class="[^"]*result__snippet[^"]*"[^>]*>([\s\S]*?)<\/a>/i);
                    all.push({ title, url, snippet: snip ? stripTags(snip[1]) : '' });
                }
                if (all.length >= 3) break;
            } catch (e) { errors.push('DDG html: ' + (e as Error).message); }
            await sleep(1000);
        }
    }

    if (all.length < 3) {
        await sleep(700);
        // 3. lite POST как последний fallback.
        try {
            const res = await request('https://lite.duckduckgo.com/lite/', {
                method: 'POST',
                body: 'q=' + encodeURIComponent(query),
                headers: { 'Content-Type': 'application/x-www-form-urlencoded', 'Referer': 'https://lite.duckduckgo.com/' },
                timeoutMs: 6000,
            });
            if (res.status !== 200) throw new Error(httpStatusError(res.status));
            if (isAnomaly(res.status, res.text)) { throw new Error('аномальная страница (anomaly)'); }
            const linkRe = /<a[^>]+class='result-link'[^>]+href="([^"]+)"[^>]*>([\s\S]*?)<\/a>|<a[^>]+href="([^"]+)"[^>]+class='result-link'[^>]*>([\s\S]*?)<\/a>/g;
            let m: RegExpExecArray | null;
            while ((m = linkRe.exec(res.text)) !== null) {
                const url = ddgNormalizeUrl(m[1] || m[3]);
                const title = stripTags(m[2] || m[4]);
                if (!url || !title) continue;
                const after = res.text.slice(m.index + m[0].length);
                const snip = after.match(/<td class='result-snippet'>([\s\S]*?)<\/td>/);
                all.push({ title, url, snippet: snip ? stripTags(snip[1]) : '' });
            }
        } catch (e) { errors.push('DDG lite: ' + (e as Error).message); }
    }

    if (all.length === 0) {
        throw new Error(errors.join('; ') || 'сервис вернул пустой ответ (антибот-защита)');
    }
    return all;
}

// ─────────────────────────── Brave ───────────────────────────

async function searchBrave(query: string, limit: number): Promise<SearchResult[]> {
    const res = await request(
        `https://search.brave.com/search?q=${encodeURIComponent(query)}&source=web&offset=0`,
        {
            headers: {
                'User-Agent': UA_CHROME_112,
                'Referer': 'https://duckduckgo.com/',
                'sec-ch-ua': '"Chromium";v="112", "Google Chrome";v="112", "Not:A-Brand";v="99"',
                'sec-ch-ua-mobile': '?0',
                'sec-ch-ua-platform': '"Windows"',
                'sec-fetch-site': 'same-origin',
                'sec-fetch-mode': 'navigate',
                'sec-fetch-dest': 'document',
            },
            timeoutMs: 5000,
        }
    );
    if (res.status !== 200) throw new Error(httpStatusError(res.status));
    if (/verify you are human|unusual traffic|access denied/i.test(res.text) && !/data-pos="/.test(res.text)) {
        throw new Error('заблокирован (капча)');
    }
    const out: SearchResult[] = [];
    for (const block of parseBlocksByClass(res.text, 'snippet')) {
        const url = firstHref(block);
        if (!url) continue;
        let title = '';
        const titleMatch = block.match(/class="[^"]*search-snippet-title[^"]*"[^>]*>([\s\S]*?)</i);
        if (titleMatch) title = stripTags(titleMatch[1]);
        if (!title) {
            const anchor = block.match(/<a[^>]+href="https?:\/\/[^"]+"[^>]*>([\s\S]*?)<\/a>/i);
            if (anchor) title = stripTags(anchor[1]);
        }
        if (!title) continue;
        const descMatch = block.match(/class="[^"]*(?:generic-snippet|snippet-description|result-description)[^"]*"[^>]*>([\s\S]*?)</i);
        const sourceMatch = block.match(/class="[^"]*site-name-wrapper[^"]*"[^>]*>([\s\S]*?)</i);
        out.push({
            title: title.slice(0, 200),
            url,
            snippet: descMatch ? stripTags(descMatch[1]).slice(0, 400) : '',
            source: sourceMatch ? stripTags(sourceMatch[1]).slice(0, 80) : '',
        });
    }
    return out;
}

// ─────────────────────────── Startpage ───────────────────────────

async function searchStartpage(query: string, limit: number): Promise<SearchResult[]> {
    const home = await request('https://www.startpage.com/', { timeoutMs: 6000 });
    if (home.status !== 200) throw new Error(httpStatusError(home.status));
    // Startpage закрыт Anubis PoW-челленджем (JS proof-of-work) — без браузера не пройти.
    if (home.text.includes('anubis_challenge') || home.text.includes('anubis')) {
        throw new Error('Anubis PoW-челлендж — нужен браузер, движок не работает без него');
    }
    const sc = (home.text.match(/name="sc"\s+value="([^"]+)"/i) || home.text.match(/value="([^"]+)"[^>]*name="sc"/i) || [])[1];
    if (!sc) throw new Error('не найден sc-токен');

    await sleep(1500);
    const postHeaders = {
        'Content-Type': 'application/x-www-form-urlencoded',
        'Origin': 'https://www.startpage.com',
        'Referer': 'https://www.startpage.com/',
    };
    let res = await request('https://www.startpage.com/sp/search', {
        method: 'POST',
        body: new URLSearchParams({ query, cat: 'web', t: 'device', sc, abp: '1', abd: '1', abe: '1' }).toString(),
        headers: postHeaders,
        timeoutMs: 6000,
    });
    if (res.status !== 200) throw new Error(httpStatusError(res.status));

    // Интерстициальная страница: первый запрос возвращает скрипт с payload, который надо отправить ещё раз.
    const dataMatch = res.text.match(/var data = (\{[\s\S]*?\});/);
    if (dataMatch) {
        let payload: any = null;
        try { payload = JSON.parse(dataMatch[1]); } catch { /* ignore */ }
        if (payload && payload.sgt) {
            await sleep(700);
            res = await request('https://www.startpage.com/sp/search', {
                method: 'POST',
                body: new URLSearchParams({ query, sgt: payload.sgt, cat: 'web', t: 'device', sc, abp: '1', abd: '1', abe: '1' }).toString(),
                headers: postHeaders,
                timeoutMs: 6000,
            });
            if (res.status !== 200) throw new Error(httpStatusError(res.status));
        }
    }
    if (/\/sp\/captcha|verify you are human|unusual traffic/i.test(res.text)) {
        throw new Error('капча');
    }

    const out: SearchResult[] = [];
    for (const block of parseBlocksByClass(res.text, 'result')) {
        const a = block.match(/<a[^>]+class="[^"]*result-link[^"]*"[^>]+href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/i) ||
            block.match(/<a[^>]+href="([^"]+)"[^>]+class="[^"]*result-link[^"]*"[^>]*>([\s\S]*?)<\/a>/i);
        if (!a) continue;
        const url = normalizeHttpUrl(a[1], 'https://www.startpage.com/');
        const titleMatch = a[2].match(/<h2[^>]*>([\s\S]*?)<\/h2>/i);
        const title = stripTags(titleMatch ? titleMatch[1] : a[2]);
        if (!url || !title) continue;
        const descMatch = block.match(/class="[^"]*description[^"]*"[^>]*>([\s\S]*?)</i);
        let source = '';
        try { source = new URL(url).hostname; } catch { /* ignore */ }
        out.push({ title: title.slice(0, 200), url, snippet: descMatch ? stripTags(descMatch[1]).slice(0, 400) : '', source });
    }
    return out;
}

// ─────────────────────────── Sogou ───────────────────────────

function sogouExpandUrl(href: string): string {
    try {
        const u = new URL(href, 'https://www.sogou.com/');
        for (const key of ['url', 'u', 'link']) {
            const val = u.searchParams.get(key);
            if (val) return normalizeHttpUrl(decodeURIComponent(val), 'https://www.sogou.com/');
        }
        return normalizeHttpUrl(href, 'https://www.sogou.com/');
    } catch { return ''; }
}

async function searchSogou(query: string, limit: number): Promise<SearchResult[]> {
    const res = await request(
        `https://www.sogou.com/web?query=${encodeURIComponent(query)}&page=1&ie=utf8`,
        { headers: { 'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0 Safari/537.36', 'Referer': 'https://www.sogou.com/' }, timeoutMs: 5000 }
    );
    if (res.status !== 200) throw new Error(httpStatusError(res.status));
    if (/antispider|请输入验证码|访问过于频繁|搜狗搜索验证/i.test(res.text)) {
        throw new Error('заблокирован (челлендж)');
    }
    const out: SearchResult[] = [];
    for (const cls of ['vrwrap', 'rb']) {
        for (const block of parseBlocksByClass(res.text, cls)) {
            const a = block.match(/<a[^>]+href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/i);
            if (!a) continue;
            const url = sogouExpandUrl(a[1]);
            const title = stripTags(a[2]);
            if (!url || !title) continue;
            const descMatch = block.match(/class="[^"]*(?:str_info|ft|text-layout|fz-mid)[^"]*"[^>]*>([\s\S]*?)</i) ||
                block.match(/<p[^>]*>([\s\S]*?)<\/p>/i);
            const citeMatch = block.match(/class="[^"]*(?:citeurl|g|url)[^"]*"[^>]*>([\s\S]*?)</i);
            out.push({
                title: title.slice(0, 200),
                url,
                snippet: descMatch ? stripTags(descMatch[1]).slice(0, 400) : '',
                source: citeMatch ? stripTags(citeMatch[1]).slice(0, 80) : '',
            });
        }
    }
    return out;
}

// ─────────────────────────── Baidu ───────────────────────────

async function searchBaidu(query: string, limit: number): Promise<SearchResult[]> {
    const res = await request(
        `https://www.baidu.com/s?wd=${encodeURIComponent(query)}&pn=0&ie=utf-8&tn=baidu`,
        { headers: { 'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36', 'Accept-Language': 'zh-CN,zh;q=0.9' }, timeoutMs: 5000 }
    );
    if (res.status !== 200) throw new Error(httpStatusError(res.status));
    if (/wappass|百度安全验证|访问过于频繁|请输入验证码/i.test(res.text)) {
        throw new Error('заблокирован (антибот)');
    }
    const out: SearchResult[] = [];
    const h3Re = /<h3[^>]*>([\s\S]*?)<\/h3>/gi;
    let m: RegExpExecArray | null;
    while ((m = h3Re.exec(res.text)) !== null) {
        const a = m[1].match(/<a[^>]+href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/i);
        if (!a) continue;
        const url = normalizeHttpUrl(a[1], 'https://www.baidu.com/');
        const title = stripTags(a[2]);
        if (!url || !title) continue;
        const after = res.text.slice(m.index);
        const descMatch = after.match(/class="[^"]*(?:c-span-last|c-color-text|cos-row)[^"]*"[^>]*>([\s\S]*?)</i);
        out.push({ title: title.slice(0, 200), url, snippet: descMatch ? stripTags(descMatch[1]).slice(0, 400) : '', source: 'baidu.com' });
    }
    return out;
}

// ─────────────────────────── Bing (request-only) ───────────────────────────

function isBingBlocked(text: string): boolean {
    const keywords = ['captcha', 'verify you are human', 'access denied', 'blocked', 'too many requests', '请验证', '验证码'];
    const hits = keywords.filter((k) => text.toLowerCase().includes(k));
    return hits.length >= 2 || (text.toLowerCase().includes('captcha') && !/<li class="b_algo"/.test(text));
}

function bingRealUrl(href: string): string {
    try {
        const u = new URL(href, 'https://www.bing.com/');
        if (u.hostname.includes('bing.com')) {
            const b64 = u.searchParams.get('u');
            if (b64) {
                const decoded = Buffer.from(b64, 'base64url').toString('utf8');
                return normalizeHttpUrl(decoded, 'https://www.bing.com/');
            }
            return '';
        }
        return normalizeHttpUrl(href, 'https://www.bing.com/');
    } catch { return ''; }
}

async function searchBing(query: string, limit: number): Promise<SearchResult[]> {
    const headers = { 'Cache-Control': 'no-cache', 'Pragma': 'no-cache' };
    let res: import("./web_http.ts").HttpResponse | null;
    try {
        res = await request(
            `https://www.bing.com/search?q=${encodeURIComponent(query)}&count=10&setlang=ru-ru`,
            { headers, timeoutMs: 5000 }
        );
    } catch (e) {
        res = null;
    }
    if (!res || res.status !== 200) {
        res = await request(
            `https://cn.bing.com/search?q=${encodeURIComponent(query)}&count=10&setlang=ru-ru&ensearch=0`,
            { headers, timeoutMs: 5000 }
        );
    }
    if (res.status !== 200) throw new Error(httpStatusError(res.status));
    if (isBingBlocked(res.text)) throw new Error('заблокирован (капча) — попробуйте позже');
    const out: SearchResult[] = [];
    for (const block of parseBlocksByClass(res.text, 'b_algo')) {
        const a = block.match(/<h2[^>]*>[\s\S]*?<a[^>]+href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/i);
        if (!a) continue;
        const url = bingRealUrl(a[1]);
        const title = stripTags(a[2]);
        if (!url || !title) continue;
        const descMatch = block.match(/<p[^>]*>([\s\S]*?)<\/p>/i);
        const citeMatch = block.match(/<cite[^>]*>([\s\S]*?)<\/cite>/i);
        out.push({
            title: title.slice(0, 200),
            url,
            snippet: descMatch ? stripTags(descMatch[1]).slice(0, 400) : '',
            source: citeMatch ? stripTags(citeMatch[1]).slice(0, 80) : '',
        });
    }
    return out;
}

// ─────────────────────────── Juejin (JSON API) ───────────────────────────

async function searchJuejin(query: string, limit: number): Promise<SearchResult[]> {
    const url = `https://api.juejin.cn/search_api/v1/search?aid=2608&uuid=7259393293459605051&spider=0&query=${encodeURIComponent(query)}&id_type=0&cursor=0&limit=${Math.min(limit, 20)}&search_type=0&sort_type=0&version=1`;
    const res = await request(url, { headers: { 'Host': 'api.juejin.cn', 'User-Agent': UA_CHROME_112, 'Accept': 'application/json' }, timeoutMs: 4000 });
    if (res.status !== 200) throw new Error(httpStatusError(res.status));
    let json: any;
    try { json = JSON.parse(res.text); } catch { throw new Error('невалидный JSON'); }
    if (json.err_no !== 0) throw new Error('err_no=' + json.err_no);
    const out: SearchResult[] = [];
    for (const item of json.data || []) {
        const m = item.result_model || {};
        const info = m.article_info || {};
        const author = (m.author_user_info || {}).user_name || '';
        const url2 = m.article_id ? `https://juejin.cn/post/${m.article_id}` : '';
        const title = stripTags(item.title_highlight || info.title || '');
        if (!title || !url2) continue;
        const desc = stripTags(item.content_highlight || info.brief_content || '').slice(0, 300);
        const meta = [m.category ? `Категория: ${m.category}` : '', author ? `Автор: ${author}` : '', info.digg_count ? `👍 ${info.digg_count}` : ''].filter(Boolean).join(' · ');
        out.push({ title: title.slice(0, 200), url: url2, snippet: [desc, meta].filter(Boolean).join('\n'), source: author || 'juejin.cn' });
    }
    return out;
}

// ─────────────────────────── CSDN (JSON API) ───────────────────────────

async function searchCsdn(query: string, limit: number): Promise<SearchResult[]> {
    const url = `https://so.csdn.net/api/v3/search?q=${encodeURIComponent(query)}&t=all&p=1&s=0&tm=0&lv=-1&ft=0&ct=-1&pnt=-1&ry=-1&ss=-1&dct=-1&vt=-1&dms=-1&vip=-1&hit=1&ec=1&c_p=1&c_r=1&showSrc=1&showBlog=1`;
    const res = await request(url, {
        headers: { 'User-Agent': 'Apifox/1.0.0 (https://apifox.com)', 'Host': 'so.csdn.net', 'Accept': 'application/json' },
        timeoutMs: 4000,
    });
    if (res.status !== 200) throw new Error(httpStatusError(res.status));
    let json: any;
    try { json = JSON.parse(res.text); } catch { throw new Error('невалидный JSON'); }
    const out: SearchResult[] = [];
    for (const r of json.result_vos || []) {
        const title = stripTags(r.title || '');
        const url2 = cleanTrackingUrl(normalizeHttpUrl(r.url_location, 'https://so.csdn.net/'));
        if (!title || !url2) continue;
        out.push({ title: title.slice(0, 200), url: url2, snippet: stripTags(r.digest || '').slice(0, 400), source: r.nickname || 'csdn.net' });
    }
    return out;
}

// ─────────────────────────── Exa (внутренний keyless endpoint) ───────────────────────────

async function searchExa(query: string, limit: number): Promise<SearchResult[]> {
    const payload = JSON.stringify({
        numResults: limit, query, type: 'auto', useAutoprompt: true, domainFilterType: 'include',
        text: true, density: 'compact', resolvedSearchType: 'neural', moderation: true, fastMode: false, rerankerType: 'default',
    });
    const res = await request('https://exa.ai/search/api/search-fast', {
        method: 'POST',
        body: payload,
        timeoutMs: 4000,
        headers: {
            'content-type': 'text/plain;charset=UTF-8',
            'origin': 'https://exa.ai',
            'referer': 'https://exa.ai/',
            'sec-fetch-site': 'same-origin',
            'sec-fetch-mode': 'cors',
            'sec-fetch-dest': 'empty',
            'Accept': '*/*',
        },
    });
    if (res.status !== 200) throw new Error(httpStatusError(res.status));
    let json: any;
    try { json = JSON.parse(res.text); } catch { throw new Error('невалидный JSON'); }
    const out: SearchResult[] = [];
    for (const r of json.results || []) {
        if (!r.url || !r.title) continue;
        const meta = [r.author ? `Автор: ${r.author}` : '', r.publishedDate ? `Опубликовано: ${r.publishedDate}` : ''].filter(Boolean).join('. ');
        let source = '';
        try { source = new URL(r.url).hostname; } catch { /* ignore */ }
        out.push({ title: r.title.slice(0, 200), url: r.url, snippet: meta, source });
    }
    return out;
}

// ─────────────────────────── Wiby (индекс старых сайтов) ───────────────────────────

async function searchWiby(query: string, limit: number): Promise<SearchResult[]> {
    const res = await request(
        `https://wiby.me/?q=${encodeURIComponent(query)}`,
        { headers: { 'User-Agent': UA_CHROME_112 }, timeoutMs: 6000 }
    );
    if (res.status !== 200) throw new Error(httpStatusError(res.status));
    const out: SearchResult[] = [];
    // Результат: <blockquote><a class="tlink" href="URL">Title</a><br><p class="url">URL</p><p>snippet</p></blockquote>
    for (const block of parseBlocksByClass(res.text, 'tlink')) {
        const a = block.match(/<a[^>]+href="([^"]+)"[^>]*>\s*([\s\S]*?)\s*<\/a>/i);
        if (!a) continue;
        const url = normalizeHttpUrl(a[1], 'https://wiby.me/');
        const title = stripTags(a[2]);
        if (!url || !title) continue;
        const after = res.text.slice(res.text.indexOf(block) + block.length);
        const next = after.match(/<p class="url">[\s\S]*?<\/p>\s*<p>([\s\S]*?)<\/p>/i);
        out.push({ title: title.slice(0, 200), url, snippet: next ? stripTags(next[1]).slice(0, 400) : '' });
    }
    // Wiby индексирует в основном англоязычные сайты: для прочих запросов выдача будет пустой (это не ошибка движка).
    return out;
}

// ─────────────────────────── Marginalia (челлендж "Wait For A Moment") ───────────────────────────

// Marginalia при бот-активности требует подождать и перейти по sst-ссылке (1-2 раза).
async function searchMarginalia(query: string, limit: number): Promise<SearchResult[]> {
    const ua = {
        'User-Agent': UA_CHROME_112,
        'Referer': 'https://search.marginalia.nu/',
    };
    const base = `https://search.marginalia.nu/search?query=${encodeURIComponent(query)}`;
    let res = await request(base, { headers: ua, timeoutMs: 8000, maxRedirects: 3 });
    for (let attempt = 0; attempt < 3; attempt++) {
        const isWait = res.text.includes('Wait A Moment') || (res.status >= 500 && res.text.includes('countdown'));
        if (!isWait) break;
        const m = res.text.match(/href="(\/search\?[^"]*sst=[^"]+)"/) ||
            res.text.match(/location\.replace\('([^']+)'/);
        if (!m) throw new Error('челлендж без sst-ссылки');
        const sst = m[1].replace(/&amp;/g, '&').replace(/\\\//g, '/');
        const tr = parseInt((res.text.match(/data-tr="(-?\d+)"/) || [])[1] || '1', 10);
        await sleep(Math.max(tr, 0) * 1000 + 1500);
        res = await request('https://marginalia-search.com' + sst, { headers: ua, timeoutMs: 8000, maxRedirects: 3 });
    }
    if (res.status !== 200) throw new Error(httpStatusError(res.status));
    const out: SearchResult[] = [];
    // Результат: <h2 ...><a href="URL" rel="noopener" ...>Title</a></h2>, сниппет в <p class="mt-2 ...">.
    const re = /<h2[^>]*>\s*<a[^>]+href="(https?:\/\/[^"]+)"[^>]*>\s*([\s\S]*?)\s*<\/a>\s*<\/h2>/gi;
    let m: RegExpExecArray | null;
    while ((m = re.exec(res.text)) !== null) {
        const url = m[1];
        const title = stripTags(m[2]).replace(/&shy;/g, '').trim();
        if (!url || !title) continue;
        const tail = res.text.slice(m.index + m[0].length);
        const snip = tail.match(/<p class="mt-2[^"]*"[^>]*>([\s\S]*?)<\/p>/i);
        out.push({ title: title.slice(0, 200), url, snippet: snip ? stripTags(snip[1]).replace(/&shy;/g, '').slice(0, 400) : '' });
    }
    // Marginalia — англоязычный индекс: для русских запросов выдача пустая (не ошибка движка).
    return out;
}

// ─────────────────────────── Движок-менеджер ───────────────────────────

const ENGINES: Record<string, (q: string, l: number) => Promise<SearchResult[]>> = {
    duckduckgo: searchDuckDuckGo,
    wiby: searchWiby,
    marginalia: searchMarginalia,
    brave: searchBrave,
    startpage: searchStartpage,
    sogou: searchSogou,
    baidu: searchBaidu,
    bing: searchBing,
    juejin: searchJuejin,
    csdn: searchCsdn,
    exa: searchExa,
};
const DEFAULT_ORDER = ['duckduckgo', 'wiby', 'marginalia', 'brave', 'startpage', 'sogou', 'juejin', 'csdn', 'baidu', 'bing', 'exa'];

function dedupe(results: SearchResult[]): SearchResult[] {
    const seen = new Set<string>();
    return results.filter((r) => {
        const key = r.url;
        if (!key || seen.has(key)) return false;
        seen.add(key);
        return true;
    });
}

// Оценка релевантности: какая доля результатов содержит хотя бы одно значимое слово запроса.
function relevantScore(query: string, results: SearchResult[]): number {
    const words = query.toLowerCase().split(/\s+/)
        .map((w) => w.replace(/[^\p{L}\p{N}]/gu, ''))
        .filter((w) => w.length >= 4);
    if (words.length === 0) return 1;
    let hit = 0;
    for (const r of results) {
        const hay = (r.title + ' ' + (r.snippet || '')).toLowerCase();
        if (words.some((w) => hay.includes(w))) hit++;
    }
    return hit / results.length;
}

// Убрать tracking-параметры из URL (CSDN/Juejin добавляют мусор).
function cleanTrackingUrl(raw: string): string {
    if (!raw) return '';
    try {
        const u = new URL(raw);
        const keys = [...u.searchParams.keys()];
        let changed = false;
        for (const k of keys) {
            if (/^(ops_request_misc|request_id|biz_id|utm_|ttm_|spm|from|share_source|share_token)/i.test(k)) {
                u.searchParams.delete(k);
                changed = true;
            }
        }
        return changed ? u.toString() : raw;
    } catch { return raw; }
}

function engineLog(name: string, status: string, extra?: string): void {
    console.error(`[ENGINE] ${name}: ${status}${extra ? ' — ' + extra : ''}`);
}

// Последовательная цепочка фолбэков (по умолчанию): первый рабочий движок выигрывает.
async function runChain(query: string, engines: string[], limit: number): Promise<{ all: SearchResult[]; failures: { name: string; message: string }[] }> {
    const start = Date.now();
    const all: SearchResult[] = [];
    const failures: { name: string; message: string }[] = [];
    const summary: string[] = [];
    for (const name of engines) {
        const t0 = Date.now();
        try {
            const found = await withDeadline(ENGINES[name](query, limit), ENGINE_DEADLINE_MS, name);
            const sec = ((Date.now() - t0) / 1000).toFixed(1);
            if (found.length > 0) {
                const score = relevantScore(query, found);
                recordEngineCall(name, true, Date.now() - t0);
                if (score >= 0.3) {
                    engineLog(name, `OK (${found.length} рез. за ${sec}с, релевантность ${Math.round(score * 100)}%)`);
                    all.push(...found);
                    summary.push(`${name}:${found.length}`);
                    if (all.length >= limit) break;
                } else {
                    engineLog(name, `нерелевантно (${found.length} рез. за ${sec}с, совпало ${Math.round(score * 100)}%)`);
                    all.push(...found); // запасной вариант — вернём, если ничего лучше не найдём
                    summary.push(`${name}:MISS`);
                }
            } else {
                engineLog(name, `пусто (0 рез. за ${sec}с)`);
                summary.push(`${name}:0`);
            }
        } catch (e) {
            const sec = ((Date.now() - t0) / 1000).toFixed(1);
            const msg = (e as Error).message;
            recordEngineCall(name, false, Date.now() - t0, msg);
            engineLog(name, `ОШИБКА за ${sec}с — ${msg}`);
            failures.push({ name, message: msg });
            summary.push(`${name}:ERR`);
        }
        if (all.length < limit) await sleep(CHAIN_PAUSE_MS);
    }
    const totalSec = ((Date.now() - start) / 1000).toFixed(1);
    console.error(`[ENGINE] Итог по запросу "${query.slice(0, 60)}": ${summary.join(', ')} | собрано ${all.length} рез. за ${totalSec}с`);
    return { all, failures };
}

// Параллельный режим (engines задан явно): запросы ко всем движкам одновременно.
async function runParallel(query: string, engines: string[], limit: number): Promise<{ all: SearchResult[]; failures: { name: string; message: string }[] }> {
    const start = Date.now();
    const perEngine = Math.max(2, Math.ceil(limit / engines.length));
    const results = await Promise.all(engines.map(async (name) => {
        const t0 = Date.now();
        try {
            const found = await withDeadline(ENGINES[name](query, perEngine), ENGINE_DEADLINE_MS, name);
            const sec = ((Date.now() - t0) / 1000).toFixed(1);
            if (found.length > 0) {
                recordEngineCall(name, true, Date.now() - t0);
                engineLog(name, `OK (${found.length} рез. за ${sec}с)`);
            } else {
                engineLog(name, `пусто (0 рез. за ${sec}с)`);
            }
            return { name, found, error: null };
        } catch (e) {
            const sec = ((Date.now() - t0) / 1000).toFixed(1);
            const msg = (e as Error).message;
            recordEngineCall(name, false, Date.now() - t0, msg);
            engineLog(name, `ОШИБКА за ${sec}с — ${msg}`);
            return { name, found: [], error: msg };
        }
    }));
    const all = results.flatMap((r) => r.found);
    const failures = results.filter((r) => r.error).map((r) => ({ name: r.name, message: r.error! }));
    const summary = results.map((r) => `${r.name}:${r.error ? 'ERR' : r.found.length}`).join(', ');
    const totalSec = ((Date.now() - start) / 1000).toFixed(1);
    console.error(`[ENGINE] Итог по запросу "${query.slice(0, 60)}": ${summary} | собрано ${all.length} рез. за ${totalSec}с`);
    return { all, failures };
}

function formatResults(all: SearchResult[], failures: { name: string; message: string }[], limit: number): string {
    const deduped = dedupe(all).slice(0, limit);
    const lines = deduped.map((r, i) => {
        const extra = [r.source ? ` (${r.source})` : ''].join('');
        return `[${i + 1}] ${r.title}${extra}\nСсылка: ${r.url}\nОписание: ${r.snippet || '—'}`;
    });
    if (failures.length > 0) {
        lines.push(`\n⚠️ Движки с ошибками: ${failures.map((f) => `${f.name} (${f.message})`).join(', ')}`);
    }
    return lines.join('\n');
}

createMcpServer({
    name: "web-search-mcp",
    version: "1.0.0",
    tools: [{
        name: "WebSearch",
        description: "Поиск в интернете (без API-ключей). Мульти-движок: wiby, duckduckgo, marginalia, brave, startpage, sogou, juejin, csdn, baidu, bing, exa. По умолчанию — последовательная цепочка фолбэков по надёжности; можно указать engines (массив) для параллельного поиска по конкретным движкам.",
        inputSchema: {
            type: "object",
            properties: {
                query: { type: "string", description: "Поисковый запрос" },
                engines: { type: "array", items: { type: "string" }, description: "Опционально: движки (wiby, duckduckgo, marginalia, brave, startpage, sogou, juejin, csdn, baidu, bing, exa). Без него — цепочка фолбэков." },
                limit: { type: "number", description: "Опционально: максимум результатов (по умолчанию 8)" }
            },
            required: ["query"]
        }
    }],
    handlers: {
        WebSearch: async (args) => {
            const query = (args.query || '').trim();
            if (!query) { throw new Error("WebSearch: укажи 'query'."); }
            let limit = Math.max(3, Math.min(20, parseInt(args.limit, 10) || MAX_RESULTS));
            let engines = DEFAULT_ORDER;
            let parallel = false;
            if (Array.isArray(args.engines) && args.engines.length > 0) {
                engines = args.engines.filter((e) => typeof e === 'string').map((e) => e.toLowerCase().trim());
                if (engines.length > 0) { parallel = true; }
            }
            const unknown = engines.filter((e) => !ENGINES[e]);
            if (unknown.length > 0) {
                engineLog('web_search', `неизвестные движки: ${unknown.join(', ')}`);
                engines = engines.filter((e) => ENGINES[e]);
                if (engines.length === 0) { throw new Error(`WebSearch: неизвестные движки: ${unknown.join(', ')}. Доступны: ${Object.keys(ENGINES).join(', ')}`); }
            }
            const cacheKeyStr = cacheKey('web', engines, parallel, query);
            const cached = cacheGet(cacheKeyStr);
            if (cached !== null) {
                engineLog('web_search', `из кеша (query: "${query.slice(0, 60)}")`);
                return cached;
            }
            const { all, failures } = parallel
                ? await runParallel(query, engines, limit)
                : await runChain(query, engines, limit);
            if (all.length === 0) {
                const reasons = failures.map((f) => `${f.name}: ${f.message}`).join('; ');
                throw new Error('Поиск не дал результатов. ' + (reasons ? 'Причины: ' + reasons : 'Все движки вернули пусто.'));
            }
            const out = formatResults(all, failures, limit);
            cachePut(cacheKeyStr, out);
            return out;
        }
    }
});
