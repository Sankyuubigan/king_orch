// Конвертер QuixBugs (Группа A): 40 классических задач bugfix из jkoppel/QuixBugs.
// 31 обычная программа — тесты json_testcases/<name>.json (построчные [input, expected]);
// 9 графовых (Node-графы) — assert-тесты python_testcases/test_<name>.py, механическая
// трансплантация (вырезается pytest-обёртка и условный импорт реализации, функция
// уже определена в main.py). node.py из репо кладётся в песочницу через files.
// Ожидаемые значения — ТОЛЬКО из данных репо (§00.3.1: без перепечатки по памяти).
const { writeJsonl, signatureRe } = require('./util.cjs');

const RAW = 'https://raw.githubusercontent.com/jkoppel/QuixBugs/master';
const SUITE = 'quixbugs_python';

// 9 графовых программ (тесты не в json_testcases, а в python_testcases/).
const GRAPH_PROGRAMS = [
    'breadth_first_search',
    'depth_first_search',
    'detect_cycle',
    'minimum_spanning_tree',
    'reverse_linked_list',
    'shortest_path_length',
    'shortest_path_lengths',
    'shortest_paths',
    'topological_ordering',
];

async function fetchText(url) {
    const resp = await fetch(url);
    if (!resp.ok) throw new Error(`HTTP ${resp.status}: ${url}`);
    return resp.text();
}

// Список обычных программ = имена файлов json_testcases/*.json (через GitHub API).
async function listJsonTestcases() {
    const resp = await fetch('https://api.github.com/repos/jkoppel/QuixBugs/contents/json_testcases');
    if (!resp.ok) throw new Error(`GitHub API json_testcases: HTTP ${resp.status}`);
    const entries = await resp.json();
    return entries.filter((e) => e.name.endsWith('.json')).map((e) => e.name.replace(/\.json$/, ''));
}

// JSON-значение → python-литерал (True/False/None, без наивных replace по строке).
function pyLiteral(v) {
    if (v === null || v === undefined) return 'None';
    if (v === true) return 'True';
    if (v === false) return 'False';
    if (typeof v === 'number') return String(v);
    if (typeof v === 'string') return JSON.stringify(v);
    if (Array.isArray(v)) return '[' + v.map(pyLiteral).join(', ') + ']';
    throw new Error(`Неизвестный тип тест-кейса: ${typeof v}`);
}

// json_testcases/<name>.json: построчные [input_list, expected].
function parseTestcases(raw) {
    return raw
        .split('\n')
        .filter((l) => l.trim())
        .map((l) => JSON.parse(l));
}

// Харнесс обычных программ: перебор тест-кейсов, вызов prog(*input) с deepcopy
// (как tester.py), генераторы материализуются в list(...), assert с деталью.
function jsonHarness(name, cases) {
    return [
        'import copy as _copy',
        '',
        '_cases = ' + pyLiteral(cases),
        '',
        '',
        'for _inp, _exp in _cases:',
        '    _got = ' + name + '(*_copy.deepcopy(_inp))',
        '    if hasattr(_got, "__next__"):',
        '        _got = list(_got)',
        '    assert _got == _exp, "FAIL: input=%r got=%r expected=%r" % (_inp, _got, _exp)',
        'print("OK")',
        '',
    ].join('\n');
}

// Трансплантация assert-тестов графовых программ: вырезается pytest-обёртка
// (import pytest, условный импорт реализации), тест-функции вызываются ранером.
function transplantGraphTests(raw) {
    const lines = raw.split('\n').filter((l) => {
        const t = l.trim();
        if (t === 'import pytest') return false;
        if (t === 'if pytest.use_correct:') return false;
        if (t === 'else:') return false;
        if (t.startsWith('from correct_python_programs.')) return false;
        if (t.startsWith('from python_programs.')) return false;
        return true;
    });
    const head = /Node/.test(raw) ? ['from node import Node', ''] : [];
    const runner = [
        '',
        'for _k, _t in list(sorted(globals().items())):',
        '    if _k.startswith("test") and callable(_t):',
        '        _t()',
        'print("OK")',
        '',
    ];
    return [...head, ...lines, ...runner].join('\n');
}

function bugfixTask(name, modelPrompt, test, files) {
    return {
        id: `${SUITE}_${name}`,
        suite: SUITE,
        language: 'python',
        category: 'bugfix',
        run_with: 'python',
        model_prompt: modelPrompt,
        solution_name: 'main.py',
        prefix: null, // заполняется вызывающим
        entry_point: name,
        signature_re: signatureRe('python', name),
        test,
        run_cmd: 'python main.py',
        max_tokens: 512,
        temperature: 0,
        timeout_sec: 120, // knapsack: последний тест-кейс медленный
        files: files || [],
    };
}

function fixPrompt(buggy) {
    return (
        'The following Python function has a bug.\n\n```python\n' +
        buggy +
        '```\n\nFix the bug. Return the complete corrected function definition (signature and body). ' +
        'Only code, no explanations, no markdown fences.'
    );
}

async function convert() {
    const nodePy = await fetchText(`${RAW}/python_programs/node.py`);
    const plainNames = await listJsonTestcases();
    const tasks = [];

    // Обычные программы: json_testcases.
    for (const name of plainNames) {
        const buggy = await fetchText(`${RAW}/python_programs/${name}.py`);
        const cases = parseTestcases(await fetchText(`${RAW}/json_testcases/${name}.json`));
        const t = bugfixTask(name, fixPrompt(buggy), jsonHarness(name, cases));
        t.prefix = buggy;
        tasks.push(t);
    }

    // Графовые программы: assert-тесты python_testcases + node.py в files.
    for (const name of GRAPH_PROGRAMS) {
        const buggy = await fetchText(`${RAW}/python_programs/${name}.py`);
        const tests = await fetchText(`${RAW}/python_testcases/test_${name}.py`);
        const t = bugfixTask(name, fixPrompt(buggy), transplantGraphTests(tests), [
            { name: 'node.py', content: nodePy },
        ]);
        t.prefix = buggy;
        tasks.push(t);
    }

    const n = writeJsonl(`${__dirname}/../../tasks_for_test_llm/${SUITE}`, tasks);
    console.log(`[QuixBugs] ${SUITE}: ${n} задач (${plainNames.length} json + ${GRAPH_PROGRAMS.length} графовых)`);
}

module.exports = { convert };
