// Диск-кеш результатов поиска (Deno, zero-dependency).
// Общий для WebSearch (web_search.ts) и SearxngSearch (searxng_search.ts):
// одна и та же строка ответа не должна повторно долбить движки.
//
// Файл: src-tauri/bins/search_cache.json (KING_ORCH_BINS_DIR).
// Ключ: scope (web|searxng) + режим + engines + query.
// TTL: CACHE_TTL_MS (по умолчанию 30 мин) — свежие данные всё ещё актуальны
//      для fast-changing вопросов, но частые повторы не долбят сеть.
// Prune: старые записи выкидываются при превышении MAX_ENTRIES (LRU по ts).
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const CACHE_FILE = 'search_cache.json';
const CACHE_TTL_MS = 30 * 60 * 1000;
const MAX_ENTRIES = 300;

interface CacheEntry {
    ts: number;
    text: string;
}

type CacheData = Record<string, CacheEntry>;

let memory: CacheData | null = null;

function cachePath(): string {
    const dir = Deno.env.get("KING_ORCH_BINS_DIR") || os.tmpdir();
    return path.join(dir, CACHE_FILE);
}

function load(): CacheData {
    if (memory) return memory;
    try {
        const raw = JSON.parse(fs.readFileSync(cachePath(), 'utf8'));
        if (raw && typeof raw === 'object') { memory = raw; return raw; }
    } catch { /* нет кеша или битый */ }
    memory = {};
    return memory;
}

function save() {
    try {
        const data = load();
        const entries = Object.entries(data);
        if (entries.length > MAX_ENTRIES) {
            entries.sort((a, b) => b[1].ts - a[1].ts);
            const keep = new Set(entries.slice(0, MAX_ENTRIES).map(([k]) => k));
            for (const k of Object.keys(data)) if (!keep.has(k)) delete data[k];
        }
        fs.mkdirSync(path.dirname(cachePath()), { recursive: true });
        fs.writeFileSync(cachePath(), JSON.stringify(data), 'utf8');
    } catch { /* кеш не критичен */ }
}

/** Ключ кеша: scope + конфиг вызова + query. */
export function cacheKey(scope: string, engines: string[], parallel: boolean, query: string): string {
    const cfg = (parallel ? 'p:' : 'c:') + engines.join(',');
    return `${scope}::${cfg}::${query}`;
}

/** Вернуть кешированный текст ответа, если TTL ещё не истёк. */
export function cacheGet(key: string): string | null {
    const data = load();
    const e = data[key];
    if (!e) return null;
    if (Date.now() - e.ts > CACHE_TTL_MS) return null;
    return e.text;
}

/** Сохранить текст ответа в кеш. */
export function cachePut(key: string, text: string): void {
    const data = load();
    data[key] = { ts: Date.now(), text };
    save();
}

/** Полностью очистить кеш (для тестов/отладки). */
export function resetCache(): void {
    memory = {};
    try { fs.unlinkSync(cachePath()); } catch { /* нет файла */ }
}