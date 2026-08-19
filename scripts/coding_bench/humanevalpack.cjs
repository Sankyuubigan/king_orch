// Конвертер bigcode/humanevalpack (6 языков, категория bugfix).
const { hfRows, writeJsonl, codeBlock, signatureRe } = require('./util.cjs');

const LANGS = [
    { config: 'python', suite: 'humanevalpack_python', language: 'python', runWith: 'python', maxTokens: 512 },
    { config: 'rust', suite: 'humanevalpack_rust', language: 'rust', runWith: 'rust', maxTokens: 512 },
    { config: 'js', suite: 'humanevalpack_js', language: 'js', runWith: 'deno', maxTokens: 512 },
    { config: 'cpp', suite: 'humanevalpack_cpp', language: 'cpp', runWith: 'skip', maxTokens: 512 },
    { config: 'go', suite: 'humanevalpack_go', language: 'go', runWith: 'skip', maxTokens: 512 },
    { config: 'java', suite: 'humanevalpack_java', language: 'java', runWith: 'skip', maxTokens: 512 },
];

const RUN_CMD = {
    python: 'python main.py',
    rust: 'rustc --edition 2021 --test main.rs -o task_test.exe && task_test.exe',
    deno: 'deno run --allow-all main.ts',
    skip: null,
};

async function convert() {
    for (const l of LANGS) {
        const rows = await hfRows('bigcode/humanevalpack', l.config, 'test');
        const tasks = rows.map((row, i) => {
            const ep = String(row.entry_point || '');
            const prompt = String(row.prompt || '');
            const buggy = prompt + (row.buggy_solution || '');
            const runCmd = RUN_CMD[l.runWith];
            const modelPrompt = l.runWith === 'skip'
                ? ''
                : `${row.instruction || ''}\n\nHere is the buggy function:\n${codeBlock(l.language, buggy)}\n` +
                  `Fix the bug. Return the complete corrected function definition (signature, docstring and body). ` +
                  `Only code, no explanations, no markdown fences.`;
            const task = {
                id: `${l.suite}_${i}`,
                suite: l.suite,
                language: l.language,
                category: 'bugfix',
                run_with: l.runWith,
                model_prompt: modelPrompt,
                solution_name: l.runWith === 'rust' ? 'main.rs' : (l.runWith === 'deno' ? 'main.ts' : 'main.py'),
                prefix: prompt,
                entry_point: ep,
                signature_re: ep ? signatureRe(l.language, ep) : null,
                test: row.test || '',
                run_cmd: runCmd,
                max_tokens: l.maxTokens,
                temperature: 0,
                timeout_sec: l.runWith === 'rust' ? 180 : 60,
                files: [],
            };
            return task;
        });
        const n = writeJsonl(`${__dirname}/../../tasks_for_test_llm/${l.suite}`, tasks);
        console.log(`[humanevalpack] ${l.config}: ${n} задач`);
    }
}

module.exports = { convert };