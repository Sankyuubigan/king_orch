const fs = require('fs');
const path = require('path');
const { createMcpServer } = require('./mcp_base');

createMcpServer({
    name: "fs-read-mcp",
    version: "1.0.0",
    tools: [
        {
            name: "Read",
            description: "Прочитать содержимое файла на диске",
            inputSchema: {
                type: "object",
                properties: { path: { type: "string", description: "Абсолютный или относительный путь к файлу" } },
                required: ["path"]
            }
        },
        {
            name: "LS",
            description: "Посмотреть список файлов и папок в указанной директории",
            inputSchema: {
                type: "object",
                properties: { path: { type: "string", description: "Абсолютный или относительный путь к папке" } },
                required: ["path"]
            }
        },
        {
            name: "Grep",
            description: "Поиск текста внутри файла по регулярному выражению (pattern)",
            inputSchema: {
                type: "object",
                properties: {
                    path: { type: "string", description: "Путь к файлу" },
                    pattern: { type: "string", description: "Регулярное выражение для поиска" }
                },
                required: ["path", "pattern"]
            }
        },
        {
            name: "Glob",
            description: "Поиск файлов в директории по маске (pattern), например *.ts или *.md",
            inputSchema: {
                type: "object",
                properties: {
                    path: { type: "string", description: "Путь к директории для поиска" },
                    pattern: { type: "string", description: "Маска поиска (например, *.js)" }
                },
                required: ["path", "pattern"]
            }
        }
    ],
    handlers: {
        Read: (args) => {
            return fs.readFileSync(path.resolve(args.path), 'utf8');
        },
        LS: (args) => {
            const targetPath = path.resolve(args.path);
            const items = fs.readdirSync(targetPath);
            return items.map(item => {
                const itemPath = path.join(targetPath, item);
                const isDir = fs.statSync(itemPath).isDirectory();
                return `${isDir ? '📁' : '📄'} ${item}`;
            }).join('\n');
        },
        Grep: (args) => {
            const targetPath = path.resolve(args.path);
            const content = fs.readFileSync(targetPath, 'utf8');
            const lines = content.split('\n');
            const regex = new RegExp(args.pattern, 'g');
            const results = [];
            for (let i = 0; i < lines.length; i++) {
                if (regex.test(lines[i])) {
                    results.push(`${i + 1}: ${lines[i]}`);
                }
            }
            return results.length > 0 ? results.join('\n') : "Совпадений не найдено.";
        },
        Glob: (args) => {
            const targetPath = path.resolve(args.path);
            const results = globDir(targetPath, args.pattern);
            return results.length > 0 ? results.join('\n') : "Файлы не найдены.";
        }
    }
});

function globDir(dir, pattern) {
    let results = [];
    const regexPattern = '^' + pattern.replace(/\./g, '\\.').replace(/\*/g, '.*') + '$';
    const regex = new RegExp(regexPattern, 'i');
    
    try {
        const items = fs.readdirSync(dir);
        for (const item of items) {
            const fullPath = path.join(dir, item);
            if (fs.statSync(fullPath).isDirectory()) {
                results = results.concat(globDir(fullPath, pattern));
            } else if (regex.test(item)) {
                results.push(fullPath);
            }
        }
    } catch (e) {}
    return results;
}