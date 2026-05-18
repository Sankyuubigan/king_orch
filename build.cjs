const fs = require('fs');
const path = require('path');
const { execSync, spawn } = require('child_process');
const https = require('https');
const http = require('http');

const scriptDir = __dirname;

// --- Helper Functions ---
function downloadFile(url, dest) {
    return new Promise((resolve, reject) => {
        const followRedirect = (currentUrl) => {
            const client = currentUrl.startsWith('https') ? https : http;
            client.get(currentUrl, { headers: { 'User-Agent': 'KingOrch-Build-Script' } }, (response) => {
                if (response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
                    followRedirect(response.headers.location);
                    return;
                }
                if (response.statusCode !== 200) {
                    reject(new Error(`Failed to download: ${response.statusCode}`));
                    return;
                }
                const file = fs.createWriteStream(dest);
                response.pipe(file);
                file.on('finish', () => { file.close(resolve); });
            }).on('error', (err) => { 
                fs.unlink(dest, () => {}); 
                reject(err); 
            });
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

// --- Main Build Process ---
async function main() {
    try {
        console.log('========================================');
        console.log('[1/7] Обновление версии...');
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

            console.log(`Локальная версия обновлена до: ${version}`);
        }

        console.log('\n========================================');
        console.log('[2/7] Установка зависимостей Node.js...');
        await runCommand('npm', ['install']);

        console.log('\n========================================');
        console.log('[3/7] Подготовка директорий...');
        const binDir = path.join(scriptDir, 'src-tauri', 'bin');
        const iconsDir = path.join(scriptDir, 'src-tauri', 'icons');
        if (!fs.existsSync(binDir)) fs.mkdirSync(binDir, { recursive: true });
        if (!fs.existsSync(iconsDir)) fs.mkdirSync(iconsDir, { recursive: true });

        console.log('\n========================================');
        console.log('[4/7] Загрузка сайдкаров...');
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
        }

        const nodeExePath = path.join(binDir, `node-${target}.exe`);
        if (!fs.existsSync(nodeExePath)) {
            console.log('Загрузка Node.js...');
            await downloadFile('https://nodejs.org/dist/v20.12.2/win-x64/node.exe', nodeExePath);
        }

        console.log('\n========================================');
        console.log('[5/7] Проверка иконок...');
        const iconPath = path.join(iconsDir, 'icon.ico');
        let needsIconGeneration = false;

        if (!isValidIco(iconPath)) {
            if (fs.existsSync(iconPath)) {
                console.log('Обнаружен поврежденный или поддельный .ico файл. Удаление...');
            } else {
                console.log('Файл icon.ico не найден.');
            }
            if (fs.existsSync(iconsDir)) {
                fs.readdirSync(iconsDir).forEach(f => fs.unlinkSync(path.join(iconsDir, f)));
            }
            needsIconGeneration = true;
        }

        if (needsIconGeneration) {
            console.log('Генерация иконок Tauri...');
            const appIconPath = path.join(scriptDir, 'app-icon.png');
            if (fs.existsSync(appIconPath)) fs.unlinkSync(appIconPath);
            console.log('Загрузка базового изображения app-icon.png...');
            await downloadFile('https://raw.githubusercontent.com/tauri-apps/tauri/v2/tooling/cli/templates/app/app-icon.png', appIconPath);
            try {
                await runCommand('npx', ['tauri', 'icon', 'app-icon.png']);
                console.log('Иконки успешно сгенерированы.');
            } catch (e) {
                console.warn('Предупреждение: автоматическая генерация иконок не удалась.');
            }
        } else {
            console.log('Файл icon.ico валиден.');
        }

        console.log('\n========================================');
        console.log('[6/7] Проверка ключей автообновления и сборка...');

        let privKeyPath = process.env.TAURI_PRIVATE_KEY_ORIGINAL || 'D:\\Projects\\docusaurus-starter\\docs\\Sega Mega Note\\Моя картотека\\software\\настройки\\tauri_signed_keys\\tauri.key';
        
        if (!fs.existsSync(privKeyPath)) {
            throw new Error(`[КРИТИЧЕСКАЯ ОШИБКА] Приватный ключ НЕ НАЙДЕН по пути:\n${privKeyPath}`);
        }
        
        console.log('Приватный ключ найден. Чтение...');
        let keyContent = fs.readFileSync(privKeyPath, 'utf8').trim();
        
        // ФИКС WINDOWS: Windows обрезает многострочные переменные окружения.
        // Объединяем ключ в одну строку без переносов. Base64 декодер Tauri это переварит!
        const singleLineKey = keyContent.replace(/\r?\n|\r/g, '');
        
        process.env.TAURI_SIGNING_PRIVATE_KEY = singleLineKey;
        process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '123';
        delete process.env.TAURI_SIGNING_PRIVATE_KEY_PATH;
        delete process.env.TAURI_PRIVATE_KEY;
        delete process.env.TAURI_KEY_PASSWORD;

        console.log('Сборка приложения Tauri...');
        await runCommand('npx', ['tauri', 'build']);

        console.log('\n========================================');
        console.log('[7/7] Генерация latest.json и подпись...');
        
        const nsisDir = path.join(scriptDir, 'src-tauri', 'target', 'release', 'bundle', 'nsis');
        
        if (fs.existsSync(nsisDir)) {
            const exeFiles = fs.readdirSync(nsisDir).filter(f => f.endsWith('-setup.exe')).sort();
            const exeFile = exeFiles[exeFiles.length - 1]; 
            
            const expectedSig = `${exeFile}.sig`;
            let sigFile = fs.readdirSync(nsisDir).find(f => f === expectedSig);

            if (exeFile && !sigFile) {
                console.log(`\n[ДИАГНОСТИКА] Файл .sig не создался автоматически. Пробую подписать вручную...`);
                const tempKeyPath = path.join(scriptDir, 'temp_tauri.key');
                fs.writeFileSync(tempKeyPath, keyContent, 'utf8');

                const exeFullPath = path.join(nsisDir, exeFile);
                try {
                    // Вызов напрямую через флаги CLI, игнорируя переменные окружения
                    execSync(`npx tauri signer sign "${exeFullPath}" --private-key-path "${tempKeyPath}" --password "123"`, {
                        stdio: 'inherit',
                        cwd: scriptDir
                    });
                    
                    if (fs.existsSync(path.join(nsisDir, expectedSig))) {
                        sigFile = expectedSig;
                        console.log('[УСПЕХ] Файл .sig успешно создан вручную!');
                    }
                } catch (signError) {
                    console.error('\n=====================================================================');
                    console.error('[ВЫЯСНЕНА ПРИЧИНА] Tauri отказался подписывать файл!');
                    console.error('Если ошибка "Invalid password", значит пароль от ключа НЕ "123".');
                    console.error('Решение: сгенерировать новую пару ключей командой:');
                    console.error('npx tauri signer generate -w ./tauri_new.key');
                    console.error('И обновить публичный ключ в src-tauri/tauri.conf.json.');
                    console.error('=====================================================================');
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
                
                fs.writeFileSync(
                    path.join(scriptDir, 'latest.json'), 
                    JSON.stringify(updateJson, null, 2), 
                    'utf8'
                );
                console.log('\nФайл latest.json УСПЕШНО СГЕНЕРИРОВАН в корне проекта!');
                console.log(`Найден установщик: ${exeFile}`);
                console.log(`Найдена подпись: ${sigFile}`);
            } else if (exeFile) {
                console.error('\n[ВНИМАНИЕ] Не удалось получить подпись .sig. Автообновление НЕ БУДЕТ РАБОТАТЬ.');
            }
        } else {
            console.warn('Папка nsis не найдена, сборка могла завершиться с ошибкой.');
        }

        console.log('\n========================================');
        console.log('Сборка завершена! Запуск приложения...');
        const exePath = path.join(scriptDir, 'src-tauri', 'target', 'release', 'king_orch.exe');
        if (fs.existsSync(exePath)) {
            spawn(exePath, [], { detached: true, stdio: 'ignore' }).unref();
        }

    } catch (e) {
        console.error('\n[ОШИБКА]', e.message);
        process.exit(1);
    }
}

main();