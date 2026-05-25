const fs = require('fs');
const path = require('path');
const { createMcpServer } = require('./mcp_base');

createMcpServer({
    name: "fs-write-mcp",
    version: "1.0.0",
    tools: [
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
        }
    ],
    handlers: {
        Write: (args) => {
            const targetPath = path.resolve(args.path);
            fs.mkdirSync(path.dirname(targetPath), { recursive: true });
            fs.writeFileSync(targetPath, args.content, 'utf8');
            return `Успешно: Файл ${args.path} записан.`;
        }
    }
});