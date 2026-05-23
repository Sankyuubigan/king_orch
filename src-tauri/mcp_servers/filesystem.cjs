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

function handleRequest(req) {
    const { id, method, params } = req;
    if (method === 'initialize') {
        sendResponse(id, {
            protocolVersion: "2024-11-05",
            capabilities: { tools: {} },
            serverInfo: { name: "filesystem-mcp", version: "1.0.0" }
        });
    } else if (method === 'tools/list') {
        sendResponse(id, {
            tools:[
                {
                    name: "Read",
                    description: "Прочитать содержимое файла на диске",
                    inputSchema: {
                        type: "object",
                        properties: {
                            path: { type: "string", description: "Абсолютный или относительный путь к файлу" }
                        },
                        required: ["path"]
                    }
                },
                {
                    name: "Write",
                    description: "Записать текст в файл на диске (создаст файл или перезапишет его)",
                    inputSchema: {
                        type: "object",
                        properties: {
                            path: { type: "string", description: "Абсолютный или относительный путь к файлу" },
                            content: { type: "string", description: "Текстовое содержимое для записи" }
                        },
                        required: ["path", "content"]
                    }
                },
                {
                    name: "LS",
                    description: "Посмотреть список файлов и папок в указанной директории",
                    inputSchema: {
                        type: "object",
                        properties: {
                            path: { type: "string", description: "Абсолютный или относительный путь к папке" }
                        },
                        required:["path"]
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
            ]
        });
    } else if (method === 'tools/call') {
        const { name, arguments: args } = params;
        let contentText = "";
        try {
            if (name === 'Read') {
                const targetPath = path.resolve(args.path);
                contentText = fs.readFileSync(targetPath, 'utf8');
            } else if (name === 'Write') {
                const targetPath = path.resolve(args.path);
                fs.mkdirSync(path.dirname(targetPath), { recursive: true });
                fs.writeFileSync(targetPath, args.content, 'utf8');
                contentText = `Успешно: Файл ${args.path} записан.`;
            } else if (name === 'LS') {
                const targetPath = path.resolve(args.path);
                const items = fs.readdirSync(targetPath);
                contentText = items.map(item => {
                    const itemPath = path.join(targetPath, item);
                    const isDir = fs.statSync(itemPath).isDirectory();
                    return `${isDir ? '📁' : '📄'} ${item}`;
                }).join('\n');
            } else if (name === 'Grep') {
                const targetPath = path.resolve(args.path);
                const content = fs.readFileSync(targetPath, 'utf8');
                const lines = content.split('\n');
                const regex = new RegExp(args.pattern, 'g');
                const results =[];
                for (let i = 0; i < lines.length; i++) {
                    if (regex.test(lines[i])) {
                        results.push(`${i + 1}: ${lines[i]}`);
                    }
                }
                contentText = results.length > 0 ? results.join('\n') : "Совпадений не найдено.";
            } else if (name === 'Glob') {
                const targetPath = path.resolve(args.path);
                const results = globDir(targetPath, args.pattern);
                contentText = results.length > 0 ? results.join('\n') : "Файлы не найдены.";
            } else {
                contentText = `Ошибка: Неизвестный инструмент ${name}`;
            }
        } catch (e) {
            contentText = `Ошибка выполнения файловой операции: ${e.message}`;
        }
        sendResponse(id, {
            content:[{ type: "text", text: contentText }]
        });
    }
}

function globDir(dir, pattern) {
    let results =[];
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