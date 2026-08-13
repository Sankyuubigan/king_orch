// Мета-поиск через публичные инстансы SearXNG (zero-dependency, без API-ключей).
// Один запрос → Google/Bing/Brave/DDG через инстанс SearXNG.
//
// Реальность публичных инстансов (2026): большинство защищено — PoW-капча
// (captcha_policy), rate-limit (429), гео-блоки, отключённый JSON API.
// Работающий подход: браузерные заголовки (Referer + sec-fetch-*) на каждый запрос,
// HTML-парсинг (JSON почти нигде не включён), ленивый пул БЕЗ предварительного
// probe (каждая попытка поиска = probe), кулдаун на забанившие инстансы,
// кеш рабочих инстансов (searxng_cache.json).
//
// Логирование — в stderr (попадает в лог приложения как [MCP Stderr]).
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { createMcpServer } from "./mcp_base.ts";
import {
    request, stripTags, normalizeHttpUrl,
} from "./web_http.ts";
import {
    recordEngineCall,
} from "./search_stats.ts";
import {
    cacheGet, cachePut, cacheKey,
} from "./search_cache.ts";

const SEARX_SPACE_URL = 'https://searx.space/data/instances.json';
// Резервные источники списка инстансов (на случай недоступности searx.space):
const INSTANCES_YML_URL = 'https://raw.githubusercontent.com/searxng/searx-instances/master/searxinstances/instances.yml';
const NOPLAGIARISM_JSON_URL = 'https://raw.githubusercontent.com/NoPlagiarism/instances-list/master/instances/search/searx/all.json';
const DISCOVERY_TIMEOUT_MS = 15000;
const DISCOVERY_MAX_BYTES = 8 * 1024 * 1024;
const CACHE_TTL_MS = 24 * 60 * 60 * 1000;      // возраст кеша рабочих инстансов
const CANDIDATES_TTL_MS = 12 * 60 * 60 * 1000; // как часто обновлять список кандидатов (разморозка пула)
const CACHE_FILE = 'searxng_cache.json';
const CACHE_VERSION = 2;
const SEARCH_TIMEOUT_MS = 15000;
const ENGINE_DEADLINE_MS = 20000;
const MAX_CANDIDATES = 30;                      // лучших из searx.space (многие бот-защищены)
const MAX_POOL_CANDIDATES = 100;                // итоговый потолок пула (с фолбэк-добавками; ленивая проба, лимит 7 попыток/поиск)
// Последний резерв discovery: проверенные рабочие + уникальные из сторонних списков.
const FALLBACK_SEED = [
    'https://searx.tiekoetter.com',   // подтверждён рабочим (кеш 12.08.2026)
    'https://sx.catgirl.cloud',       // подтверждён рабочим (кеш 12.08.2026)
    'https://search.mdosch.de',
    'https://search.liuzj.net',
    'https://searx.party',
    'https://etsi.me',
    'https://ooglester.com',
    'https://priv.au',
    'https://searx.ro',
    'https://searxng.site',
    'https://search.mectov.my.id',
    'https://metacat.online',
    'https://nyc1.sx.ggtyler.dev',
    'https://search.080609.xyz',
    'https://search.citw.lgbt',
    'https://search.federicociro.com',
    'https://searx.neocities.org',
    'https://search.seddens.net',
];
const MAX_ATTEMPTS = 7;                         // попыток (инстансов) за один поиск
const MAX_JSON_ATTEMPTS = 2;                    // из них — сначала известные JSON-инстансы
const MAX_RESULTS = 8;
const MIN_POOL_SCORE = 2;                       // ниже — заведомо мёртвые/пустые
const STEP_PAUSE_MS = 700;                      // пауза между инстансами (деликатный темп)
const COOLDOWN_MS = 5 * 60 * 1000;              // кулдаун после отказа инстанса (в рамках процесса)
const HARD_COOLDOWN_MS = 30 * 60 * 1000;        // после капчи/челленджа

// Браузерные заголовки: без них инстансы редиректят /search → / (index).
const BROWSER_HEADERS: Record<string, string> = {
    'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36',
    'Referer': 'https://www.google.com/',
    'sec-ch-ua': '"Chromium";v="126", "Google Chrome";v="126", "Not:A-Brand";v="99"',
    'sec-ch-ua-mobile': '?0',
    'sec-ch-ua-platform': '"Windows"',
    'sec-fetch-site': 'cross-site',
    'sec-fetch-mode': 'navigate',
    'sec-fetch-user': '?1',
    'sec-fetch-dest': 'document',
    'Accept-Language': 'ru-RU,ru;q=0.9,en;q=0.8',
};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function withDeadline<T>(promise: Promise<T>, ms: number, name: string): Promise<T> {
    let timer: ReturnType<typeof setTimeout>;
    const timeout = new Promise<T>((_, reject) => {
        timer = setTimeout(() => reject(new Error('инстанс завис (дедлайн)')), ms);
    });
    return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function searxLog(status: string, extra?: string) {
    console.error(`[SEARX] ${status}${extra ? ' — ' + extra : ''}`);
}

// ─────────────────────────── Кеш рабочих инстансов ───────────────────────────

function cachePath(): string {
    const dir = Deno.env.get("KING_ORCH_BINS_DIR") || os.tmpdir();
    return path.join(dir, CACHE_FILE);
}

function loadCache(): { version: number; working: WorkingInstance[] } | null {
    try {
        const raw = JSON.parse(fs.readFileSync(cachePath(), 'utf8'));
        if (raw && raw.version === CACHE_VERSION && Array.isArray(raw.working)) return raw;
    } catch { /* нет кеша или битый */ }
    return null;
}

function saveCache(data: unknown) {
    try {
        fs.mkdirSync(path.dirname(cachePath()), { recursive: true });
        fs.writeFileSync(cachePath(), JSON.stringify(data), 'utf8');
    } catch (e) { searxLog('не удалось сохранить кеш', (e as Error).message); }
}

// ─────────────────────────── Discovery (searx.space + фолбэки) ───────────────────────────

// Скоринг инстанса по данным searx.space: аптайм, сеть, версия, живые движки.
function scoreInstance(info: Record<string, unknown>): number {
    let score = 0;
    const up = (info.uptime as Record<string, unknown>) || {};
    const day = up.uptimeDay as number ?? 0;
    const week = up.uptimeWeek as number ?? 0;
    if (day >= 99) score += 3;
    else if (day >= 95) score += 2;
    else if (day >= 90) score += 1;
    else return 0;
    if (week >= 95) score += 1;

    const net = (info.network as Record<string, unknown>) || {};
    if (net.error) return 0;
    if (net.ipv6) score += 1;
    if (net.dnssec) score += 1;

    const m = String(info.version || '').match(/^(\d+)\.(\d+)\.(\d+)/);
    if (m) {
        const major = parseInt(m[1], 10);
        if (major >= 2024) score += 1;
        if (major >= 2025) score += 1;
    }

    const eng = (info.engines as Record<string, unknown>) || {};
    let alive = 0;
    for (const want of ['google', 'google web', 'bing', 'brave', 'duckduckgo', 'duckduckgo web']) {
        const e = eng[want] as Record<string, unknown> | undefined;
        if (e && ((e.error_rate as number) ?? -1) <= 25) alive++;
    }
    score += Math.min(alive, 4);
    return score;
}

// Источник №1: searx.space (статистика + скоринг живых движков).
async function discoverFromSearxSpace(): Promise<string[]> {
    const res = await request(SEARX_SPACE_URL, { timeoutMs: DISCOVERY_TIMEOUT_MS, maxBytes: DISCOVERY_MAX_BYTES });
    if (res.status !== 200) throw new Error(`searx.space HTTP ${res.status}`);
    let json: { instances?: Record<string, Record<string, unknown>> };
    try { json = JSON.parse(res.text); } catch { throw new Error('searx.space: невалидный JSON'); }
    const entries: { url: string; score: number }[] = [];
    const seen = new Set<string>();
    for (const [rawUrl, info] of Object.entries(json.instances || {})) {
        const url = normalizeHttpUrl(rawUrl, '');
        if (!url || seen.has(url)) continue;
        seen.add(url);
        const score = scoreInstance(info);
        if (score >= MIN_POOL_SCORE) entries.push({ url: url.replace(/\/+$/, ''), score });
    }
    entries.sort((a, b) => b.score - a.score);
    return entries.slice(0, MAX_CANDIDATES).map((e) => e.url);
}

// Источник №2: официальный курируемый список searxng/searx-instances (строки `https://domain: {}`
// и `https://domain:` с onion-зеркалами в additional_urls).
function parseInstancesYaml(text: string): string[] {
    const urls: string[] = [];
    for (const line of text.split('\n')) {
        const m = line.match(/^\s*https:\/\/([^\s:{}]+(?::\d+)?)\s*:\s*(?:\{\})?\s*$/);
        if (m) urls.push('https://' + m[1].replace(/\/+$/, ''));
    }
    return urls;
}

async function discoverFromInstancesYml(): Promise<string[]> {
    const res = await request(INSTANCES_YML_URL, { timeoutMs: DISCOVERY_TIMEOUT_MS, maxBytes: DISCOVERY_MAX_BYTES });
    if (res.status !== 200) throw new Error(`instances.yml HTTP ${res.status}`);
    const urls = parseInstancesYaml(res.text);
    if (!urls.length) throw new Error('instances.yml: пусто');
    return urls;
}

// Источник №3: NoPlagiarism/instances-list (JSON, домены без схемы).
async function discoverFromNoPlagiarism(): Promise<string[]> {
    const res = await request(NOPLAGIARISM_JSON_URL, { timeoutMs: DISCOVERY_TIMEOUT_MS, maxBytes: DISCOVERY_MAX_BYTES });
    if (res.status !== 200) throw new Error(`NoPlagiarism HTTP ${res.status}`);
    let json: { instances?: unknown[] };
    try { json = JSON.parse(res.text); } catch { throw new Error('NoPlagiarism: невалидный JSON'); }
    const urls = (json.instances || [])
        .map((d) => normalizeHttpUrl('https://' + String(d).trim(), ''))
        .filter((u): u is string => Boolean(u))
        .map((u) => u.replace(/\/+$/, ''));
    if (!urls.length) throw new Error('NoPlagiarism: пусто');
    return urls;
}

// Каскад discovery: searx.space (со скорингом) → instances.yml → NoPlagiarism → seed-лист.
// Первый успешный источник становится основным; остальные обогащают пул уникальными URL.
async function discoverCandidates(): Promise<string[]> {
    const sources = [
        { name: 'searx.space', fn: discoverFromSearxSpace },
        { name: 'instances.yml', fn: discoverFromInstancesYml },
        { name: 'NoPlagiarism', fn: discoverFromNoPlagiarism },
    ];
    const seen = new Set<string>();
    const all: string[] = [];
    let primary: string | null = null;
    for (const src of sources) {
        let urls: string[];
        try {
            urls = await src.fn();
        } catch (e) {
            searxLog(`discovery: ${src.name} недоступен`, (e as Error).message);
            continue;
        }
        if (!primary) {
            primary = src.name;
            searxLog(`discovery: основной источник — ${primary}`);
        }
        const added = urls.filter((u) => !seen.has(u));
        for (const u of added) seen.add(u);
        all.push(...added);
        searxLog(`discovery: ${src.name} → +${added.length} (всего ${all.length})`);
        if (primary !== src.name && all.length >= MAX_POOL_CANDIDATES) break;
    }
    const seedAdded = FALLBACK_SEED.filter((u) => !seen.has(u));
    for (const u of seedAdded) seen.add(u);
    all.push(...seedAdded);
    searxLog(`discovery: seed-лист → +${seedAdded.length} (всего ${all.length})`);
    if (!all.length) throw new Error('discovery: ни один источник не вернул инстансы');
    return all.slice(0, MAX_POOL_CANDIDATES);
}

// ─────────────────────────── Состояние сессии (кулдауны, рабочее) ───────────────────────────

interface WorkingInstance { url: string; mode: 'json' | 'html'; lastOkTs: number; }

const cooldowns = new Map<string, number>(); // url → timestamp до которого не трогаем
let working: WorkingInstance[] = [];          // [{url, mode: 'json'|'html', lastOkTs}]
let candidates: string[] = [];                // свежие кандидаты (для lazy-пробы)
let lastDiscoveryTs = 0;                      // когда последний раз обновляли кандидатов

// «Разморозка пула»: пул не должен жить вечно на старом кеше. Даже при наличии
// рабочих инстансов кандидаты обновляются не реже CANDIDATES_TTL_MS — чтобы
// цепочка могла находить новые живые инстансы и не зависеть от 2-3 старых.
// Первый discovery в сессии — короткий (не блокирует поиск надолго);
// последующие (старше CANDIDATES_TTL_MS) — в фоне, без ожидания поиском.
function refreshCandidatesBackground(): void {
    if (Date.now() - lastDiscoveryTs <= CANDIDATES_TTL_MS) return;
    lastDiscoveryTs = Date.now();
    discoverCandidates()
        .then((urls) => {
            candidates = urls;
            searxLog(`кандидаты обновлены (фон): ${urls.length}`);
        })
        .catch((e) => searxLog('фоновый discovery не удался', (e as Error).message));
}

function isInCooldown(url: string): boolean {
    const until = cooldowns.get(url);
    return !!until && Date.now() < until;
}

function markFailed(url: string, hard: boolean) {
    cooldowns.set(url, Date.now() + (hard ? HARD_COOLDOWN_MS : COOLDOWN_MS));
}

function markWorking(url: string, mode: 'json' | 'html') {
    const w = working.find((x) => x.url === url);
    if (w) {
        w.mode = mode; // json приоритетнее html
        w.lastOkTs = Date.now();
    } else {
        working.push({ url, mode, lastOkTs: Date.now() });
    }
}

function saveWorkingCache() {
    saveCache({ version: CACHE_VERSION, fetchedAt: Date.now(), working });
}

interface Attempt { url: string; mode: 'json' | 'html' | null; }

// Сборка пула для поиска: рабочие HTML (JSON у большинства инстансов выключен
// и чаще капчит → html-first), затем свежие кандидаты, в конце — JSON-перки.
function buildAttemptList(): Attempt[] {
    const attempts: Attempt[] = [];
    const seen = new Set<string>();
    const fresh = (x: WorkingInstance) => Date.now() - x.lastOkTs < CACHE_TTL_MS;
    for (const w of working.filter((x) => fresh(x) && x.mode === 'html')) {
        if (!seen.has(w.url)) { seen.add(w.url); attempts.push({ url: w.url, mode: 'html' }); }
    }
    for (const url of candidates) {
        if (seen.has(url)) continue;
        seen.add(url);
        attempts.push({ url, mode: null }); // неизвестный инстанс — пробуем HTML
    }
    for (const w of working.filter((x) => fresh(x) && x.mode === 'json').slice(0, MAX_JSON_ATTEMPTS)) {
        if (!seen.has(w.url)) { seen.add(w.url); attempts.push({ url: w.url, mode: 'json' }); }
    }
    return attempts.filter((a) => !isInCooldown(a.url));
}

// Загрузить кеш рабочих инстансов; кандидатов — с фоновым обновлением.
async function ensurePool() {
    const cached = loadCache();
    if (cached) {
        working = cached.working.filter((w) => Date.now() - w.lastOkTs < CACHE_TTL_MS);
        if (working.length > 0) searxLog(`рабочие инстансы из кеша: ${working.length}`);
    }
    if (candidates.length === 0) {
        try {
            candidates = await discoverCandidates();
            lastDiscoveryTs = Date.now();
            searxLog(`кандидатов для lazy-пробы: ${candidates.length}`);
        } catch (e) {
            searxLog('discovery не удался', (e as Error).message);
        }
    } else {
        refreshCandidatesBackground();
    }
    if (working.length === 0 && candidates.length === 0) {
        if (cached && cached.working.length > 0) working = cached.working;
        else throw new Error('Не удалось получить список инстансов SearXNG');
    }
}

// ─────────────────────────── Поиск по одному инстансу ───────────────────────────

interface SearchOpts { limit: number; language: string; time_range: string; minScore: number | null; }

function buildQueryUrl(base: string, query: string, opts: SearchOpts, json: boolean): string {
    const params = new URLSearchParams({
        q: query,
        safesearch: '0',
        language: opts.language || 'all',
        categories: 'general',
    });
    if (opts.time_range) params.set('time_range', opts.time_range);
    if ((opts as Record<string, unknown>).pageno) params.set('pageno', String((opts as Record<string, unknown>).pageno));
    if (json) params.set('format', 'json');
    return `${base}/search?${params.toString()}`;
}

interface SearchResult {
    title: string;
    url: string;
    snippet: string;
    source: string;
    engines: string;
    score: number | null;
    published: string;
}

function normalizeJsonResult(r: Record<string, unknown>): SearchResult | null {
    const url = normalizeHttpUrl(String(r.url || ''), '');
    const title = stripTags(String(r.title || ''));
    if (!url || !title) return null;
    let source = '';
    try { source = new URL(url).hostname; } catch { /* ignore */ }
    return {
        title: title.slice(0, 200),
        url,
        snippet: stripTags(String(r.content || '')).slice(0, 400),
        source,
        engines: Array.isArray(r.engines) ? r.engines.join(', ') : String(r.engine || ''),
        score: typeof r.score === 'number' ? r.score : null,
        published: String(r.publishedDate || ''),
    };
}

// ─────────────────────────── Классификация причин отказа ───────────────────────────

// Точная причина по статусу/заголовкам/телу ответа: rate-limit, Cloudflare, WAF,
// гео-блок, капча, требование searxng_token, либо просто бот-гейт (endpoint=index).
function classifyHttpFailure(status: number, headers: Record<string, string>, body: string): { type: string; msg: string } {
    const h = headers || {};
    const server = String(h.server || '').toLowerCase();
    const cf = /cloudflare/i.test(server) || /cf-mitigated|cf-chl|challenge-platform|__cf_chl_opt/i.test(body);
    const waf = /sucuri|incapsula|imperva|akamai/i.test(server) || /sucuri|incapsula/i.test(body);
    const retryAfter = h['retry-after'];
    const rateHeaders = /(^|[\s-])(x-)?ratelimit(-|_)|^ratelimit-|x-ratelimit/i.test(Object.keys(h).join(' '));
    if (status === 429) return { type: 'rate-limit', msg: retryAfter ? `429 rate limit (повторить через ~${retryAfter} сек)` : '429 rate limit' };
    if (cf) return { type: 'cloudflare', msg: status === 200 ? 'Cloudflare challenge' : 'Cloudflare блок' };
    if (waf) return { type: 'waf', msg: status === 200 ? 'WAF-страница' : 'WAF-блок' };
    if (status === 451) return { type: 'geo', msg: '451 (гео-блок)' };
    if (status === 403) return { type: 'http-403', msg: '403 запрещено' };
    if (status === 401) return { type: 'http-403', msg: '401 не авторизован' };
    if (status !== 200) return { type: 'http', msg: `HTTP ${status}` };
    if (cf) return { type: 'cloudflare', msg: 'Cloudflare challenge' };
    if (rateHeaders) return { type: 'rate-limit', msg: 'rate-limit (по заголовкам ответа)' };
    if (/captcha|challenge|аномал|unusual traffic|verify you are human|attention required/i.test(body)) {
        return { type: 'captcha', msg: 'капча/челлендж' };
    }
    if (/searxng_token|name="token"/i.test(body)) return { type: 'token', msg: 'требует searxng_token (CSRF-флоу)' };
    return { type: 'index', msg: 'страница-форма (endpoint=index)' };
}

// Тип причины для сводки в итоговом логе (матчится по уже сформированному сообщению).
function classifyMsg(msg: string): string {
    if (/rate limit|ratelimit/i.test(msg)) return 'rate-limit';
    if (/Cloudflare/i.test(msg)) return 'cloudflare';
    if (/WAF/i.test(msg)) return 'waf';
    if (/капча|challenge|unusual|аномал|verify/i.test(msg)) return 'captcha';
    if (/searxng_token|CSRF/i.test(msg)) return 'token';
    if (/451|гео-блок/i.test(msg)) return 'geo';
    if (/403|401|запрещено|авторизован/i.test(msg)) return 'http-403';
    if (/HTTP \d+/i.test(msg)) return 'http';
    if (/endpoint=index|страница-форму/i.test(msg)) return 'index';
    if (/JSON API|не-JSON/i.test(msg)) return 'json-off';
    if (/дедлайн|Таймаут|DNS|TLS|соединение|отклонено|недоступна|разорвано|Сеть/i.test(msg)) return 'network';
    return 'other';
}

function reasonSummary(counts: Record<string, number>): string {
    const parts = Object.entries(counts).map(([k, v]) => `${k}: ${v}`);
    return parts.length ? `, причины: ${parts.join(', ')}` : '';
}

interface SearchPayload {
    results: SearchResult[];
    answers: string[];
    infoboxes: unknown[];
    corrections: string[];
}

async function searchJson(base: string, query: string, opts: SearchOpts): Promise<SearchPayload> {
    const res = await request(buildQueryUrl(base, query, opts, true), {
        timeoutMs: SEARCH_TIMEOUT_MS,
        headers: Object.assign({ 'Accept': 'application/json' }, BROWSER_HEADERS),
    });
    if (res.status !== 200) {
        const f = classifyHttpFailure(res.status, res.headers, res.text);
        throw new Error(f.msg);
    }
    let j: { results?: unknown[]; answers?: unknown[]; infoboxes?: unknown[]; corrections?: unknown[] };
    try { j = JSON.parse(res.text); } catch { throw new Error('не-JSON (JSON API выключен)'); }
    if (!Array.isArray(j.results)) throw new Error('нет results в ответе');
    return {
        results: j.results.map((r) => normalizeJsonResult(r as Record<string, unknown>)).filter((r): r is SearchResult => Boolean(r)),
        answers: Array.isArray(j.answers) ? j.answers.map((a) => stripTags(String(a))).filter(Boolean) : [],
        infoboxes: Array.isArray(j.infoboxes) ? j.infoboxes : [],
        corrections: Array.isArray(j.corrections) ? j.corrections.map((c) => String(c)) : [],
    };
}

// Парсер HTML-результатов нового тема SearXNG:
// <article class="result ..."><a class="url_header">…</a><h3><a href="URL">title</a></h3><p class="content">…</p><div class="engines"><span>bing</span></div></article>
function parseHtmlResults(html: string, base: string): SearchResult[] {
    const out: SearchResult[] = [];
    const re = /<article\b[^>]*class="[^"]*result[^"]*"[^>]*>[\s\S]*?<\/article>/gi;
    let m: RegExpExecArray | null;
    while ((m = re.exec(html)) !== null) {
        const block = m[0];
        const a = block.match(/<h3[^>]*>\s*<a[^>]+href="([^"]+)"[^>]*>([\s\S]*?)<\/a>\s*<\/h3>/i);
        if (!a) continue;
        const url = normalizeHttpUrl(a[1], `${base}/`);
        const title = stripTags(a[2]);
        if (!url || !title) continue;
        let desc = block.match(/<p[^>]*class="[^"]*content[^"]*"[^>]*>([\s\S]*?)<\/p>/i);
        if (!desc) desc = block.match(/<p[^>]*>([\s\S]*?)<\/p>/i);
        const eng = (block.match(/<div class="engines">[\s\S]*?<span[^>]*>([\s\S]*?)<\/span>/i) || [])[1] || '';
        let source = '';
        try { source = new URL(url).hostname; } catch { /* ignore */ }
        out.push({
            title: title.slice(0, 200),
            url,
            snippet: desc ? stripTags(desc[1]).slice(0, 400) : '',
            source,
            engines: stripTags(eng) || 'html',
            score: null,
            published: '',
        });
    }
    return out;
}

async function searchHtml(base: string, query: string, opts: SearchOpts): Promise<SearchPayload> {
    const res = await request(buildQueryUrl(base, query, opts, false), {
        timeoutMs: SEARCH_TIMEOUT_MS,
        headers: BROWSER_HEADERS,
    });
    if (res.status !== 200) {
        const f = classifyHttpFailure(res.status, res.headers, res.text);
        throw new Error(f.msg);
    }
    const endpoint = (res.text.match(/name="endpoint" content="([^"]+)"/) || [])[1];
    if (endpoint !== 'results') {
        const f = classifyHttpFailure(200, res.headers, res.text);
        throw new Error(f.msg);
    }
    return {
        results: parseHtmlResults(res.text, base),
        answers: [], infoboxes: [], corrections: [],
    };
}

// ─────────────────────────── Failover по пулу ───────────────────────────

// Убрать tracking-мусор из URL, чтобы одинаковые страницы с разными параметрами
// не дублировались в выдаче.
function dedupeKey(url: string): string {
    try {
        const u = new URL(url);
        u.hash = '';
        for (const k of [...u.searchParams.keys()]) {
            if (/^(utm_|fbclid|gclid|yclid|from|ref|referrer|source|mc_|spm)/i.test(k)) u.searchParams.delete(k);
        }
        return u.toString().replace(/\/+$/, '');
    } catch { return url; }
}

function dedupe(results: SearchResult[]): SearchResult[] {
    const seen = new Set<string>();
    const out: SearchResult[] = [];
    for (const r of results) {
        if (!r.url || seen.has(dedupeKey(r.url))) continue;
        seen.add(dedupeKey(r.url));
        out.push(r);
    }
    return out;
}

// Отсев мусорных результатов: редирект-обёртки (redirect?url=..., /goto, /out.php),
// пустые/невалидные URL.
function isJunkResult(r: SearchResult): boolean {
    if (!r.url) return true;
    if (/\/(redirect|goto|out|away|click|url)\b[\/?]|url=.*%3A%2F%2F/i.test(r.url)) return true;
    return false;
}

function normalizeResults(results: SearchResult[]): SearchResult[] {
    return dedupe(results.filter((r) => !isJunkResult(r)));
}

function applyMinScore(results: SearchResult[], minScore: number | null | undefined): SearchResult[] {
    if (!minScore) return results;
    return results.filter((r) => r.score === null || r.score >= minScore);
}

interface RunResult {
    all: SearchResult[];
    failures: string[];
    used: string[];
    meta: { answers: string[]; infoboxes: unknown[]; corrections: string[] };
}

async function runChain(attempts: Attempt[], query: string, opts: SearchOpts): Promise<RunResult> {
    const start = Date.now();
    const all: SearchResult[] = [];
    const failures: string[] = [];
    const used: string[] = [];
    const reasonCounts: Record<string, number> = {};
    const meta = { answers: [] as string[], infoboxes: [] as unknown[], corrections: [] as string[] };
    let tried = 0;
    let freshProbed = 0;

    for (const att of attempts) {
        if (tried >= MAX_ATTEMPTS || all.length >= opts.limit) break;
        tried++;
        const t0 = Date.now();
        const isFresh = att.mode === null;
        try {
            const r = await withDeadline(
                att.mode === 'json' ? searchJson(att.url, query, opts) : searchHtml(att.url, query, opts),
                ENGINE_DEADLINE_MS, att.url
            );
            markWorking(att.url, att.mode === 'json' ? 'json' : 'html');
            if (isFresh) freshProbed++;
            if (r.results.length > 0) {
                recordEngineCall(att.url, true, Date.now() - t0);
                used.push(att.url);
                all.push(...r.results);
                meta.answers.push(...r.answers);
                meta.infoboxes.push(...r.infoboxes);
                meta.corrections.push(...r.corrections);
                searxLog(`OK ${att.url} (${r.results.length} рез. за ${((Date.now() - t0) / 1000).toFixed(1)}с, ${att.mode === 'json' ? 'JSON' : 'HTML'})`);
            } else {
                searxLog(`пусто ${att.url} (${att.mode === 'json' ? 'JSON' : 'HTML'})`);
            }
        } catch (e) {
            const msg = (e as Error).message || String(e);
            const hard = /капча|challenge|403|451|unusual|аномал|WAF|Cloudflare|searxng_token/i.test(msg);
            recordEngineCall(att.url, false, Date.now() - t0, msg);
            markFailed(att.url, hard);
            const sec = ((Date.now() - t0) / 1000).toFixed(1);
            const type = classifyMsg(msg);
            reasonCounts[type] = (reasonCounts[type] || 0) + 1;
            searxLog(`ОШИБКА ${att.url} за ${sec}с — ${msg}`);
            failures.push(`${att.url}: ${msg}`);
        }
        if (all.length < opts.limit) await sleep(STEP_PAUSE_MS);
    }

    searxLog(`итог: ${all.length} рез. за ${((Date.now() - start) / 1000).toFixed(1)}с из ${tried} попыток (${used.length} ок, ${failures.length} ошибок${freshProbed ? `, новых рабочих: ${freshProbed}` : ''})${reasonSummary(reasonCounts)}`);
    if (used.length > 0) saveWorkingCache();
    return { all, failures, used, meta };
}

// Fan-out: параллельно опрашиваем до MAX_ATTEMPTS РАЗНЫХ инстансов, результаты сливаем.
// Безопасно: каждый инстанс получает ровно один запрос (rate-limit ломают повторы в один).
async function runFanout(attempts: Attempt[], query: string, opts: SearchOpts): Promise<RunResult> {
    const start = Date.now();
    const pool = attempts.slice(0, MAX_ATTEMPTS);
    if (pool.length === 0) return runChain(attempts, query, opts);
    const meta = { answers: [] as string[], infoboxes: [] as unknown[], corrections: [] as string[] };
    const reasonCounts: Record<string, number> = {};
    const results = await Promise.all(pool.map(async (att) => {
        const t0 = Date.now();
        try {
            const r = await withDeadline(
                att.mode === 'json' ? searchJson(att.url, query, opts) : searchHtml(att.url, query, opts),
                ENGINE_DEADLINE_MS, att.url
            );
            markWorking(att.url, att.mode === 'json' ? 'json' : 'html');
            if (r.results.length > 0) {
                recordEngineCall(att.url, true, Date.now() - t0);
                meta.answers.push(...r.answers);
                meta.infoboxes.push(...r.infoboxes);
                meta.corrections.push(...r.corrections);
                searxLog(`fan-out OK ${att.url} (${r.results.length} рез. за ${((Date.now() - t0) / 1000).toFixed(1)}с)`);
                return { url: att.url, results: r.results, error: null as string | null };
            }
            searxLog(`пусто ${att.url} (${att.mode === 'json' ? 'JSON' : 'HTML'})`);
            return { url: att.url, results: [] as SearchResult[], error: null as string | null };
        } catch (e) {
            const msg = (e as Error).message || String(e);
            const hard = /капча|challenge|403|451|unusual|аномал|WAF|Cloudflare|searxng_token/i.test(msg);
            recordEngineCall(att.url, false, Date.now() - t0, msg);
            markFailed(att.url, hard);
            const type = classifyMsg(msg);
            reasonCounts[type] = (reasonCounts[type] || 0) + 1;
            searxLog(`fan-out ОШИБКА ${att.url} — ${msg}`);
            return { url: att.url, results: [] as SearchResult[], error: msg };
        }
    }));
    const all = results.flatMap((r) => r.results);
    const failures = results.filter((r) => r.error).map((r) => `${r.url}: ${r.error}`);
    const used = results.filter((r) => r.results.length > 0).map((r) => r.url);
    searxLog(`итог (fan-out): ${all.length} рез. за ${((Date.now() - start) / 1000).toFixed(1)}с (${used.length} ок, ${failures.length} ошибок)${reasonSummary(reasonCounts)}`);
    if (used.length > 0) saveWorkingCache();
    return { all, failures, used, meta };
}

// Self-healing: если все инстансы недоступны — обновить кандидатов и повторить.
async function searchWithRecovery(query: string, opts: SearchOpts, mode: string): Promise<RunResult> {
    const attempt = () => (mode === 'fanout' ? runFanout(buildAttemptList(), query, opts) : runChain(buildAttemptList(), query, opts));
    await ensurePool();
    let res = await attempt();
    if (res.all.length === 0) {
        searxLog('все инстансы недоступны — обновляю кандидатов и повторяю');
        try {
            candidates = await discoverCandidates();
            searxLog(`свежих кандидатов: ${candidates.length}`);
        } catch (e) {
            searxLog('повторный discovery не удался', (e as Error).message);
        }
        res = await attempt();
    }
    return res;
}

// ─────────────────────────── Форматирование ───────────────────────────

function formatInfobox(infobox: Record<string, unknown>): string {
    const lines: string[] = [];
    const title = stripTags(String(infobox.infobox || infobox.title || ''));
    if (title) lines.push(`📌 ${title}`);
    if (infobox.content) lines.push(stripTags(String(infobox.content)));
    if (Array.isArray(infobox.urls) && infobox.urls.length > 0) {
        const urls = infobox.urls.slice(0, 3)
            .map((u) => `- ${stripTags(String((u as Record<string, string>).title || (u as Record<string, string>).url || ''))}: ${normalizeHttpUrl(String((u as Record<string, string>).url), '')}`)
            .filter((l) => l.includes('http'));
        if (urls.length > 0) lines.push(urls.join('\n'));
    }
    return lines.join('\n').slice(0, 600);
}

function formatResults(res: RunResult & { minScore?: number | null }, limit: number): string {
    const lines: string[] = [];
    if (res.meta.answers.length > 0) {
        lines.push('Прямые ответы SearXNG:\n' + res.meta.answers.map((a) => `- ${a}`).join('\n'));
    }
    if (res.meta.infoboxes.length > 0) {
        const box = formatInfobox(res.meta.infoboxes[0] as Record<string, unknown>);
        if (box) lines.push(box);
    }
    if (res.meta.corrections.length > 0) {
        lines.push(`Уточнение запроса: ${res.meta.corrections[0]}`);
    }
    const deduped = applyMinScore(normalizeResults(res.all), res.minScore).slice(0, limit);
    for (let i = 0; i < deduped.length; i++) {
        const r = deduped[i];
        const extra = [r.source ? ` (${r.source})` : '', r.engines && r.engines !== 'html' ? ` [${r.engines}]` : ''].join('');
        lines.push(`[${i + 1}] ${r.title}${extra}\nСсылка: ${r.url}\nОписание: ${r.snippet || '—'}`);
    }
    if (res.used.length > 0) {
        lines.push(`\nИнстансы: ${res.used.join(', ')}`);
    }
    if (res.failures.length > 0) {
        lines.push(`\n⚠️ Инстансы с ошибками: ${res.failures.join('; ')}`);
    }
    return lines.join('\n');
}

// ─────────────────────────── MCP-сервер ───────────────────────────

createMcpServer({
    name: "searxng-search-mcp",
    version: "2.1.0",
    tools: [{
        name: "SearxngSearch",
        description: "Мета-поиск через публичные инстансы SearXNG (без API-ключей): один запрос → Google/Bing/Brave/DDG через инстанс. Рабочие инстансы подбираются автоматически из searx.space и резервных списков (instances.yml, NoPlagiarism, seed), проверяются лениво (попытка поиска = проверка), рабочие кешируются, забанившие (429/капча) уходят в кулдаун. HTML-парсинг + JSON API, когда доступен. Режимы: цепочка (chain, по умолчанию) или параллельный fan-out (parallel: true). Поддерживает language, time_range, min_score, instance.",
        inputSchema: {
            type: "object",
            properties: {
                query: { type: "string", description: "Поисковый запрос" },
                limit: { type: "number", description: "Максимум результатов (по умолчанию 8, максимум 20)" },
                language: { type: "string", description: "Код языка результатов, например 'ru' или 'en' (по умолчанию all — без фильтра)" },
                time_range: { type: "string", description: "Фильтр по времени публикации: day, week, month, year" },
                min_score: { type: "number", description: "Минимальный score релевантности 0.0-1.0 (работает только для JSON-инстансов)" },
                parallel: { type: "boolean", description: "Параллельный опрос нескольких инстансов (fan-out, быстрее, больше результатов) вместо последовательной цепочки" },
                instance: { type: "string", description: "Конкретный инстанс вместо автовыбора, например https://search.mdosch.de" }
            },
            required: ["query"]
        }
    }],
    handlers: {
        SearxngSearch: async (args: Record<string, unknown>) => {
            const query = String(args.query || '').trim();
            if (!query) throw new Error("SearxngSearch: укажи 'query'.");
            const limit = Math.max(3, Math.min(20, parseInt(String(args.limit), 10) || MAX_RESULTS));
            const opts: SearchOpts = {
                limit,
                language: typeof args.language === 'string' && args.language.trim() ? args.language.trim() : '',
                time_range: typeof args.time_range === 'string' ? args.time_range.trim() : '',
                minScore: typeof args.min_score === 'number' ? args.min_score : null,
            };

            const mode = typeof args.instance === 'string' && args.instance.trim()
                ? 'instance'
                : (args.parallel === true ? 'fanout' : 'chain');
            const cacheKeyStr = cacheKey('searxng', [mode, opts.language ? `lang:${opts.language}` : '', opts.time_range ? `tr:${opts.time_range}` : '', opts.minScore ? `ms:${opts.minScore}` : ''].filter(Boolean), false, query);
            const cached = cacheGet(cacheKeyStr);
            if (cached !== null) {
                searxLog(`из кеша (query: "${query.slice(0, 60)}")`);
                return cached;
            }

            let res: RunResult;
            if (mode === 'instance') {
                const base = normalizeHttpUrl(String(args.instance).trim(), '');
                if (!base) throw new Error(`SearxngSearch: невалидный URL инстанса: ${args.instance}`);
                searxLog('используем указанный инстанс', base);
                const attempts: Attempt[] = [{ url: base.replace(/\/+$/, ''), mode: 'html' }];
                res = await runChain(attempts, query, opts);
            } else {
                res = await searchWithRecovery(query, opts, mode);
            }

            if (res.all.length === 0) {
                const reasons = res.failures.length > 0 ? res.failures.join('; ') : 'все инстансы вернули пусто';
                throw new Error(`Поиск не дал результатов. Причины: ${reasons}`);
            }
            res.minScore = opts.minScore;
            const out = formatResults(res, limit);
            cachePut(cacheKeyStr, out);
            return out;
        }
    }
});
