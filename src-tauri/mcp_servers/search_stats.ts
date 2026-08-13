// Статистика падений поисковых движков (Deno, zero-dependency).
// Цель: в реальных (живых) поисках накапливать, кто из движков как часто сбоит
// и почему — чтобы через время было видно, какие движки стабильны, а какие нет.
//
// Данные: src-tauri/bins/search_stats.json (KING_ORCH_BINS_DIR).
// Формат: { engine: { calls, ok, fails, lastOkTs, lastFailTs, lastFailReason, reasons: {тип: n} } }
//
// Подключение: web_search.ts (runChain/runParallel) и searxng_search.ts (runChain/runFanout).
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const STATS_FILE = 'search_stats.json';

interface EngineStats {
    calls: number;
    ok: number;
    fails: number;
    lastOkTs: number | null;
    lastFailTs: number | null;
    lastFailReason: string;
    reasons: Record<string, number>;
}

type StatsData = Record<string, EngineStats>;

let memory: StatsData | null = null; // кеш в рантайме (переживает MCP-сессию)

function statsPath(): string {
    const dir = Deno.env.get("KING_ORCH_BINS_DIR") || os.tmpdir();
    return path.join(dir, STATS_FILE);
}

function load(): StatsData {
    if (memory) return memory;
    try {
        const raw = JSON.parse(fs.readFileSync(statsPath(), 'utf8'));
        if (raw && typeof raw === 'object') { memory = raw; return raw; }
    } catch { /* нет файла или битый */ }
    memory = {};
    return memory;
}

function save() {
    try {
        const data = load();
        fs.mkdirSync(path.dirname(statsPath()), { recursive: true });
        fs.writeFileSync(statsPath(), JSON.stringify(data, null, 1), 'utf8');
    } catch { /* статистика не критична */ }
}

// Категоризация причины отказа для сводки по типам (без дублирования классификаторов движков).
export function categorizeReason(reason: string): string {
    const r = String(reason || '');
    if (/rate limit|ratelimit|429/i.test(r)) return 'rate-limit';
    if (/капча|captcha|challenge|anomaly|unusual|аномал|verify you are human/i.test(r)) return 'captcha';
    if (/Cloudflare|WAF|Anubis|PoW|угрожает|нужен браузер/i.test(r)) return 'antibot';
    if (/451|гео-блок/i.test(r)) return 'geo';
    if (/403|401|запрещено|доступ запрещён/i.test(r)) return 'forbidden';
    if (/дедлайн|завис/i.test(r)) return 'deadline';
    if (/Таймаут|DNS|TLS|соединение|отклонено|недоступна|разорвано|Сеть|ETIMEDOUT|ECONN/i.test(r)) return 'network';
    if (/HTTP \d|невалидный JSON|err_no|500|503|404/i.test(r)) return 'http';
    if (/пусто|не найдено результатов/i.test(r)) return 'empty';
    return 'other';
}

/**
 * Зафиксировать исход одного вызова движка/инстанса в статистике.
 * Вызывается при КАЖДОМ обращении к движку (ok или fail) — по требованию
 * «логировать неудачные движки в реальных поисках».
 */
export function recordEngineCall(engine: string, ok: boolean, ms: number, reason?: string): void {
    if (!engine) return;
    const stats = load();
    let s = stats[engine];
    if (!s) {
        s = { calls: 0, ok: 0, fails: 0, lastOkTs: null, lastFailTs: null, lastFailReason: '', reasons: {} };
        stats[engine] = s;
    }
    s.calls++;
    if (ok) {
        s.ok++;
        s.lastOkTs = Date.now();
    } else {
        s.fails++;
        s.lastFailTs = Date.now();
        const reasonStr = String(reason || 'неизвестная ошибка').slice(0, 200);
        s.lastFailReason = reasonStr;
        const type = categorizeReason(reasonStr);
        s.reasons[type] = (s.reasons[type] || 0) + 1;
    }
    save();
}

/**
 * Краткая сводка по движкам (для отчётов и диагностики).
 * Сортировка — по убыванию доли отказов (самые проблемные первыми).
 */
export function statsSummary(): string {
    const stats = load();
    const entries = Object.entries(stats).filter((e) => e[1].calls > 0);
    if (entries.length === 0) return 'статистика поисковых движков пуста';
    entries.sort((a, b) => (b[1].fails / b[1].calls) - (a[1].fails / a[1].calls));
    return entries.map(([name, s]) => {
        const failPct = Math.round((s.fails / s.calls) * 100);
        const top = Object.entries(s.reasons).sort((a, b) => b[1] - a[1]).slice(0, 2)
            .map(([t, n]) => `${t}×${n}`).join(', ');
        const last = s.lastFailTs ? `, посл. сбой: ${new Date(s.lastFailTs).toLocaleString('ru-RU')} (${s.lastFailReason.slice(0, 60)})` : '';
        return `${name}: вызовов ${s.calls}, ок ${s.ok}, сбоев ${s.fails} (${failPct}%)${top ? ', причины: ' + top : ''}${last}`;
    }).join('\n');
}

/** Очистить статистику (для тестов/отладки). */
export function resetStats(): void {
    memory = {};
    try { fs.unlinkSync(statsPath()); } catch { /* нет файла */ }
}