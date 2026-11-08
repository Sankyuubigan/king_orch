// Конвертер mcq-наборов (Группа A): MMLU-подмножество, ARC-Challenge, HellaSwag,
// Winogrande, GSM8K (число), TruthfulQA MC1. Оценка без песочницы: run_with "mcq",
// извлечение буквы/числа из ответа модели (mcq.rs на стороне Rust).
// Форма ответа задаётся промптом-якорем («Answer:» / «#### N»).
const { hfRowsSampled, writeJsonl } = require('./util.cjs');

// ~500 задач на набор: stride-сэмплинг по всему датасету (равномерно, а не первые N).
const TARGET_PER_SUITE = 500;
const MMLU_PER_SUBJECT = 100;
const MMLU_SUBJECTS = [
    'abstract_algebra',
    'college_mathematics',
    'high_school_computer_science',
    'high_school_physics',
    'professional_medicine',
    'miscellaneous',
];
const LETTERS = 'ABCDEFGH';

function stride(rows, target) {
    if (rows.length <= target) return rows;
    const s = Math.max(1, Math.floor(rows.length / target));
    return rows.filter((_, i) => i % s === 0);
}

function letterPrompt(question, choices) {
    const lines = choices.map((c, i) => `${LETTERS[i]}. ${c}`).join('\n');
    return (
        'The following is a multiple choice question. Answer with the letter of the correct option.\n\n' +
        question +
        '\n\n' +
        lines +
        '\n\nAnswer:'
    );
}

function numberPrompt(question) {
    return (
        'Solve the following math problem. End your reply with the final numeric answer on its own line in the format:\n#### <number>\n\n' +
        question
    );
}

function mcqTask(suite, modelPrompt, choices, answer, mode, maxTokens) {
    return {
        id: `${suite}_placeholder`, // пересобирается по порядку набора перед записью
        suite,
        language: 'en',
        category: 'mcq',
        run_with: 'mcq',
        model_prompt: modelPrompt,
        solution_name: 'answer.txt',
        choices: choices || null,
        answer,
        mcq_mode: mode,
        test: '',
        run_cmd: null,
        max_tokens: maxTokens,
        temperature: 0,
        timeout_sec: 60,
        files: [],
    };
}

function pushLetter(tasks, suite, row, questionField, choices, answerIdx, expectLen) {
    if (answerIdx < 0 || answerIdx >= expectLen) {
        throw new Error(`${suite}: индекс ответа ${answerIdx} вне диапазона (${expectLen} вариантов)`);
    }
    tasks.push(
        mcqTask(suite, letterPrompt(String(row[questionField]), choices), choices, LETTERS[answerIdx], 'letter', 512),
    );
}

async function convertMmlu() {
    const tasks = [];
    for (const subject of MMLU_SUBJECTS) {
        const rows = await hfRowsSampled('cais/mmlu', subject, 'test', MMLU_PER_SUBJECT);
        for (const row of stride(rows, MMLU_PER_SUBJECT)) {
            const choices = row.choices.map(String);
            pushLetter(tasks, 'mmlu_subset', row, 'question', choices, Number(row.answer), choices.length);
        }
    }
    return tasks;
}

async function convertArc() {
    const tasks = [];
    const rows = await hfRowsSampled('allenai/ai2_arc', 'ARC-Challenge', 'test', TARGET_PER_SUITE);
    for (const row of stride(rows, TARGET_PER_SUITE)) {
        const labels = row.choices.label.map(String);
        const idx = labels.indexOf(String(row.answerKey));
        if (idx < 0) throw new Error(`arc_challenge: answerKey "${row.answerKey}" не найден в labels`);
        const choices = row.choices.text.map(String);
        pushLetter(tasks, 'arc_challenge', row, 'question', choices, idx, choices.length);
    }
    return tasks;
}

async function convertHellaswag() {
    const tasks = [];
    const rows = await hfRowsSampled('Rowan/hellaswag', 'default', 'validation', TARGET_PER_SUITE);
    for (const row of stride(rows, TARGET_PER_SUITE)) {
        const choices = row.endings.map(String);
        const ctx = String(row.ctx || [row.ctx_a, row.ctx_b].filter(Boolean).join(' '));
        const idx = Number(row.label);
        if (idx < 0 || idx >= choices.length) {
            throw new Error(`hellaswag: индекс ответа ${idx} вне диапазона (${choices.length} вариантов)`);
        }
        tasks.push(mcqTask('hellaswag', letterPrompt(ctx, choices), choices, LETTERS[idx], 'letter', 512));
    }
    return tasks;
}

async function convertWinogrande() {
    const tasks = [];
    const rows = await hfRowsSampled('allenai/winogrande', 'winogrande_xl', 'validation', TARGET_PER_SUITE);
    for (const row of stride(rows, TARGET_PER_SUITE)) {
        const choices = [String(row.option1), String(row.option2)];
        const idx = Number(row.answer) - 1;
        pushLetter(tasks, 'winogrande', row, 'sentence', choices, idx, choices.length);
    }
    return tasks;
}

async function convertGsm8k() {
    const tasks = [];
    const rows = await hfRowsSampled('openai/gsm8k', 'main', 'test', TARGET_PER_SUITE);
    for (const row of stride(rows, TARGET_PER_SUITE)) {
        const parts = String(row.answer).split('####');
        const expected = parts[parts.length - 1].trim();
        if (!/[0-9]/.test(expected)) throw new Error('gsm8k: в эталонном ответе нет числа');
        tasks.push(mcqTask('gsm8k', numberPrompt(String(row.question)), null, expected, 'number', 1024));
    }
    return tasks;
}

async function convertTruthfulqa() {
    const tasks = [];
    const rows = await hfRowsSampled('truthfulqa/truthful_qa', 'multiple_choice', 'validation', TARGET_PER_SUITE);
    for (const row of stride(rows, TARGET_PER_SUITE)) {
        const mc1 = row.mc1_targets;
        const idx = mc1.labels.findIndex((l) => l === 1 || l === true);
        if (idx < 0) throw new Error('truthfulqa_mc1: нет верного варианта в mc1_targets');
        const choices = mc1.choices.map(String);
        pushLetter(tasks, 'truthfulqa_mc1', row, 'question', choices, idx, choices.length);
    }
    return tasks;
}

async function convert() {
    // Каждый набор пишется сразу после своего конвертера — частичный сбой сети
    // не теряет уже скачанное (инцидент 07.11.26: падение truthfulqa теряло всё).
    const plan = [
        ['mmlu_subset', convertMmlu],
        ['arc_challenge', convertArc],
        ['hellaswag', convertHellaswag],
        ['winogrande', convertWinogrande],
        ['gsm8k', convertGsm8k],
        ['truthfulqa_mc1', convertTruthfulqa],
    ];
    for (const [suite, fn] of plan) {
        const list = await fn();
        list.forEach((t, i) => {
            t.id = `${suite}_${i}`;
        });
        const n = writeJsonl(`${__dirname}/../../tasks_for_test_llm/${suite}`, list);
        console.log(`[mcq] ${suite}: ${n} задач`);
    }
}

module.exports = { convert };
