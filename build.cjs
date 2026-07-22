const fs = require('fs');
const path = require('path');
const { execSync, spawn } = require('child_process');
const https = require('https');
const http = require('http');

const scriptDir = __dirname;

function downloadFile(url, dest) {
    return new Promise((resolve, reject) => {
        const followRedirect = (currentUrl) => {
            const client = currentUrl.startsWith('https') ? https : http;
            client.get(currentUrl, { headers: { 'User-Agent': 'KingOrch-Dev-Script' } }, (response) => {
                if (response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
                    followRedirect(response.headers.location);
                    return;
                }
                if (response.statusCode !== 200) {
                    reject(new Error(`Не удалось скачать: ${response.statusCode}`));
                    return;
                }
                const totalSize = parseInt(response.headers['content-length'], 10);
                let downloaded = 0;
                const file = fs.createWriteStream(dest);
                response.on('data', (chunk) => {
                    downloaded += chunk.length;
                    if (totalSize) {
                        const percent = ((downloaded / totalSize) * 100).toFixed(1);
                        const mbDown = (downloaded / 1024 / 1024).toFixed(1);
                        const mbTotal = (totalSize / 1024 / 1024).toFixed(1);
                        process.stdout.write(`\r  📥 ${mbDown} MB / ${mbTotal} MB (${percent}%)   `);
                    }
                });
                response.pipe(file);
                file.on('finish', () => { file.close(); process.stdout.write('\n'); resolve(); });
            }).on('error', (err) => { fs.unlink(dest, () => {}); reject(err); });
        };
        followRedirect(url);
    });
}

function runCommand(command, args = [], options = {}) {
    return new Promise((resolve, reject) => {
        const proc = spawn(command, args, { stdio: 'inherit', shell: true, cwd: scriptDir, ...options });
        proc.on('close', (code) => {
            if (code === 0) resolve();
            else reject(new Error(`Команда завершилась с кодом ошибки ${code}`));
        });
    });
}

function isValidIco(filePath) {
    if (!fs.existsSync(filePath)) return false;
    const buf = fs.readFileSync(filePath);
    return buf.length > 4 && buf[0] === 0 && buf[1] === 0 && buf[2] === 1 && buf[3] === 0;
}

async function downloadAndExtractDeno(binDir, target) {
    const denoExePath = path.join(binDir, `deno-${target}.exe`);
    if (fs.existsSync(denoExePath)) {
        console.log('  ✅ Deno уже загружен.');
        return;
    }

    console.log('Загрузка Deno (v2.1.4)...');
    const zipPath = path.join(binDir, 'deno-download.zip');
    const extractDir = path.join(binDir, 'deno-extract');

    try {
        await downloadFile('https://github.com/denoland/deno/releases/download/v2.1.4/deno-x86_64-pc-windows-msvc.zip', zipPath);

        console.log('  Распаковка Deno...');
        if (fs.existsSync(extractDir)) fs.rmSync(extractDir, { recursive: true, force: true });
        fs.mkdirSync(extractDir, { recursive: true });

        execSync(`powershell -NoProfile -Command "Expand-Archive -Path '${zipPath}' -DestinationPath '${extractDir}' -Force"`, { stdio: 'pipe' });

        const extractedExe = path.join(extractDir, 'deno.exe');
        if (fs.existsSync(extractedExe)) {
            fs.copyFileSync(extractedExe, denoExePath);
            console.log('  ✅ Deno загружен и распакован.');
        } else {
            throw new Error('deno.exe не найден в архиве');
        }
    } catch (e) {
        console.error(`  ⚠️ Ошибка загрузки Deno: ${e.message}`);
        console.error('  Deno-песочница будет недоступна. Скачайте вручную из https://deno.land/');
    } finally {
        try { if (fs.existsSync(zipPath)) fs.unlinkSync(zipPath); } catch (e) {}
        try { if (fs.existsSync(extractDir)) fs.rmSync(extractDir, { recursive: true, force: true }); } catch (e) {}
    }
}

async function main() {
    try {
        const prepOnly = process.argv.includes('--prep-only');

        // 0. Обновление версии (правило YY.M.P)
        console.log('========================================');
        console.log('[1/6] Обновление версии...');
        const confPath = path.join(scriptDir, 'src-tauri', 'tauri.conf.json');
        const cargoPath = path.join(scriptDir, 'src-tauri', 'Cargo.toml');
        let confText = fs.readFileSync(confPath, 'utf8');
        const match = confText.match(/"version"\s*:\s*"(\d+)\.(\d+)\.(\d+)"/);
        if (match) {
            const oldMaj = match[1];
            const oldMin = match[2];
            const oldPat = parseInt(match[3], 10);
            const now = new Date();
            const newMaj = now.getFullYear().toString().slice(-2);
            const newMin = (now.getMonth() + 1).toString();
            const newPat = (oldMaj === newMaj && oldMin === newMin) ? oldPat + 1 : 1;
            const version = `${newMaj}.${newMin}.${newPat}`;
            confText = confText.replace(/"version"\s*:\s*".*?"/, `"version": "${version}"`);
            fs.writeFileSync(confPath, confText, 'utf8');
            let cargoText = fs.readFileSync(cargoPath, 'utf8');
            cargoText = cargoText.replace(/^version\s*=\s*".*?"/m, `version = "${version}"`);
            fs.writeFileSync(cargoPath, cargoText, 'utf8');
            console.log(`Версия обновлена до: ${version}`);
        }

        console.log('\n========================================');
        console.log('[2/6] Установка зависимостей Node.js...');
        await runCommand('npm', ['install', '--legacy-peer-deps']);

        console.log('\n========================================');
        console.log('[3/6] Подготовка сайдкаров...');
        const binDir = path.join(scriptDir, 'src-tauri', 'bin');
        const iconsDir = path.join(scriptDir, 'src-tauri', 'icons');
        if (!fs.existsSync(binDir)) fs.mkdirSync(binDir, { recursive: true });
        if (!fs.existsSync(iconsDir)) fs.mkdirSync(iconsDir, { recursive: true });

        let target = 'x86_64-pc-windows-msvc';
        try {
            const rustcInfo = execSync('rustc -vV', { encoding: 'utf8' });
            const hostMatch = rustcInfo.match(/host:\s*(.*)/);
            if (hostMatch) target = hostMatch[1].trim();
        } catch (e) {}
        console.log(`Target: ${target}`);

        const ytdlpPath = path.join(binDir, `yt-dlp-${target}.exe`);
        if (!fs.existsSync(ytdlpPath)) {
            console.log('Загрузка yt-dlp...');
            await downloadFile('https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe', ytdlpPath);
        } else {
            console.log('  ✅ yt-dlp уже загружен.');
        }

        const nodeExePath = path.join(binDir, `node-${target}.exe`);
        if (!fs.existsSync(nodeExePath)) {
            console.log('Загрузка Node.js...');
            await downloadFile('https://nodejs.org/dist/v20.12.2/win-x64/node.exe', nodeExePath);
        } else {
            console.log('  ✅ Node.js уже загружен.');
        }

        await downloadAndExtractDeno(binDir, target);

        console.log('\n========================================');
        console.log('[4/6] Проверка иконок...');
        const iconPath = path.join(iconsDir, 'icon.ico');
        let needsIconGeneration = false;
        if (!isValidIco(iconPath)) {
            if (fs.existsSync(iconsDir)) {
                fs.readdirSync(iconsDir).forEach(f => fs.unlinkSync(path.join(iconsDir, f)));
            }
            needsIconGeneration = true;
        }
        if (needsIconGeneration) {
            console.log('Генерация иконок Tauri...');
            const appIconPath = path.join(scriptDir, 'app-icon.png');
            if (fs.existsSync(appIconPath)) fs.unlinkSync(appIconPath);
            await downloadFile('https://raw.githubusercontent.com/tauri-apps/tauri/v2/tooling/cli/templates/app/app-icon.png', appIconPath);
            try {
                await runCommand('npx', ['tauri', 'icon', 'app-icon.png']);
                console.log('Иконки сгенерированы.');
            } catch (e) {
                console.warn('⚠️ Автоматическая генерация иконок не удалась.');
            }
        } else {
            console.log('  ✅ icon.ico валиден.');
        }

        if (prepOnly) {
            console.log('\n✅ Prep-only: версия, npm, сайдкары, иконки готовы. Компиляция пропущена.');
            return;
        }

        console.log('\n========================================');
        console.log('[5/6] Сборка приложения (без установщика)...');

        // Переопределяем конфиг Tauri — отключаем бандлер (NSIS), но компиляция идёт штатно
        const overridePath = path.join(scriptDir, 'src-tauri', 'tauri-dev-override.json');
        fs.writeFileSync(overridePath, JSON.stringify({ bundle: { active: false } }));

        let buildOk = false;
        try {
            await runCommand('npx', ['tauri', 'build', '--config', overridePath]);
            buildOk = true;
        } finally {
            try { fs.unlinkSync(overridePath); } catch (e) {}
        }

        if (!buildOk) {
            throw new Error('Сборка Tauri не удалась');
        }

        // Копируем сайдкары рядом с exe
        const releaseDir = path.join(scriptDir, 'src-tauri', 'target', 'release');
        const releaseBinDir = path.join(releaseDir, 'bin');
        if (fs.existsSync(binDir) && fs.existsSync(releaseDir)) {
            if (!fs.existsSync(releaseBinDir)) fs.mkdirSync(releaseBinDir, { recursive: true });
            for (const file of fs.readdirSync(binDir)) {
                const src = path.join(binDir, file);
                const dst = path.join(releaseBinDir, file);
                try { fs.copyFileSync(src, dst); } catch (e) {}
            }
            console.log('  📋 Сайдкары скопированы.');
        }

        const exePath = path.join(releaseDir, 'king_orch.exe');
        if (!fs.existsSync(exePath)) {
            throw new Error('king_orch.exe не найден! Сборка не удалась.');
        }

        const exeStat = fs.statSync(exePath);
        const exeAgeSec = (Date.now() - exeStat.mtimeMs) / 1000;
        if (exeAgeSec > 600) {
            throw new Error(`king_orch.exe устарел (${Math.round(exeAgeSec)} сек назад). Сборка не удалась — это старый экзешник!`);
        }

        console.log('\n========================================');
        console.log('[6/6] Запуск приложения...');
        console.log('🚀 Запуск King Orch (без консоли)...');
        const child = spawn(exePath, [], {
            detached: true,
            stdio: 'ignore',
            windowsHide: true
        });
        child.unref();
        console.log('✅ Приложение запущено! Эта консоль закроется автоматически.');
        setTimeout(() => process.exit(0), 1500);

    } catch (e) {
        console.error('\n========================================');
        console.error('❌ ОШИБКА:', e.message);
        console.error('========================================');
        console.error('Консоль НЕ закроется. Скопируйте ошибки и исправьте.');
        process.exit(1);
    }
}

main();
