const fs = require('fs');
const path = require('path');
const { execSync, spawn } = require('child_process');

const scriptDir = __dirname;

const { syncResources } = require('./scripts/sync-resources.cjs');

function runCommand(command, args = [], options = {}) {
    return new Promise((resolve, reject) => {
        const proc = spawn(command, args, { stdio: 'inherit', shell: true, cwd: scriptDir, ...options });
        proc.on('close', (code) => {
            if (code === 0) resolve();
            else reject(new Error(`Command exited with code ${code}`));
        });
    });
}

async function main() {
    try {
        // Step 2: Signing key
        console.log('========================================');
        console.log('[2/4] Setting up signing key...');
        console.log('========================================\n');

        const defaultKeyPath = 'D:\\Projects\\docusaurus-starter\\docs\\Sega Mega Note\\Моя картотека\\software\\настройки\\tauri_signed_keys\\tauri.key';
        const keyPath = process.env.TAURI_PRIVATE_KEY_ORIGINAL || defaultKeyPath;

        if (fs.existsSync(keyPath)) {
            const keyContent = fs.readFileSync(keyPath, 'utf8').trim();
            const singleLineKey = keyContent.replace(/\r?\n|\r/g, '');
            process.env.TAURI_SIGNING_PRIVATE_KEY = singleLineKey;
            process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '123';
            console.log('Signing key loaded.\n');
        } else {
            console.warn('WARNING: Signing key not found. Building without signature.\n');
        }

        // Step 3: Tauri build
        console.log('========================================');
        console.log('[3/4] Building Tauri app (release + NSIS)...');
        console.log('========================================\n');

        // Зеркалим agents/ и mcp_servers/ в target-копии (убираем файлы-призраки).
        syncResources(scriptDir);

        await runCommand('npx', ['tauri', 'build']);

        // Step 4: Post-build
        console.log('\n========================================');
        console.log('[4/4] Post-build verification...');
        console.log('========================================\n');

        const releaseDir = path.join(scriptDir, 'src-tauri', 'target', 'release');
        const nsisDir = path.join(releaseDir, 'bundle', 'nsis');
        let installerFound = false;

        // Читаем версию из tauri.conf.json, чтобы выбрать установщик по ТОЧНОЙ версии
        // (а не по алфавитной сортировке — анти-паттерн rules.md §3.7: "26.8.87" > "26.8.133").
        let confVersion = '0.0.0';
        try {
            const confText = fs.readFileSync(path.join(scriptDir, 'src-tauri', 'tauri.conf.json'), 'utf8');
            const m = confText.match(/"version"\s*:\s*"(\d+\.\d+\.\d+)"/);
            if (m) confVersion = m[1];
        } catch (e) { /* пропускаем, отчёт не критичен */ }

        if (fs.existsSync(nsisDir)) {
            const exeRe = new RegExp('_' + confVersion.replace(/\./g, '\\.') + '_x64-setup\\.exe$');
            const matched = fs.readdirSync(nsisDir).filter(f => f.endsWith('-setup.exe') && exeRe.test(f));
            if (matched.length > 0) {
                const finalPath = path.join(nsisDir, matched[0]);
                console.log(`\n✅ Installer successfully generated:\n   👉 ${finalPath}\n`);
                installerFound = true;
            }
        }

        if (!installerFound) {
            console.error('\n❌ ERROR: The installer (-setup.exe) was NOT generated in the NSIS folder!');
            console.error('   This means "npx tauri build" finished, but skipped the NSIS bundle generation.');
            process.exit(1);
        }

    } catch (e) {
        console.error('\nERROR:', e.message);
        process.exit(1);
    }
}

main();