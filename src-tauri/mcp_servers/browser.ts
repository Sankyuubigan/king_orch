// MCP-сервер браузерного слоя search-specialist (Deno).
// JS-рендеринг, PDF-спасение, куки-сессии, поиск через поисковики с PoW-защитой (Startpage/Anubis).
// Браузер: Chrome-for-Testing (авто-докачка через bin_downloader.rs, env KING_ORCH_CHROME_PATH / KING_ORCH_BINS_DIR).
// Права (runtime.rs): --allow-net --allow-run --allow-read --allow-write --allow-env
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createMcpServer, log } from "./mcp_base.ts";

import * as puppeteerNs from "npm:puppeteer-core@^23";
import { JSDOM } from "npm:jsdom@^25";
import { Readability } from "npm:@mozilla/readability@^0.5.0";

const puppeteer: any = (puppeteerNs as any).default ?? puppeteerNs;

const UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

// ---------- пути ----------

function binsDir(): string {
    return Deno.env.get("KING_ORCH_BINS_DIR") || path.join(os.tmpdir(), "king_orch_bins");
}

log(`browser: KING_ORCH_BINS_DIR=${Deno.env.get("KING_ORCH_BINS_DIR")} CHROME_PATH=${Deno.env.get("KING_ORCH_CHROME_PATH")}`);

function profilesDir(): string {
    return path.join(binsDir(), "browser_profiles");
}

function pdfDir(): string {
    return path.join(binsDir(), "browser_pdf");
}

function resolveChromeExe(): string | null {
    const envExe = Deno.env.get("KING_ORCH_CHROME_PATH");
    if (envExe && fs.existsSync(envExe)) return envExe;
    const searchRoot = path.join(binsDir(), "chrome");
    const known = path.join(searchRoot, "chrome-win64", "chrome.exe");
    if (fs.existsSync(known)) return known;
    return findFile(searchRoot, "chrome.exe");
}

function findFile(root: string, name: string, depth = 0): string | null {
    if (depth > 6 || !fs.existsSync(root)) return null;
    for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
        const p = path.join(root, entry.name);
        if (entry.isDirectory()) {
            const r = findFile(p, name, depth + 1);
            if (r) return r;
        } else if (entry.name.toLowerCase() === name.toLowerCase()) {
            return p;
        }
    }
    return null;
}

// ---------- SSRF-защита (как в docs_fetcher, только по URL до рендера) ----------

function isPrivateHost(hostname: string): boolean {
    const h = hostname.toLowerCase().replace(/[\[\]]/g, "");
    if (h === "localhost" || h.endsWith(".localhost") || h === "::1" || h === "0.0.0.0" || h === "127.0.0.1") return true;
    const ipv4 = h.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
    if (ipv4) {
        const a = parseInt(ipv4[1]);
        const b = parseInt(ipv4[2]);
        if (a === 10 || a === 127) return true;
        if (a === 169 && b === 254) return true;
        if (a === 172 && b >= 16 && b <= 31) return true;
        if (a === 192 && b === 168) return true;
        if (a === 0 || (a === 100 && b >= 64 && b <= 127)) return true;
    }
    return false;
}

function assertPublicUrl(rawUrl: string, allowPrivate: boolean): string {
    let u: URL;
    try { u = new URL(rawUrl); } catch { throw new Error(`Некорректный URL: ${rawUrl}`); }
    if (u.protocol !== "http:" && u.protocol !== "https:") throw new Error(`Допускаются только http/https, получено: ${u.protocol}`);
    if (!allowPrivate && isPrivateHost(u.hostname)) throw new Error(`Запрещён доступ к приватному адресу: ${u.hostname} (allowPrivate=true для обхода)`);
    return u.toString();
}

// ---------- HTML -> markdown (jsdom + Readability + свой блочный конвертер) ----------

function htmlToMarkdown(html: string, url: string): string {
    const dom = new JSDOM(html, { url });
    const doc = dom.window.document;
    const article = new Readability(doc).parse();
    const title = article?.title || doc.title || url;

    let text = "";
    const articleContent = article ? (article as any).content : null;
    if (typeof articleContent === "string") {
        const cdom = new JSDOM(articleContent, { url });
        text = elementToText(cdom.window.document.body as Element);
    } else if (articleContent) {
        text = elementToText(articleContent as Element);
    } else if (doc.body) {
        text = elementToText(doc.body);
    }

    let md = `# ${title}\n\n`;
    if (article?.byline) md += `Автор: ${article.byline}\n\n`;
    md += text;
    return md.trim();
}

const BLOCK_TAGS = new Set(["p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "blockquote", "section", "article", "ul", "ol", "table", "tr", "td", "th", "figure", "figcaption", "header", "footer", "form", "fieldset"]);

function elementToText(el: Element): string {
    let out = "";
    const walk = (node: Element): void => {
        const tag = node.tagName.toLowerCase();
        if (tag === "script" || tag === "style" || tag === "noscript" || tag === "svg") return;
        if (tag === "br") { out += "\n"; return; }
        if (tag === "a") {
            const href = node.getAttribute("href") || "";
            const text = (node.textContent || "").trim();
            if (text) out += href && /^https?:\/\//i.test(href) ? `[${text}](${href})` : text;
            return;
        }
        if (tag === "img") {
            const alt = node.getAttribute("alt") || node.getAttribute("title") || "";
            if (alt) out += `[изображение: ${alt}]`;
            return;
        }
        if (tag === "pre") { out += "```\n" + (node.textContent || "") + "\n```\n"; return; }
        const isBlock = BLOCK_TAGS.has(tag);
        const isHeading = tag.match(/^h[1-6]$/);
        if (isBlock && out && !out.endsWith("\n")) out += "\n";
        if (isHeading && node.textContent) out += "#".repeat(Number(tag[1])) + " ";
        for (const child of Array.from(node.childNodes)) {
            if (child.nodeType === 3) out += child.textContent ?? "";
            else if (child.nodeType === 1) walk(child as Element);
        }
        if (isHeading && out && !out.endsWith("\n")) out += "\n";
        else if (isBlock && out && !out.endsWith("\n") && !out.endsWith(" ")) out += "\n";
    };
    walk(el);
    return out.replace(/\n{3,}/g, "\n\n").replace(/[ \t]+\n/g, "\n").trim();
}

function truncate(text: string, maxLength: number): string {
    if (text.length <= maxLength) return text;
    const cut = text.slice(0, maxLength);
    const lastSpace = cut.lastIndexOf(" ");
    return (lastSpace > maxLength * 0.7 ? cut.slice(0, lastSpace) : cut) + "\n… (обрезано, увеличьте maxLength)";
}

// ---------- детект защитных страниц ----------

function isChallengePage(html: string): boolean {
    const probe = html.slice(0, 200000).toLowerCase();
    return probe.includes("cf-challenge")
        || probe.includes("just a moment")
        || probe.includes("attention required")
        || probe.includes("enable javascript and cookies")
        || probe.includes("anubis") && probe.includes("proof of work")
        || probe.includes("verify you are human")
        || probe.includes("captcha");
}

// ---------- браузер (ленивый, по сессиям) ----------

const browsers: Record<string, any> = {};

async function getBrowser(session: string): Promise<any> {
    if (browsers[session]) return browsers[session];
    const exe = resolveChromeExe();
    if (!exe) throw new Error("Chrome не найден. Авто-докачка не удалась — см. логи приложения (KING_ORCH_CHROME_PATH или bins/chrome/).");
    const userDataDir = path.join(profilesDir(), session);
    fs.mkdirSync(userDataDir, { recursive: true });
    fs.mkdirSync(pdfDir(), { recursive: true });
    browsers[session] = await puppeteer.launch({
        executablePath: exe,
        headless: true,
        userDataDir,
        defaultViewport: { width: 1280, height: 900 },
        protocolTimeout: 120000,
        args: [
            "--disable-gpu",
            "--disable-blink-features=AutomationControlled",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-background-networking",
            "--mute-audio",
        ],
    });
    return browsers[session];
}

// ---------- быстрый fetch (без браузера) ----------

async function fetchWithTimeout(url: string, timeoutMs: number): Promise<Response> {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), timeoutMs);
    try {
        return await fetch(url, {
            signal: ctrl.signal,
            redirect: "follow",
            headers: {
                "User-Agent": UA,
                "Accept": "text/html,application/xhtml+xml,application/json;q=0.9,*/*;q=0.8",
                "Accept-Language": "ru,en;q=0.8",
            },
        });
    } finally {
        clearTimeout(timer);
    }
}

async function htmlOrText(response: Response, url: string, maxLength: number): Promise<string> {
    const bytes = await response.arrayBuffer();
    const bodyLen = bytes.byteLength;
    const text = new TextDecoder("utf-8", { fatal: false }).decode(bytes);
    const contentType = (response.headers.get("content-type") || "").toLowerCase();
    if (contentType.includes("html") || /^\s*<!doctype|<html/i.test(text)) {
        if (isChallengePage(text)) throw new Error("Страница за Cloudflare/Anubis-защитой — нужен режим render");
        return truncate(htmlToMarkdown(text, url), maxLength);
    }
    const label = contentType.split(";")[0] || "unknown";
    return truncate(`[${label}, ${(bodyLen / 1024).toFixed(0)} КБ]\n${text}`, maxLength);
}

async function tryFastFetch(url: string, timeoutMs: number, maxLength: number): Promise<{ ok: boolean; text?: string; reason?: string }> {
    let resp: Response;
    try {
        resp = await fetchWithTimeout(url, timeoutMs);
    } catch (e) {
        return { ok: false, reason: `сетевая ошибка: ${(e as Error).name}` };
    }
    if (!resp.ok) return { ok: false, reason: `HTTP ${resp.status}` };
    try {
        return { ok: true, text: await htmlOrText(resp, url, maxLength) };
    } catch (e) {
        return { ok: false, reason: (e as Error).message };
    }
}

// ---------- рендер в браузере ----------

async function renderPage(url: string, timeoutMs: number): Promise<string> {
    const page = await (await getBrowser("default")).newPage();
    try {
        page.setDefaultNavigationTimeout(timeoutMs);
        await page.setUserAgent(UA);
        try {
            await page.goto(url, { waitUntil: "networkidle2", timeout: timeoutMs });
        } catch {
            await page.goto(url, { waitUntil: "domcontentloaded", timeout: timeoutMs });
        }
        await new Promise((r) => setTimeout(r, 800));
        return await page.content();
    } finally {
        await page.close().catch(() => {});
    }
}

async function renderToMarkdown(url: string, timeoutMs: number, maxLength: number): Promise<string> {
    const html = await renderPage(url, timeoutMs);
    if (isChallengePage(html)) throw new Error("Страница за Cloudflare/Anubis-защитой даже после рендера");
    const md = htmlToMarkdown(html, url);
    if (!md.trim()) throw new Error("На странице нет текста после рендера");
    return truncate(md, maxLength);
}

async function renderToPdf(url: string, timeoutMs: number, maxLength: number): Promise<string> {
    const page = await (await getBrowser("default")).newPage();
    let html = "";
    try {
        page.setDefaultNavigationTimeout(timeoutMs);
        await page.setUserAgent(UA);
        try {
            await page.goto(url, { waitUntil: "networkidle2", timeout: timeoutMs });
        } catch {
            await page.goto(url, { waitUntil: "domcontentloaded", timeout: timeoutMs });
        }
        await new Promise((r) => setTimeout(r, 800));
        html = await page.content();
        const pdf = await page.pdf({ format: "A4", printBackground: true });
        const u = new URL(url);
        const slug = (u.hostname + u.pathname).replace(/[^\w.-]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 60) || "page";
        const file = path.join(pdfDir(), `${slug}-${Date.now()}.pdf`);
        await Deno.writeFile(file, new Uint8Array(pdf));
        const text = htmlToMarkdown(html, url);
        return `PDF сохранён: ${file} (${(pdf.byteLength / 1024).toFixed(0)} КБ)\n\nТекст со страницы (если DOM-парсинг помог):\n${truncate(text, maxLength)}`;
    } finally {
        await page.close().catch(() => {});
    }
}

// ---------- поиск через поисковики ----------

const SEARCH_ENGINES: Record<string, { url: (q: string) => string; selector: string }> = {
    startpage: {
        url: (q) => `https://www.startpage.com/sp/search?query=${encodeURIComponent(q)}`,
        selector: ".w-gl__result, .result",
    },
    google: {
        url: (q) => `https://www.google.com/search?q=${encodeURIComponent(q)}&num=20&hl=${encodeURIComponent("ru")}`,
        selector: "div.g, div[data-sokoban-container]",
    },
    bing: {
        url: (q) => `https://www.bing.com/search?q=${encodeURIComponent(q)}&count=20&setlang=${encodeURIComponent("ru")}`,
        selector: "li.b_algo",
    },
    duckduckgo: {
        url: (q) => `https://duckduckgo.com/?q=${encodeURIComponent(q)}&ia=web&kl=${encodeURIComponent("ru-ru")}`,
        selector: "article[data-testid='result'], .react-results article",
    },
};

function cleanSearchUrl(raw: string, baseUrl: string): string {
    try {
        if (raw.startsWith("/url?q=")) {
            const m = raw.match(/[?&]q=([^&]+)/);
            if (m) return decodeURIComponent(m[1]);
        }
        return new URL(raw, baseUrl).toString();
    } catch {
        return raw;
    }
}

async function searchWeb(engine: string, query: string, maxResults: number, timeoutMs: number): Promise<string> {
    const conf = SEARCH_ENGINES[engine];
    if (!conf) throw new Error(`Неизвестный движок: ${engine}. Доступны: ${Object.keys(SEARCH_ENGINES).join(", ")}`);

    const page = await (await getBrowser("default")).newPage();
    let html = "";
    try {
        page.setDefaultNavigationTimeout(timeoutMs);
        await page.setUserAgent(UA);
        try {
            await page.goto(conf.url(query), { waitUntil: "networkidle2", timeout: timeoutMs });
        } catch {
            await page.goto(conf.url(query), { waitUntil: "domcontentloaded", timeout: timeoutMs });
        }
        try {
            await page.waitForSelector(conf.selector, { timeout: 8000 });
        } catch {}
        await new Promise((r) => setTimeout(r, 1500));
        html = await page.content();

        const items = await page.evaluate((sel) => {
            const out: { title: string; url: string; snippet: string }[] = [];
            document.querySelectorAll(sel).forEach((el) => {
                let a = el.querySelector("a[data-testid='result-title-a']");
                if (!a) {
                    for (const cand of Array.from(el.querySelectorAll("a[href]"))) {
                        const h = cand.getAttribute("href") || "";
                        if (cand.getAttribute("data-testid") === "result-extras-site-search-link") continue;
                        if (h.startsWith("http") || h.startsWith("//") || h.startsWith("/")) { a = cand; break; }
                    }
                }
                if (!a) return;
                const url = a.getAttribute("href") || "";
                if (url.startsWith("#") || url.startsWith("javascript:")) return;
                const title = (a.textContent || a.getAttribute("title") || "")
                    .replace(/<style[\s\S]*?<\/style>/gi, "")
                    .replace(/<[^>]*>/g, "")
                    .replace(/\.[\w-]+\{[^}]*\}/g, "")
                    .replace(/\s+/g, " ")
                    .trim();
                if (!title) return;
                const snippetEl = el.querySelector("[data-testid='result-snippet'], .result__snippet")
                    || Array.from(el.querySelectorAll("p")).find((p) => {
                        const t = (p.textContent || "").replace(/\s+/g, " ").trim();
                        return t.length > 60 && !/^https?:\/\/|^www\./i.test(t);
                    });
                let snippet = (snippetEl ? snippetEl.textContent : "").replace(/\s+/g, " ").trim().slice(0, 300);
                if (!snippet) snippet = (el.textContent || "").replace(/\s+/g, " ").trim().slice(0, 300);
                snippet = snippet
                    .replace(/Включать только результаты с этого сайта|Повторить поиск без этого сайта|Заблокируй этот сайт во всех результатах|Отправить отзыв о сайте|Block this site|Перевести эту страницу|Перевод этой страницы/g, " ")
                    .replace(/\s+/g, " ")
                    .trim();
                out.push({ title, url, snippet });
            });
            return out;
        }, conf.selector);

        const seen = new Set<string>();
        const results: { title: string; url: string; snippet: string }[] = [];
        for (const item of items) {
            if (item.title.length > 200) continue;
            if (item.title.includes("Включать") || item.title.includes("Block this site") || item.title.includes("Отправить отзыв")) continue;
            const raw = item.url;
            if (!raw.startsWith("http") && !raw.startsWith("/")) continue;
            const url = cleanSearchUrl(raw, conf.url(query));
            if (seen.has(url)) continue;
            seen.add(url);
            results.push(item);
            if (results.length >= maxResults) break;
        }

        if (results.length === 0) {
            if (isChallengePage(html)) throw new Error("Поисковик показал защитную страницу (Cloudflare/Anubis)");
            const md = htmlToMarkdown(html, conf.url(query));
            return `Результаты не найдены по селектору. Страница содержит:\n${truncate(md, 4000)}`;
        }

        return results.map((r, i) => `${i + 1}. [${r.title}](${r.url})\n   ${r.snippet}`).join("\n");
    } finally {
        await page.close().catch(() => {});
    }
}

// ---------- сессии ----------

async function listSessions(): Promise<string> {
    let out = "";
    const dir = profilesDir();
    if (fs.existsSync(dir)) {
        const sessions = fs.readdirSync(dir).filter((n) => fs.statSync(path.join(dir, n)).isDirectory());
        out += "Профили-сессии: " + (sessions.length ? sessions.join(", ") : "(нет)") + "\n";
    }
    const pdfs = pdfDir();
    if (fs.existsSync(pdfs)) {
        const files = fs.readdirSync(pdfs).filter((n) => n.endsWith(".pdf"));
        out += `Сохранённые PDF (${files.length}): ` + (files.length ? files.slice(-5).join(", ") : "(нет)") + "\n";
    }
    return out.trim() || "(нет профилей и PDF)";
}

// ---------- регистрация тулов ----------

createMcpServer({
    name: "browser",
    version: "1.0.0",
    tools: [
        {
            name: "BrowserFetch",
            description: "Получить содержимое веб-страницы с поддержкой JavaScript. mode=auto: сначала быстрый HTTP-fetch (SSRF-защита, таймаут), при Cloudflare/Anubis/ошибке — автоматический рендер в Chrome. mode=render: принудительный рендер (JS-сайты). mode=fast: только быстрый fetch. mode=pdf: рендер + сохранение PDF в bins/browser_pdf (PDF-спасение, путь вернётся в ответе). Конвертация DOM в markdown (Readability). Разрешены только публичные http/https URL; allowPrivate=true снимает SSRF-блок (не рекомендуется).",
            inputSchema: {
                type: "object",
                properties: {
                    url: { type: "string", description: "URL страницы" },
                    mode: { type: "string", enum: ["auto", "fast", "render", "pdf"], description: "Режим получения (по умолчанию auto)" },
                    timeout: { type: "number", description: "Таймаут в мс (по умолчанию 30000, максимум 120000)" },
                    maxLength: { type: "number", description: "Максимум символов ответа (по умолчанию 20000)" },
                    allowPrivate: { type: "boolean", description: "Разрешить приватные адреса (по умолчанию false)" }
                },
                required: ["url"]
            }
        },
        {
            name: "BrowserSearch",
            description: "Поиск в веб через движок (startpage/google/bing/duckduckgo) с рендером страницы результатов — обходит Anubis (Startpage) и Cloudflare. Возвращает пронумерованный список [заголовок](url) + сниппет. Полезен, когда обычный web_search падает на защите.",
            inputSchema: {
                type: "object",
                properties: {
                    query: { type: "string", description: "Поисковый запрос" },
                    engine: { type: "string", enum: ["startpage", "google", "bing", "duckduckgo"], description: "Поисковик (по умолчанию startpage)" },
                    maxResults: { type: "number", description: "Сколько результатов вернуть (по умолчанию 8, максимум 20)" },
                    timeout: { type: "number", description: "Таймаут в мс (по умолчанию 30000)" }
                },
                required: ["query"]
            }
        },
        {
            name: "BrowserSession",
            description: "Управление браузерными сессиями: куки сохраняются в профиле (bins/browser_profiles/<имя>) и персистентны между вызовами. list — показать профили и сохранённые PDF; clear — удалить профиль (сбросить куки/логины).",
            inputSchema: {
                type: "object",
                properties: {
                    action: { type: "string", enum: ["list", "clear"], description: "Действие (по умолчанию list)" },
                    session: { type: "string", description: "Имя сессии для clear (по умолчанию default)" }
                },
                required: ["action"]
            }
        },
        {
            name: "BrowserDownload",
            description: "Скачивает файл через headless Chrome (BoringSSL — обход DPI/block). Chrome использует тот же TLS-стек что и обычный браузер, поэтому Kaspersky/DPI пропускает трафик. Возвращает путь к скачанному файлу и размер в байтах.",
            inputSchema: {
                type: "object",
                properties: {
                    url: { type: "string", description: "URL для скачивания" },
                    outputPath: { type: "string", description: "Путь для сохранения файла (обязательно)" },
                    timeout: { type: "number", description: "Таймаут в мс (по умолчанию 300000, макс 600000)" }
                },
                required: ["url", "outputPath"]
            }
        }
    ],
    handlers: {
        BrowserFetch: async (args: Record<string, unknown>) => {
            const url = assertPublicUrl(String(args.url || ""), args.allowPrivate === true);
            const mode = String(args.mode || "auto");
            const maxLength = Math.max(1000, Math.min(Number(args.maxLength) || 20000, 100000));
            const timeoutMs = Math.max(5000, Math.min(Number(args.timeout) || 30000, 120000));

            if (mode === "fast") {
                const fast = await tryFastFetch(url, timeoutMs, maxLength);
                if (fast.ok) return fast.text as string;
                throw new Error(`Быстрый fetch не удался: ${fast.reason}`);
            }
            if (mode === "render") return await renderToMarkdown(url, timeoutMs, maxLength);
            if (mode === "pdf") return await renderToPdf(url, timeoutMs, maxLength);

            const fast = await tryFastFetch(url, timeoutMs, maxLength);
            if (fast.ok) return fast.text as string;
            try {
                return await renderToMarkdown(url, timeoutMs, maxLength);
            } catch (e) {
                throw new Error(`Fast-fetch: ${fast.reason}; рендер тоже не удался: ${(e as Error).message}`);
            }
        },
        BrowserSearch: async (args: Record<string, unknown>) => {
            return await searchWeb(
                String(args.engine || "startpage"),
                String(args.query || ""),
                Math.max(1, Math.min(Number(args.maxResults) || 8, 20)),
                Math.max(5000, Math.min(Number(args.timeout) || 30000, 120000)),
            );
        },
        BrowserSession: async (args: Record<string, unknown>) => {
            const action = String(args.action || "list");
            if (action === "clear") {
                const session = String(args.session || "default");
                const dir = path.join(profilesDir(), session);
                if (session === "default" && browsers[session]) {
                    throw new Error("Сессия default активна прямо сейчас — она сотрётся при следующем вызове. Перезапустите диалог или не очищайте default.");
                }
                if (!fs.existsSync(dir)) throw new Error(`Профиль '${session}' не найден`);
                fs.rmSync(dir, { recursive: true, force: true });
                delete browsers[session];
                return `Профиль '${session}' удалён.`;
            }
            return await listSessions();
        },
        BrowserDownload: async (args: Record<string, unknown>) => {
            const url = String(args.url || "");
            const outputPath = String(args.outputPath || "");
            if (!url) throw new Error("url обязателен");
            if (!outputPath) throw new Error("outputPath обязателен");

            const timeoutMs = Math.max(30000, Math.min(Number(args.timeout) || 300000, 600000));
            const downloadDir = path.dirname(outputPath);
            fs.mkdirSync(downloadDir, { recursive: true });

            const exe = resolveChromeExe();
            if (!exe) throw new Error("Chrome не найден для скачивания");

            const tempProfile = path.join(profilesDir(), `_dl_${Date.now()}`);
            fs.mkdirSync(tempProfile, { recursive: true });

            let browser: any = null;
            try {
                browser = await puppeteer.launch({
                    executablePath: exe,
                    headless: true,
                    userDataDir: tempProfile,
                    args: [
                        "--no-sandbox",
                        "--disable-gpu",
                        "--disable-extensions",
                        "--disable-blink-features=AutomationControlled",
                    ],
                });

                const page = await browser.newPage();

                // Устанавливаем download behavior через CDP
                const cdpSession = await page.createCDPSession();
                await cdpSession.send("Browser.setDownloadBehavior", {
                    behavior: "allow",
                    downloadPath: downloadDir,
                    eventsEnabled: true,
                });

                // Навигация на URL (начинает скачивание)
                try {
                    await page.goto(url, { waitUntil: "commit", timeout: timeoutMs });
                } catch {
                    // Навигация может "упасть" из-за скачивания — это нормально
                }

                // Ждём завершения скачивания (мониторим файл)
                const filename = path.basename(outputPath);
                const deadline = Date.now() + timeoutMs;
                let downloaded = false;

                while (Date.now() < deadline) {
                    await new Promise(r => setTimeout(r, 1000));
                    // Проверяем наличие файла (или .crdownload — временный)
                    const files = fs.readdirSync(downloadDir);
                    const target = files.find(f => f === filename);
                    const partial = files.find(f => f.endsWith(".crdownload"));

                    if (target) {
                        // Файл появился — ждём немного для завершения записи
                        await new Promise(r => setTimeout(r, 1000));
                        // Переименовываем если нужно
                        const src = path.join(downloadDir, target);
                        if (src !== outputPath) {
                            fs.copyFileSync(src, outputPath);
                        }
                        downloaded = true;
                        break;
                    }
                }

                await page.close().catch(() => {});

                if (!downloaded) {
                    throw new Error(`Таймаут скачивания (${timeoutMs / 1000}с)`);
                }

                const stat = fs.statSync(outputPath);
                log(`[browser] BrowserDownload: OK ${stat.size} bytes → ${outputPath}`);
                return JSON.stringify({
                    success: true,
                    bytes: stat.size,
                    path: outputPath,
                    method: "chrome_cdp",
                });
            } catch (e) {
                log(`[browser] BrowserDownload FAILED: ${(e as Error).message}`);
                throw e;
            } finally {
                if (browser) await browser.close().catch(() => {});
                fs.rmSync(tempProfile, { recursive: true, force: true });
            }
        }
    }
});