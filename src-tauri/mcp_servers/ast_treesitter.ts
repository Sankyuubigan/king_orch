// AST Map MCP v3.0 (Deno). Генерирует карту кода (дерево файлов с токенами + AST функций).
// tree-sitter и @huggingface/transformers подключаются через npm: в try/catch —
// при недоступности автоматически включается фоллбэк (регекс-парсеры, оценка токенов длина/3.7).
import fs from "node:fs";
import path from "node:path";

function log(msg: string) { console.error(`[AST-SERVER] ${msg}`); }
log(`=== AST Map MCP v3.0 ===`);

// ============================================
// @huggingface/transformers — точный подсчёт токенов
// ============================================
// У библиотеки нет встроенного универсального токенизатора — всегда нужен tokenizer.json
// от конкретной модели. После первого скачивания он кэшируется локально.
// Дефолт: Xenova/Meta-Llama-3-8B-Instruct (128k словарь, хорошо подходит для большинства моделей)
let hfTokenizer: unknown = null;
let hfInitPromise: Promise<unknown> | null = null;
let hfModelName = 'Xenova/Meta-Llama-3-8B-Instruct';
let hfAvailable = false;

async function initTokenizer(modelName?: string): Promise<unknown> {
    if (hfTokenizer) return hfTokenizer;
    if (hfInitPromise) return hfInitPromise;

    const target = modelName || hfModelName;
    hfInitPromise = (async () => {
        try {
            const hfModule = await import('npm:@huggingface/transformers');
            const AutoTokenizer = (hfModule as Record<string, unknown>).AutoTokenizer as {
                from_pretrained: (name: string, opts: { progress_callback: (p: Record<string, unknown>) => void }) => Promise<unknown>;
            };
            log(`⏳ Загрузка токенизатора ${target}...`);
            hfTokenizer = await AutoTokenizer.from_pretrained(target, {
                progress_callback: (progress: Record<string, unknown>) => {
                    if (progress.status === 'progress') {
                        const pct = progress.progress;
                        log(`📥 ${progress.file}: ${typeof pct === 'number' ? pct.toFixed(0) : '?'}%`);
                    } else if (progress.status === 'done') {
                        log(`✅ ${progress.file} загружен`);
                    }
                }
            });
            hfAvailable = true;
            log(`✅ @huggingface/transformers загружен (${target}) — точный подсчёт токенов`);
            return hfTokenizer;
        } catch (e) {
            log(`⚠️ @huggingface/transformers недоступен: ${(e as Error).message.split('\n')[0]}`);
            log(`⚠️ Фоллбэк: длина/3.7 (неточный, занижает русский)`);
            return null;
        }
    })();

    return hfInitPromise;
}

async function countTokens(text: string): Promise<number> {
    if (!text || !text.trim()) return 0;
    const tokenizer = await initTokenizer();
    if (tokenizer) {
        try {
            const tokens = (tokenizer as { encode: (t: string, o: { add_special_tokens: boolean }) => unknown[] }).encode(text, { add_special_tokens: false });
            return tokens.length;
        } catch (e) {
            log(`⚠️ HF encode error: ${(e as Error).message}`);
        }
    }
    return Math.ceil(text.length / 3.7);
}

// ============================================
// TREE-SITTER (npm: — фоллбэк на регекс при недоступности)
// ============================================
let Parser: unknown = null;
let useFallback = false;
try {
    const tsMod = await import('npm:tree-sitter');
    Parser = (tsMod as Record<string, unknown>).default || tsMod;
    log(`✅ tree-sitter`);
} catch { useFallback = true; log(`⚠️ Фоллбэк (tree-sitter недоступен)`); }
const grammars: Record<string, unknown> = {};
if (Parser && !useFallback) {
    for (const [ext, pkg] of [['.rs', 'tree-sitter-rust'], ['.js', 'tree-sitter-javascript'], ['.cjs', 'tree-sitter-javascript'], ['.mjs', 'tree-sitter-javascript']] as const) {
        try {
            const mod = await import(`npm:${pkg}`);
            grammars[ext] = (mod as Record<string, unknown>).default || mod;
            log(`✅ ${ext}`);
        } catch (e) {}
    }
    try {
        const m = await import('npm:tree-sitter-typescript') as Record<string, unknown>;
        const base = m.typescript ?? (m.default as Record<string, unknown>)?.typescript;
        const tsx = m.tsx ?? (m.default as Record<string, unknown>)?.tsx;
        grammars['.ts'] = base;
        grammars['.tsx'] = tsx;
        log(`✅ .ts/.tsx`);
    } catch (e) {}
}
if (!useFallback && Object.keys(grammars).length === 0) useFallback = true;

const { createMcpServer } = await import('./mcp_base.ts');

createMcpServer({
    name: "ast-map-mcp",
    version: "3.0.0",
    tools: [{
        name: "generate_and_save_ast",
        description: "Полная карта: дерево файлов с токенами + AST функций.",
        inputSchema: { type: "object", properties: { target_path: { type: "string" }, tokenizer_model: { type: "string" } }, required: ["target_path"] }
    }],
    handlers: {
        generate_and_save_ast: async (args: Record<string, unknown>) => await processPathAndSave(String(args.target_path || ''), typeof args.tokenizer_model === 'string' ? args.tokenizer_model : undefined)
    }
});

// ============================================
// ПАРСЕРЫ КОДА
// ============================================
function rxRust(code: string): string[] {
    const r: string[] = [], l = code.split('\n');
    for (let i = 0; i < l.length; i++) {
        const s = l[i].trim();
        let m: RegExpMatchArray | null;
        if ((m = s.match(/^(?:pub\s+)?(?:async\s+)?fn\s+(\w+)/))) r.push(`  - [fn] ${m[1]} (L${i+1})`);
        else if ((m = s.match(/^(?:pub\s+)?struct\s+(\w+)/))) r.push(`  - [struct] ${m[1]} (L${i+1})`);
        else if ((m = s.match(/^(?:pub\s+)?enum\s+(\w+)/))) r.push(`  - [enum] ${m[1]} (L${i+1})`);
        else if ((m = s.match(/^(?:pub\s+)?trait\s+(\w+)/))) r.push(`  - [trait] ${m[1]} (L${i+1})`);
        else if ((m = s.match(/^(?:pub\s+)?impl(?:\s+<[^>]+>)?\s+(?:\w+\s+for\s+)?(\w+)/))) r.push(`  - [impl] ${m[1]} (L${i+1})`);
        else if ((m = s.match(/^(?:pub\s+)?(?:const|static)\s+(\w+)/))) r.push(`  - [const] ${m[1]} (L${i+1})`);
    }
    return r;
}
function rxJS(code: string, fp: string): string[] {
    const r: string[] = [], l = code.split('\n');
    const ts = ['.ts', '.tsx'].includes(path.extname(fp).toLowerCase());
    for (let i = 0; i < l.length; i++) {
        const s = l[i].trim();
        let m: RegExpMatchArray | null;
        if ((m = s.match(/^(?:export\s+)?(?:async\s+)?function\s+(\w+)/))) r.push(`  - [fn] ${m[1]} (L${i+1})`);
        else if ((m = s.match(/^(?:export\s+)?(?:default\s+)?class\s+(\w+)/))) r.push(`  - [class] ${m[1]} (L${i+1})`);
        else if ((m = s.match(/^(?:export\s+)?const\s+(\w+)\s*=/))) r.push(`  - [const] ${m[1]} (L${i+1})`);
        else if (ts && (m = s.match(/^(?:export\s+)?interface\s+(\w+)/))) r.push(`  - [interface] ${m[1]} (L${i+1})`);
        else if (ts && (m = s.match(/^(?:export\s+)?type\s+(\w+)\s*=/))) r.push(`  - [type] ${m[1]} (L${i+1})`);
    }
    return r;
}
function rxPy(code: string): string[] {
    const r: string[] = [], l = code.split('\n');
    for (let i = 0; i < l.length; i++) {
        const s = l[i];
        let m: RegExpMatchArray | null;
        if ((m = s.match(/^(?:async\s+)?def\s+(\w+)/))) r.push(`  - [def] ${m[1]} (L${i+1})`);
        else if ((m = s.match(/^class\s+(\w+)/))) r.push(`  - [class] ${m[1]} (L${i+1})`);
    }
    return r;
}
function tsParse(code: string, ext: string): string[] | null {
    const lang = grammars[ext];
    if (!lang) return null;
    try {
        const p = new (Parser as { new (): { setLanguage: (l: unknown) => void; parse: (c: string) => { rootNode: TsNode } } })();
        p.setLanguage(lang);
        return extract(p.parse(code).rootNode, 0, ext);
    } catch (e) { return null; }
}
interface TsNode {
    type: string;
    text: string;
    childCount: number;
    child: (i: number) => TsNode;
    startPosition: { row: number };
}
function extract(node: TsNode, depth: number, lang: string): string[] {
    let r: string[] = [];
    let name = "anon";
    let ok = false;
    let ts = "";
    const isR = lang === '.rs';
    const isJS = ['.js', '.ts', '.tsx', '.jsx', '.cjs', '.mjs'].includes(lang);
    if (isR) {
        if (['function_item', 'struct_item', 'enum_item', 'trait_item'].includes(node.type)) { ok = true; ts = node.type.replace('_item', ''); }
        else if (node.type === 'impl_item') { ok = true; ts = 'impl'; }
    } else if (isJS && ['function_declaration', 'class_declaration', 'method_definition', 'interface_declaration', 'type_alias_declaration'].includes(node.type)) {
        ok = true;
        ts = node.type.replace('_declaration', '').replace('_definition', '');
    }
    if (ok) {
        for (let i = 0; i < node.childCount; i++) {
            const c = node.child(i);
            if (['identifier', 'type_identifier', 'property_identifier'].includes(c.type)) { name = c.text; break; }
        }
        if (node.type === 'impl_item') for (let i = 0; i < node.childCount; i++) {
            const c = node.child(i);
            if (c.type === 'type_identifier' || c.type === 'scoped_type_identifier') name = c.text;
        }
        r.push(`${'  '.repeat(depth)}- [${ts}] ${name} (L${node.startPosition.row+1})`);
        depth++;
    }
    for (let i = 0; i < node.childCount; i++) r = r.concat(extract(node.child(i), depth, lang));
    return r;
}
function parseCodeFile(fp: string): string[] | null {
    const ext = path.extname(fp).toLowerCase();
    if (!['.rs', '.js', '.ts', '.tsx', '.jsx', '.cjs', '.mjs', '.py'].includes(ext)) return null;
    let code: string;
    try { code = fs.readFileSync(fp, 'utf8'); } catch (e) { return null; }
    if (!code.trim()) return null;
    if (!useFallback && grammars[ext]) { const r = tsParse(code, ext); if (r && r.length > 0) return r; }
    if (ext === '.rs') return rxRust(code);
    if (['.js', '.ts', '.tsx', '.jsx', '.cjs', '.mjs'].includes(ext)) return rxJS(code, fp);
    if (ext === '.py') return rxPy(code);
    return null;
}

// ============================================
// ФОРМАТИРОВАНИЕ
// ============================================
function fmtTok(n: number): string { return n >= 1e6 ? `${(n/1e6).toFixed(1)}M` : n >= 1e3 ? `${(n/1e3).toFixed(1)}K` : `${n}`; }
function fmtSize(b: number): string { return b >= 1048576 ? `${(b/1048576).toFixed(1)}MB` : b >= 1024 ? `${(b/1024).toFixed(1)}KB` : `${b}B`; }

// ============================================
// GITIGNORE
// ============================================
interface GitRule { pattern: string; neg: boolean; dirOnly: boolean; }
function loadGitignore(rootDir: string): GitRule[] {
    const rules: GitRule[] = [];
    try {
        for (let raw of fs.readFileSync(path.join(rootDir, '.gitignore'), 'utf8').split('\n')) {
            let line = raw.trim();
            if (!line || line.startsWith('#')) continue;
            const neg = line.startsWith('!');
            if (neg) line = line.substring(1);
            const dirOnly = line.endsWith('/');
            if (dirOnly) line = line.slice(0, -1);
            if (line) rules.push({ pattern: line, neg, dirOnly });
        }
    } catch (e) {}
    return rules;
}
function gitMatch(relPath: string, name: string, pattern: string): boolean {
    if (pattern.startsWith('/')) {
        const rp = pattern.substring(1);
        return relPath === rp || relPath.startsWith(rp + '/');
    }
    if (pattern.includes('/')) {
        return relPath === pattern || relPath.startsWith(pattern + '/') || relPath.endsWith('/' + pattern);
    }
    if (pattern.includes('*')) {
        const re = new RegExp('^' + pattern.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '[^/]*') + '$');
        return re.test(name) || re.test(relPath);
    }
    return name === pattern || relPath === pattern || relPath.startsWith(pattern + '/') || relPath.endsWith('/' + pattern);
}
function isGitIgnored(relPath: string, name: string, isDir: boolean, rules: GitRule[]): boolean {
    let result = false;
    for (const r of rules) {
        if (r.dirOnly && !isDir) continue;
        if (gitMatch(relPath, name, r.pattern)) result = !r.neg;
    }
    return result;
}

// ============================================
// ФИЛЬТРАЦИЯ
// ============================================
const SKIP_DIRS = new Set(['node_modules', 'dist', '.git', '.svn', '.hg', '.idea', '.vscode', 'target', '.agents_workspace']);
const SKIP_EXTS = new Set(['.lock']);
const SKIP_NAMES = new Set(['package-lock.json']);
const SHOW_EXTS = new Set([
    '.rs', '.js', '.ts', '.tsx', '.jsx', '.cjs', '.mjs', '.py', '.c', '.cpp', '.h', '.go', '.java', '.kt', '.rb',
    '.json', '.toml', '.yaml', '.yml', '.ini', '.css', '.html', '.md', '.txt', '.bat', '.sh', '.conf', '.srt', '.vtt',
]);
const BINARY_EXTS = new Set(['.exe', '.dll', '.so', '.png', '.jpg', '.jpeg', '.gif', '.ico', '.woff', '.ttf', '.mp3', '.mp4', '.zip', '.pdf']);
function shouldShow(name: string): boolean {
    const ext = path.extname(name).toLowerCase();
    if (SHOW_EXTS.has(ext) || BINARY_EXTS.has(ext)) return true;
    if (!ext && ['Makefile', 'Dockerfile', 'LICENSE', 'README', '.gitignore', '.dockerignore', '.env'].some(n => name.toLowerCase() === n.toLowerCase())) return true;
    return false;
}

// ============================================
// СКАНИРОВАНИЕ ДЕРЕВА (async для countTokens)
// ============================================
interface TreeNode {
    name: string;
    fullPath: string;
    dirNodes: TreeNode[];
    fileNodes: { name: string; fullPath: string; tokens: number; size: number; isBinary: boolean }[];
    totalTokens: number;
    totalSize: number;
}
async function scanDir(dirPath: string, rootDir: string, gitRules: GitRule[]): Promise<TreeNode | null> {
    let entries: string[];
    try { entries = fs.readdirSync(dirPath); } catch (e) { return null; }
    const relDir = path.relative(rootDir, dirPath).replace(/\\/g, '/');
    let dirNodes: TreeNode[] = [];
    let fileNodes: TreeNode['fileNodes'] = [];

    for (const entry of entries) {
        if (entry.startsWith('.') && !entry.match(/\.\w+$/)) continue;
        const fullPath = path.join(dirPath, entry);
        const relPath = relDir ? relDir + '/' + entry : entry;
        let stat: fs.Stats;
        try { stat = fs.statSync(fullPath); } catch (e) { continue; }

        if (stat.isDirectory()) {
            if (isGitIgnored(relPath, entry, true, gitRules)) continue;
            if (SKIP_DIRS.has(entry) || SKIP_DIRS.has(entry.toLowerCase())) continue;
            const child = await scanDir(fullPath, rootDir, gitRules);
            if (child && (child.dirNodes.length + child.fileNodes.length > 0)) dirNodes.push(child);
        } else {
            if (isGitIgnored(relPath, entry, false, gitRules)) continue;
            if (SKIP_EXTS.has(path.extname(entry).toLowerCase()) || SKIP_NAMES.has(entry)) continue;
            if (entry.endsWith('_ast_map.md')) continue;
            if (!shouldShow(entry)) continue;

            const ext = path.extname(entry).toLowerCase();
            const bin = BINARY_EXTS.has(ext);
            let tokens = 0;
            if (!bin && stat.size < 5242880) {
                try { tokens = await countTokens(fs.readFileSync(fullPath, 'utf8')); } catch (e) {}
            }
            fileNodes.push({ name: entry, fullPath, tokens, size: stat.size, isBinary: bin });
        }
    }

    dirNodes.sort((a, b) => a.name.localeCompare(b.name));
    fileNodes.sort((a, b) => a.name.localeCompare(b.name));
    return {
        name: path.basename(dirPath), fullPath: dirPath, dirNodes, fileNodes,
        totalTokens: dirNodes.reduce((s, d) => s + d.totalTokens, 0) + fileNodes.reduce((s, f) => s + f.tokens, 0),
        totalSize: dirNodes.reduce((s, d) => s + d.totalSize, 0) + fileNodes.reduce((s, f) => s + f.size, 0),
    };
}

function formatTree(node: TreeNode, prefix: string): string[] {
    let lines: string[] = [];
    const items: { type: 'dir' | 'file'; node: TreeNode | TreeNode['fileNodes'][number] }[] = [
        ...node.dirNodes.map(d => ({ type: 'dir' as const, node: d })),
        ...node.fileNodes.map(f => ({ type: 'file' as const, node: f })),
    ];
    for (let i = 0; i < items.length; i++) {
        const item = items[i];
        const last = i === items.length - 1;
        const conn = last ? '└── ' : '├── ';
        const next = prefix + (last ? '    ' : '│   ');
        if (item.type === 'dir') {
            const d = item.node as TreeNode;
            lines.push(`${prefix}${conn}${d.name}/  ~${fmtTok(d.totalTokens)}`);
            lines.push(...formatTree(d, next));
        } else {
            const f = item.node as TreeNode['fileNodes'][number];
            const ann = f.isBinary || f.tokens === 0 ? fmtSize(f.size) : `~${fmtTok(f.tokens)}`;
            lines.push(`${prefix}${conn}${f.name}  ${ann}`);
        }
    }
    return lines;
}

function collectCode(node: TreeNode, rootDir: string): { relPath: string; fullPath: string }[] {
    let files: { relPath: string; fullPath: string }[] = [];
    const CODE_EXTS = new Set(['.rs', '.js', '.ts', '.tsx', '.jsx', '.cjs', '.mjs', '.py']);
    for (const f of node.fileNodes) {
        if (CODE_EXTS.has(path.extname(f.name).toLowerCase())) {
            files.push({ relPath: path.relative(rootDir, f.fullPath).replace(/\\/g, '/'), fullPath: f.fullPath });
        }
    }
    for (const d of node.dirNodes) files = files.concat(collectCode(d, rootDir));
    return files;
}

// ============================================
// ОСНОВНАЯ ЛОГИКА
// ============================================
async function processPathAndSave(targetPath: string, tokenizerModel?: string): Promise<string> {
    const absPath = path.resolve(targetPath);
    log(`Генерация карты: ${absPath}`);
    let stats: fs.Stats;
    try { stats = fs.statSync(absPath); } catch (e) { return `❌ Путь не существует: ${absPath}`; }
    const rootDir = stats.isDirectory() ? absPath : path.dirname(absPath);
    const folderName = path.basename(rootDir);

    // Инициализируем токенизатор (если указана кастомная модель — используем её)
    if (tokenizerModel && tokenizerModel !== hfModelName) {
        hfModelName = tokenizerModel;
        hfTokenizer = null;
        hfInitPromise = null;
    }
    await initTokenizer(tokenizerModel);

    const gitRules = loadGitignore(rootDir);
    log(`Правил .gitignore: ${gitRules.length}`);

    const tree = await scanDir(rootDir, rootDir, gitRules);
    if (!tree) return `❌ Не удалось просканировать: ${absPath}`;
    log(`Токенов: ~${fmtTok(tree.totalTokens)} (${hfAvailable ? '🤗 HuggingFace' : '⚠️ оценка/3.7'})`);

    const now = new Date().toISOString().replace('T', ' ').substring(0, 19);
    const tokMode = hfAvailable ? `🤗 @huggingface/transformers (${hfModelName})` : '⚠️ длина/3.7 (неточный!)';
    let out = `# Code Map: ${absPath}\n\n`;
    out += `> ${now} | Токенов: ~${fmtTok(tree.totalTokens)} | Подсчёт: ${tokMode}\n\n`;

    const treeLines = formatTree(tree, '');
    out += `## 📁 Структура проекта\n\n\`\`\`\n${folderName}/  ~${fmtTok(tree.totalTokens)}\n${treeLines.join('\n')}\n\`\`\`\n\n`;

    const codeFiles = collectCode(tree, rootDir);
    codeFiles.sort((a, b) => a.relPath.localeCompare(b.relPath));

    let astContent = '';
    let parsedCount = 0;
    for (const f of codeFiles) {
        const res = parseCodeFile(f.fullPath);
        if (res && res.length > 0) {
            astContent += `### ${f.relPath}\n${res.join('\n')}\n\n`;
            parsedCount++;
        }
    }

    if (astContent) out += `## 💻 Функции и классы (${parsedCount} файлов)\n\n${astContent}`;

    // Создаем папку .agents_workspace и сохраняем карту туда
    const workspaceDir = path.join(rootDir, '.agents_workspace');
    if (!fs.existsSync(workspaceDir)) {
        try { fs.mkdirSync(workspaceDir, { recursive: true }); } catch (e) { log(`⚠️ Ошибка создания .agents_workspace: ${(e as Error).message}`); }
    }
    const savePath = path.join(workspaceDir, `${folderName}_ast_map.md`);
    try { fs.writeFileSync(savePath, out, 'utf8'); } catch (e) { return `❌ Не сохранилось: ${(e as Error).message}`; }

    const msg = `✅ Карта готова!\n📁 ${savePath}\n📊 ~${fmtTok(tree.totalTokens)} токенов (${hfAvailable ? '🤗 HF' : 'оценка'}) | ${parsedCount} файлов с AST`;
    log(msg);
    return msg;
}

log(`✅ Готов. Токены: ${hfAvailable ? '🤗 HuggingFace' : '⚠️ будет загружен при первом вызове'} | Парсер: ${useFallback ? 'Регекс' : 'Tree-sitter'}`);
