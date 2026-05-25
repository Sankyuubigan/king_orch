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
            client.get(currentUrl, { headers: { 'User-Agent': 'KingOrch-Release-Script' } }, (response) => {
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

function formatBytes(bytes) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / 1024 / 1024).toFixed(1) + ' MB';
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
        throw new Error('Deno необходим для релизной сборки (песочница).');
    } finally {
        try { if (fs.existsSync(zipPath)) fs.unlinkSync(zipPath); } catch (e) {}
        try { if (fs.existsSync(extractDir)) fs.rmSync(extractDir, { recursive: true, force: true }); } catch (e) {}
    }
}

async function main() {
    try {
        // 1. Обновление версии
        console.log('========================================');
        console.log('[1/8] Обновление версии...');
        const confPath = path.join(scriptDir, 'src-tauri', 'tauri.conf.json');
        const cargoPath = path.join(scriptDir, 'src-tauri', 'Cargo.toml');

        let confText = fs.readFileSync(confPath, 'utf8');
        const match = confText.match(/"version"\s*:\s*"(\d+)\.(\d+)\.(\d+)"/);
        let version = "0.0.0";

        if (match) {
            const oldMaj = match[1];
            const oldMin = match[2];
            const oldPat = parseInt(match[3], 10);
            const now = new Date();
            const newMaj = now.getFullYear().toString().slice(-2);
            const newMin = (now.getMonth() + 1).toString();
            const newPat = (oldMaj === newMaj && oldMin === newMin) ? oldPat + 1 : 1;
            version = `${newMaj}.${newMin}.${newPat}`;

            confText = confText.replace(/"version"\s*:\s*".*?"/, `"version": "${version}"`);
            fs.writeFileSync(confPath, confText, 'utf8');

            let cargoText = fs.readFileSync(cargoPath, 'utf8');
            cargoText = cargoText.replace(/^version\s*=\s*".*?"/m, `version = "${version}"`);
            fs.writeFileSync(cargoPath, cargoText, 'utf8');
            console.log(`Версия обновлена до: ${version}`);
        }

        // 2. npm install
        console.log('\n========================================');
        console.log('[2/8] Установка зависимостей Node.js...');
        await runCommand('npm', ['install', '--legacy-peer-deps']);

        // 3. Подготовка директорий
        console.log('\n========================================');
        console.log('[3/8] Подготовка директорий...');
        const binDir = path.join(scriptDir, 'src-tauri', 'bin');
        const iconsDir = path.join(scriptDir, 'src-tauri', 'icons');
        if (!fs.existsSync(binDir)) fs.mkdirSync(binDir, { recursive: true });
        if (!fs.existsSync(iconsDir)) fs.mkdirSync(iconsDir, { recursive: true });

        // 4. Загрузка сайдкаров
        console.log('\n========================================');
        console.log('[4/8] Загрузка сайдкаров...');
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

        // === Загрузка Deno для песочницы ===
        await downloadAndExtractDeno(binDir, target);

        // 5. Проверка иконок
        console.log('\n========================================');
        console.log('[5/8] Проверка иконок...');
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
            } catch (e) {
                console.warn('⚠️ Генерация иконок не удалась.');
            }
        } else {
            console.log('  ✅ icon.ico валиден.');
        }

        // 6. Сборка с подписью
        console.log('\n========================================');
        console.log('[6/8] Сборка приложения Tauri (release с подписью)...');

        let privKeyPath = process.env.TAURI_PRIVATE_KEY_ORIGINAL || 'D:\\Projects\\docusaurus-starter\\docs\\Sega Mega Note\\Моя картотека\\software\\настройки\\tauri_signed_keys\\tauri.key';

        if (!fs.existsSync(privKeyPath)) {
            throw new Error(`Приватный ключ НЕ НАЙДЕН:\n${privKeyPath}\n\nБез ключа нельзя создать подписанный релиз.`);
        }

        console.log('🔑 Приватный ключ найден.');
        let keyContent = fs.readFileSync(privKeyPath, 'utf8').trim();
        const singleLineKey = keyContent.replace(/\r?\n|\r/g, '');

        process.env.TAURI_SIGNING_PRIVATE_KEY = singleLineKey;
        process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '123';
        delete process.env.TAURI_SIGNING_PRIVATE_KEY_PATH;
        delete process.env.TAURI_PRIVATE_KEY;
        delete process.env.TAURI_KEY_PASSWORD;

        await runCommand('npx', ['tauri', 'build']);

        // 7. Генерация latest.json и подпись
        console.log('\n========================================');
        console.log('[7/8] Генерация latest.json и подпись...');

        const nsisDir = path.join(scriptDir, 'src-tauri', 'target', 'release', 'bundle', 'nsis');

        if (fs.existsSync(nsisDir)) {
            const exeFiles = fs.readdirSync(nsisDir).filter(f => f.endsWith('-setup.exe')).sort();
            const exeFile = exeFiles[exeFiles.length - 1];
            const expectedSig = `${exeFile}.sig`;
            let sigFile = fs.readdirSync(nsisDir).find(f => f === expectedSig);

            if (exeFile && !sigFile) {
                console.log('Файл .sig не создался автоматически. Подписываем вручную...');
                const tempKeyPath = path.join(scriptDir, 'temp_tauri.key');
                fs.writeFileSync(tempKeyPath, keyContent, 'utf8');
                const exeFullPath = path.join(nsisDir, exeFile);
                try {
                    execSync(`npx tauri signer sign "${exeFullPath}" --private-key-path "${tempKeyPath}" --password "123"`, {
                        stdio: 'inherit', cwd: scriptDir
                    });
                    if (fs.existsSync(path.join(nsisDir, expectedSig))) {
                        sigFile = expectedSig;
                        console.log('✅ .sig создан вручную!');
                    }
                } catch (signError) {
                    console.error('Tauri отказался подписывать файл!');
                    console.error('Сгенерируйте новую пару: npx tauri signer generate -w ./tauri_new.key');
                } finally {
                    if (fs.existsSync(tempKeyPath)) fs.unlinkSync(tempKeyPath);
                }
            }

            if (exeFile && sigFile) {
                const signature = fs.readFileSync(path.join(nsisDir, sigFile), 'utf8').trim();
                const encodedExeName = encodeURIComponent(exeFile);
                const updateJson = {
                    version: version,
                    notes: `Обновление King Orch до версии ${version}`,
                    pub_date: new Date().toISOString(),
                    platforms: {
                        "windows-x86_64": {
                            signature: signature,
                            url: `https://github.com/Sankyuubigan/king_orch/releases/download/v${version}/${encodedExeName}`
                        }
                    }
                };
                fs.writeFileSync(path.join(scriptDir, 'latest.json'), JSON.stringify(updateJson, null, 2), 'utf8');
                console.log(`latest.json сгенерирован. Установщик: ${exeFile}`);
            }
        }

        // 8. Публикация на GitHub
        console.log('\n========================================');
        console.log('[8/8] Публикация релиза на GitHub...');

        console.log('Проверка авторизации GitHub CLI...');
        try {
            execSync('gh auth status', { stdio: 'pipe', cwd: scriptDir });
            console.log('✅ Авторизация пройдена!');
        } catch (e) {
            console.error('\n[ОШИБКА] Вы не авторизованы в GitHub CLI!');
            console.error('Выполните: gh auth login');
            process.exit(1);
        }

        const tag = `v${version}`;
        const nsisDir2 = path.join(scriptDir, 'src-tauri', 'target', 'release', 'bundle', 'nsis');
        const exeFiles2 = fs.readdirSync(nsisDir2).filter(f => f.endsWith('-setup.exe')).sort();
        const exeFile2 = exeFiles2[exeFiles2.length - 1];
        const sigFile2 = fs.readdirSync(nsisDir2).find(f => f === `${exeFile2}.sig`);
        const latestJsonPath = path.join(scriptDir, 'latest.json');

        if (!exeFile2) throw new Error('.exe не найден! Сначала соберите проект.');
        if (!sigFile2) throw new Error('.sig не найден! Сборка не подписана.');
        if (!fs.existsSync(latestJsonPath)) throw new Error('latest.json не найден!');

        const exePathFull = path.join(nsisDir2, exeFile2);
        const sigPathFull = path.join(nsisDir2, sigFile2);
        const exeSize = fs.statSync(exePathFull).size;

        console.log(`\n📦 Файлы для загрузки:`);
        console.log(`   ${exeFile2} (${formatBytes(exeSize)})`);
        console.log(`   ${sigFile2}`);
        console.log(`   latest.json`);
        console.log(`\n🚀 Загрузка на GitHub (v${version})...`);
        console.log(`   ⏳ Прогресс загрузки ниже:\n`);

        await new Promise((resolve, reject) => {
            const cmd = `gh release create ${tag} "${exePathFull}" "${sigPathFull}" "${latestJsonPath}" --title "${tag}" --notes "Автоматический релиз King Orch ${tag}"`;
            const proc = spawn(cmd, [], { stdio: 'inherit', shell: true, cwd: scriptDir });
            proc.on('close', (code) => {
                if (code === 0) resolve();
                else reject(new Error(`gh release create завершился с кодом ${code}`));
            });
        });

        console.log('\n========================================');
        console.log(`✅ Релиз ${tag} УСПЕШНО ОПУБЛИКОВАН!`);
        console.log('Пользователи смогут обновиться.');

    } catch (e) {
        console.error('\n[ОШИБКА]', e.message);
        process.exit(1);
    }
}

main();