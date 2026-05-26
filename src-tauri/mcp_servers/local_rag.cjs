const fs = require('fs');
const path = require('path');
const { createMcpServer } = require('./mcp_base');

let indexedFiles = [];

createMcpServer({
    name: "local-rag-mcp",
    version: "1.0.0",
    tools: [
        {
            name: "index_directory",
            description: "Индексировать все файлы исходного кода в указанной директории для быстрого поиска",
            inputSchema: { type: "object", properties: { path: { type: "string", description: "Путь к директории проекта" } }, required: ["path"] }
        },
        {
            name: "search_code",
            description: "Найти релевантные фрагменты кода в проиндексированном проекте по ключевым словам",
            inputSchema: { type: "object", properties: { query: { type: "string", description: "Поисковый запрос" } }, required: ["query"] }
        }
    ],
    handlers: {
        index_directory: (args) => {
            const targetPath = path.resolve(args.path);
            const count = indexDirectory(targetPath);
            return `Успешно проиндексировано файлов: ${count} в папке ${targetPath}`;
        },
        search_code: (args) => {
            const results = searchCode(args.query);
            if (results.length === 0) return "Ничего не найдено. Сначала вызовите index_directory.";
            return results.map((r, i) => `[Результат ${i+1}] Файл: ${r.path} (Строка ${r.lineStart})\n---\n${r.text}\n---\n`).join('\n');
        }
    }
});

function indexDirectory(dirPath) {
    indexedFiles = [];
    const walk = (dir) => {
        let files;
        try { files = fs.readdirSync(dir); } catch (e) { return; }
        for (const file of files) {
            const fullPath = path.join(dir, file);
            let stat;
            try { stat = fs.statSync(fullPath); } catch (e) { continue; }
            if (stat.isDirectory()) {
                if (['node_modules', '.git', 'target', 'dist', 'build', '.agents_workspace'].includes(file)) continue;
                walk(fullPath);
            } else {
                const ext = path.extname(file).toLowerCase();
                if (['.js', '.ts', '.tsx', '.jsx', '.rs', '.py', '.md', '.txt', '.json', '.html', '.css', '.toml', '.yaml', '.yml'].includes(ext)) {
                    try {
                        const content = fs.readFileSync(fullPath, 'utf8');
                        const lines = content.split('\n');
                        const chunks = [];
                        const chunkSize = 40;
                        const overlap = 10;
                        for (let i = 0; i < lines.length; i += (chunkSize - overlap)) {
                            chunks.push({ text: lines.slice(i, i + chunkSize).join('\n'), lineStart: i + 1 });
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

function searchCode(query) {
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
            if (score > 0) results.push({ path: file.path, lineStart: chunk.lineStart, text: chunk.text, score });
        }
    }
    results.sort((a, b) => b.score - a.score);
    return results.slice(0, 10);
}