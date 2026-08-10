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
const fs = require('fs');
const path = require('path');
const os = require('os');
const { createMcpServer } = require('./mcp_base.cjs');
const {
    request, stripTags, normalizeHttpUrl,
} = require('./web_http.cjs');

const SEARX_SPACE_URL = 'https://searx.space/data/instances.json';
// Резервные источники списка инстансов (на случай недоступности searx.space):
const INSTANCES_YML_URL = 'https://raw.githubusercontent.com/searxng/searx-instances/master/searxinstances/instances.yml';
const NOPLAGIARISM_JSON_URL = 'https://raw.githubusercontent.com/NoPlagiarism/instances-list/master/instances/search/searx/all.json';
const DISCOVERY_TIMEOUT_MS = 20000;
const DISCOVERY_MAX_BYTES = 8 * 1024 * 1024;
const CACHE_TTL_MS = 24 * 60 * 60 * 1000;      // возраст кеша рабочих инстансов
const CACHE_FILE = 'searxng_cache.json';
const CACHE_VERSION = 2;
const SEARCH_TIMEOUT_MS = 15000;
const ENGINE_DEADLINE_MS = 20000;
const MAX_CANDIDATES = 30;                      // лучших из searx.space (многие бот-защищены)
const MAX_POOL_CANDIDATES = 100;                // итоговый потолок пула (с фолбэк-добавками; ленивая проба, лимит 7 попыток/поиск)
// Последний резерв discovery: проверенные рабочие + уникальные из сторонних списков.
const FALLBACK_SEED = [
    'https://search.mdosch.de',
    'https://searx.tiekoetter.com',
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
const BROWSER_HEADERS = {
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

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function withDeadline(promise, ms, name) {
    let timer;
    const timeout = new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error('инстанс завис (дедлайн)')), ms);
    });
    return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function searxLog(status, extra) {
    console.error(`[SEARX] ${status}${extra ? ' — ' + extra : ''}`);
}

// ─────────────────────────── Кеш рабочих инстансов ───────────────────────────

function cachePath() {
    const dir = process.env.KING_ORCH_BINS_DIR || os.tmpdir();
    return path.join(dir, CACHE_FILE);
}

function loadCache() {
    try {
        const raw = JSON.parse(fs.readFileSync(cachePath(), 'utf8'));
        if (raw && raw.version === CACHE_VERSION && Array.isArray(raw.working)) return raw;
    } catch { /* нет кеша или битый */ }
    return null;
}

function saveCache(data) {
    try {
        fs.mkdirSync(path.dirname(cachePath()), { recursive: true });
        fs.writeFileSync(cachePath(), JSON.stringify(data), 'utf8');
    } catch (e) { searxLog('не удалось сохранить кеш', e.message); }
}

// ─────────────────────────── Discovery (searx.space + фолбэки) ───────────────────────────

// Скоринг инстанса по данным searx.space: аптайм, сеть, версия, живые движки.
function scoreInstance(info) {
    let score = 0;
    const up = info.uptime || {};
    const day = up.uptimeDay ?? 0;
    const week = up.uptimeWeek ?? 0;
    if (day >= 99) score += 3;
    else if (day >= 95) score += 2;
    else if (day >= 90) score += 1;
    else return 0;
    if (week >= 95) score += 1;

    const net = info.network || {};
    if (net.error) return 0;
    if (net.ipv6) score += 1;
    if (net.dnssec) score += 1;

    const m = String(info.version || '').match(/^(\d+)\.(\d+)\.(\d+)/);
    if (m) {
        const major = parseInt(m[1], 10);
        if (major >= 2024) score += 1;
        if (major >= 2025) score += 1;
    }

    const eng = info.engines || {};
    let alive = 0;
    for (const want of ['google', 'google web', 'bing', 'brave', 'duckduckgo', 'duckduckgo web']) {
        const e = eng[want];
        if (e && (e.error_rate ?? -1) <= 25) alive++;
    }
    score += Math.min(alive, 4);
    return score;
}

// Источник №1: searx.space (статистика + скоринг живых движков).
async function discoverFromSearxSpace() {
    const res = await request(SEARX_SPACE_URL, { timeoutMs: DISCOVERY_TIMEOUT_MS, maxBytes: DISCOVERY_MAX_BYTES });
    if (res.status !== 200) throw new Error(`searx.space HTTP ${res.status}`);
    let json;
    try { json = JSON.parse(res.text); } catch { throw new Error('searx.space: невалидный JSON'); }
    const entries = [];
    const seen = new Set();
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
function parseInstancesYaml(text) {
    const urls = [];
    for (const line of text.split('\n')) {
        const m = line.match(/^\s*https:\/\/([^\s:{}]+(?::\d+)?)\s*:\s*(?:\{\})?\s*$/);
        if (m) urls.push('https://' + m[1].replace(/\/+$/, ''));
    }
    return urls;
}

async function discoverFromInstancesYml() {
    const res = await request(INSTANCES_YML_URL, { timeoutMs: DISCOVERY_TIMEOUT_MS, maxBytes: DISCOVERY_MAX_BYTES });
    if (res.status !== 200) throw new Error(`instances.yml HTTP ${res.status}`);
    const urls = parseInstancesYaml(res.text);
    if (!urls.length) throw new Error('instances.yml: пусто');
    return urls;
}

// Источник №3: NoPlagiarism/instances-list (JSON, домены без схемы).
async function discoverFromNoPlagiarism() {
    const res = await request(NOPLAGIARISM_JSON_URL, { timeoutMs: DISCOVERY_TIMEOUT_MS, maxBytes: DISCOVERY_MAX_BYTES });
    if (res.status !== 200) throw new Error(`NoPlagiarism HTTP ${res.status}`);
    let json;
    try { json = JSON.parse(res.text); } catch { throw new Error('NoPlagiarism: невалидный JSON'); }
    const urls = (json.instances || [])
        .map((d) => normalizeHttpUrl('https://' + String(d).trim(), ''))
        .filter(Boolean)
        .map((u) => u.replace(/\/+$/, ''));
    if (!urls.length) throw new Error('NoPlagiarism: пусто');
    return urls;
}

// Каскад discovery: searx.space (со скорингом) → instances.yml → NoPlagiarism → seed-лист.
// Первый успешный источник становится основным; остальные обогащают пул уникальными URL.
async function discoverCandidates() {
    const sources = [
        { name: 'searx.space', fn: discoverFromSearxSpace },
        { name: 'instances.yml', fn: discoverFromInstancesYml },
        { name: 'NoPlagiarism', fn: discoverFromNoPlagiarism },
    ];
    const seen = new Set();
    const all = [];
    let primary = null;
    for (const src of sources) {
        let urls;
        try {
            urls = await src.fn();
        } catch (e) {
            searxLog(`discovery: ${src.name} недоступен`, e.message);
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

const cooldowns = new Map(); // url → timestamp до которого не трогаем
let working = [];            // [{url, mode: 'json'|'html', lastOkTs}]

function isInCooldown(url) {
    const until = cooldowns.get(url);
    return !!until && Date.now() < until;
}

function markFailed(url, hard) {
    cooldowns.set(url, Date.now() + (hard ? HARD_COOLDOWN_MS : COOLDOWN_MS));
}

function markWorking(url, mode) {
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

// Сборка пула для поиска: известные рабочие (JSON → HTML) + свежие кандидаты.
function buildAttemptList() {
    const attempts = [];
    const seen = new Set();
    const fresh = (x) => Date.now() - x.lastOkTs < CACHE_TTL_MS;
    for (const w of working.filter((x) => fresh(x) && x.mode === 'json').slice(0, MAX_JSON_ATTEMPTS)) {
        if (!seen.has(w.url)) { seen.add(w.url); attempts.push({ url: w.url, mode: 'json' }); }
    }
    for (const w of working.filter((x) => fresh(x) && x.mode === 'html')) {
        if (!seen.has(w.url)) { seen.add(w.url); attempts.push({ url: w.url, mode: 'html' }); }
    }
    for (const url of candidates) {
        if (seen.has(url)) continue;
        seen.add(url);
        attempts.push({ url, mode: null }); // неизвестный инстанс — пробуем HTML
    }
    return attempts.filter((a) => !isInCooldown(a.url));
}

let candidates = []; // свежие кандидаты (для lazy-пробы)

// Загрузить кеш и кандидатов. Требует одного discovery-запроса, если нет кеша.
async function ensurePool() {
    const cached = loadCache();
    if (cached) {
        working = cached.working.filter((w) => Date.now() - w.lastOkTs < CACHE_TTL_MS);
        if (working.length > 0) {
            searxLog(`рабочие инстансы из кеша: ${working.length}`);
            return;
        }
    }
    try {
        candidates = await discoverCandidates();
        searxLog(`кандидатов для lazy-пробы: ${candidates.length}`);
    } catch (e) {
        searxLog('discovery не удался', e.message);
        if (cached && cached.working.length > 0) working = cached.working;
        else throw new Error(`Не удалось получить список инстансов SearXNG: ${e.message}`);
    }
}

// ─────────────────────────── Поиск по одному инстансу ───────────────────────────

function buildQueryUrl(base, query, opts, json) {
    const params = new URLSearchParams({
        q: query,
        safesearch: '0',
        language: opts.language || 'all',
        categories: 'general',
    });
    if (opts.time_range) params.set('time_range', opts.time_range);
    if (opts.pageno) params.set('pageno', String(opts.pageno));
    if (json) params.set('format', 'json');
    return `${base}/search?${params.toString()}`;
}

function normalizeJsonResult(r) {
    const url = normalizeHttpUrl(r.url, '');
    const title = stripTags(r.title || '');
    if (!url || !title) return null;
    let source = '';
    try { source = new URL(url).hostname; } catch { /* ignore */ }
    return {
        title: title.slice(0, 200),
        url,
        snippet: stripTags(r.content || '').slice(0, 400),
        source,
        engines: Array.isArray(r.engines) ? r.engines.join(', ') : (r.engine || ''),
        score: typeof r.score === 'number' ? r.score : null,
        published: r.publishedDate || '',
    };
}

// ─────────────────────────── Классификация причин отказа ───────────────────────────

// Точная причина по статусу/заголовкам/телу ответа: rate-limit, Cloudflare, WAF,
// гео-блок, капча, требование searxng_token, либо просто бот-гейт (endpoint=index).
function classifyHttpFailure(status, headers, body) {
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
function classifyMsg(msg) {
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

function reasonSummary(counts) {
    const parts = Object.entries(counts).map(([k, v]) => `${k}: ${v}`);
    return parts.length ? `, причины: ${parts.join(', ')}` : '';
}

async function searchJson(base, query, opts) {
    const res = await request(buildQueryUrl(base, query, opts, true), {
        timeoutMs: SEARCH_TIMEOUT_MS,
        headers: Object.assign({ 'Accept': 'application/json' }, BROWSER_HEADERS),
    });
    if (res.status !== 200) {
        const f = classifyHttpFailure(res.status, res.headers, res.text);
        throw new Error(f.msg);
    }
    let j;
    try { j = JSON.parse(res.text); } catch { throw new Error('не-JSON (JSON API выключен)'); }
    if (!Array.isArray(j.results)) throw new Error('нет results в ответе');
    return {
        results: j.results.map(normalizeJsonResult).filter(Boolean),
        answers: Array.isArray(j.answers) ? j.answers.map((a) => stripTags(String(a))).filter(Boolean) : [],
        infoboxes: Array.isArray(j.infoboxes) ? j.infoboxes : [],
        corrections: Array.isArray(j.corrections) ? j.corrections : [],
    };
}

// Парсер HTML-результатов нового тема SearXNG:
// <article class="result ..."><a class="url_header">…</a><h3><a href="URL">title</a></h3><p class="content">…</p><div class="engines"><span>bing</span></div></article>
function parseHtmlResults(html, base) {
    const out = [];
    const re = /<article\b[^>]*class="[^"]*result[^"]*"[^>]*>[\s\S]*?<\/article>/gi;
    let m;
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

async function searchHtml(base, query, opts) {
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

function dedupe(results) {
    const seen = new Set();
    return results.filter((r) => {
        if (!r.url || seen.has(r.url)) return false;
        seen.add(r.url);
        return true;
    });
}

function applyMinScore(results, minScore) {
    if (!minScore) return results;
    return results.filter((r) => r.score === null || r.score >= minScore);
}

async function runChain(attempts, query, opts) {
    const start = Date.now();
    const all = [];
    const failures = [];
    const used = [];
    const reasonCounts = {};
    const meta = { answers: [], infoboxes: [], corrections: [] };
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
            const msg = e.message || String(e);
            const hard = /капча|challenge|403|451|unusual|аномал|WAF|Cloudflare|searxng_token/i.test(msg);
            markFailed(att.url, hard);
            const sec = ((Date.now() - t0) / 1000).toFixed(1);
            const type = classifyMsg(msg);
            reasonCounts[type] = (reasonCounts[type] || 0) + 1;
            searxLog(`ОШИБКА ${att.url} за ${sec}с — ${msg}`);
            failures.push(`${att.url}: ${msg}`);
        }
        if (all.length < opts.limit) await sleep(STEP_PAUSE_MS);
    }

    const freshCount = attempts.filter((a) => a.mode === null).length;
    searxLog(`итог: ${all.length} рез. за ${((Date.now() - start) / 1000).toFixed(1)}с из ${tried} попыток (${used.length} ок, ${failures.length} ошибок${freshProbed ? `, новых рабочих: ${freshProbed}` : ''})${reasonSummary(reasonCounts)}`);
    if (used.length > 0) saveWorkingCache();
    return { all, failures, used, meta };
}

// Fan-out: параллельно опрашиваем до MAX_ATTEMPTS РАЗНЫХ инстансов, результаты сливаем.
// Безопасно: каждый инстанс получает ровно один запрос (rate-limit ломают повторы в один).
async function runFanout(attempts, query, opts) {
    const start = Date.now();
    const pool = attempts.slice(0, MAX_ATTEMPTS);
    if (pool.length === 0) return runChain(attempts, query, opts);
    const meta = { answers: [], infoboxes: [], corrections: [] };
    const reasonCounts = {};
    const results = await Promise.all(pool.map(async (att) => {
        const t0 = Date.now();
        try {
            const r = await withDeadline(
                att.mode === 'json' ? searchJson(att.url, query, opts) : searchHtml(att.url, query, opts),
                ENGINE_DEADLINE_MS, att.url
            );
            markWorking(att.url, att.mode === 'json' ? 'json' : 'html');
            if (r.results.length > 0) {
                meta.answers.push(...r.answers);
                meta.infoboxes.push(...r.infoboxes);
                meta.corrections.push(...r.corrections);
                searxLog(`fan-out OK ${att.url} (${r.results.length} рез. за ${((Date.now() - t0) / 1000).toFixed(1)}с)`);
                return { url: att.url, results: r.results, error: null };
            }
            searxLog(`пусто ${att.url} (${att.mode === 'json' ? 'JSON' : 'HTML'})`);
            return { url: att.url, results: [], error: null };
        } catch (e) {
            const msg = e.message || String(e);
            const hard = /капча|challenge|403|451|unusual|аномал|WAF|Cloudflare|searxng_token/i.test(msg);
            markFailed(att.url, hard);
            const type = classifyMsg(msg);
            reasonCounts[type] = (reasonCounts[type] || 0) + 1;
            searxLog(`fan-out ОШИБКА ${att.url} — ${msg}`);
            return { url: att.url, results: [], error: msg };
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
async function searchWithRecovery(query, opts, mode) {
    const attempt = () => (mode === 'fanout' ? runFanout(buildAttemptList(), query, opts) : runChain(buildAttemptList(), query, opts));
    await ensurePool();
    let res = await attempt();
    if (res.all.length === 0) {
        searxLog('все инстансы недоступны — обновляю кандидатов и повторяю');
        try {
            candidates = await discoverCandidates();
            searxLog(`свежих кандидатов: ${candidates.length}`);
        } catch (e) {
            searxLog('повторный discovery не удался', e.message);
        }
        res = await attempt();
    }
    return res;
}

// ─────────────────────────── Форматирование ───────────────────────────

function formatInfobox(infobox) {
    const lines = [];
    const title = stripTags(infobox.infobox || infobox.title || '');
    if (title) lines.push(`📌 ${title}`);
    if (infobox.content) lines.push(stripTags(String(infobox.content)));
    if (Array.isArray(infobox.urls) && infobox.urls.length > 0) {
        const urls = infobox.urls.slice(0, 3)
            .map((u) => `- ${stripTags(u.title || u.url || '')}: ${normalizeHttpUrl(u.url, '')}`)
            .filter((l) => l.includes('http'));
        if (urls.length > 0) lines.push(urls.join('\n'));
    }
    return lines.join('\n').slice(0, 600);
}

function formatResults(res, limit) {
    const lines = [];
    if (res.meta.answers.length > 0) {
        lines.push('Прямые ответы SearXNG:\n' + res.meta.answers.map((a) => `- ${a}`).join('\n'));
    }
    if (res.meta.infoboxes.length > 0) {
        const box = formatInfobox(res.meta.infoboxes[0]);
        if (box) lines.push(box);
    }
    if (res.meta.corrections.length > 0) {
        lines.push(`Уточнение запроса: ${res.meta.corrections[0]}`);
    }
    const deduped = applyMinScore(dedupe(res.all), res.minScore).slice(0, limit);
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
        SearxngSearch: async (args) => {
            const query = (args.query || '').trim();
            if (!query) throw new Error("SearxngSearch: укажи 'query'.");
            const limit = Math.max(3, Math.min(20, parseInt(args.limit, 10) || MAX_RESULTS));
            const opts = {
                limit,
                language: typeof args.language === 'string' && args.language.trim() ? args.language.trim() : '',
                time_range: typeof args.time_range === 'string' ? args.time_range.trim() : '',
                minScore: typeof args.min_score === 'number' ? args.min_score : null,
            };

            let res;
            if (typeof args.instance === 'string' && args.instance.trim()) {
                const base = normalizeHttpUrl(args.instance.trim(), '');
                if (!base) throw new Error(`SearxngSearch: невалидный URL инстанса: ${args.instance}`);
                searxLog('используем указанный инстанс', base);
                const attempts = [{ url: base.replace(/\/+$/, ''), mode: 'html' }];
                res = await runChain(attempts, query, opts);
            } else {
                const mode = args.parallel === true ? 'fanout' : 'chain';
                res = await searchWithRecovery(query, opts, mode);
            }

            if (res.all.length === 0) {
                const reasons = res.failures.length > 0 ? res.failures.join('; ') : 'все инстансы вернули пусто';
                throw new Error(`Поиск не дал результатов. Причины: ${reasons}`);
            }
            res.minScore = opts.minScore;
            return formatResults(res, limit);
        }
    }
});
