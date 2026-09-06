// MCP-сервер чтения секций markdown (Deno). Инструмент: ReadSection.
import fs from "node:fs";
import path from "node:path";
import { createMcpServer } from "./mcp_base.ts";

function readSection(targetPath: string, heading: string): string {
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

/// Нечёткий поиск .md файла: модели часто сокращают имена (например,
/// inner_contour_9_patterns.md → inner_contour.md). Ищем по токенам basename
/// (split по `_` и `-`), ближайший по совпадению токенов — первый.
function fuzzyFindMd(targetPath: string): string[] {
    const wantedBase = path.basename(targetPath).toLowerCase().replace(/\.md$/, "");
    const skipDirs = new Set(["node_modules", "target", "dist", ".git", ".svn"]);
    const found: { score: number; full: string }[] = [];
    const tokenize = (s: string) => s.split(/[_\-]/).filter(t => t.length > 0);
    const wantedTokens = tokenize(wantedBase);

    const walk = (dir: string) => {
        let entries: fs.Dirent[];
        try {
            entries = fs.readdirSync(dir, { withFileTypes: true });
        } catch {
            return;
        }
        for (const e of entries) {
            if (e.isDirectory()) {
                if (!skipDirs.has(e.name)) walk(path.join(dir, e.name));
            } else if (e.isFile() && e.name.toLowerCase().endsWith(".md")) {
                const base = e.name.toLowerCase().replace(/\.md$/, "");
                const fileTokens = tokenize(base);
                const intersection = wantedTokens.filter(t => fileTokens.includes(t));
                const ratio = intersection.length / Math.max(wantedTokens.length, 1);
                if (ratio >= 0.5) {
                    found.push({ score: Math.abs(fileTokens.length - wantedTokens.length) + (1 - ratio) * 10, full: path.join(dir, e.name) });
                }
            }
        }
    };
    walk(process.cwd());
    found.sort((a, b) => a.score - b.score || a.full.length - b.full.length);
    return found.slice(0, 5).map(f => f.full);
}

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
                const candidates = fuzzyFindMd(targetPath);
                if (candidates.length > 0) {
                    console.error(`[ReadSection] Файл не найден: ${targetPath}; использован нечёткий фолбэк: ${candidates[0]}`);
                    return readSection(candidates[0], heading);
                }
                throw new Error(`Файл не найден: ${targetPath}`);
            }

            return readSection(targetPath, heading);
        }
    }
});
