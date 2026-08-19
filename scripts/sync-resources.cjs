// Mirror-синхронизация ресурсов agents/ и mcp_servers/ в target/{profile}.
//
// Зачем: Tauri копирует bundle.resources аддитивно (перезаписывает изменённые
// файлы, но НЕ удаляет файлы, выпиленные из исходников). Из-за этого в
// target/{debug,release}/agents и mcp_servers накапливаются "призраки" —
// старые файлы (например, search-specialist.md после перехода на граф-режим),
// которые показываются юзеру в списке агентов.
//
// syncResources() делает target-копию зеркалом исходников: копирует новые,
// перезаписывает изменённые, удаляет отсутствующие в исходниках.

const fs = require('fs');
const path = require('path');

function syncDir(src, dest) {
    if (!fs.existsSync(src)) return;
    if (!fs.existsSync(dest)) fs.mkdirSync(dest, { recursive: true });

    const srcNames = new Set(fs.readdirSync(src));
    for (const name of fs.readdirSync(dest)) {
        if (!srcNames.has(name)) {
            fs.rmSync(path.join(dest, name), { recursive: true, force: true });
        }
    }

    for (const name of srcNames) {
        const s = path.join(src, name);
        const d = path.join(dest, name);
        const st = fs.statSync(s);
        if (st.isDirectory()) {
            syncDir(s, d);
        } else if (st.isFile()) {
            fs.copyFileSync(s, d);
        }
    }
}

function syncResources(scriptDir) {
    const pairs = [
        [path.join(scriptDir, 'agents'), 'agents'],
        [path.join(scriptDir, 'tasks_for_test_llm'), 'tasks_for_test_llm'],
        [path.join(scriptDir, 'src-tauri', 'mcp_servers'), 'mcp_servers'],
    ];

    for (const [src, rel] of pairs) {
        if (!fs.existsSync(src)) continue;
        for (const profile of ['debug', 'release']) {
            const dest = path.join(scriptDir, 'src-tauri', 'target', profile, rel);
            if (!fs.existsSync(path.dirname(dest))) continue;
            syncDir(src, dest);
            console.log(`[sync-resources] ${rel}/ -> ${dest}`);
        }
    }
}

module.exports = { syncDir, syncResources };