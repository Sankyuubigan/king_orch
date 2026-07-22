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
                    reject(new Error(`Failed to download: ${response.statusCode}`));
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
                        process.stdout.write(`\r  ${mbDown} MB / ${mbTotal} MB (${percent}%)   `);
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
            else reject(new Error(`Command failed with code ${code}`));
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
        // 1. Version bump
        console.log('========================================');
        console.log('[1/5] Version bump...');
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
            console.log(`Version: ${version}`);
        }

        // 2. npm install
        console.log('\n========================================');
        console.log('[2/5] npm install...');
        await runCommand('npm', ['install', '--legacy-peer-deps']);

        // 3. Check icons
        console.log('\n========================================');
        console.log('[3/5] Check icons...');
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
            console.log('Generating icons...');
            const appIconPath = path.join(scriptDir, 'app-icon.png');
            if (fs.existsSync(appIconPath)) fs.unlinkSync(appIconPath);
            await downloadFile('https://raw.githubusercontent.com/tauri-apps/tauri/v2/tooling/cli/templates/app/app-icon.png', appIconPath);
            try {
                await runCommand('npx', ['tauri', 'icon', 'app-icon.png']);
            } catch (e) {
                console.warn('Icon generation failed.');
            }
        } else {
            console.log('  icon.ico OK.');
        }

        // 4. Build with signing
        console.log('\n========================================');
        console.log('[4/5] Tauri build (release with signing)...');

        let privKeyPath = process.env.TAURI_PRIVATE_KEY_ORIGINAL || 'D:\\Projects\\docusaurus-starter\\docs\\Sega Mega Note\\Моя картотека\\software\\настройки\\tauri_signed_keys\\tauri.key';

        if (!fs.existsSync(privKeyPath)) {
            throw new Error(`Private key NOT FOUND:\n${privKeyPath}`);
        }

        console.log('Private key found.');
        let keyContent = fs.readFileSync(privKeyPath, 'utf8').trim();
        const singleLineKey = keyContent.replace(/\r?\n|\r/g, '');

        process.env.TAURI_SIGNING_PRIVATE_KEY = singleLineKey;
        process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '123';
        delete process.env.TAURI_SIGNING_PRIVATE_KEY_PATH;
        delete process.env.TAURI_PRIVATE_KEY;
        delete process.env.TAURI_KEY_PASSWORD;

        await runCommand('npx', ['tauri', 'build']);

        // 5. Sign installer if needed
        console.log('\n========================================');
        console.log('[5/5] Sign installer...');

        const nsisDir = path.join(scriptDir, 'src-tauri', 'target', 'release', 'bundle', 'nsis');

        if (!fs.existsSync(nsisDir)) throw new Error('NSIS directory not found!');

        const exeFiles = fs.readdirSync(nsisDir).filter(f => f.endsWith('-setup.exe')).sort();
        const exeFile = exeFiles[exeFiles.length - 1];
        if (!exeFile) throw new Error('Setup.exe not found!');

        const sigFile = `${exeFile}.sig`;
        if (!fs.readdirSync(nsisDir).find(f => f === sigFile)) {
            console.log(`Signing ${exeFile}...`);
            const tempKeyPath = path.join(scriptDir, 'temp_tauri.key');
            fs.writeFileSync(tempKeyPath, keyContent, 'utf8');
            try {
                execSync(`npx tauri signer sign "${path.join(nsisDir, exeFile)}" --private-key-path "${tempKeyPath}" --password "123"`, {
                    stdio: 'inherit', cwd: scriptDir
                });
                console.log('Signed!');
            } catch (signError) {
                console.error('Signing failed!');
            } finally {
                if (fs.existsSync(tempKeyPath)) fs.unlinkSync(tempKeyPath);
            }
        }

        // 6. Publish release on GitHub (exe + sig only, NO latest.json)
        console.log('\n========================================');
        console.log('Publishing release on GitHub...');

        console.log('Checking GitHub CLI auth...');
        try {
            execSync('gh auth status', { stdio: 'pipe', cwd: scriptDir });
            console.log('Auth OK!');
        } catch (e) {
            console.error('\n[ERROR] Not authenticated in GitHub CLI!');
            console.error('Run: gh auth login');
            process.exit(1);
        }

        const tag = `v${version}`;
        const exePathFull = path.join(nsisDir, exeFile);
        const sigPathFull = path.join(nsisDir, sigFile);

        if (!fs.existsSync(exePathFull)) throw new Error('exe not found!');
        if (!fs.existsSync(sigPathFull)) throw new Error('.sig not found!');

        console.log(`\nFiles to upload:`);
        console.log(`   ${exeFile}`);
        console.log(`   ${sigFile}`);
        console.log(`\nUploading to GitHub (${tag})...\n`);

        await new Promise((resolve, reject) => {
            const cmd = `gh release create ${tag} "${exePathFull}" "${sigPathFull}" --title "${tag}" --notes "Auto release King Orch ${tag}"`;
            const proc = spawn(cmd, [], { stdio: 'inherit', shell: true, cwd: scriptDir });
            proc.on('close', (code) => {
                if (code === 0) resolve();
                else reject(new Error(`gh release create failed with code ${code}`));
            });
        });

        console.log(`\nRelease ${tag} PUBLISHED!`);

        // 7. Generate latest.json from GitHub API (correct asset names) and push to main
        console.log('\n========================================');
        console.log('Generating latest.json from GitHub API...');

        try {
            const apiResponse = execSync(
                `gh api repos/Sankyuubigan/king_orch/releases/tags/${tag} --jq ".assets[] | .name + \":\" + .browser_download_url"`,
                { encoding: 'utf8', cwd: scriptDir }
            ).trim();

            const sigContent = fs.readFileSync(sigPathFull, 'utf8').trim();

            const latestJson = {
                version: version,
                notes: `King Orch ${version}`,
                pub_date: new Date().toISOString(),
                platforms: {
                    "windows-x86_64": {
                        signature: sigContent,
                        url: ""
                    }
                }
            };

            for (const line of apiResponse.split('\n')) {
                const colonIdx = line.indexOf(':');
                if (colonIdx === -1) continue;
                const assetName = line.substring(0, colonIdx);
                let downloadUrl = line.substring(colonIdx + 1);
                if (!downloadUrl.startsWith('http')) {
                    downloadUrl = 'https://' + downloadUrl.substring(downloadUrl.indexOf('github.com'));
                }
                if (assetName.endsWith('-setup.exe') && !assetName.endsWith('.sig')) {
                    latestJson.platforms["windows-x86_64"].url = downloadUrl;
                }
            }

            if (!latestJson.platforms["windows-x86_64"].url) {
                throw new Error('Could not find installer URL from GitHub API');
            }

            const latestJsonPath = path.join(scriptDir, 'latest.json');
            fs.writeFileSync(latestJsonPath, JSON.stringify(latestJson, null, 2), 'utf8');
            console.log(`latest.json generated. URL: ${latestJson.platforms["windows-x86_64"].url}`);

            execSync('git add latest.json', { stdio: 'inherit', cwd: scriptDir });
            execSync('git commit -m "chore(release): update latest.json for ' + tag + '"', { stdio: 'inherit', cwd: scriptDir });
            execSync('git push origin main', { stdio: 'inherit', cwd: scriptDir });
            console.log('latest.json pushed to main.');
        } catch (e) {
            console.error('Failed to generate/push latest.json:', e.message);
        }

        console.log('\n========================================');
        console.log(`DONE! Release ${tag} complete.`);

    } catch (e) {
        console.error('\n[ERROR]', e.message);
        process.exit(1);
    }
}

main();
