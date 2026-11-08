// Общие утилиты для конвертеров датасетов coding-бенчмарка.
const fs = require('fs');
const path = require('path');

const ROOT = path.resolve(__dirname, '..', '..');
const OUT = path.join(ROOT, 'tasks_for_test_llm');
const HF = 'https://datasets-server.huggingface.co';

// datasets-server лимитирует анонимные запросы (429): повтор с выдержкой.
async function fetchJsonWithRetry(url, attempts = 8) {
    for (let i = 1; ; i++) {
        const resp = await fetch(url);
        if (resp.ok) return resp.json();
        if (i >= attempts || ![429, 500, 502, 503].includes(resp.status)) {
            throw new Error(`HTTP ${resp.status}: ${url}`);
        }
        const waitSec = Math.min(90, 8 * i);
        console.log(`  HTTP ${resp.status}, повтор ${i}/${attempts} через ${waitSec}с...`);
        await new Promise((r) => setTimeout(r, waitSec * 1000));
    }
}

// Постраничная выгрузка всех строк датасета HF (datasets-server rows API).
// ВНИМАНИЕ: для больших датасетов используй hfRowsSampled — сплошная выгрузка
// 100+ страниц исчерпывает анонимную квоту (инцидент 07.11.2026: hellaswag
// падал с 429 на 8-й странице после расхода квоты предыдущими наборами).
async function hfRows(dataset, config, split) {
    const rows = [];
    let offset = 0;
    let total = Infinity;
    for (;;) {
        const url = `${HF}/rows?dataset=${dataset}&config=${config}&split=${split}&offset=${offset}&length=100`;
        const json = await fetchJsonWithRetry(url);
        const batch = json.rows || [];
        for (const b of batch) rows.push(b.row);
        total = typeof json.num_rows_total === 'number' ? json.num_rows_total : total;
        if (batch.length === 0 || rows.length >= total) break;
        offset += batch.length;
        // Пауза между страницами — меньше шансов словить 429.
        await new Promise((r) => setTimeout(r, 400));
    }
    return rows;
}

function strideRows(rows, target) {
    if (rows.length <= target) return rows;
    const s = Math.max(1, Math.floor(rows.length / target));
    return rows.filter((_, i) => i % s === 0);
}

// Выборка ~target строк, равномерно по всему датасету: первая страница rows API
// даёт num_rows_total, дальше забираем только нужные страницы (по ~5 на датасет
// вместо 100+ — остаёмся в анонимной квоте datasets-server, инцидент 07.11.2026).
// Страничное распределение репрезентативнее row-stride. Детерминировано.
async function hfRowsSampled(dataset, config, split, target) {
    const url0 = `${HF}/rows?dataset=${dataset}&config=${config}&split=${split}&offset=0&length=100`;
    const first = await fetchJsonWithRetry(url0);
    const rows = [];
    for (const b of first.rows || []) rows.push(b.row);
    const total = typeof first.num_rows_total === 'number' ? first.num_rows_total : rows.length;
    const pages = Math.ceil(total / 100);
    const want = Math.min(pages, Math.ceil(target / 100));
    const indices = new Set();
    for (let i = 1; i < want; i++) {
        indices.add(Math.round((i * (pages - 1)) / (want - 1 || 1)));
    }
    for (const p of indices) {
        const url = `${HF}/rows?dataset=${dataset}&config=${config}&split=${split}&offset=${p * 100}&length=100`;
        const json = await fetchJsonWithRetry(url);
        for (const b of json.rows || []) rows.push(b.row);
        await new Promise((r) => setTimeout(r, 400));
    }
    return strideRows(rows, target);
}

module.exports = { ROOT, OUT, HF, hfRows, hfRowsSampled, writeJsonl, codeBlock, signatureRe };
// Запись задач в JSONL набора.
function writeJsonl(suiteDir, tasks) {
    fs.mkdirSync(suiteDir, { recursive: true });
    const file = path.join(suiteDir, 'tasks.jsonl');
    const lines = tasks.map((t) => JSON.stringify(t));
    fs.writeFileSync(file, lines.join('\n') + '\n');
    return tasks.length;
}

// Обёртка блока кода для промпта.
function codeBlock(lang, code) {
    return `\`\`\`${lang}\n${code}\n\`\`\``;
}

// Поиск сигнатуры определения функции в коде.
function signatureRe(language, entryPoint) {
    const ep = String(entryPoint).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    switch (language) {
        case 'python': return `def\\s+${ep}\\b`;
        case 'rust': return `fn\\s+${ep}\\b`;
        default: return `\\b${ep}\\b\\s*(=|\\()`; // js/ts: const/function
    }
}