// WebFetch (generic + сайт-фетчеры статей + GitHub README).
// SSRF-защита: только публичные http/https, DNS-резолв с блокировкой приватных адресов (по open-websearch/urlSafety).
import net from "node:net";
import dns from "node:dns";
import { createMcpServer } from "./mcp_base.ts";
import {
    request, unescapeHtml, stripTags, normalizeText,
    findBalancedBlock, extractTagContent, findTagsByAttr,
} from "./web_http.ts";

const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36';
const UA_MOBILE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1';
const TIMEOUT_MS = 20000;
const MAX_REDIRECTS = 5;
const MAX_BYTES = 2 * 1024 * 1024; // 2 MB

// ─────────────────────────── SSRF-защита (по open-websearch/urlSafety) ───────────────────────────

function stripIpv6Brackets(host: string): string {
    return host.startsWith('[') && host.endsWith(']') ? host.slice(1, -1) : host;
}

function ipv4InCidr(ip: string, cidr: string, bits: number): boolean {
    const parts = ip.split('.').map(Number);
    if (parts.length !== 4 || parts.some(p => isNaN(p) || p < 0 || p > 255)) return false;
    const int = ((parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3]) >>> 0;
    const mask = bits === 0 ? 0 : (0xFFFFFFFF << (32 - bits)) >>> 0;
    const [c0, c1, c2, c3] = cidr.split('.').map(Number);
    const cint = ((c0 << 24) | (c1 << 16) | (c2 << 8) | c3) >>> 0;
    return (int & mask) === (cint & mask);
}

function isPrivateIpv4(ip: string): boolean {
    if (!net.isIPv4(ip)) return false;
    if (ipv4InCidr(ip, '0.0.0.0', 8)) return true;        // 0.0.0.0/8
    if (ipv4InCidr(ip, '10.0.0.0', 8)) return true;       // 10/8
    if (ipv4InCidr(ip, '100.64.0.0', 10)) return true;    // CGNAT
    if (ipv4InCidr(ip, '127.0.0.0', 8)) return true;      // loopback
    if (ipv4InCidr(ip, '169.254.0.0', 16)) return true;   // link-local
    if (ipv4InCidr(ip, '172.16.0.0', 12)) return true;    // 172.16/12
    if (ipv4InCidr(ip, '192.168.0.0', 16)) return true;   // 192.168/16
    if (ipv4InCidr(ip, '198.18.0.0', 15)) return true;    // benchmark
    if (ipv4InCidr(ip, '224.0.0.0', 4)) return true;      // multicast
    if (ipv4InCidr(ip, '240.0.0.0', 4)) return true;      // reserved
    return false;
}

function isPrivateIpv6(ip: string): boolean {
    if (!net.isIPv6(ip)) return false;
    const lower = ip.toLowerCase().replace(/^::ffff:/, ''); // IPv4-mapped
    if (net.isIPv4(lower)) return isPrivateIpv4(lower);
    if (lower === '::' || lower === '::1') return true;     // unspecified / loopback
    if (lower.startsWith('fc') || lower.startsWith('fd')) return true; // fc00::/7 (ULA)
    if (lower.startsWith('fe8') || lower.startsWith('fe9') || lower.startsWith('fea') || lower.startsWith('feb')) return true; // fe80::/10 (link-local)
    if (lower.startsWith('ff')) return true;                // multicast
    return false;
}

function isPrivateOrLocalHostname(hostname: string): boolean {
    const host = stripIpv6Brackets(hostname.trim().toLowerCase());
    if (!host) return true;
    if (host === 'localhost' || host.endsWith('.localhost')) return true;
    if (net.isIP(host) === 0) return false; // доменное имя — проверим DNS-резолвом
    return isPrivateIpv4(host) || isPrivateIpv6(host);
}

async function assertPublicUrlResolved(rawUrl: string, label: string): Promise<URL> {
    let parsed: URL;
    try { parsed = new URL(rawUrl); }
    catch { throw new Error(`${label}: невалидный URL`); }
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        throw new Error(`${label}: разрешён только http/https, получено '${parsed.protocol}'`);
    }
    if (parsed.username || parsed.password) {
        throw new Error(`${label}: URL с логином/паролем запрещён`);
    }
    const host = stripIpv6Brackets(parsed.hostname);
    if (isPrivateOrLocalHostname(host)) {
        throw new Error(`${label}: приватный/локальный адрес '${parsed.hostname}' запрещён (SSRF)`);
    }
    if (net.isIP(host) !== 0) return parsed;

    let addresses: dns.LookupAddress[];
    try {
        addresses = await dns.promises.lookup(host, { all: true, verbatim: true });
    } catch {
        throw new Error(`${label}: не удалось разрешить DNS для '${host}'`);
    }
    for (const entry of addresses) {
        if (isPrivateIpv4(entry.address) || isPrivateIpv6(entry.address)) {
            throw new Error(`${label}: '${host}' резолвится в приватный/локальный адрес ${entry.address} (SSRF)`);
        }
    }
    return parsed;
}

// ─────────────────────────── HTTP с безопасными редиректами ───────────────────────────

async function fetchUrlSafe(startUrl: string, opts: Record<string, unknown> = {}) {
    return request(startUrl, {
        ...opts,
        timeoutMs: (opts.timeoutMs as number) || TIMEOUT_MS,
        maxBytes: MAX_BYTES,
        maxRedirects: MAX_REDIRECTS,
        checkUrl: (url: string, hop: number) => assertPublicUrlResolved(url, `URL (хоп ${hop})`),
    });
}

// ─────────────────────────── Лёгкий парсер HTML (без jsdom/cheerio) ───────────────────────────

function matchTitle(html: string): string {
    const m = html.match(/<title[^>]*>([\s\S]*?)<\/title>/i);
    return m ? unescapeHtml(m[1].replace(/<[^>]+>/g, ' ')).replace(/\s+/g, ' ').trim() : '';
}

function matchMetaDescription(html: string): string {
    const m = html.match(/<meta[^>]+(?:name|property)=["']description["'][^>]*>/i) ||
        html.match(/<meta[^>]+(?:name|property)=["']og:description["'][^>]*>/i);
    if (!m) return '';
    const c = m[0].match(/content=["']([^"']*)["']/i);
    return c ? unescapeHtml(c[1]).trim() : '';
}

function stripNoiseTags(html: string): string {
    let out = html;
    for (const tag of ['script', 'style', 'noscript', 'template', 'iframe', 'svg', 'canvas', 'nav', 'footer', 'header']) {
        for (let i = 0; i < 5; i++) {
            const re = new RegExp(`<${tag}\\b`, 'i');
            const m = out.match(re);
            if (!m) break;
            const block = findBalancedBlock(out, tag, m.index ?? 0);
            if (!block) break;
            out = out.slice(0, block.start) + out.slice(block.end);
        }
    }
    return out.replace(/<!--[\s\S]*?-->/g, '');
}

function blockToText(block: string): string {
    const hasParagraphs = /<p[\s>]/i.test(block);
    let t = block.replace(/<h[1-6][^>]*>[\s\S]*?<\/h[1-6]>/gi, (m) => '\n' + unescapeHtml(m.replace(/<[^>]+>/g, ' ')) + '\n');
    if (hasParagraphs) {
        // Контент-режим: параграфы/списки/цитаты (навигация в div-ах отсеивается).
        t = t.replace(/<(p|li|blockquote)[^>]*>([\s\S]*?)<\/(p|li|blockquote)>/gi, (m, op, inner) => '\n' + unescapeHtml(inner.replace(/<[^>]+>/g, ' ')) + '\n');
    } else {
        t = t.replace(/<(p|div|li|tr|br|blockquote|pre|ul|ol|table)[^>]*>/gi, '\n');
    }
    t = t.replace(/<[^>]+>/g, ' ');
    return unescapeHtml(t);
}

function extractMainText(html: string): { title: string; text: string; mode: string } {
    const title = matchTitle(html);
    const metaDesc = matchMetaDescription(html);
    const cleaned = stripNoiseTags(html);

    const containers: { sel: string; type: string; attr?: string; value: string }[] = [
        { sel: 'article', type: 'tag', value: 'article' },
        { sel: 'main', type: 'tag', value: 'main' },
        { sel: '[role=main]', type: 'attr', attr: 'role', value: 'main' },
        { sel: '.markdown-body', type: 'class', value: 'markdown-body' },
        { sel: '.article-content', type: 'class', value: 'article-content' },
        { sel: '.post-content', type: 'class', value: 'post-content' },
        { sel: '.entry-content', type: 'class', value: 'entry-content' },
        { sel: '.content', type: 'class', value: 'content' },
    ];

    for (const c of containers) {
        let content: string | null = null;
        if (c.type === 'tag') {
            content = extractTagContent(cleaned, c.value);
        } else if (c.type === 'attr' || c.type === 'class') {
            const found = findTagsByAttr(cleaned, c.attr as string, c.value);
            for (const f of found) {
                const block = findBalancedBlock(cleaned, f.tag, f.index);
                if (!block) continue;
                content = cleaned.slice(block.start, block.end);
                const innerOpen = cleaned.slice(block.start).match(/<[^>]+>/);
                if (innerOpen) content = cleaned.slice(block.start + innerOpen[0].length, block.end - `</${f.tag}>`.length);
                if (content) break;
            }
        }
        if (content) {
            const text = normalizeText(blockToText(content));
            if (text.length >= 120) return { title, text, mode: c.sel };
        }
    }

    const body = extractTagContent(cleaned, 'body');
    const bodyText = normalizeText(blockToText(body || cleaned));
    if (bodyText) return { title, text: bodyText, mode: 'body' };

    return { title, text: normalizeText([title, metaDesc].filter(Boolean).join('\n\n')), mode: 'metadata' };
}

function looksLikeHtml(raw: string): boolean {
    return /<!doctype html|<html[\s>]|<body[\s>]/i.test(raw);
}

// ─────────────────────────── Сайт-фетчеры статей (по open-websearch/targetValidation) ───────────────────────────

function validateCsdnUrl(url: URL): boolean {
    return url.hostname === 'blog.csdn.net' && url.pathname.includes('/article/details/');
}

function validateJuejinUrl(url: URL): boolean {
    return url.hostname === 'juejin.cn' && url.pathname.includes('/post/');
}

function validateLinuxdoUrl(url: URL): boolean {
    return url.hostname === 'linux.do' && /\/t\/(?:[^/]+\/)?\d+$/.test(url.pathname);
}

async function fetchCsdnArticle(urlStr: string): Promise<string> {
    const { status, text, url } = await fetchUrlSafe(urlStr);
    if (status === 521 || status === 403 || status === 503) {
        throw new Error(`CSDN вернул HTTP ${status} (антибот-защита) — попробуй WebFetch на зеркало статьи`);
    }
    if (text && !/<[a-z][^>]*>/i.test(text)) {
        return `Страница ${url} (HTTP ${status}): нет HTML-контента.`;
    }
    const html = text;
    const cleaned = stripNoiseTags(html);
    let content: string | null = null;
    const found = findTagsByAttr(cleaned, 'class', 'content_views');
    for (const f of found) {
        const block = findBalancedBlock(cleaned, f.tag, f.index);
        if (block) {
            const innerOpen = cleaned.slice(block.start).match(/<[^>]+>/);
            content = innerOpen
                ? cleaned.slice(block.start + innerOpen[0].length, block.end - `</${f.tag}>`.length)
                : cleaned.slice(block.start, block.end);
            if (content) break;
        }
    }
    if (!content) throw new Error(`Не найден контент статьи (HTTP ${status})`);
    const title = matchTitle(html);
    return `# ${title}\nИсточник: ${url}\n\n${normalizeText(blockToText(content))}`;
}

async function fetchJuejinArticle(urlStr: string): Promise<string> {
    const { status, text, url } = await fetchUrlSafe(urlStr, { userAgent: UA_MOBILE });
    if (status === 521 || status === 403 || status === 503) {
        throw new Error(`Juejin вернул HTTP ${status} (антибот-защита)`);
    }
    if (text && !/<[a-z][^>]*>/i.test(text)) {
        return `Страница ${url} (HTTP ${status}): нет HTML-контента.`;
    }
    const html = text;
    const cleaned = stripNoiseTags(html);
    let content: string | null = null;
    for (const cls of ['markdown-body', 'article-content', 'content', 'bytemd-preview']) {
        const found = findTagsByAttr(cleaned, 'class', cls);
        for (const f of found) {
            const block = findBalancedBlock(cleaned, f.tag, f.index);
            if (!block) continue;
            const innerOpen = cleaned.slice(block.start).match(/<[^>]+>/);
            const inner = innerOpen
                ? cleaned.slice(block.start + innerOpen[0].length, block.end - `</${f.tag}>`.length)
                : cleaned.slice(block.start, block.end);
            if (normalizeText(blockToText(inner)).length >= 120) { content = inner; break; }
        }
        if (content) break;
    }
    if (!content) {
        const body = extractTagContent(cleaned, 'body');
        const bodyText = normalizeText(blockToText(body || cleaned));
        if (bodyText.length < 120) throw new Error('Не удалось извлечь текст статьи (нужен рендер JS)');
        return `# ${matchTitle(html)}\nИсточник: ${url}\n\n${bodyText}`;
    }
    const title = matchTitle(html);
    return `# ${title}\nИсточник: ${url}\n\n${normalizeText(blockToText(content))}`;
}

async function fetchLinuxdoArticle(urlStr: string): Promise<string> {
    // Discourse JSON API: https://linux.do/t/{topicId}.json
    const parsed = new URL(urlStr);
    const topicId = (parsed.pathname.match(/\/t\/(?:[^/]+\/)?(\d+)$/) || [])[1];
    const jsonUrl = `https://linux.do/t/${topicId}.json`;
    const res = await fetchUrlSafe(jsonUrl, { timeoutMs: 15000 });
    let json: { post_stream?: { posts?: { cooked?: string }[] }; title?: string };
    try { json = JSON.parse(res.text); } catch { throw new Error('linux.do вернул не JSON (вероятно Cloudflare-блок)'); }
    const post = json.post_stream && json.post_stream.posts && json.post_stream.posts[0];
    if (!post || !post.cooked) throw new Error('В теме нет постов');
    const text = normalizeText(stripTags(post.cooked));
    return `# ${json.title || 'Тема linux.do'}\nИсточник: https://linux.do/t/${topicId}\n\n${text}`;
}

// ─────────────────────────── GitHub README (по open-websearch/github.ts) ───────────────────────────

const README_CANDIDATES = [
    'README.md', 'README.mdx', 'README.markdown', 'README.txt', 'README',
    'readme.md', 'readme.mdx', 'readme.markdown', 'readme.txt', 'readme', 'Readme.md',
];

function parseGithubRepo(urlStr: string): { owner: string; repo: string } | null {
    let m = urlStr.trim().match(/^git@github\.com:([^/]+)\/([^/]+?)(?:\.git)?$/);
    if (m) return { owner: m[1], repo: m[2] };
    let parsed: URL;
    try { parsed = new URL(urlStr); } catch { return null; }
    if (parsed.hostname !== 'github.com' && parsed.hostname !== 'www.github.com') return null;
    const parts = parsed.pathname.split('/').filter(Boolean);
    if (parts.length < 2) return null;
    return { owner: parts[0], repo: parts[1].replace(/\.git$/, '') };
}

async function fetchGithubReadme(urlStr: string): Promise<string> {
    const repo = parseGithubRepo(urlStr);
    if (!repo) throw new Error('Не похоже на GitHub-репозиторий (ожидался https://github.com/owner/repo или git@github.com:owner/repo.git)');
    const owner = encodeURIComponent(repo.owner);
    const name = encodeURIComponent(repo.repo);
    let lastError: Error | null = null;
    for (const candidate of README_CANDIDATES) {
        const rawUrl = `https://raw.githubusercontent.com/${owner}/${name}/HEAD/${candidate}`;
        let res: { status: number; text: string };
        try {
            res = await request(rawUrl, {
                userAgent: 'GitHub-README-Fetcher/1.0',
                timeoutMs: 10000,
                checkUrl: (u: string, hop: number) => assertPublicUrlResolved(u, `URL (хоп ${hop})`),
            });
        } catch (e) {
            lastError = e as Error;
            break; // сетевая ошибка — дальше бессмысленно
        }
        if (res.status === 200) {
            const text = res.text.trim();
            return `# ${repo.owner}/${repo.repo}\nИсточник: https://github.com/${repo.owner}/${repo.repo}\n\n${text}`;
        }
        if (res.status !== 404) { lastError = new Error('HTTP ' + res.status); break; }
    }
    throw new Error(`README не найден в ${repo.owner}/${repo.repo}${lastError ? ' (' + lastError.message + ')' : ''}`);
}

// ─────────────────────────── Обработчики ───────────────────────────

function formatResult(extraction: { title: string; text: string }, finalUrl: string): string {
    const lines: string[] = [];
    if (extraction.title) lines.push(`# ${extraction.title}`);
    lines.push(`Источник: ${finalUrl}`);
    lines.push('');
    lines.push(extraction.text);
    return lines.join('\n');
}

function normalizeUrlForArticle(raw: string): URL | null {
    try {
        const u = new URL(raw);
        if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
        return u;
    } catch { return null; }
}

createMcpServer({
    name: "docs-fetcher-mcp",
    version: "2.0.0",
    tools: [
        {
            name: "WebFetch",
            description: "Скачать веб-страницу по URL и извлечь читаемый текст (защита от SSRF: только публичные http/https адреса).",
            inputSchema: { type: "object", properties: { url: { type: "string", description: "URL адрес страницы" } }, required: ["url"] }
        },
        {
            name: "FetchArticle",
            description: "Извлечь полный текст статьи с сайтов: blog.csdn.net (CSDN), juejin.cn (Juejin), linux.do (Discourse-форум). Валидация URL по домену.",
            inputSchema: {
                type: "object",
                properties: {
                    url: { type: "string", description: "URL статьи (например https://blog.csdn.net/xxx/article/details/123)" },
                    type: { type: "string", description: "Тип сайта: csdn | juejin | linuxdo" }
                },
                required: ["url", "type"]
            }
        },
        {
            name: "FetchGithubReadme",
            description: "Загрузить README репозитория GitHub (raw.githubusercontent, без API-ключей). Поддерживает https://github.com/owner/repo и git@github.com:owner/repo.git.",
            inputSchema: { type: "object", properties: { url: { type: "string", description: "URL репозитория GitHub" } }, required: ["url"] }
        }
    ],
    handlers: {
        WebFetch: async (args: Record<string, string>) => {
            const url = (args.url || '').trim();
            if (!url) { throw new Error("WebFetch: укажи 'url'."); }
            const { text, url: finalUrl, status } = await fetchUrlSafe(url);
            if (!text || (!looksLikeHtml(text) && !/<[a-z][^>]*>/i.test(text))) {
                return `Страница ${finalUrl} (HTTP ${status}): нет HTML-контента.\n${text.slice(0, 2000)}`;
            }
            const extraction = extractMainText(text);
            return formatResult(extraction, finalUrl);
        },
        FetchArticle: async (args: Record<string, string>) => {
            const url = (args.url || '').trim();
            const type = (args.type || '').trim().toLowerCase();
            if (!url || !type) { throw new Error("FetchArticle: укажи 'url' и 'type' (csdn | juejin | linuxdo)."); }
            const parsed = normalizeUrlForArticle(url);
            if (!parsed) throw new Error('FetchArticle: невалидный URL');
            if (type === 'csdn') {
                if (!validateCsdnUrl(parsed)) throw new Error('Ожидался URL вида https://blog.csdn.net/xxx/article/details/yyy');
                return fetchCsdnArticle(url);
            }
            if (type === 'juejin') {
                if (!validateJuejinUrl(parsed)) throw new Error('Ожидался URL вида https://juejin.cn/post/yyy');
                return fetchJuejinArticle(url);
            }
            if (type === 'linuxdo') {
                if (!validateLinuxdoUrl(parsed)) throw new Error('Ожидался URL вида https://linux.do/t/yyy');
                return fetchLinuxdoArticle(url);
            }
            throw new Error("Неизвестный тип сайта: " + type + " (доступны: csdn, juejin, linuxdo)");
        },
        FetchGithubReadme: async (args: Record<string, string>) => {
            const url = (args.url || '').trim();
            if (!url) { throw new Error("FetchGithubReadme: укажи 'url'."); }
            return fetchGithubReadme(url);
        }
    }
});
