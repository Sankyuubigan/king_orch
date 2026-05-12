const fs = require('fs');
const path = require('path');
const readline = require('readline');

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
    console.log(JSON.stringify({
        jsonrpc: "2.0",
        id: id,
        result: result
    }));
}

let indexedFiles = []; // { path, chunks: [{ text, lineStart }] }

function handleRequest(req) {
    const { id, method, params } = req;
    if (method === 'initialize') {
        sendResponse(id, {
            protocolVersion: "2024-11-05",
            capabilities: { tools: {} },
            serverInfo: { name: "local-rag-mcp", version: "1.0.0" }
        });
    } else if (method === 'tools/list') {
        sendResponse(id, {
            tools: [
                {
                    name: "index_directory",
                    description: "Индексировать все файлы исходного кода в указанной директории для быстрого поиска",
                    inputSchema: {
                        type: "object",
                        properties: {
                            path: { type: "string", description: "Путь к директории проекта" }
                        },
                        required: ["path"]
                    }
                },
                {
                    name: "search_code",
                    description: "Найти релевантные фрагменты кода в проиндексированном проекте по ключевым словам",
                    inputSchema: {
                        type: "object",
                        properties: {
                            query: { type: "string", description: "Поисковый запрос (ключевые слова, названия функций, классов)" }
                        },
                        required: ["query"]
                    }
                }
            ]
        });
    } else if (method === 'tools/call') {
        const { name, arguments: args } = params;
        if (name === 'index_directory') {
            try {
                const targetPath = path.resolve(args.path);
                const count = indexDirectory(targetPath);
                sendResponse(id, { content: [{ type: "text", text: `Успешно проиндексировано файлов: ${count} в папке ${targetPath}` }] });
            } catch (e) {
                sendResponse(id, { content: [{ type: "text", text: `Ошибка индексирования: ${e.message}` }] });
            }
        } else if (name === 'search_code') {
            try {
                const results = searchCode(args.query);
                let textOutput = results.map((r, i) => `[Результат ${i+1}] Файл: ${r.path} (Строка ${r.lineStart})\n---\n${r.text}\n---\n`).join('\n');
                if (results.length === 0) textOutput = "Ничего не найдено. Пожалуйста, убедитесь, что вы сначала вызвали index_directory.";
                sendResponse(id, { content: [{ type: "text", text: textOutput }] });
            } catch (e) {
                sendResponse(id, { content: [{ type: "text", text: `Ошибка поиска: ${e.message}` }] });
            }
        }
    }
}

function indexDirectory(dirPath) {
    indexedFiles = [];
    const walk = (dir) => {
        let files = [];
        try { files = fs.readdirSync(dir); } catch (e) { return; }
        for (const file of files) {
            const fullPath = path.join(dir, file);
            let stat;
            try { stat = fs.statSync(fullPath); } catch (e) { continue; }
            if (stat.isDirectory()) {
                if (file === 'node_modules' || file === '.git' || file === 'target' || file === 'dist' || file === 'build') continue;
                walk(fullPath);
            } else {
                const ext = path.extname(file).toLowerCase();
                const allowedExts = ['.js', '.ts', '.tsx', '.jsx', '.rs', '.py', '.md', '.txt', '.json', '.html', '.css', '.toml', '.yaml', '.yml', '.cpp', '.h', '.c', '.go', '.sh', '.bat'];
                if (allowedExts.includes(ext)) {
                    try {
                        const content = fs.readFileSync(fullPath, 'utf8');
                        const lines = content.split('\n');
                        const chunks = [];
                        const chunkSize = 40;
                        const overlap = 10;
                        for (let i = 0; i < lines.length; i += (chunkSize - overlap)) {
                            const chunkLines = lines.slice(i, i + chunkSize);
                            chunks.push({
                                text: chunkLines.join('\n'),
                                lineStart: i + 1
                            });
                            if (i + chunkSize >= lines.length) break;
                        }
                        indexedFiles.push({ path: fullPath, chunks });
                    } catch (e) {}
                }
            }
        }
    };
    walk(dirPath);
    return indexedFiles.length;
}

fn searchCode(query) {
    const queryTerms = query.toLowerCase().split(/\s+/).filter(t => t.length > 1);
    if (queryTerms.length === 0) return [];
    const results = [];
    for (const file of indexedFiles) {
        for (const chunk of file.chunks) {
            let score = 0;
            const chunkLower = chunk.text.toLowerCase();
            for (const term of queryTerms) {
                if (chunkLower.includes(term)) {
                    score += 1;
                    const regex = new RegExp('\\b' + term.replace(/[-\/\\^$*+?.()|[\]{}]/g, '\\$&') + '\\b', 'i');
                    if (regex.test(chunk.text)) score += 2;
                }
            }
            if (score > 0) {
                results.push({
                    path: file.path,
                    lineStart: chunk.lineStart,
                    text: chunk.text,
                    score: score
                });
            }
        }
    }
    results.sort((a, b) => b.score - a.score);
    return results.slice(0, 10);
}