import { AutoTokenizer } from '@huggingface/transformers';

let tokenizerCache: Record<string, any> = {};
let currentInitPromise: Promise<any> | null = null;
let currentModelId: string | null = null;

/**
 * Глобальный сервис подсчета токенов во фронтенде (Live preview).
 * Автоматически скачивает и кэширует tokenizer.json из HuggingFace.
 */
export async function countTokens(text: string, tokenizerId: string): Promise<number> {
    if (!text) return 0;
    try {
        if (!tokenizerCache[tokenizerId]) {
            if (currentModelId !== tokenizerId || !currentInitPromise) {
                currentModelId = tokenizerId;
                currentInitPromise = AutoTokenizer.from_pretrained(tokenizerId);
            }
            tokenizerCache[tokenizerId] = await currentInitPromise;
        }
        const tokens = await tokenizerCache[tokenizerId].encode(text, { add_special_tokens: false });
        return tokens.length;
    } catch (e) {
        console.warn("Ошибка токенизатора HF, используется резервный подсчет:", e);
        // Резервный неточный алгоритм, если нет интернета
        return Math.ceil(text.length / 3.7);
    }
}