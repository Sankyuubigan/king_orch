// Общий HTTP-слой для MCP-серверов (zero-dependency: только node:https/http/zlib).
// Возможности: gzip/deflate/brotli, merge Set-Cookie между редиректами, лимит байт,
// колбэк-проверка каждого хопа (SSRF), таймауты. Плюс лёгкие утилиты парсинга HTML.
const https = require('https');
const http = require('http');
const zlib = require('zlib');

const DEFAULT_UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36';
const DEFAULT_TIMEOUT_MS = 20000;
const DEFAULT_MAX_BYTES = 5 * 1024 * 1024;

function decodeBody(buf, headers) {
    const enc = String(headers['content-encoding'] || '').toLowerCase().trim();
    try {
        if (enc === 'gzip') return zlib.gunzipSync(buf);
        if (enc === 'deflate') return zlib.inflateSync(buf);
        if (enc === 'br') {
            try { return zlib.brotliDecompressSync(buf); } catch { return buf; }
        }
    } catch { return buf; }
    return buf;
}

/**
 * HTTP-запрос с ручными редиректами (каждый хоп проходит через opts.checkUrl).
 * @param {string} url
 * @param {object} opts { method, headers, body, timeoutMs, maxBytes, maxRedirects, checkUrl }
 * @returns {Promise<{status:number, headers:object, body:Buffer, text:string, url:string}>}
 */
async function request(url, opts = {}) {
    const method = opts.method || 'GET';
    const timeoutMs = opts.timeoutMs || DEFAULT_TIMEOUT_MS;
    const maxBytes = opts.maxBytes || DEFAULT_MAX_BYTES;
    const maxRedirects = opts.maxRedirects === undefined ? 5 : opts.maxRedirects;
    const cookieJar = new Map();

    let current = url;
    for (let hop = 0; ; hop++) {
        if (opts.checkUrl) await opts.checkUrl(current, hop);
        const parsed = new URL(current);
        const isHttps = parsed.protocol === 'https:';
        const lib = isHttps ? https : http;
        const cookieHeader = [...cookieJar.values()].join('; ');

        const result = await new Promise((resolve, reject) => {
            const req = lib.request({
                hostname: parsed.hostname,
                port: parsed.port || undefined,
                path: parsed.pathname + parsed.search,
                method,
                headers: {
                    'User-Agent': opts.userAgent || DEFAULT_UA,
                    'Accept': 'text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8',
                    'Accept-Encoding': 'gzip, deflate, br',
                    'Accept-Language': 'ru,en;q=0.8',
                    ...(cookieHeader ? { 'Cookie': cookieHeader } : {}),
                    ...(opts.headers || {}),
                },
                timeout: timeoutMs,
            }, (res) => {
                const chunks = [];
                let size = 0;
                let tooLarge = false;
                res.on('data', (c) => {
                    size += c.length;
                    if (size > maxBytes) { tooLarge = true; res.destroy(); return; }
                    chunks.push(c);
                });
                res.on('end', () => {
                    if (tooLarge) { reject(new Error(`Ответ больше лимита ${maxBytes} байт`)); return; }
                    resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks) });
                });
            });
            // Жёсткий таймаут: destroy наверняка (срабатывает и при зависшем DNS/connect-фазе).
            const killer = setTimeout(() => req.destroy(new Error('Таймаут запроса')), timeoutMs + 1000);
            req.on('timeout', () => req.destroy(new Error('Таймаут запроса')));
            req.on('close', () => clearTimeout(killer));
            req.on('error', (err) => reject(err));
            if (opts.body !== undefined && opts.body !== null) req.write(opts.body);
            req.end();
        });

        const setCookies = result.headers['set-cookie'];
        if (Array.isArray(setCookies)) {
            for (const c of setCookies) {
                const pair = c.split(';')[0];
                const eq = pair.indexOf('=');
                if (eq > 0) cookieJar.set(pair.slice(0, eq).trim(), pair.slice(eq + 1).trim());
            }
        }

        const location = result.headers.location;
        if (result.status >= 300 && result.status < 400 && location && hop < maxRedirects) {
            current = new URL(location, current).toString();
            continue;
        }
        const decodedBody = decodeBody(result.body, result.headers);
        return {
            status: result.status,
            headers: result.headers,
            body: decodedBody,
            text: decodedBody.toString('utf8'),
            url: current,
        };
    }
}

// ─────────────────────────── Утилиты парсинга HTML ───────────────────────────

function unescapeHtml(str) {
    return String(str)
        .replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>')
        .replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&#39;/g, "'")
        .replace(/&nbsp;/g, ' ').replace(/&hellip;/g, '…');
}

function stripTags(s) {
    return unescapeHtml(String(s || '').replace(/<[^>]+>/g, ' ')).replace(/\s+/g, ' ').trim();
}

function normalizeText(text) {
    return String(text)
        .replace(/\r\n/g, '\n')
        .replace(/\u00a0/g, ' ')
        .replace(/[ \t]+\n/g, '\n')
        .replace(/\n{3,}/g, '\n\n')
        .split('\n').map(l => l.trim()).filter(l => l.length > 0).join('\n')
        .trim();
}

// Найти сбалансированный блок от открывающего тега на позиции startIdx.
// Возвращает { start, end } — диапазон, включающий весь блок (от '<tag' до '</tag>').
function findBalancedBlock(html, tag, startIdx) {
    const openRe = new RegExp(`<${tag}\\b[^>]*>`, 'i');
    const openMatch = html.slice(startIdx).match(openRe);
    if (!openMatch) return null;
    const blockStart = startIdx + openMatch.index;
    const contentStart = blockStart + openMatch[0].length;

    const tokenRe = new RegExp(`<(\/?)\\s*${tag}\\b([^>]*?)(\\/?)>`, 'gi');
    tokenRe.lastIndex = contentStart;
    let depth = 1;
    let m;
    while ((m = tokenRe.exec(html)) !== null) {
        const isClose = m[1] === '/';
        const isSelfClose = m[3] === '/';
        if (!isClose && !isSelfClose) { depth++; continue; }
        if (isClose) {
            depth--;
            if (depth === 0) return { start: blockStart, end: tokenRe.lastIndex };
        }
    }
    return null;
}

function extractTagContent(html, tag) {
    const re = new RegExp(`<${tag}\\b[^>]*>`, 'i');
    const m = html.match(re);
    if (!m) return null;
    const block = findBalancedBlock(html, tag, m.index);
    if (!block) return null;
    const openClose = html.slice(m.index).match(/<\/?[^>]+>/);
    const openTag = openClose ? openClose[0] : '';
    return html.slice(m.index + openTag.length, block.end - `</${tag}>`.length);
}

// Все теги с атрибутом attr="value" (value может быть в class-списке).
function findTagsByAttr(html, attr, value) {
    const re = new RegExp(`<([a-zA-Z][a-zA-Z0-9]*)\\b[^>]*${attr}\\s*=\\s*["']([^"']*)["'][^>]*>`, 'gi');
    const out = [];
    let m;
    while ((m = re.exec(html)) !== null) {
        const list = m[2].split(/\s+/).map(s => s.toLowerCase());
        if (value && !list.includes(value.toLowerCase())) continue;
        out.push({ tag: m[1].toLowerCase(), index: m.index });
    }
    return out;
}

// Разрезать HTML на сбалансированные блоки, у которых в class есть className.
function parseBlocksByClass(html, className) {
    const out = [];
    const re = new RegExp(`<([a-z][a-z0-9]*)\\b[^>]*class="[^"]*\\b${className}\\b[^"]*"[^>]*>`, 'gi');
    let m;
    while ((m = re.exec(html)) !== null) {
        const block = findBalancedBlock(html, m[1], m.index);
        if (!block) continue;
        out.push(html.slice(m.index, block.end));
        re.lastIndex = block.end;
    }
    return out;
}

// Первый URL из блока (http/https).
function firstHref(block) {
    const m = block.match(/href="(https?:\/\/[^"]+)"/i);
    return m ? m[1] : '';
}

// Нормализация http(s) URL, отсев мусорных схем.
function normalizeHttpUrl(raw, base) {
    if (!raw) return '';
    if (raw.startsWith('//')) raw = 'https:' + raw;
    try {
        const u = new URL(raw, base || undefined);
        if (u.protocol !== 'http:' && u.protocol !== 'https:') return '';
        u.hash = '';
        return u.toString();
    } catch { return ''; }
}

module.exports = {
    request,
    unescapeHtml,
    stripTags,
    normalizeText,
    findBalancedBlock,
    extractTagContent,
    findTagsByAttr,
    parseBlocksByClass,
    firstHref,
    normalizeHttpUrl,
};
