const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

const scriptDir = __dirname;

async function main() {
    try {
        // 1. Проверка авторизации GitHub CLI
        console.log('Проверка авторизации GitHub CLI (gh)...');
        try {
            execSync('gh auth status', { stdio: 'pipe', cwd: scriptDir });
            console.log('Авторизация пройдена!');
        } catch (e) {
            console.error('\n[ОШИБКА] Вы не авторизованы в GitHub CLI!');
            console.error('Пожалуйста, выполните в терминале команду: gh auth login');
            console.error('Выберите GitHub.com -> HTTPS -> Войти через веб-браузер.');
            process.exit(1);
        }

        // 2. Читаем текущую версию
        const confPath = path.join(scriptDir, 'src-tauri', 'tauri.conf.json');
        const confText = fs.readFileSync(confPath, 'utf8');
        const match = confText.match(/"version"\s*:\s*"([^"]+)"/);
        if (!match) throw new Error('Версия не найдена в tauri.conf.json');
        const version = match[1];
        const tag = `v${version}`;
        console.log(`Текущая версия для релиза: ${tag}`);

        // 3. Проверяем наличие файлов
        const nsisDir = path.join(scriptDir, 'src-tauri', 'target', 'release', 'bundle', 'nsis');
        const exeFile = fs.readdirSync(nsisDir).find(f => f.endsWith('-setup.exe'));
        const sigFile = fs.readdirSync(nsisDir).find(f => f.endsWith('-setup.exe.sig'));
        const latestJsonPath = path.join(scriptDir, 'latest.json');

        if (!exeFile) throw new Error(`Файл .exe не найден в ${nsisDir}\nСначала запустите build.bat!`);
        if (!sigFile) throw new Error(`Файл .sig не найден в ${nsisDir}\nСборка не подписана! Запустите build.bat заново.`);
        if (!fs.existsSync(latestJsonPath)) throw new Error(`Файл latest.json не найден в корне.\nЗапустите build.bat заново.`);

        const exePath = path.join(nsisDir, exeFile);
        const sigPath = path.join(nsisDir, sigFile);
        
        console.log(`Найден установщик: ${exeFile}`);
        console.log(`Найдена подпись: ${sigFile}`);
        console.log(`Найден latest.json`);

        // 4. Создаем релиз на GitHub и загружаем файлы
        console.log(`\nСоздание релиза ${tag} на GitHub...`);
        
        const cmd = `gh release create ${tag} "${exePath}" "${sigPath}" "${latestJsonPath}" --title "${tag}" --notes "Автоматический релиз King Orch ${tag}"`;
        
        console.log(`Выполнение: ${cmd}`);
        execSync(cmd, { stdio: 'inherit', cwd: scriptDir });

        console.log('\n========================================');
        console.log(`Релиз ${tag} УСПЕШНО ОПУБЛИКОВАН!`);
        console.log('Пользователи теперь смогут обновиться.');

    } catch (e) {
        console.error('\n[ОШИБКА РЕЛИЗА]', e.message);
        process.exit(1);
    }
}

main();