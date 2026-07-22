const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');
const os = require('os');
const { createMcpServer } = require('./mcp_base.cjs');

const videoCache = {};

createMcpServer({
    name: "youtube-mcp",
    version: "1.0.0",
    tools: [
        {
            name: "prepare_video",
            description: "Скачивает видео, очищает от рекламы и разбивает на части (chunks). Вызови это ПЕРВЫМ.",
            inputSchema: { type: "object", properties: { url: { type: "string", description: "Ссылка на YouTube видео" }, chunk_size: { type: "number", description: "Размер части в символах (рекомендуется 12000)" } }, required: ["url"] }
        },
        {
            name: "get_chunk",
            description: "Получить конкретную часть текста видео.",
            inputSchema: { type: "object", properties: { url: { type: "string" }, chunk_index: { type: "number", description: "Номер части (начиная с 1)" } }, required: ["url", "chunk_index"] }
        }
    ],
    handlers: {
        prepare_video: (args) => prepareVideo(args.url, args.chunk_size || 12000),
        get_chunk: (args) => getChunk(args.url, args.chunk_index)
    }
});

function getChunk(url, chunkIndex) {
    if (!videoCache[url]) throw new Error("Видео не найдено в кэше. Сначала вызови prepare_video.");
    const chunks = videoCache[url];
    if (chunkIndex < 1 || chunkIndex > chunks.length) throw new Error(`Неверный индекс. Доступны части от 1 до ${chunks.length}.`);
    return `--- ЧАСТЬ ${chunkIndex} ИЗ ${chunks.length} ---\n` + chunks[chunkIndex - 1];
}

function prepareVideo(url, chunkSize) {
    if (videoCache[url]) return `Видео уже загружено. Всего частей: ${videoCache[url].length}. Теперь вызывай get_chunk.`;
    const match = url.match(/(?:v=|youtu\.be\/|embed\/|shorts\/)([a-zA-Z0-9_-]{11})/);
    const videoId = match ? match[1] : null;
    if (!videoId) throw new Error("Не удалось извлечь ID видео.");

    let sponsors = [];
    try {
        const curlCmd = `curl -s "https://sponsor.ajay.app/api/skipSegments?videoID=${videoId}&categories=[\\"sponsor\\",\\"selfpromo\\",\\"interaction\\"]"`;
        const sponsorData = execSync(curlCmd, { encoding: 'utf8' });
        if (sponsorData && !sponsorData.includes("Not Found")) sponsors = JSON.parse(sponsorData).map(item => item.segment);
    } catch (e) {}

    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'yt-'));
    const outTmpl = path.join(tmpDir, '%(id)s.%(ext)s');
    const isWin = process.platform === 'win32'; const ytName = isWin ? 'yt-dlp.exe' : 'yt-dlp'; let ytDlpCmd = 'yt-dlp';
    const searchPaths = [path.join(process.cwd(), 'bin', ytName), path.join(process.cwd(), ytName), path.join(__dirname, '..', '..', 'bin', ytName)]; if (process.env.KING_ORCH_BINS_DIR) searchPaths.push(path.join(process.env.KING_ORCH_BINS_DIR, ytName)); for (const p of searchPaths) { if (fs.existsSync(p)) { ytDlpCmd = `"${p}"`; break; } }

    let cookiesArg = "";
    for (const p of [path.join(process.cwd(), 'cookies'), path.join(__dirname, '..', '..', 'cookies')]) {
        if (fs.existsSync(p)) { const cFiles = fs.readdirSync(p); const txtFile = cFiles.find(f => f.endsWith('.txt')); if (txtFile) { cookiesArg = `--cookies "${path.join(p, txtFile)}"`; break; } }
    }

    try { execSync(`${ytDlpCmd} ${cookiesArg} --skip-download --write-auto-subs --write-subs --sub-langs "ru.*,en.*,ru,en" --convert-subs vtt --output "${outTmpl}" "${url}"`, { stdio: 'ignore' }); } catch(e) {}
    const files = fs.readdirSync(tmpDir); const vttFile = files.find(f => f.endsWith('.vtt'));
    if (!vttFile) { fs.rmSync(tmpDir, { recursive: true, force: true }); throw new Error("Субтитры не найдены."); }

    const vttContent = fs.readFileSync(path.join(tmpDir, vttFile), 'utf8');
    fs.rmSync(tmpDir, { recursive: true, force: true });

    const lines = vttContent.split('\n'); let cleanText = ""; let currentStart = 0; let lastText = "";
    for (let line of lines) {
        line = line.trim(); const timeMatch = line.match(/(\d{2}):(\d{2}):(\d{2})\.\d{3}/);
        if (timeMatch) { currentStart = parseInt(timeMatch[1])*3600 + parseInt(timeMatch[2])*60 + parseInt(timeMatch[3]); continue; }
        if (line && !line.includes('WEBVTT') && !line.includes('-->') && !line.match(/^\d+$/)) {
            let isSponsor = false; for (const [start, end] of sponsors) { if (currentStart >= start && currentStart <= end) { isSponsor = true; break; } }
            if (!isSponsor) { const text = line.replace(/<[^>]+>/g, '').trim(); if (text && text !== lastText) { const m = Math.floor(currentStart / 60); const s = (currentStart % 60).toString().padStart(2, '0'); cleanText += `[${m}:${s}] ${text}\n`; lastText = text; } }
        }
    }

    let chunks = []; let currentChunk = "";
    for (let line of cleanText.split('\n')) { if (currentChunk.length + line.length > chunkSize && currentChunk.length > 0) { chunks.push(currentChunk); currentChunk = ""; } currentChunk += line + '\n'; }
    if (currentChunk.trim().length > 0) chunks.push(currentChunk);
    videoCache[url] = chunks;
    return `Успешно. Видео разбито на ${chunks.length} частей. Вызывай get_chunk.`;
}