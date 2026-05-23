const fs = require('fs');
const path = require('path');
const readline = require('readline');

// ============================================
// ДИАГНОСТИЧЕСКОЕ ЛОГИРОВАНИЕ (stderr -> логи GUI)
// ============================================
function log(msg) {
    process.stderr.write(`[AST-SERVER] ${msg}\n`);
}

log(`=== AST Tree-sitter MCP Server v1.3.0 ===`);
log(`Рабочая директория (cwd): ${process.cwd()}`);
log(`Директория скрипта (__dirname): ${__dirname}`);
log(`Node.js версия: ${process.version}`);
log(`Архитектура: ${process.arch}`);

// ============================================
// ПОИСК node_modules
// ============================================
function findNodeModules() {
    let dir = __dirname;
    const tried = [];
    for (let i = 0; i < 20; i++) {
        const nm = path.join(dir, 'node_modules');
        tried.push(nm);
        try {
            if (fs.existsSync(nm) && fs.statSync(nm).isDirectory()) {
                return { found: nm, tried };
            }
        } catch (e) {
            tried.push(`${nm} -> ОШИБКА: ${e.message}`);
        }
        const parent = path.dirname(dir);
        if (parent === dir) break;
        dir = parent;
    }
    return { found: null, tried };
}

const nmResult = findNodeModules();
log(`Поиск node_modules от __dirname вверх:`);
nmResult.tried.forEach((p, i) => log(`  [${i}] ${p}`));
log(`Результат: ${nmResult.found || 'НЕ НАЙДЕН!'}`);

if (nmResult.found && !module.paths.includes(nmResult.found)) {
    module.paths.unshift(nmResult.found);
}

const cwdModules = path.resolve(process.cwd(), 'node_modules');
if (!module.paths.includes(cwdModules)) module.paths.push(cwdModules);

// ============================================
// ЗАГРУЗКА TREE-SITTER И ГРАММАТИК
// ============================================
let Parser = null;
let useFallback = false;
let treeSitterErrors = [];

log(`--- Загрузка tree-sitter ---`);
log(`module.paths (первые 5): ${module.paths.slice(0, 5).join('; ')}`);

try {
    Parser = require('tree-sitter');
    log(`✅ tree-sitter загружен успешно`);
} catch (e) {
    treeSitterErrors.push(`tree-sitter: ${e.message.split('\n')[0]}`);
    log(`❌ tree-sitter НЕ ЗАГРУЗИЛСЯ: ${e.message.split('\n')[0]}`);
    log(`❌ Полная ошибка: ${e.message}`);
    useFallback = true;
}

const grammars = {};
const grammarNames = {
    '.rs': 'tree-sitter-rust',
    '.js': 'tree-sitter-javascript',
    '.cjs': 'tree-sitter-javascript',
    '.mjs': 'tree-sitter-javascript',
    '.ts': 'tree-sitter-typescript (typescript)',
    '.tsx': 'tree-sitter-typescript (tsx)',
};

if (Parser && !useFallback) {
    for (const [ext, pkg] of Object.entries(grammarNames)) {
        try {
            if (ext === '.ts') {
                const mod = require('tree-sitter-typescript');
                grammars[ext] = mod.typescript;
            } else if (ext === '.tsx') {
                const mod = require('tree-sitter-typescript');
                grammars[ext] = mod.tsx;
            } else {
                grammars[ext] = require(pkg);
            }
            log(`✅ Грамматика ${ext} (${pkg}) загружена`);
        } catch (e) {
            const shortErr = e.message.split('\n')[0].substring(0, 150);
            treeSitterErrors.push(`${ext} (${pkg}): ${shortErr}`);
            log(`❌ Грамматика ${ext} (${pkg}) НЕ ЗАГРУЗИЛАСЬ: ${shortErr}`);
        }
    }
}

if (useFallback || Object.keys(grammars).length === 0) {
    log(`⚠️ Переключаюсь на РЕГЕКС-ФОЛЛБЭК (tree-sitter недоступен)`);
    useFallback = true;
} else {
    log(`✅ Все проверки пройдены. Готов к работе с tree-sitter.`);
}

// ============================================
// MCP PROTOCOL
// ============================================
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
    console.log(JSON.stringify({ jsonrpc: "2.0", id: id, result: result }));
}

function handleRequest(req) {
    const { id, method, params } = req;
    if (method === 'initialize') {
        sendResponse(id, {
            protocolVersion: "2024-11-05",
            capabilities: { tools: {} },
            serverInfo: { name: "ast-treesitter-mcp", version: "1.3.0" }
        });
    } else if (method === 'tools/list') {
        sendResponse(id, {
            tools: [{
                name: "generate_and_save_ast",
                description: "ПАРСЕР: Читает Rust/JS/TS/Python файлы проекта и сохраняет AST-карту в Markdown файл. Работает через tree-sitter (если доступен) или через регекс-фоллбэк.",
                inputSchema: {
                    type: "object",
                    properties: {
                        target_path: { type: "string", description: "Путь к папке проекта" }
                    },
                    required: ["target_path"]
                }
            }]
        });
    } else if (method === 'tools/call') {
        const { name, arguments: args } = params;
        if (name === 'generate_and_save_ast') {
            try {
                const result = processPathAndSave(args.target_path);
                sendResponse(id, { content: [{ type: "text", text: result }] });
            } catch (e) {
                sendResponse(id, { content: [{ type: "text", text: `❌ Критическая ошибка: ${e.message}\n${e.stack}` }] });
            }
        }
    }
}

// ============================================
// РЕГЕКС-ФОЛЛБЭК ПАРСЕР
// ============================================
function regexParseRust(code, filePath) {
    const results = [];
    const lines = code.split('\n');
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i].trim();
        let m;
        if ((m = line.match(/^(?:pub\s+)?(?:async\s+)?fn\s+(\w+)/))) {
            results.push(`  - [function] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:pub\s+)?struct\s+(\w+)/))) {
            results.push(`  - [struct] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:pub\s+)?enum\s+(\w+)/))) {
            results.push(`  - [enum] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:pub\s+)?trait\s+(\w+)/))) {
            results.push(`  - [trait] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:pub\s+)?impl(?:\s+<[^>]+>)?\s+(?:\w+\s+for\s+)?(\w+)/))) {
            results.push(`  - [impl] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:pub\s+)?(?:const|static)\s+(\w+)/))) {
            results.push(`  - [const] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:pub\s+)?type\s+(\w+)/))) {
            results.push(`  - [type_alias] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:pub\s+)?mod\s+(\w+)/))) {
            results.push(`  - [mod] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:pub\s+)?use\s+([\w:]+)/))) {
            results.push(`  - [use] ${m[1]} (Line ${i + 1})`);
        }
    }
    return results;
}

function regexParseJS(code, filePath) {
    const results = [];
    const lines = code.split('\n');
    const ext = path.extname(filePath).toLowerCase();
    const isTS = ext === '.ts' || ext === '.tsx';
    
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i].trim();
        let m;
        if ((m = line.match(/^(?:export\s+)?(?:async\s+)?function\s+(\w+)/))) {
            results.push(`  - [function] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:export\s+)?(?:default\s+)?class\s+(\w+)/))) {
            results.push(`  - [class] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:export\s+)?const\s+(\w+)\s*=/))) {
            results.push(`  - [const] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:export\s+)?let\s+(\w+)\s*=/))) {
            results.push(`  - [let] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:export\s+)?var\s+(\w+)\s*=/))) {
            results.push(`  - [var] ${m[1]} (Line ${i + 1})`);
        } else if (isTS && (m = line.match(/^(?:export\s+)?interface\s+(\w+)/))) {
            results.push(`  - [interface] ${m[1]} (Line ${i + 1})`);
        } else if (isTS && (m = line.match(/^(?:export\s+)?type\s+(\w+)\s*=/))) {
            results.push(`  - [type] ${m[1]} (Line ${i + 1})`);
        } else if (isTS && (m = line.match(/^(?:export\s+)?enum\s+(\w+)/))) {
            results.push(`  - [enum] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^(?:export\s+)?async\s+function\s*\*?\s*(\w+)/))) {
            results.push(`  - [generator] ${m[1]} (Line ${i + 1})`);
        }
    }
    return results;
}

function regexParsePython(code, filePath) {
    const results = [];
    const lines = code.split('\n');
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        let m;
        if ((m = line.match(/^(?:async\s+)?def\s+(\w+)/))) {
            results.push(`  - [def] ${m[1]} (Line ${i + 1})`);
        } else if ((m = line.match(/^class\s+(\w+)/))) {
            results.push(`  - [class] ${m[1]} (Line ${i + 1})`);
        }
    }
    return results;
}

// ============================================
// TREE-SITTER ПАРСЕР
// ============================================
function treesitterParse(code, ext) {
    const langModule = grammars[ext];
    if (!langModule) return null;
    
    try {
        const parser = new Parser();
        parser.setLanguage(langModule);
        const tree = parser.parse(code);
        return extractStructure(tree.rootNode, 0, ext);
    } catch (e) {
        log(`  ❌ Ошибка tree-sitter парсинга (${ext}): ${e.message.split('\n')[0]}`);
        return null;
    }
}

function extractStructure(node, depth, lang) {
    let results = [];
    const indent = '  '.repeat(depth);
    
    const isRust = lang === '.rs';
    const isJS = ['.js', '.ts', '.tsx', '.jsx', '.cjs', '.mjs'].includes(lang);

    let name = "anonymous";
    let isInteresting = false;
    let typeStr = "";

    if (isRust) {
        if (['function_item', 'struct_item', 'enum_item', 'trait_item'].includes(node.type)) {
            isInteresting = true;
            typeStr = node.type.replace('_item', '');
        } else if (node.type === 'impl_item') {
            isInteresting = true;
            typeStr = 'impl';
        }
    } else if (isJS) {
        if (['function_declaration', 'class_declaration', 'method_definition', 'interface_declaration', 'type_alias_declaration'].includes(node.type)) {
            isInteresting = true;
            typeStr = node.type.replace('_declaration', '').replace('_definition', '');
        }
    }

    if (isInteresting) {
        for (let i = 0; i < node.childCount; i++) {
            const child = node.child(i);
            if (['identifier', 'type_identifier', 'property_identifier'].includes(child.type)) {
                name = child.text;
                break;
            }
        }
        if (node.type === 'impl_item') {
            for (let i = 0; i < node.childCount; i++) {
                const child = node.child(i);
                if (child.type === 'type_identifier' || child.type === 'scoped_type_identifier') name = child.text;
            }
        }
        results.push(`${indent}- [${typeStr}] ${name} (Line ${node.startPosition.row + 1})`);
        depth += 1;
    }

    for (let i = 0; i < node.childCount; i++) {
        results = results.concat(extractStructure(node.child(i), depth, lang));
    }
    return results;
}

// ============================================
// УНИВЕРСАЛЬНЫЙ ПАРСЕР ФАЙЛА
// ============================================
function parseFile(filePath) {
    const ext = path.extname(filePath).toLowerCase();
    const supportedExts = ['.rs', '.js', '.ts', '.tsx', '.jsx', '.cjs', '.mjs', '.py'];
    
    if (!supportedExts.includes(ext)) return null;
    
    let code;
    try {
        code = fs.readFileSync(filePath, 'utf8');
    } catch (e) {
        log(`  ❌ Не удалось прочитать файл ${filePath}: ${e.message}`);
        return null;
    }
    
    if (!code.trim()) return null;
    
    // Пробуем tree-sitter сначала
    if (!useFallback && grammars[ext]) {
        const tsResult = treesitterParse(code, ext);
        if (tsResult && tsResult.length > 0) return tsResult;
        // Если tree-sitter дал 0 результатов — пробуем регекс
    }
    
    // Регекс-фоллбэк
    if (ext === '.rs') return regexParseRust(code, filePath);
    if (['.js', '.ts', '.tsx', '.jsx', '.cjs', '.mjs'].includes(ext)) return regexParseJS(code, filePath);
    if (ext === '.py') return regexParsePython(code, filePath);
    
    return null;
}

// ============================================
// ОСНОВНАЯ ЛОГИКА
// ============================================
function processPathAndSave(targetPath) {
    const absPath = path.resolve(targetPath);
    log(`--- Запрос генерации карты ---`);
    log(`Запрошенный путь: ${targetPath}`);
    log(`Абсолютный путь: ${absPath}`);
    
    let stats;
    try {
        stats = fs.statSync(absPath);
    } catch (e) {
        const msg = `❌ Путь не существует или недоступен: ${absPath}\nОшибка: ${e.message}`;
        log(msg);
        return msg;
    }
    
    let output = "";
    let saveDir = "";
    let folderName = "";

    const supportedExts = ['.rs', '.js', '.ts', '.tsx', '.jsx', '.cjs', '.mjs', '.py'];
    let allFiles = [];
    let supportedFiles = [];
    let extCounts = {};
    let parsedCount = 0;
    let parseErrors = 0;

    if (stats.isDirectory()) {
        output = `# AST Map for directory: ${absPath}\n\n`;
        allFiles = walkSync(absPath);
        log(`Всего файлов найдено (без исключений): ${allFiles.length}`);
        
        for (const f of allFiles) {
            const ext = path.extname(f).toLowerCase();
            extCounts[ext] = (extCounts[ext] || 0) + 1;
            if (supportedExts.includes(ext)) {
                supportedFiles.push(f);
            }
        }
        
        const extSummary = Object.entries(extCounts).sort((a,b) => b[1] - a[1]).slice(0, 20).map(([e,c]) => `${e}: ${c}`).join(', ');
        log(`Расширения файлов: ${extSummary}`);
        log(`Поддерживаемых файлов: ${supportedFiles.length}`);
        
        for (const f of supportedFiles) {
            const relPath = path.relative(absPath, f);
            try {
                const res = parseFile(f);
                if (res && res.length > 0) {
                    output += `### ${relPath}\n${res.join('\n')}\n\n`;
                    parsedCount++;
                } else {
                    parseErrors++;
                }
            } catch (e) {
                parseErrors++;
                log(`  ❌ Ошибка парсинга ${relPath}: ${e.message}`);
            }
        }
        
        saveDir = absPath;
        folderName = path.basename(absPath);
    } else {
        const ext = path.extname(absPath).toLowerCase();
        if (supportedExts.includes(ext)) {
            supportedFiles.push(absPath);
            const res = parseFile(absPath);
            if (res && res.length > 0) {
                output = `### ${path.basename(absPath)}\n${res.join('\n')}\n`;
                parsedCount++;
            }
        }
        saveDir = path.dirname(absPath);
        folderName = path.basename(saveDir);
    }

    log(`--- Результат ---`);
    log(`Поддерживаемых файлов: ${supportedFiles.length}`);
    log(`Успешно распарсено: ${parsedCount}`);
    log(`Пустых файлов (нет определений): ${parseErrors}`);
    log(`Режим парсинга: ${useFallback ? 'РЕГЕКС-ФОЛЛБЭК' : 'tree-sitter'}`);

    if (parsedCount === 0) {
        let msg = `❌ Не удалось создать карту проекта.\n`;
        msg += `Путь: ${absPath}\n`;
        msg += `Всего файлов найдено: ${allFiles.length}\n`;
        msg += `Поддерживаемых файлов (.rs/.js/.ts/.tsx/.jsx/.py): ${supportedFiles.length}\n`;
        msg += `Успешно распарсено: 0\n\n`;
        
        if (supportedFiles.length === 0) {
            msg += `Причина: В проекте нет файлов с поддерживаемыми расширениями.\n`;
            if (Object.keys(extCounts).length > 0) {
                const extList = Object.entries(extCounts).sort((a,b) => b[1] - a[1]).slice(0, 15).map(([e,c]) => `${e} (${c} шт.)`).join(', ');
                msg += `Найденные расширения: ${extList}\n`;
            }
        } else {
            msg += `Причина: Файлы найдены, но парсинг не дал результатов.\n`;
            msg += `Режим парсинга: ${useFallback ? 'РЕГЕКС-ФОЛЛБЭК (tree-sitter недоступен!)' : 'tree-sitter'}\n`;
        }
        
        if (treeSitterErrors.length > 0) {
            msg += `\n⚠️ Ошибки загрузки tree-sitter:\n`;
            treeSitterErrors.forEach(e => msg += `  - ${e}\n`);
            msg += `\nnode_modules: ${nmResult.found || 'НЕ НАЙДЕН!'}\n`;
        }
        
        log(msg);
        return msg;
    }

    const fileName = `${folderName}_ast_map.md`;
    const savePath = path.join(saveDir, fileName);
    
    try {
        fs.writeFileSync(savePath, output, 'utf8');
    } catch (e) {
        const msg = `❌ Карта сгенерирована, но НЕ СОХРАНЕНА!\nОшибка записи в ${savePath}: ${e.message}\n\nВот содержимое карты:\n\n${output.substring(0, 3000)}...`;
        log(msg);
        return msg;
    }
    
    let resultMsg = `✅ Карта проекта сгенерирована!\n`;
    resultMsg += `Файл: ${savePath}\n`;
    resultMsg += `Распарсено файлов: ${parsedCount} из ${supportedFiles.length} поддерживаемых\n`;
    resultMsg += `Режим: ${useFallback ? '📋 Регекс (tree-sitter недоступен)' : '🌳 Tree-sitter'}\n`;
    
    if (parseErrors > 0) {
        resultMsg += `ℹ️ Файлов без определений (пустые/конфиги): ${parseErrors}\n`;
    }
    
    log(resultMsg);
    return resultMsg;
}

// ============================================
// РАСШИРЕННЫЙ СПИСОК ИСКЛЮЧЕНИЙ
// ============================================
const SKIP_DIRS = new Set([
    // JS/TS экосистема
    'node_modules', 'dist', '.next', '.nuxt', '.cache', '.parcel-cache', 
    'coverage', '.coverage', 'out', '.output', '.svelte-kit', '.vercel',
    // Rust экосистема
    'target',
    // Python экосистема
    'venv', '.venv', 'env', '__pycache__', '.tox', '.mypy_cache', 
    '.pytest_cache', '.ruff_cache', 'site-packages', 'egg-info', '.eggs',
    'htmlcov', '.hypothesis',
    // Go
    'vendor', 
    // Java/Kotlin
    'build', '.gradle', '.mvn',
    // C/C++
    'cmake-build-', '.cmake',
    // Общие
    '.git', '.svn', '.hg', '.idea', '.vscode', '.vs',
    'Debug', 'Release', 'x64', 'x86', 'Win32',
    'DerivedData', '.dart_tool',
    // Docker/CI
    '.docker', '.github',
    // Мусор
    'temp', 'tmp', '.temp', '.tmp', '.trash',
    // Документация-билды
    '_site', '_build', 'docusaurus',
]);

const SKIP_FILES_STARTING_WITH = [
    '.min.', '.bundle.', '.chunk.', '.vendor.',
];

function walkSync(dir, filelist) {
    filelist = filelist || [];
    let files;
    try {
        files = fs.readdirSync(dir);
    } catch (e) {
        return filelist;
    }
    
    for (const file of files) {
        // Пропускаем файлы и папки начинающиеся с точки (кроме .rs, .ts и т.д.)
        if (file.startsWith('.') && !file.match(/\.\w+$/)) continue;
        
        const dirFile = path.join(dir, file);
        try {
            const stat = fs.statSync(dirFile);
            if (stat.isDirectory()) {
                if (!SKIP_DIRS.has(file.toLowerCase()) && !SKIP_DIRS.has(file)) {
                    filelist = walkSync(dirFile, filelist);
                }
            } else {
                // Пропускаем минифицированные и бандл-файлы
                const shouldSkip = SKIP_FILES_STARTING_WITH.some(prefix => file.includes(prefix));
                if (!shouldSkip) {
                    filelist.push(dirFile);
                }
            }
        } catch (err) {}
    }
    return filelist;
}

log(`✅ Сервер готов к приему запросов. Режим: ${useFallback ? 'РЕГЕКС-ФОЛЛБЭК' : 'TREE-SITTER'}`);