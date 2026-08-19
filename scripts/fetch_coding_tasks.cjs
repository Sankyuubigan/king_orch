// Загрузка и конвертация датасетов для LLM coding-бенчмарка.
// Результат: tasks_for_test_llm/<suite>/tasks.jsonl.
// Запуск: node scripts/fetch_coding_tasks.cjs
const humanevalpack = require('./coding_bench/humanevalpack.cjs');
const evalplus = require('./coding_bench/evalplus.cjs');
const refactor = require('./coding_bench/refactor.cjs');
const custom_ts = require('./coding_bench/custom_ts.cjs');

async function main() {
    await humanevalpack.convert();
    await evalplus.convert();
    await refactor.convert();
    await custom_ts.convert();
    console.log('\nГотово. Наборы записаны в tasks_for_test_llm/<suite>/tasks.jsonl');
}

main().catch((e) => {
    console.error('Ошибка:', e);
    process.exit(1);
});