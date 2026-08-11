// MCP-сервер чтения секций markdown (Deno). Инструмент: ReadSection.
import fs from "node:fs";
import path from "node:path";
import { createMcpServer } from "./mcp_base.ts";

createMcpServer({
    name: "markdown-section-reader-mcp",
    version: "1.0.0",
    tools: [{
        name: "ReadSection",
        description: "Извлечь содержимое секции из markdown-файла по заголовку. Возвращает текст от указанного заголовка до следующего заголовка того же(#) или более высокого(##) уровня, либо до конца файла.",
        inputSchema: {
            type: "object",
            properties: {
                path: { type: "string", description: "Путь к .md файлу (относительный или абсолютный)" },
                heading: { type: "string", description: "Заголовок для поиска, например '## 3' или '## Введение'" }
            },
            required: ["path", "heading"]
        }
    }],
    handlers: {
        ReadSection: (args) => {
            const targetPath = path.resolve(args.path);
            const heading = args.heading;

            if (!fs.existsSync(targetPath)) {
                return `Файл не найден: ${targetPath}`;
            }

            const content = fs.readFileSync(targetPath, 'utf8');
            const escapedHeading = heading.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

            // Найти позицию заголовка в начале строки (без \b — не матчит кириллицу)
            const headingRe = new RegExp(`^${escapedHeading}(?=\\s|$).*$`, 'm');
            const headingMatch = content.match(headingRe);
            if (!headingMatch) {
                return `Секция с заголовком "${heading}" не найдена в файле ${targetPath}`;
            }

            const startIdx = headingMatch.index;
            const afterHeading = startIdx + headingMatch[0].length;

            // Найти следующий заголовок H1 или H2 после этой позиции
            const nextRe = /^#{1,2}[ \t]/m;
            const rest = content.slice(afterHeading);
            const nextMatch = rest.match(nextRe);
            const endIdx = nextMatch ? afterHeading + nextMatch.index : content.length;

            return content.slice(startIdx, endIdx).trim();
        }
    }
});
