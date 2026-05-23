const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');
const os = require('os');
const readline = require('readline');

const videoCache = {};

const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
    terminal: false
});

rl.on('line', (line) => {
    try {
        const req = JSON.parse(line);
        handleRequest(req);
    } catch (e) {}
});

function sendResponse(id, result) {
    console.log(JSON.stringify({ jsonrpc: "2.0", id: id, result: result }));
}

function handleRequest(req) {
    const { id, method, params } = req;
    if (method === 'initialize') {
        sendResponse(id, {
            protocolVersion: "2024-11-05",
            capabilities: { tools: {} },
            serverInfo: { name: "youtube-mcp", version: "1.0.0" }
        });
    } else if (method === 'tools/list') {
        sendResponse(id, {
            tools: [
                {
                    name: "prepare_video",
                    description: "Скачивает видео, очищает от рекламы и разбивает на части (chunks). Вызови это ПЕРВЫМ.",
                    inputSchema: {
                        type: "object",
                        properties: {
                            url: { type: "string", description: "Ссылка на YouTube видео" },
                            chunk_size: { type: "number", description: "Размер части в символах (рекомендуется 12000)" }
                        },
                        required: ["url"]
                    }
                },
                {
                    name: "get_chunk",
                    description: "Получить конкретную часть текста видео.",
                    inputSchema: {
                        type: "object",
                        properties: {
                            url: { type: "string" },
                            chunk_index: { type: "number", description: "Номер части (начиная с 1)" }
                        },
                        required: ["url", "chunk_index"]
                    }
                }
            ]
        });
    } else if (method === 'tools/call') {
        const { name, arguments: args } = params;
        try {
            if (name === 'prepare_video') {
                const res = prepareVideo(args.url, args.chunk_size || 12000);
                sendResponse(id, { content: [{ type: "text", text: res }] });
            } else if (name === 'get_chunk') {
                const res = getChunk(args.url, args.chunk_index);
                sendResponse(id, { content: [{ type: "text", text: res }] });
            }
        } catch (e) {
            sendResponse(id, { content: [{ type: "text", text: `Ошибка: ${e.message}` }] });
        }
    }
}

function getChunk(url, chunkIndex) {
    if (!videoCache[url]) {
        throw new Error("Видео не найдено в кэше. Сначала вызови prepare_video.");
    }
    const chunks = videoCache[url];
    if (chunkIndex < 1 || chunkIndex > chunks.length) {
        throw new Error(`Неверный индекс. Доступны части от 1 до ${chunks.length}.`);
    }
    return `--- ЧАСТЬ ${chunkIndex} ИЗ ${chunks.length} ---\n` + chunks[chunkIndex - 1];
}

function prepareVideo(url, chunkSize) {
    if (videoCache[url]) {
        return `Видео уже загружено. Всего частей: ${videoCache[url].length}. Теперь вызывай get_chunk для каждой части по очереди.`;
    }

    const match = url.match(/(?:v=|youtu\.be\/|embed\/|shorts\/)([a-zA-Z0-9_-]{11})/);
    const videoId = match ? match[1] : null;
    if (!videoId) throw new Error("Не удалось извлечь ID видео из ссылки.");

    let sponsors = [];
    try {
        const curlCmd = `curl -s "https://sponsor.ajay.app/api/skipSegments?videoID=${videoId}&categories=[\\"sponsor\\",\\"selfpromo\\",\\"interaction\\"]"`;
        const sponsorData = execSync(curlCmd, { encoding: 'utf8' });
        if (sponsorData && !sponsorData.includes("Not Found")) {
            const parsed = JSON.parse(sponsorData);
            sponsors = parsed.map(item => item.segment);
        }
    } catch (e) {}

    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'yt-'));
    const outTmpl = path.join(tmpDir, '%(id)s.%(ext)s');
    
    const isWin = process.platform === 'win32';
    const ytName = isWin ? 'yt-dlp.exe' : 'yt-dlp';
    let ytDlpCmd = 'yt-dlp';
    
    const possiblePaths = [
        path.join(process.cwd(), 'bin', ytName),
        path.join(process.cwd(), ytName),
        path.join(__dirname, '..', '..', 'bin', ytName),
        path.join(__dirname, '..', 'bin', ytName)
    ];
    for (const p of possiblePaths) {
        if (fs.existsSync(p)) { ytDlpCmd = `"${p}"`; break; }
    }

    let cookiesArg = "";
    const cookiesPaths = [
        path.join(process.cwd(), 'cookies'),
        path.join(__dirname, '..', '..', 'cookies')
    ];
    for (const p of cookiesPaths) {
        if (fs.existsSync(p)) {
            const cFiles = fs.readdirSync(p);
            const txtFile = cFiles.find(f => f.endsWith('.txt'));
            if (txtFile) {
                cookiesArg = `--cookies "${path.join(p, txtFile)}"`;
                break;
            }
        }
    }

    const cmd = `${ytDlpCmd} ${cookiesArg} --skip-download --write-auto-subs --write-subs --sub-langs "ru.*,en.*,ru,en" --convert-subs vtt --output "${outTmpl}" "${url}"`;
    try { execSync(cmd, { stdio: 'ignore' }); } catch(e) {}

    const files = fs.readdirSync(tmpDir);
    const vttFile = files.find(f => f.endsWith('.vtt'));
    if (!vttFile) {
        fs.rmSync(tmpDir, { recursive: true, force: true });
        throw new Error("Субтитры не найдены.");
    }

    const vttContent = fs.readFileSync(path.join(tmpDir, vttFile), 'utf8');
    fs.rmSync(tmpDir, { recursive: true, force: true });

    const lines = vttContent.split('\n');
    let cleanText = "";
    let currentStart = 0;
    let lastText = "";

    for (let line of lines) {
        line = line.trim();
        const timeMatch = line.match(/(\d{2}):(\d{2}):(\d{2})\.\d{3}/);
        if (timeMatch) {
            currentStart = parseInt(timeMatch[1])*3600 + parseInt(timeMatch[2])*60 + parseInt(timeMatch[3]);
            continue;
        }
        
        if (line && !line.includes('WEBVTT') && !line.includes('-->') && !line.match(/^\d+$/)) {
            let isSponsor = false;
            for (const [start, end] of sponsors) {
                if (currentStart >= start && currentStart <= end) {
                    isSponsor = true; break;
                }
            }
            if (!isSponsor) {
                const text = line.replace(/<[^>]+>/g, '').trim();
                if (text && text !== lastText) {
                    const m = Math.floor(currentStart / 60);
                    const s = (currentStart % 60).toString().padStart(2, '0');
                    cleanText += `[${m}:${s}] ${text}\n`;
                    lastText = text;
                }
            }
        }
    }

    let chunks = [];
    let currentChunk = "";
    const cleanLines = cleanText.split('\n');
    for (let line of cleanLines) {
        if (currentChunk.length + line.length > chunkSize && currentChunk.length > 0) {
            chunks.push(currentChunk);
            currentChunk = "";
        }
        currentChunk += line + '\n';
    }
    if (currentChunk.trim().length > 0) chunks.push(currentChunk);

    videoCache[url] = chunks;
    return `Успешно. Видео разбито на ${chunks.length} частей. Теперь вызывай get_chunk для каждой части по очереди.`;
}