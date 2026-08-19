// Конвертер Aider-AI/refactor-benchmark (python, категория refactor).
// Фильтр: модуль ≤ 20 КБ (влезает в контекст 8B-моделей).
const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');
const { writeJsonl, codeBlock } = require('./util.cjs');

const MAX_KB = 20;
const BENCH_REPO = 'https://github.com/Aider-AI/refactor-benchmark';
const TMP = path.join(process.env.TEMP || '/tmp', 'ko_coding_tasks');
const BENCH_DIR = path.join(TMP, 'refactor-benchmark');

function ensureClone() {
    if (!fs.existsSync(path.join(BENCH_DIR, 'README.md'))) {
        fs.mkdirSync(TMP, { recursive: true });
        console.log('[refactor] клонируем refactor-benchmark...');
        execSync(`git clone --depth 1 ${BENCH_REPO} "${BENCH_DIR}"`, { stdio: 'inherit' });
    }
}

async function convert() {
    ensureClone();
    const toolsContent = fs.readFileSync(path.join(__dirname, 'refactor_tools.py'), 'utf8');
    // Репо содержит подпапку refactor-benchmark/ с наборами задач.
    const benchRoot = fs.existsSync(path.join(BENCH_DIR, 'refactor-benchmark'))
        ? path.join(BENCH_DIR, 'refactor-benchmark')
        : BENCH_DIR;
    const tasksDir = fs.readdirSync(benchRoot, { withFileTypes: true })
        .filter((d) => d.isDirectory() && !d.name.startsWith('.') && !d.name.startsWith('bench'))
        .map((d) => d.name);

    const tasks = [];
    for (const slug of tasksDir) {
        const dir = path.join(benchRoot, slug);
        const files = fs.readdirSync(dir);
        const moduleName = files.find((f) => f.endsWith('.py') && !f.endsWith('_test.py'));
        const testName = files.find((f) => f.endsWith('_test.py'));
        if (!moduleName || !testName) continue;

        const modulePath = path.join(dir, moduleName);
        const sizeKB = fs.statSync(modulePath).size / 1024;
        if (sizeKB > MAX_KB) continue;

        const module = fs.readFileSync(modulePath, 'utf8');
        const instructions = fs.existsSync(path.join(dir, '.docs', 'instructions.md'))
            ? fs.readFileSync(path.join(dir, '.docs', 'instructions.md'), 'utf8')
            : '';

        const modelPrompt =
            `${instructions}\n\nHere is the current file ${moduleName}:\n` +
            `${codeBlock('python', module)}\n` +
            `Apply the refactoring described above to ${moduleName}. ` +
            `Return the complete updated file content. Only code, no explanations, no markdown fences.`;

        tasks.push({
            id: `refactor_python_${slug}`,
            suite: 'refactor_python',
            language: 'python',
            category: 'refactor',
            run_with: 'python',
            model_prompt: modelPrompt,
            solution_name: moduleName,
            prefix: null,
            entry_point: null,
            signature_re: null,
            test: '',
            run_cmd: `python ${testName}`,
            max_tokens: 2048,
            temperature: 0,
            timeout_sec: 120,
            files: [
                { name: path.join('benchmark', 'refactor_tools.py'), content: toolsContent },
                { name: testName, content: fs.readFileSync(path.join(dir, testName), 'utf8') },
            ],
        });
    }

    const n = writeJsonl(`${__dirname}/../../tasks_for_test_llm/refactor_python`, tasks);
    console.log(`[refactor] refactor_python: ${n} задач (модули ≤ ${MAX_KB} КБ)`);
}

module.exports = { convert };