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

function copyDirRecursive(src, dest) {
    if (!fs.existsSync(src)) return;
    if (!fs.existsSync(dest)) fs.mkdirSync(dest, { recursive: true });
    for (const entry of fs.readdirSync(src, { withFileTypes: true })) {
        const srcPath = path.join(src, entry.name);
        const destPath = path.join(dest, entry.name);
        if (entry.isDirectory()) {
            copyDirRecursive(srcPath, destPath);
        } else {
            if (!fs.existsSync(destPath) || fs.statSync(srcPath).mtimeMs > fs.statSync(destPath).mtimeMs) {
                fs.copyFileSync(srcPath, destPath);
            }
        }
    }
}

async function main() {
    try {
        console.log('========================================');
        console.log('[1/5] Установка зависимостей Node.js...');
        await runCommand('npm', ['install']);

        console.log('\n========================================');
        console.log('[2/5] Подготовка сайдкаров...');
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

        console.log('\n========================================');
        console.log('[3/5] Проверка иконок...');
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

        console.log('\n========================================');
        console.log('[4/5] Сборка приложения (release)...');
        console.log('Используется RELEASE режим — артефакты кэшируются Cargo.');
        console.log('Повторная сборка без изменений займёт секунды.\n');

        // Подпись (опционально для локального тестирования)
        let privKeyPath = process.env.TAURI_PRIVATE_KEY_ORIGINAL || 'D:\\Projects\\docusaurus-starter\\docs\\Sega Mega Note\\Моя картотека\\software\\настройки\\tauri_signed_keys\\tauri.key';
        let hasKey = fs.existsSync(privKeyPath);

        if (hasKey) {
            console.log('🔑 Ключ подписи найден.');
            let keyContent = fs.readFileSync(privKeyPath, 'utf8').trim();
            const singleLineKey = keyContent.replace(/\r?\n|\r/g, '');
            process.env.TAURI_SIGNING_PRIVATE_KEY = singleLineKey;
            process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '123';
        } else {
            console.log('⚠️ Ключ подписи не найден — сборка без подписи (для локального тестирования это нормально).');
            // Устанавливаем пустой ключ чтобы tauri build не падал
            process.env.TAURI_SIGNING_PRIVATE_KEY = '';
            process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '';
        }
        delete process.env.TAURI_SIGNING_PRIVATE_KEY_PATH;
        delete process.env.TAURI_PRIVATE_KEY;
        delete process.env.TAURI_KEY_PASSWORD;

        try {
            await runCommand('npx', ['tauri', 'build']);
        } catch (e) {
            console.log('\n⚠️ Сборка завершилась с ошибкой (возможно проблема подписи). Проверяю exe...');
        }

        // Копируем сайдкары рядом с exe чтобы они работали при запуске из target/release/
        const releaseDir = path.join(scriptDir, 'src-tauri', 'target', 'release');
        const releaseBinDir = path.join(releaseDir, 'bin');
        if (fs.existsSync(binDir) && fs.existsSync(releaseDir)) {
            if (!fs.existsSync(releaseBinDir)) fs.mkdirSync(releaseBinDir, { recursive: true });
            for (const file of fs.readdirSync(binDir)) {
                const src = path.join(binDir, file);
                const dst = path.join(releaseBinDir, file);
                try {
                    fs.copyFileSync(src, dst);
                } catch (e) {}
            }
            console.log('  📋 Сайдкары скопированы.');
        }

        console.log('\n========================================');
        console.log('[5/5] Запуск приложения...');
        const exePath = path.join(releaseDir, 'king_orch.exe');
        if (fs.existsSync(exePath)) {
            console.log('🚀 Запуск King Orch (без консоли)...');
            const child = spawn(exePath, [], {
                detached: true,
                stdio: 'ignore',
                windowsHide: true
            });
            child.unref();
            console.log('✅ Приложение запущено! Эта консоль закроется автоматически.');
            setTimeout(() => process.exit(0), 1500);
        } else {
            throw new Error('king_orch.exe не найден! Сборка не удалась.');
        }

    } catch (e) {
        console.error('\n[ОШИБКА]', e.message);
        process.exit(1);
    }
}

main();