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

async function main() {
    try {
        // 1. Обновление версии
        console.log('========================================');
        console.log('[1/6] Обновление версии...');
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
        console.log('[2/6] Установка зависимостей Node.js...');
        await runCommand('npm', ['install', '--legacy-peer-deps']);

        // 3. Проверка иконок
        console.log('\n========================================');
        console.log('[3/6] Проверка иконок...');
        const iconsDir = path.join(scriptDir, 'src-tauri', 'icons');
        if (!fs.existsSync(iconsDir)) fs.mkdirSync(iconsDir, { recursive: true });

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

        // 4. Сборка с подписью
        console.log('\n========================================');
        console.log('[4/6] Сборка приложения Tauri (release с подписью)...');

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

        // 5. Генерация latest.json и подпись
        console.log('\n========================================');
        console.log('[5/6] Генерация latest.json и подпись...');

        const nsisDir = path.join(scriptDir, 'src-tauri', 'target', 'release', 'bundle', 'nsis');

        if (fs.existsSync(nsisDir)) {
            const exeFiles = fs.readdirSync(nsisDir).filter(f => f.endsWith('-setup.exe')).sort();
            const exeFile = exeFiles[exeFiles.length - 1];
            
            const zipFiles = fs.readdirSync(nsisDir).filter(f => f.endsWith('.zip')).sort();
            const zipFile = zipFiles.length > 0 ? zipFiles[zipFiles.length - 1] : null;

            if (!exeFile) throw new Error('Экзешник не найден!');

            let targetForUpdater = zipFile || exeFile;
            let expectedSig = `${targetForUpdater}.sig`;
            let sigFile = fs.readdirSync(nsisDir).find(f => f === expectedSig);

            if (!sigFile) {
                console.log(`Файл .sig для ${targetForUpdater} не найден. Подписываем вручную...`);
                const tempKeyPath = path.join(scriptDir, 'temp_tauri.key');
                fs.writeFileSync(tempKeyPath, keyContent, 'utf8');
                const targetFullPath = path.join(nsisDir, targetForUpdater);
                try {
                    execSync(`npx tauri signer sign "${targetFullPath}" --private-key-path "${tempKeyPath}" --password "123"`, {
                        stdio: 'inherit', cwd: scriptDir
                    });
                    if (fs.existsSync(path.join(nsisDir, expectedSig))) {
                        sigFile = expectedSig;
                        console.log('✅ .sig создан вручную!');
                    }
                } catch (signError) {
                    console.error('Tauri отказался подписывать файл!');
                } finally {
                    if (fs.existsSync(tempKeyPath)) fs.unlinkSync(tempKeyPath);
                }
            }

            if (targetForUpdater && sigFile) {
                const signature = fs.readFileSync(path.join(nsisDir, sigFile), 'utf8').trim();
                const encodedFileName = encodeURIComponent(targetForUpdater);
                const updateJson = {
                    version: version,
                    notes: `Обновление King Orch до версии ${version}`,
                    pub_date: new Date().toISOString(),
                    platforms: {
                        "windows-x86_64": {
                            signature: signature,
                            url: `https://github.com/Sankyuubigan/king_orch/releases/download/v${version}/${encodedFileName}`
                        }
                    }
                };
                fs.writeFileSync(path.join(scriptDir, 'latest.json'), JSON.stringify(updateJson, null, 2), 'utf8');
                console.log(`latest.json сгенерирован. Файл автообновления: ${targetForUpdater}`);
            }
        }

        const latestJsonPath = path.join(scriptDir, 'latest.json');

        // 6. Публикация на GitHub
        console.log('\n========================================');
        console.log('[6/6] Публикация релиза на GitHub...');

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
        
        const zipFiles2 = fs.readdirSync(nsisDir2).filter(f => f.endsWith('.zip')).sort();
        const zipFile2 = zipFiles2.length > 0 ? zipFiles2[zipFiles2.length - 1] : null;

        let targetForUpdater2 = zipFile2 || exeFile2;
        const sigFile2 = fs.readdirSync(nsisDir2).find(f => f === `${targetForUpdater2}.sig`);

        if (!exeFile2) throw new Error('.exe не найден! Сначала соберите проект.');
        if (!sigFile2) throw new Error('.sig не найден! Сборка не подписана.');
        if (!fs.existsSync(latestJsonPath)) throw new Error('latest.json не найден!');

        const exePathFull = path.join(nsisDir2, exeFile2);
        const zipPathFull = zipFile2 ? path.join(nsisDir2, zipFile2) : "";
        const sigPathFull = path.join(nsisDir2, sigFile2);
        
        let uploadCmdFiles = `"${exePathFull}" "${sigPathFull}" "${latestJsonPath}"`;
        if (zipFile2) {
            uploadCmdFiles += ` "${zipPathFull}"`;
        }

        console.log(`\n📦 Файлы для загрузки:`);
        console.log(`   ${exeFile2}`);
        if (zipFile2) console.log(`   ${zipFile2}`);
        console.log(`   ${sigFile2}`);
        console.log(`   latest.json`);
        console.log(`\n🚀 Загрузка на GitHub (v${version})...`);
        console.log(`   ⏳ Прогресс загрузки ниже:\n`);

        await new Promise((resolve, reject) => {
            const cmd = `gh release create ${tag} ${uploadCmdFiles} --title "${tag}" --notes "Автоматический релиз King Orch ${tag}"`;
            const proc = spawn(cmd, [], { stdio: 'inherit', shell: true, cwd: scriptDir });
            proc.on('close', (code) => {
                if (code === 0) resolve();
                else reject(new Error(`gh release create завершился с кодом ${code}`));
            });
        });

        console.log('\n========================================');
        console.log(`✅ Релиз ${tag} УСПЕШНО ОПУБЛИКОВАН!`);

        // 7. Обновление latest.json с реальными именами ассетов и коммит в main
        console.log('\n========================================');
        console.log('[7/7] Обновление latest.json с реальными URL ассетов...');
        try {
            const apiResponse = execSync(
                `gh api repos/Sankyuubigan/king_orch/releases/tags/${tag} --jq ".assets[] | .name + \":\" + .browser_download_url"`,
                { encoding: 'utf8', cwd: scriptDir }
            ).trim();

            let latestJsonText = fs.readFileSync(latestJsonPath, 'utf8');
            const latestJson = JSON.parse(latestJsonText);

            for (const line of apiResponse.split('\n')) {
                const colonIdx = line.indexOf(':');
                if (colonIdx === -1) continue;
                const assetName = line.substring(0, colonIdx);
                let downloadUrl = line.substring(colonIdx + 1);
                if (!downloadUrl.startsWith('http')) {
                    downloadUrl = 'https://' + downloadUrl.substring(downloadUrl.indexOf('github.com'));
                }

                for (const [platform, info] of Object.entries(latestJson.platforms)) {
                    const localFileName = decodeURIComponent(info.url.split('/').pop());
                    const localBase = localFileName.replace(/\.(exe|sig|zip)$/i, '');
                    const apiBase = assetName.replace(/\.(exe|sig|zip)$/i, '');
                    if (apiBase === localBase || apiBase.replace(/\./g, ' ') === localBase) {
                        info.url = downloadUrl;
                    }
                }
            }

            fs.writeFileSync(latestJsonPath, JSON.stringify(latestJson, null, 2), 'utf8');
            console.log('✅ latest.json обновлён с реальными URL.');

            execSync('git add latest.json', { stdio: 'inherit', cwd: scriptDir });
            execSync('git commit -m "chore(release): fix asset URLs in latest.json for ' + tag + '"', { stdio: 'inherit', cwd: scriptDir });
            execSync('git push origin main', { stdio: 'inherit', cwd: scriptDir });
            console.log('✅ Исправленный latest.json запушен в main.');
        } catch (e) {
            console.error('⚠️ Не удалось обновить URL ассетов:', e.message);
        }
        console.log('Пользователи смогут обновиться.');

    } catch (e) {
        console.error('\n[ОШИБКА]', e.message);
        process.exit(1);
    }
}

main();