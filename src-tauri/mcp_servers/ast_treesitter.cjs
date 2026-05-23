const fs = require('fs');
const path = require('path');
const readline = require('readline');

function log(msg) { process.stderr.write(`[AST-SERVER] ${msg}\n`); }
log(`=== AST Map MCP v2.3 ===`);

// ============================================
// NODE_MODULES
// ============================================
function findNm() {
    let d = __dirname;
    for (let i = 0; i < 20; i++) {
        const nm = path.join(d, 'node_modules');
        try { if (fs.existsSync(nm) && fs.statSync(nm).isDirectory()) return nm; } catch(e) {}
        const p = path.dirname(d); if (p === d) break; d = p;
    }
    return null;
}
const nmFound = findNm();
if (nmFound && !module.paths.includes(nmFound)) module.paths.unshift(nmFound);
if (!module.paths.includes(path.resolve(process.cwd(), 'node_modules'))) module.paths.push(path.resolve(process.cwd(), 'node_modules'));

// ============================================
// TIKTOKEN — точный подсчёт токенов
// ============================================
let tiktokenEncoder = null;
let tiktokenAvailable = false;

try {
    const tiktoken = require('js-tiktoken');
    tiktokenEncoder = tiktoken.encodingForModel('gpt-4');
    tiktokenAvailable = true;
    log(`✅ tiktoken загружен (cl100k_base) — точный подсчёт токенов`);
} catch (e) {
    log(`⚠️ tiktoken недоступен: ${e.message.split('\n')[0]}`);
    log(`⚠️ Фоллбэк: длина/3.7 (неточный, занижает русский)`);
}

function countTokens(text) {
    if (!text || !text.trim()) return 0;
    if (tiktokenAvailable && tiktokenEncoder) {
        try { return tiktokenEncoder.encode(text).length; } catch(e) {}
    }
    return Math.ceil(text.length / 3.7);
}

// ============================================
// TREE-SITTER
// ============================================
let Parser = null, useFallback = false;
try { Parser = require('tree-sitter'); log(`✅ tree-sitter`); } catch(e) { useFallback = true; log(`⚠️ Фоллбэк`); }
const grammars = {};
if (Parser && !useFallback) {
    for (const [ext, pkg] of [['.rs','tree-sitter-rust'],['.js','tree-sitter-javascript'],['.cjs','tree-sitter-javascript'],['.mjs','tree-sitter-javascript']]) {
        try { grammars[ext] = require(pkg); log(`✅ ${ext}`); } catch(e) {}
    }
    try { const m = require('tree-sitter-typescript'); grammars['.ts'] = m.typescript; grammars['.tsx'] = m.tsx; log(`✅ .ts/.tsx`); } catch(e) {}
}
if (!useFallback && Object.keys(grammars).length === 0) useFallback = true;

// ============================================
// MCP PROTOCOL
// ============================================
const rl = readline.createInterface({ input: process.stdin, output: process.stdout, terminal: false });
rl.on('line', line => { try { handleRequest(JSON.parse(line)); } catch(e) {} });
function send(id, result) { console.log(JSON.stringify({ jsonrpc: "2.0", id, result })); }
function handleRequest(req) {
    const { id, method, params } = req;
    if (method === 'initialize') send(id, { protocolVersion: "2024-11-05", capabilities: { tools: {} }, serverInfo: { name: "ast-map-mcp", version: "2.3.0" } });
    else if (method === 'tools/list') send(id, { tools: [{ name: "generate_and_save_ast", description: "Полная карта: дерево файлов с токенами + AST функций. Уважает .gitignore, точный подсчёт токенов (tiktoken).", inputSchema: { type: "object", properties: { target_path: { type: "string", description: "Путь к папке проекта" } }, required: ["target_path"] } }] });
    else if (method === 'tools/call' && params.name === 'generate_and_save_ast') {
        try { send(id, { content: [{ type: "text", text: processPathAndSave(params.arguments.target_path) }] }); }
        catch (e) { send(id, { content: [{ type: "text", text: `❌ ${e.message}` }] }); }
    }
}

// ============================================
// ПАРСЕРЫ КОДА
// ============================================
function rxRust(code) {
    const r = [], l = code.split('\n');
    for (let i = 0; i < l.length; i++) { const s = l[i].trim(); let m;
        if ((m=s.match(/^(?:pub\s+)?(?:async\s+)?fn\s+(\w+)/))) r.push(`  - [fn] ${m[1]} (L${i+1})`);
        else if ((m=s.match(/^(?:pub\s+)?struct\s+(\w+)/))) r.push(`  - [struct] ${m[1]} (L${i+1})`);
        else if ((m=s.match(/^(?:pub\s+)?enum\s+(\w+)/))) r.push(`  - [enum] ${m[1]} (L${i+1})`);
        else if ((m=s.match(/^(?:pub\s+)?trait\s+(\w+)/))) r.push(`  - [trait] ${m[1]} (L${i+1})`);
        else if ((m=s.match(/^(?:pub\s+)?impl(?:\s+<[^>]+>)?\s+(?:\w+\s+for\s+)?(\w+)/))) r.push(`  - [impl] ${m[1]} (L${i+1})`);
        else if ((m=s.match(/^(?:pub\s+)?(?:const|static)\s+(\w+)/))) r.push(`  - [const] ${m[1]} (L${i+1})`);
    } return r;
}
function rxJS(code, fp) {
    const r = [], l = code.split('\n'), ts = ['.ts','.tsx'].includes(path.extname(fp).toLowerCase());
    for (let i = 0; i < l.length; i++) { const s = l[i].trim(); let m;
        if ((m=s.match(/^(?:export\s+)?(?:async\s+)?function\s+(\w+)/))) r.push(`  - [fn] ${m[1]} (L${i+1})`);
        else if ((m=s.match(/^(?:export\s+)?(?:default\s+)?class\s+(\w+)/))) r.push(`  - [class] ${m[1]} (L${i+1})`);
        else if ((m=s.match(/^(?:export\s+)?const\s+(\w+)\s*=/))) r.push(`  - [const] ${m[1]} (L${i+1})`);
        else if (ts && (m=s.match(/^(?:export\s+)?interface\s+(\w+)/))) r.push(`  - [interface] ${m[1]} (L${i+1})`);
        else if (ts && (m=s.match(/^(?:export\s+)?type\s+(\w+)\s*=/))) r.push(`  - [type] ${m[1]} (L${i+1})`);
    } return r;
}
function rxPy(code) {
    const r = [], l = code.split('\n');
    for (let i = 0; i < l.length; i++) { const s = l[i]; let m;
        if ((m=s.match(/^(?:async\s+)?def\s+(\w+)/))) r.push(`  - [def] ${m[1]} (L${i+1})`);
        else if ((m=s.match(/^class\s+(\w+)/))) r.push(`  - [class] ${m[1]} (L${i+1})`);
    } return r;
}
function tsParse(code, ext) {
    const lang = grammars[ext]; if (!lang) return null;
    try { const p = new Parser(); p.setLanguage(lang); return extract(p.parse(code).rootNode, 0, ext); } catch(e) { return null; }
}
function extract(node, depth, lang) {
    let r = [], name = "anon", ok = false, ts = "";
    const isR = lang === '.rs', isJS = ['.js','.ts','.tsx','.jsx','.cjs','.mjs'].includes(lang);
    if (isR) {
        if (['function_item','struct_item','enum_item','trait_item'].includes(node.type)) { ok=true; ts=node.type.replace('_item',''); }
        else if (node.type==='impl_item') { ok=true; ts='impl'; }
    } else if (isJS && ['function_declaration','class_declaration','method_definition','interface_declaration','type_alias_declaration'].includes(node.type)) {
        ok=true; ts=node.type.replace('_declaration','').replace('_definition','');
    }
    if (ok) {
        for (let i=0;i<node.childCount;i++){const c=node.child(i);if(['identifier','type_identifier','property_identifier'].includes(c.type)){name=c.text;break;}}
        if(node.type==='impl_item')for(let i=0;i<node.childCount;i++){const c=node.child(i);if(c.type==='type_identifier'||c.type==='scoped_type_identifier')name=c.text;}
        r.push(`${'  '.repeat(depth)}- [${ts}] ${name} (L${node.startPosition.row+1})`); depth++;
    }
    for(let i=0;i<node.childCount;i++) r=r.concat(extract(node.child(i),depth,lang));
    return r;
}
function parseCodeFile(fp) {
    const ext = path.extname(fp).toLowerCase();
    if (!['.rs','.js','.ts','.tsx','.jsx','.cjs','.mjs','.py'].includes(ext)) return null;
    let code; try { code = fs.readFileSync(fp,'utf8'); } catch(e) { return null; }
    if (!code.trim()) return null;
    if (!useFallback && grammars[ext]) { const r = tsParse(code,ext); if (r && r.length > 0) return r; }
    if (ext==='.rs') return rxRust(code);
    if (['.js','.ts','.tsx','.jsx','.cjs','.mjs'].includes(ext)) return rxJS(code,fp);
    if (ext==='.py') return rxPy(code);
    return null;
}

// ============================================
// ФОРМАТИРОВАНИЕ
// ============================================
function fmtTok(n) { return n >= 1e6 ? `${(n/1e6).toFixed(1)}M` : n >= 1e3 ? `${(n/1e3).toFixed(1)}K` : `${n}`; }
function fmtSize(b) { return b >= 1048576 ? `${(b/1048576).toFixed(1)}MB` : b >= 1024 ? `${(b/1024).toFixed(1)}KB` : `${b}B`; }

// ============================================
// GITIGNORE
// ============================================
function loadGitignore(rootDir) {
    const rules = [];
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
function gitMatch(relPath, name, pattern) {
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
function isGitIgnored(relPath, name, isDir, rules) {
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
const SKIP_DIRS = new Set(['node_modules','dist','.git','.svn','.hg','.idea','.vscode','target']);
const SKIP_EXTS = new Set(['.lock']);
const SKIP_NAMES = new Set(['package-lock.json']);
const SHOW_EXTS = new Set([
    '.rs','.js','.ts','.tsx','.jsx','.cjs','.mjs','.py','.c','.cpp','.h','.go','.java','.kt','.rb',
    '.json','.toml','.yaml','.yml','.ini','.css','.html','.md','.txt','.bat','.sh','.conf','.srt','.vtt',
]);
const BINARY_EXTS = new Set(['.exe','.dll','.so','.png','.jpg','.jpeg','.gif','.ico','.woff','.ttf','.mp3','.mp4','.zip','.pdf']);
function shouldShow(name) {
    const ext = path.extname(name).toLowerCase();
    if (SHOW_EXTS.has(ext) || BINARY_EXTS.has(ext)) return true;
    if (!ext && ['Makefile','Dockerfile','LICENSE','README','.gitignore','.dockerignore','.env'].some(n => name.toLowerCase() === n.toLowerCase())) return true;
    return false;
}

// ============================================
// СКАНИРОВАНИЕ ДЕРЕВА
// ============================================
function scanDir(dirPath, rootDir, gitRules) {
    let entries;
    try { entries = fs.readdirSync(dirPath); } catch(e) { return null; }
    const relDir = path.relative(rootDir, dirPath).replace(/\\/g, '/');
    let dirNodes = [], fileNodes = [];

    for (const entry of entries) {
        if (entry.startsWith('.') && !entry.match(/\.\w+$/)) continue;
        const fullPath = path.join(dirPath, entry);
        const relPath = relDir ? relDir + '/' + entry : entry;
        let stat; try { stat = fs.statSync(fullPath); } catch(e) { continue; }

        if (stat.isDirectory()) {
            if (isGitIgnored(relPath, entry, true, gitRules)) continue;
            if (SKIP_DIRS.has(entry) || SKIP_DIRS.has(entry.toLowerCase())) continue;
            const child = scanDir(fullPath, rootDir, gitRules);
            if (child && (child.dirNodes.length + child.fileNodes.length > 0)) dirNodes.push(child);
        } else {
            if (isGitIgnored(relPath, entry, false, gitRules)) continue;
            if (SKIP_EXTS.has(path.extname(entry).toLowerCase()) || SKIP_NAMES.has(entry)) continue;
            if (entry.endsWith('_ast_map.md')) continue;
            if (!shouldShow(entry)) continue;

            const ext = path.extname(entry).toLowerCase();
            const bin = BINARY_EXTS.has(ext);
            let tokens = 0;
            // Считаем токены только для текстовых файлов < 5MB
            if (!bin && stat.size < 5242880) {
                try { tokens = countTokens(fs.readFileSync(fullPath, 'utf8')); } catch(e) {}
            }
            fileNodes.push({ name: entry, fullPath, tokens, size: stat.size, isBinary: bin });
        }
    }

    dirNodes.sort((a,b) => a.name.localeCompare(b.name));
    fileNodes.sort((a,b) => a.name.localeCompare(b.name));
    return {
        name: path.basename(dirPath), fullPath: dirPath, dirNodes, fileNodes,
        totalTokens: dirNodes.reduce((s,d) => s + d.totalTokens, 0) + fileNodes.reduce((s,f) => s + f.tokens, 0),
        totalSize: dirNodes.reduce((s,d) => s + d.totalSize, 0) + fileNodes.reduce((s,f) => s + f.size, 0),
    };
}

function formatTree(node, prefix) {
    let lines = [];
    const items = [
        ...node.dirNodes.map(d => ({ type: 'dir', node: d })),
        ...node.fileNodes.map(f => ({ type: 'file', node: f })),
    ];
    for (let i = 0; i < items.length; i++) {
        const item = items[i], last = i === items.length - 1;
        const conn = last ? '└── ' : '├── ';
        const next = prefix + (last ? '    ' : '│   ');
        if (item.type === 'dir') {
            lines.push(`${prefix}${conn}${item.node.name}/  ~${fmtTok(item.node.totalTokens)}`);
            lines.push(...formatTree(item.node, next));
        } else {
            const f = item.node;
            const ann = f.isBinary || f.tokens === 0 ? fmtSize(f.size) : `~${fmtTok(f.tokens)}`;
            lines.push(`${prefix}${conn}${f.name}  ${ann}`);
        }
    }
    return lines;
}

function collectCode(node, rootDir) {
    let files = [];
    const CODE_EXTS = new Set(['.rs','.js','.ts','.tsx','.jsx','.cjs','.mjs','.py']);
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
function processPathAndSave(targetPath) {
    const absPath = path.resolve(targetPath);
    log(`Генерация карты: ${absPath}`);
    let stats; try { stats = fs.statSync(absPath); } catch(e) { return `❌ Путь не существует: ${absPath}`; }
    const rootDir = stats.isDirectory() ? absPath : path.dirname(absPath);
    const folderName = path.basename(rootDir);

    const gitRules = loadGitignore(rootDir);
    log(`Правил .gitignore: ${gitRules.length}`);

    const tree = scanDir(rootDir, rootDir, gitRules);
    if (!tree) return `❌ Не удалось просканировать: ${absPath}`;
    log(`Токенов: ~${fmtTok(tree.totalTokens)} (${tiktokenAvailable ? 'tiktoken' : 'оценка/3.7'})`);

    const now = new Date().toISOString().replace('T',' ').substring(0,19);
    const tokMode = tiktokenAvailable ? '🎯 tiktoken (точный)' : '⚠️ длина/3.7 (неточный!)';
    let out = `# Code Map: ${absPath}\n\n`;
    out += `> ${now} | Токенов: ~${fmtTok(tree.totalTokens)} | Подсчёт: ${tokMode}\n\n`;

    const treeLines = formatTree(tree, '');
    out += `## 📁 Структура проекта\n\n\`\`\`\n${folderName}/  ~${fmtTok(tree.totalTokens)}\n${treeLines.join('\n')}\n\`\`\`\n\n`;

    const codeFiles = collectCode(tree, rootDir);
    codeFiles.sort((a,b) => a.relPath.localeCompare(b.relPath));

    let astContent = '', parsedCount = 0;
    for (const f of codeFiles) {
        const res = parseCodeFile(f.fullPath);
        if (res && res.length > 0) {
            astContent += `### ${f.relPath}\n${res.join('\n')}\n\n`;
            parsedCount++;
        }
    }

    if (astContent) out += `## 💻 Функции и классы (${parsedCount} файлов)\n\n${astContent}`;

    const savePath = path.join(rootDir, `${folderName}_ast_map.md`);
    try { fs.writeFileSync(savePath, out, 'utf8'); } catch(e) { return `❌ Не сохранилось: ${e.message}`; }

    const msg = `✅ Карта готова!\n📁 ${savePath}\n📊 ~${fmtTok(tree.totalTokens)} токенов (${tiktokenAvailable ? 'tiktoken' : 'оценка'}) | ${parsedCount} файлов с AST`;
    log(msg);
    return msg;
}

log(`✅ Готов. Токены: ${tiktokenAvailable ? '🎯 tiktoken' : '⚠️ оценка/3.7'} | Парсер: ${useFallback ? 'Регекс' : 'Tree-sitter'}`);