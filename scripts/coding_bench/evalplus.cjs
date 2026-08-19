// Конвертер evalplus: humanevalplus и mbppplus (python, категория codegen).
const { hfRows, writeJsonl, codeBlock, signatureRe } = require('./util.cjs');

const SUITES = [
    { dataset: 'evalplus/humanevalplus', suite: 'humanevalplus_python', name: 'HumanEval+' },
    { dataset: 'evalplus/mbppplus', suite: 'mbppplus_python', name: 'MBPP+' },
];

async function convert() {
    for (const s of SUITES) {
        const rows = await hfRows(s.dataset, 'default', 'test');
        const tasks = rows.map((row, i) => {
            const ep = String(row.entry_point || '');
            const prompt = String(row.prompt || '');
            // MBPP+: сигнатура извлекается из канонического решения.
            let prefix = prompt;
            if (s.name === 'MBPP+') {
                const code = String(row.code || '');
                const m = code.match(/^(def\s+[A-Za-z_]\w*\s*\([^)]*\)\s*->?[^:]*:\s*)$/m);
                prefix = m ? m[1] : prompt;
            }
            const modelPrompt =
                s.name === 'MBPP+'
                    ? `${row.prompt || ''}\n\nReturn the complete function definition. Only code, no explanations, no markdown fences.`
                    : `Complete the following Python function. Return only the function body, ` +
                      `no explanations, no markdown fences.\n\n${codeBlock('python', prefix)}`;
            const task = {
                id: `${s.suite}_${i}`,
                suite: s.suite,
                language: 'python',
                category: 'codegen',
                run_with: 'python',
                model_prompt: modelPrompt,
                solution_name: 'main.py',
                prefix: prefix,
                entry_point: ep,
                signature_re: ep ? signatureRe('python', ep) : null,
                test: row.test || '',
                run_cmd: 'python main.py',
                max_tokens: 512,
                temperature: 0,
                timeout_sec: 60,
                files: [],
            };
            return task;
        });
        const n = writeJsonl(`${__dirname}/../../tasks_for_test_llm/${s.suite}`, tasks);
        console.log(`[${s.name}] ${s.suite}: ${n} задач`);
    }
}

module.exports = { convert };