// MCP-сервер записи файлов (Deno). Инструмент: Write.
import fs from "node:fs";
import path from "node:path";
import { createMcpServer } from "./mcp_base.ts";

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
