// Общие утилиты для конвертеров датасетов coding-бенчмарка.
const fs = require('fs');
const path = require('path');

const ROOT = path.resolve(__dirname, '..', '..');
const OUT = path.join(ROOT, 'tasks_for_test_llm');
const HF = 'https://datasets-server.huggingface.co';

// Постраничная выгрузка всех строк датасета HF (datasets-server rows API).
async function hfRows(dataset, config, split) {
    const rows = [];
    let offset = 0;
    let total = Infinity;
    for (;;) {
        const url = `${HF}/rows?dataset=${dataset}&config=${config}&split=${split}&offset=${offset}&length=100`;
        const resp = await fetch(url);
        if (!resp.ok) throw new Error(`HTTP ${resp.status}: ${url}`);
        const json = await resp.json();
        const batch = json.rows || [];
        for (const b of batch) rows.push(b.row);
        total = typeof json.num_rows_total === 'number' ? json.num_rows_total : total;
        if (batch.length === 0 || rows.length >= total) break;
        offset += batch.length;
    }
    return rows;
}

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

module.exports = { ROOT, OUT, HF, hfRows, writeJsonl, codeBlock, signatureRe };