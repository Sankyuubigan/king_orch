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

// Видимый текст стрима тул-агента (Шаг 7 плана 01.11.26, D1).
// При expect_tool_call=true всё от ПЕРВОЙ ```json-оградки в ЛЮБОМ месте
// буфера — служебный тул-конверт и не рендерится: оградка может быть
// разорвана между чанками, поэтому резка каждый раз пересчитывается по
// ПОЛНОМУ накопленному буферу (idempotent). Префикс («Сейчас сделаю…»)
// остаётся видимым — это реальный прогресс, не мусор. При false буфер не
// режется вовсе: легитимные ```json-блоки чат/синтез-агентов не трогаем.
export function visibleStreamText(buffer: string, expectToolCall: boolean): string {
    if (!expectToolCall) return buffer;
    const idx = buffer.indexOf("```json");
    return idx === -1 ? buffer : buffer.slice(0, idx);
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
