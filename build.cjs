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
    // Истинный ICO файл всегда начинается с байтов 00 00 01 00
    return buf.length > 4 && buf[0] === 0 && buf[1] === 0 && buf[2] === 1 && buf[3] === 0;
}

// --- Main Build Process ---
async function main() {
    try {
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

            console.log(`Локальная версия обновлена до: ${version}`);
        }

        console.log('\n========================================');
        console.log('[2/6] Установка зависимостей Node.js...');
        await runCommand('npm', ['install']);

        console.log('\n========================================');
        console.log('[3/6] Подготовка директорий...');
        const binDir = path.join(scriptDir, 'src-tauri', 'bin');
        const iconsDir = path.join(scriptDir, 'src-tauri', 'icons');
        if (!fs.existsSync(binDir)) fs.mkdirSync(binDir, { recursive: true });
        if (!fs.existsSync(iconsDir)) fs.mkdirSync(iconsDir, { recursive: true });

        console.log('\n========================================');
        console.log('[4/6] Загрузка сайдкаров...');
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
        console.log('[5/6] Проверка иконок...');
        const iconPath = path.join(iconsDir, 'icon.ico');
        let needsIconGeneration = false;

        if (!isValidIco(iconPath)) {
            if (fs.existsSync(iconPath)) {
                console.log('Обнаружен поврежденный или поддельный .ico файл (возможно, переименованный PNG). Удаление...');
            } else {
                console.log('Файл icon.ico не найден.');
            }
            // Удаляем всю папку иконок, чтобы генератор начал с чистого листа
            if (fs.existsSync(iconsDir)) {
                fs.readdirSync(iconsDir).forEach(f => fs.unlinkSync(path.join(iconsDir, f)));
            }
            needsIconGeneration = true;
        }

        if (needsIconGeneration) {
            console.log('Генерация иконок Tauri...');
            const appIconPath = path.join(scriptDir, 'app-icon.png');
            // На всякий случай удаляем старый исходник иконки, чтобы скачать свежий
            if (fs.existsSync(appIconPath)) fs.unlinkSync(appIconPath);
            
            console.log('Загрузка базового изображения app-icon.png...');
            await downloadFile('https://raw.githubusercontent.com/tauri-apps/tauri/v2/tooling/cli/templates/app/app-icon.png', appIconPath);
            
            try {
                await runCommand('npx', ['tauri', 'icon', 'app-icon.png']);
                console.log('Иконки успешно сгенерированы.');
            } catch (e) {
                console.warn('Предупреждение: автоматическая генерация иконок не удалась. Вам может потребоваться добавить icon.ico вручную.');
            }
        } else {
            console.log('Файл icon.ico валиден.');
        }

        console.log('\n========================================');
        console.log('[6/6] Проверка ключей автообновления и сборка...');

        if (!process.env.TAURI_PRIVATE_KEY) {
            process.env.TAURI_PRIVATE_KEY = 'D:\\Projects\\docusaurus-starter\\docs\\Sega Mega Note\\Моя картотека\\software\\настройки\\tauri_signed_keys\\tauri.key';
        }
        process.env.TAURI_KEY_PASSWORD = '123';

        if (!fs.existsSync(process.env.TAURI_PRIVATE_KEY)) {
            throw new Error(`Приватный ключ не найден по пути:\n${process.env.TAURI_PRIVATE_KEY}`);
        }

        console.log('Сборка приложения Tauri...');
        await runCommand('npx', ['tauri', 'build']);

        console.log('\n========================================');
        console.log('Сборка завершена! Запуск приложения...');
        const exePath = path.join(scriptDir, 'src-tauri', 'target', 'release', 'king_orch.exe');
        if (fs.existsSync(exePath)) {
            spawn(exePath, [], { detached: true, stdio: 'ignore' }).unref();
        } else {
            console.log(`Внимание: Собранный файл не найден по пути ${exePath}`);
        }

    } catch (e) {
        console.error('\n[ОШИБКА]', e.message);
        process.exit(1);
    }
}

main();