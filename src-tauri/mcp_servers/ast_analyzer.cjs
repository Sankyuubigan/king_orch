const fs = require('fs');
const path = require('path');
const readline = require('readline');

function log(msg) { process.stderr.write(`[AST-ANALYZER] ${msg}\n`); }
log('=== AST Analyzer MCP v1.0 ===');

// === Node Modules Resolution ===
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

const { createMcpServer } = require('./mcp_base.cjs');

createMcpServer({
    name: "ast-analyzer-mcp",
    version: "1.0.0",
    tools: [
        {
            name: "search_code",
            description: "Поиск кода по ключевым словам с определением функции-контейнера и окружающим контекстом. Ищет по всем файлам проекта.",
            inputSchema: { type: "object", properties: { target_path: { type: "string" }, query: { type: "string" } }, required: ["target_path", "query"] }
        },
        {
            name: "analyze_file",
            description: "Глубокий анализ файла: импорты, функции, граф вызовов, обработка ошибок, подозрительные паттерны",
            inputSchema: { type: "object", properties: { file_path: { type: "string" } }, required: ["file_path"] }
        },
        {
            name: "trace_function",
            description: "Трассировка функции: где определена, кто её вызывает, что она вызывает сама.",
            inputSchema: { type: "object", properties: { target_path: { type: "string" }, function_name: { type: "string" } }, required: ["target_path", "function_name"] }
        }
    ],
    handlers: {
        search_code: (args) => searchCode(args.target_path, args.query),
        analyze_file: (args) => analyzeFile(args.file_path),
        trace_function: (args) => traceFunction(args.target_path, args.function_name)
    }
});

// === Gitignore ===
function loadGitignore(rootDir) {
    const rules = [];
    try {
        for (let raw of fs.readFileSync(path.join(rootDir, '.gitignore'), 'utf8').split('\n')) {
            let line = raw.trim();
            if (!line || line.startsWith('#')) continue;
            const neg = line.startsWith('!'); if (neg) line = line.substring(1);
            const dirOnly = line.endsWith('/'); if (dirOnly) line = line.slice(0, -1);
            if (line) rules.push({ pattern: line, neg, dirOnly });
        }
    } catch(e) {}
    return rules;
}

function isIgnored(relPath, name, isDir, rules) {
    let result = false;
    for (const r of rules) {
        if (r.dirOnly && !isDir) continue;
        const p = r.pattern;
        if (p.startsWith('/')) {
            const rp = p.substring(1);
            if (relPath === rp || relPath.startsWith(rp + '/')) result = !r.neg;
        } else if (p.includes('*')) {
            const re = new RegExp('^' + p.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '[^/]*') + '$');
            if (re.test(name) || re.test(relPath)) result = !r.neg;
        } else {
            if (name === p || relPath === p || relPath.startsWith(p + '/') || relPath.endsWith('/' + p)) result = !r.neg;
        }
    }
    return result;
}

// === File Scanning ===
const SKIP_DIRS = new Set(['node_modules','dist','.git','.svn','.hg','.idea','.vscode','target','build','.agents_workspace']);
const CODE_EXTS = new Set(['.rs','.js','.ts','.tsx','.jsx','.cjs','.mjs','.py']);

function collectCodeFiles(dirPath, rootDir, gitRules) {
    let files = [], entries;
    try { entries = fs.readdirSync(dirPath); } catch(e) { return files; }
    const relDir = path.relative(rootDir, dirPath).replace(/\\/g, '/');
    for (const entry of entries) {
        if (entry.startsWith('.') && !entry.match(/\.\w+$/)) continue;
        const fullPath = path.join(dirPath, entry);
        const relPath = relDir ? relDir + '/' + entry : entry;
        let stat; try { stat = fs.statSync(fullPath); } catch(e) { continue; }
        if (stat.isDirectory()) {
            if (isIgnored(relPath, entry, true, gitRules)) continue;
            if (SKIP_DIRS.has(entry) || SKIP_DIRS.has(entry.toLowerCase())) continue;
            files = files.concat(collectCodeFiles(fullPath, rootDir, gitRules));
        } else if (CODE_EXTS.has(path.extname(entry).toLowerCase())) {
            if (!isIgnored(relPath, entry, false, gitRules)) files.push({ relPath, fullPath });
        }
    }
    return files;
}

// === Regex-based Function Extraction ===
const JS_EXTS = ['.js','.ts','.tsx','.jsx','.cjs','.mjs'];

function extractFunctions(code, ext) {
    const funcs = [], lines = code.split('\n');
    if (ext === '.rs') {
        for (let i = 0; i < lines.length; i++) { const s = lines[i].trim(); let m;
            if ((m=s.match(/^(?:pub\s+)?(?:async\s+)?fn\s+(\w+)/))) funcs.push({name:m[1],line:i+1,type:'fn'});
            else if ((m=s.match(/^(?:pub\s+)?struct\s+(\w+)/))) funcs.push({name:m[1],line:i+1,type:'struct'});
            else if ((m=s.match(/^(?:pub\s+)?impl(?:\s+<[^>]+>)?\s+(?:\w+\s+for\s+)?(\w+)/))) funcs.push({name:m[1],line:i+1,type:'impl'});
        }
    } else if (JS_EXTS.includes(ext)) {
        for (let i = 0; i < lines.length; i++) { const s = lines[i].trim(); let m;
            if ((m=s.match(/^(?:export\s+)?(?:async\s+)?function\s+(\w+)/))) funcs.push({name:m[1],line:i+1,type:'fn'});
            else if ((m=s.match(/^(?:export\s+)?(?:default\s+)?class\s+(\w+)/))) funcs.push({name:m[1],line:i+1,type:'class'});
            else if ((m=s.match(/^(?:export\s+)?const\s+(\w+)\s*=\s*(?:async\s+)?(?:\([^)]*\)|[\w]+)\s*=>/))) funcs.push({name:m[1],line:i+1,type:'arrow_fn'});
        }
    } else if (ext === '.py') {
        for (let i = 0; i < lines.length; i++) { const s = lines[i]; let m;
            if ((m=s.match(/^(?:async\s+)?def\s+(\w+)/))) funcs.push({name:m[1],line:i+1,type:'def'});
            else if ((m=s.match(/^class\s+(\w+)/))) funcs.push({name:m[1],line:i+1,type:'class'});
        }
    }
    // Вычисляем endLine для каждой функции
    for (let i = 0; i < funcs.length; i++) {
        funcs[i].endLine = i < funcs.length - 1 ? funcs[i + 1].line - 1 : lines.length;
    }
    return funcs;
}

// === Tool: search_code ===
function searchCode(targetPath, query) {
    const absPath = path.resolve(targetPath);
    if (!fs.existsSync(absPath)) return `❌ Путь не существует: ${absPath}`;
    const gitRules = loadGitignore(absPath);
    const codeFiles = collectCodeFiles(absPath, absPath, gitRules);
    const queryTerms = query.toLowerCase().split(/\s+/).filter(t => t.length > 1);
    if (queryTerms.length === 0) return '❌ Пустой поисковый запрос';

    // Строим индекс функций для всех файлов
    const funcIndex = {};
    for (const f of codeFiles) {
        try {
            const code = fs.readFileSync(f.fullPath, 'utf8');
            const ext = path.extname(f.fullPath).toLowerCase();
            const funcs = extractFunctions(code, ext);
            if (funcs.length > 0) funcIndex[f.relPath] = funcs;
        } catch(e) {}
    }

    const results = [];
    for (const f of codeFiles) {
        let code; try { code = fs.readFileSync(f.fullPath, 'utf8'); } catch(e) { continue; }
        const lines = code.split('\n');
        for (let i = 0; i < lines.length; i++) {
            const lineLower = lines[i].toLowerCase();
            let score = 0;
            for (const term of queryTerms) {
                if (lineLower.includes(term)) {
                    score += 1;
                    try {
                        const re = new RegExp('\\b' + term.replace(/[-\/\\^$*+?.()|[\]{}]/g, '\\$&') + '\\b', 'i');
                        if (re.test(lines[i])) score += 2;
                    } catch(e) {}
                }
            }
            if (score > 0) {
                let currentFunc = '(глобальная область)';
                if (funcIndex[f.relPath]) {
                    for (let j = funcIndex[f.relPath].length - 1; j >= 0; j--) {
                        if (funcIndex[f.relPath][j].line <= i + 1) {
                            currentFunc = `${funcIndex[f.relPath][j].type} ${funcIndex[f.relPath][j].name}`;
                            break;
                        }
                    }
                }
                const cs = Math.max(0, i - 2), ce = Math.min(lines.length - 1, i + 2);
                const context = lines.slice(cs, ce + 1).map((l, idx) => `${cs + idx + 1}: ${l}`).join('\n');
                results.push({ file: f.relPath, line: i + 1, score, currentFunc, context });
            }
        }
    }

    results.sort((a, b) => b.score - a.score);
    const top = results.slice(0, 20);
    if (top.length === 0) return `🔍 По запросу "${query}" ничего не найдено.`;
    let out = `🔍 Результаты поиска "${query}" (${top.length} из ${results.length}):\n\n`;
    for (const r of top) {
        out += `📄 ${r.file} → L${r.line} внутри [${r.currentFunc}] (релевантность: ${r.score})\n${r.context}\n\n`;
    }
    return out;
}

// === Tool: analyze_file ===
function analyzeFile(filePath) {
    const absPath = path.resolve(filePath);
    let code; try { code = fs.readFileSync(absPath, 'utf8'); } catch(e) { return `❌ Не удалось прочитать: ${e.message}`; }
    const ext = path.extname(absPath).toLowerCase();
    const lines = code.split('\n');
    const fileName = path.basename(absPath);

    let out = `📋 Анализ: ${fileName} (${lines.length} строк, ${ext})\n\n`;

    // 1. Imports
    const imports = [];
    for (let i = 0; i < lines.length; i++) {
        const s = lines[i].trim();
        if (ext === '.rs' && /^(use|mod)\s/.test(s)) imports.push(`  L${i+1}: ${s}`);
        else if (JS_EXTS.includes(ext) && /^(import\s|const\s+.*require\()/.test(s)) imports.push(`  L${i+1}: ${s}`);
        else if (ext === '.py' && /^(import\s|from\s+)/.test(s)) imports.push(`  L${i+1}: ${s}`);
    }
    if (imports.length > 0) out += `📦 Импорты (${imports.length}):\n${imports.join('\n')}\n\n`;

    // 2. Functions
    const funcs = extractFunctions(code, ext);
    if (funcs.length > 0) {
        out += `🔧 Функции/структуры (${funcs.length}):\n`;
        for (const f of funcs) out += `  [${f.type}] ${f.name} (L${f.line}-${f.endLine})\n`;
        out += '\n';
    }

    // 3. Error handling
    const errs = [];
    for (let i = 0; i < lines.length; i++) {
        const s = lines[i];
        if (/\b(try|catch|except|unwrap|expect\(|panic!|throw|raise|\.map_err|anyhow|resolve|reject)\b/.test(s)) {
            errs.push(`  L${i+1}: ${s.trim()}`);
        }
    }
    if (errs.length > 0) out += `⚠️ Обработка ошибок (${errs.length}):\n${errs.slice(0, 40).join('\n')}\n\n`;

    // 4. Call graph
    if (funcs.length > 0 && ['.rs', ...JS_EXTS].includes(ext)) {
        out += `🔗 Граф вызовов:\n`;
        const stdRust = new Set(['if','for','while','match','let','mut','pub','fn','struct','enum','impl','use','mod','self','super','crate','return','break','continue','as','ref','move','async','await','unsafe','static','const','type','trait','Some','None','Ok','Err','Vec','String','Box','Arc','Rc','Option','Result','println','eprintln','format','to_string','clone','into','from','new','default','as_ref','as_mut','unwrap_or','unwrap_or_else','is_some','is_none','is_ok','is_err','iter','len','push','pop','get','insert','remove','contains','extend','collect','map','filter','for_each','and_then','or_else','spawn','lock','unwrap','drop','copy','read','write','flush','open','close','seek','connect','send','recv','read_to_string','write_all','create_dir','read_dir','exists','metadata','emit','serialize','deserialize','parse','from_str','to_owned','borrow','deref']);
        const stdJS = new Set(['if','for','while','switch','return','new','typeof','const','let','var','function','class','async','await','try','catch','throw','else','import','export','from','require','true','false','null','undefined','console','this','Math','JSON','Object','Array','String','Number','Promise','Error','document','window','log','warn','error','info','debug','push','pop','shift','unshift','map','filter','reduce','forEach','find','includes','indexOf','join','split','slice','splice','concat','length','keys','values','entries','has','get','set','delete','toString','valueOf','parseInt','parseFloat','isNaN','isFinite','setTimeout','setInterval','clearTimeout','clearInterval','addEventListener','removeEventListener','querySelector','getElementById','createElement','appendChild','removeChild','replaceChild','insertBefore','cloneNode','setAttribute','getAttribute','classList','style','innerHTML','innerText','textContent','appendChild','requestAnimationFrame','fetch','then','catch','finally','resolve','reject','all','race','allSettled','any']);

        for (const func of funcs) {
            const funcCode = lines.slice(func.line - 1, func.endLine).join('\n');
            const calls = new Set();
            let m;
            if (ext === '.rs') {
                const re = /(?:::(\w+)\s*\(|\.(\w+)\s*\(|(?<![:\.\w])(\w+)\s*\()/g;
                while ((m = re.exec(funcCode)) !== null) {
                    const n = m[1] || m[2] || m[3];
                    if (n && !stdRust.has(n) && n !== func.name && n.length > 1) calls.add(n);
                }
            } else {
                const re = /(?:\.(\w+)\s*\()/g;
                while ((m = re.exec(funcCode)) !== null) {
                    const n = m[1];
                    if (n && !stdJS.has(n) && n !== func.name && n.length > 1) calls.add(n);
                }
                // Также ищем прямые вызовы функций (не методов)
                const re2 = /(?<![.\w])([a-z_]\w*)\s*\(/gi;
                while ((m = re2.exec(funcCode)) !== null) {
                    const n = m[1];
                    if (n && !stdJS.has(n) && n !== func.name && n.length > 1 && !calls.has(n)) calls.add(n);
                }
            }
            if (calls.size > 0) out += `  ${func.name} → ${[...calls].join(', ')}\n`;
        }
        out += '\n';
    }

    // 5. Suspicious patterns
    const suspicious = [];
    for (let i = 0; i < lines.length; i++) {
        const s = lines[i];
        if (ext === '.rs' && /\.unwrap\(\)/.test(s) && !/\.unwrap_or/.test(s))
            suspicious.push(`  L${i+1}: [unwrap без обработки] ${s.trim()}`);
        if (/catch\s*\(\s*\w*\s*\)\s*\{\s*\}/.test(s))
            suspicious.push(`  L${i+1}: [пустой catch] ${s.trim()}`);
        if (/\.catch\(\s*\(\s*\)\s*=>\s*\{\s*\}/.test(s))
            suspicious.push(`  L${i+1}: [пустой .catch] ${s.trim()}`);
        if (/\bas\s+any\b/.test(s))
            suspicious.push(`  L${i+1}: [any cast — потеря типизации] ${s.trim()}`);
        if (ext === '.rs' && /\.expect\(\s*\)/.test(s))
            suspicious.push(`  L${i+1}: [expect без сообщения] ${s.trim()}`);
        if (JS_EXTS.includes(ext) && /[^=!]==[^=]/.test(s) && !/==\s*(null|undefined)/.test(s))
            suspicious.push(`  L${i+1}: [== вместо ===] ${s.trim()}`);
        if (/TODO|FIXME|HACK|XXX|BUG/.test(s))
            suspicious.push(`  L${i+1}: [маркер проблемы] ${s.trim()}`);
    }
    if (suspicious.length > 0) out += `🚨 Подозрительные паттерны (${suspicious.length}):\n${suspicious.slice(0, 30).join('\n')}\n\n`;

    return out;
}

// === Tool: trace_function ===
function traceFunction(targetPath, functionName) {
    const absPath = path.resolve(targetPath);
    if (!fs.existsSync(absPath)) return `❌ Путь не существует: ${absPath}`;
    const gitRules = loadGitignore(absPath);
    const codeFiles = collectCodeFiles(absPath, absPath, gitRules);

    const definitions = [];
    const callers = [];
    const calleesByDef = {};

    for (const f of codeFiles) {
        let code; try { code = fs.readFileSync(f.fullPath, 'utf8'); } catch(e) { continue; }
        const ext = path.extname(f.fullPath).toLowerCase();
        const lines = code.split('\n');
        const funcs = extractFunctions(code, ext);

        // Ищем определение функции
        for (const func of funcs) {
            if (func.name === functionName) {
                definitions.push({ file: f.relPath, line: func.line, endLine: func.endLine, type: func.type });
                // Извлекаем вызовы из тела функции
                const funcCode = lines.slice(func.line - 1, func.endLine).join('\n');
                const calls = new Set();
                const re = ext === '.rs'
                    ? /(?:::(\w+)\s*\(|\.(\w+)\s*\(|(?<![:\.\w])(\w+)\s*\()/g
                    : /(?:\.(\w+)\s*\(|(?<![.\w])([a-z_]\w*)\s*\()/gi;
                let m;
                while ((m = re.exec(funcCode)) !== null) {
                    const n = m[1] || m[2] || m[3];
                    if (n && n !== functionName && n.length > 1) calls.add(n);
                }
                calleesByDef[`${f.relPath}:${func.line}`] = [...calls];
            }
        }

        // Ищем вызовы функции (callers)
        const escaped = functionName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
        const callRe = new RegExp('\\b' + escaped + '\\s*\\(', 'g');
        for (let i = 0; i < lines.length; i++) {
            callRe.lastIndex = 0;
            if (callRe.test(lines[i])) {
                // Не считаем строку определения как вызов
                const isDefLine = funcs.some(fn => fn.name === functionName && fn.line === i + 1);
                if (!isDefLine) {
                    let callerFunc = '(глобальная область)';
                    for (let j = funcs.length - 1; j >= 0; j--) {
                        if (funcs[j].line <= i + 1 && funcs[j].endLine >= i + 1) {
                            callerFunc = `${funcs[j].type} ${funcs[j].name}`;
                            break;
                        }
                    }
                    callers.push({ file: f.relPath, line: i + 1, callerFunc, code: lines[i].trim() });
                }
            }
        }
    }

    let out = `🔎 Трассировка: ${functionName}\n\n`;

    if (definitions.length > 0) {
        out += `📍 Определения (${definitions.length}):\n`;
        for (const d of definitions) {
            out += `  ${d.file} L${d.line}-${d.endLine} [${d.type}]\n`;
            const callees = calleesByDef[`${d.file}:${d.line}`];
            if (callees && callees.length > 0) out += `    → вызывает: ${callees.join(', ')}\n`;
        }
        out += '\n';
    } else {
        out += `❌ Функция "${functionName}" не найдена в проекте.\n\n`;
    }

    if (callers.length > 0) {
        out += `📞 Вызывается из (${callers.length}):\n`;
        for (const c of callers.slice(0, 30)) {
            out += `  ${c.file} L${c.line} внутри [${c.callerFunc}]\n    ${c.code}\n`;
        }
        out += '\n';
    } else if (definitions.length > 0) {
        out += `📞 Вызовов не найдено (возможно, неиспользуемая функция?)\n\n`;
    }

    return out;
}

log('✅ AST Analyzer готов');