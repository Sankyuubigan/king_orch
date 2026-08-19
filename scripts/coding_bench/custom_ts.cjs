// Собственные TS-задачи для бенчмарка (категория codegen, запуск через Deno).
const { writeJsonl, codeBlock, signatureRe } = require('./util.cjs');

// [имя, сигнатура-заглушка, описание, тест-тело]
const TASKS = [
    ['sumEven', 'function sumEven(numbers: number[]): number {\n', 'Return the sum of all even numbers in the array.', `
const t = () => {
  console.assert(sumEven([1, 2, 3, 4, 5]) === 6);
  console.assert(sumEven([2, 4, 6]) === 12);
  console.assert(sumEven([]) === 0);
  console.assert(sumEven([1, 3, 5]) === 0);
};
t();`],
    ['reverseString', 'function reverseString(s: string): string {\n', 'Return the input string reversed.', `
const t = () => {
  console.assert(reverseString("hello") === "olleh");
  console.assert(reverseString("") === "");
  console.assert(reverseString("a") === "a");
  console.assert(reverseString("racecar") === "racecar");
};
t();`],
    ['countVowels', 'function countVowels(s: string): number {\n', 'Count the vowels (a, e, i, o, u) in the string, case-insensitive.', `
const t = () => {
  console.assert(countVowels("hello world") === 3);
  console.assert(countVowels("AEIOU") === 5);
  console.assert(countVowels("xyz") === 0);
  console.assert(countVowels("") === 0);
};
t();`],
    ['findMax', 'function findMax(arr: number[]): number {\n', 'Return the maximum element of the array. Return -Infinity for an empty array.', `
const t = () => {
  console.assert(findMax([1, 5, 3, 9, 2]) === 9);
  console.assert(findMax([-5, -1, -3]) === -1);
  console.assert(findMax([7]) === 7);
  console.assert(findMax([]) === -Infinity);
};
t();`],
    ['isPalindrome', 'function isPalindrome(s: string): boolean {\n', 'Return true if the string reads the same forwards and backwards.', `
const t = () => {
  console.assert(isPalindrome("level") === true);
  console.assert(isPalindrome("hello") === false);
  console.assert(isPalindrome("") === true);
  console.assert(isPalindrome("Aba") === false);
};
t();`],
    ['chunkArray', 'function chunkArray<T>(arr: T[], size: number): T[][] {\n', 'Split the array into chunks of the given size. The last chunk may be smaller.', `
const t = () => {
  const a = chunkArray([1, 2, 3, 4, 5], 2);
  console.assert(JSON.stringify(a) === JSON.stringify([[1, 2], [3, 4], [5]]));
  const b = chunkArray([1, 2, 3], 5);
  console.assert(JSON.stringify(b) === JSON.stringify([[1, 2, 3]]));
  console.assert(JSON.stringify(chunkArray([], 3)) === JSON.stringify([]));
};
t();`],
    ['mostFrequentWord', 'function mostFrequentWord(text: string): string {\n', 'Return the most frequent word in the text. Words are separated by whitespace. Ties: return the first most frequent word.', `
const t = () => {
  console.assert(mostFrequentWord("cat dog cat bird dog cat") === "cat");
  console.assert(mostFrequentWord("a b c") === "a");
  console.assert(mostFrequentWord("") === "");
  console.assert(mostFrequentWord("one one two two") === "one");
};
t();`],
    ['flatten', 'function flatten<T>(arr: unknown[]): T[] {\n', 'Flatten a nested array one level deep (only flatten arrays one level).', `
const t = () => {
  console.assert(JSON.stringify(flatten([1, [2, 3], 4])) === JSON.stringify([1, 2, 3, 4]));
  console.assert(JSON.stringify(flatten([[[1]]])) === JSON.stringify([[1]]));
  console.assert(JSON.stringify(flatten([])) === JSON.stringify([]));
};
t();`],
    ['factorial', 'function factorial(n: number): number {\n', 'Return n! for n >= 0. 0! = 1.', `
const t = () => {
  console.assert(factorial(0) === 1);
  console.assert(factorial(1) === 1);
  console.assert(factorial(5) === 120);
  console.assert(factorial(10) === 3628800);
};
t();`],
    ['intersection', 'function intersection<T>(a: T[], b: T[]): T[] {\n', 'Return the array of elements present in both arrays, without duplicates, preserving first-array order.', `
const t = () => {
  console.assert(JSON.stringify(intersection([1, 2, 2, 3], [2, 3, 4])) === JSON.stringify([2, 3]));
  console.assert(JSON.stringify(intersection([1, 2], [3, 4])) === JSON.stringify([]));
  console.assert(JSON.stringify(intersection(["a", "b"], ["b", "c"])) === JSON.stringify(["b"]));
};
t();`],
    ['titleCase', 'function titleCase(s: string): string {\n', 'Capitalize the first letter of each word and lowercase the rest. Words are separated by spaces.', `
const t = () => {
  console.assert(titleCase("hello WORLD") === "Hello World");
  console.assert(titleCase("javaScript is fun") === "Javascript Is Fun");
  console.assert(titleCase("") === "");
};
t();`],
    ['removeDuplicates', 'function removeDuplicates<T>(arr: T[]): T[] {\n', 'Return a new array with duplicate elements removed, preserving first-occurrence order.', `
const t = () => {
  console.assert(JSON.stringify(removeDuplicates([1, 2, 1, 3, 2])) === JSON.stringify([1, 2, 3]));
  console.assert(JSON.stringify(removeDuplicates([])) === JSON.stringify([]));
  console.assert(JSON.stringify(removeDuplicates(["x", "x", "y"])) === JSON.stringify(["x", "y"]));
};
t();`],
];

async function convert() {
    const tasks = TASKS.map(([name, stub, desc, test], i) => ({
        id: `custom_typescript_${i}`,
        suite: 'custom_typescript',
        language: 'ts',
        category: 'codegen',
        run_with: 'deno',
        model_prompt: `${desc}\n\nComplete the following TypeScript function. Return only the function body, ` +
            `no explanations, no markdown fences.\n\n${codeBlock('typescript', stub)}`,
        solution_name: 'main.ts',
        prefix: stub,
        entry_point: name,
        signature_re: signatureRe('ts', name),
        test,
        run_cmd: 'deno run --allow-all main.ts',
        max_tokens: 512,
        temperature: 0,
        timeout_sec: 60,
        files: [],
    }));
    const n = writeJsonl(`${__dirname}/../../tasks_for_test_llm/custom_typescript`, tasks);
    console.log(`[custom_ts] custom_typescript: ${n} задач`);
}

module.exports = { convert };