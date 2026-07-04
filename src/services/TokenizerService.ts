import { AutoTokenizer } from '@huggingface/transformers';

let tokenizerCache: Record<string, any> = {};
let currentInitPromise: Promise<any> | null = null;
let currentModelId: string | null = null;

/**
 * Глобальный сервис подсчета токенов во фронтенде (Live preview).
 * Автоматически скачивает и кэширует tokenizer.json из HuggingFace.
 */
export async function countTokens(text: string, hfModelId: string): Promise<number> {
    if (!text) return 0;
    try {
        if (!tokenizerCache[hfModelId]) {
            if (currentModelId !== hfModelId || !currentInitPromise) {
                currentModelId = hfModelId;
                currentInitPromise = AutoTokenizer.from_pretrained(hfModelId);
            }
            tokenizerCache[hfModelId] = await currentInitPromise;
        }
        const tokens = await tokenizerCache[hfModelId].encode(text, { add_special_tokens: false });
        return tokens.length;
    } catch (e) {
        console.warn("Ошибка токенизатора HF, используется резервный подсчет:", e);
        // Резервный неточный алгоритм, если нет интернета
        return Math.ceil(text.length / 3.7);
    }
}