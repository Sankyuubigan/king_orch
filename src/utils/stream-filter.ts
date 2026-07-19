// Утилиты очистки служебных тегов LLM из потокового текста.
// Формат Gemma: <|channel>KIND открывает канал (KIND = thought/json/...),
// <channel|> или </|channel> закрывает его и переключает на контент.
// Эти маркеры не должны отображаться пользователю в чате.

// Удаляет из текста все служебные маркеры каналов вместе с их содержимым.
export function stripStreamArtifacts(text: string): string {
    let result = text;
    // Полный блок: <|channel> ... <channel|>  ИЛИ  <|channel> ... </|channel>
    // (открывающий маркер без обязательного `>` после KIND, т.к. модель пишет
    // `<|channel>thought\n<channel|>`).
    result = result.replace(/<\|channel>[^<]*(?:[\s\S]*?(?:<channel\|>|<\/\|channel>))/gi, "");
    // Одиночные непарные маркеры
    result = result.replace(/<\|channel>[^\n<]*/gi, "");
    result = result.replace(/<channel\|>/gi, "");
    result = result.replace(/<\/\|channel>[^\n<]*/gi, "");
    result = result.replace(/<\|turn>/gi, "");
    result = result.replace(/<\|[a-z_]+>/gi, "");
    result = result.replace(/<[a-z_]+\|>/gi, "");
    // Артефакт: слово "thought"/"json" в начале строки
    result = result.replace(/^\s*thought\s*(?:\n|$)/i, "");
    result = result.replace(/^\s*json\s*(?:\n|$)/i, "");
    return result;
}

// Возвращает содержимое последнего блока <|channel>thought>...</|channel>,
// либо null. Используется для вывода рассуждений в блок «Мысли агентов».
export function extractChannelThought(text: string): string | null {
    // Ищем последний блок <|channel>thought ... <channel|> / </|channel>
    const openRe = /<\|channel>thought[^\n<]*/gi;
    let lastContent: string | null = null;
    let searchFrom = 0;
    while (true) {
        openRe.lastIndex = searchFrom;
        const open = openRe.exec(text);
        if (!open) break;
        const afterOpen = open.index + open[0].length;
        // найти закрывающий тег
        const closeIdx = text.indexOf("<channel|>", afterOpen);
        const closeSlashedIdx = text.indexOf("</|channel>", afterOpen);
        let end: number;
        let closeLen: number;
        if (closeIdx === -1 && closeSlashedIdx === -1) {
            end = text.length; // незакрытый (стриминг)
            closeLen = 0;
        } else if (closeIdx === -1) {
            end = closeSlashedIdx;
            closeLen = "</|channel>".length;
        } else if (closeSlashedIdx === -1) {
            end = closeIdx;
            closeLen = "<channel|>".length;
        } else {
            end = Math.min(closeIdx, closeSlashedIdx);
            closeLen = (end === closeIdx) ? "<channel|>".length : "</|channel>".length;
        }
        const content = text.slice(afterOpen, end).trim();
        if (content.length > 0) lastContent = content;
        searchFrom = afterOpen + (end - afterOpen) + closeLen;
        if (closeLen === 0) break;
    }
    return lastContent;
}
