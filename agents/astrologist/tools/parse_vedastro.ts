/**
 * parse_vedastro — СЛУЖЕБНЫЙ скрипт (НЕ тул графа): парсит XML-данные VedAstro
 * (MIT-лицензия, github.com/VedAstro/VedAstro, Library/XMLData) в структурный JSON
 * для пополнения базы знаний астролога. Запуск вручную:
 *   deno run --no-check --no-config --allow-read --allow-write agents/astrologist/tools/parse_vedastro.ts
 *
 * Вход: ../database/source/vedastro/{HoroscopeDataList,EventDataList}.xml
 * Выход: ../database/source/vedastro/{parsed_horoscope,parsed_events}.json
 *        + опись в stderr (количество записей по Tag/Nature).
 *
 * Формат записи Event: Name, Nature (Good/Bad/Neutral), ConditionDescription,
 * Description, Tag. Пробельные поля схлопываются; пустые записи — честная ошибка.
 */

interface VedAstroEntry {
    name: string;
    nature: string;
    condition: string;
    description: string;
    tag: string;
}

function collapse(text: string): string {
    return text
        .replace(/\r/g, "")
        .split("\n")
        .map((line) => line.trim())
        .join(" ")
        .replace(/\s+/g, " ")
        .trim();
}

function extractChild(block: string, tag: string): string {
    const re = new RegExp(`<${tag}>([\\s\\S]*?)</${tag}>`, "i");
    const m = re.exec(block);
    return m ? collapse(m[1]) : "";
}

function parseXml(xml: string, sourceName: string): VedAstroEntry[] {
    const events: VedAstroEntry[] = [];
    const blockRe = /<Event>([\s\S]*?)<\/Event>/g;
    let m: RegExpExecArray | null;
    while ((m = blockRe.exec(xml)) !== null) {
        const entry: VedAstroEntry = {
            name: extractChild(m[1], "Name"),
            nature: extractChild(m[1], "Nature"),
            condition: extractChild(m[1], "ConditionDescription"),
            description: extractChild(m[1], "Description"),
            tag: extractChild(m[1], "Tag"),
        };
        if (!entry.name) {
            throw new Error(`${sourceName}: запись #${events.length} без Name — файл повреждён`);
        }
        if (!entry.description) {
            console.error(`${sourceName}: предупреждение — "${entry.name}" без Description`);
        }
        events.push(entry);
    }
    if (events.length === 0) {
        throw new Error(`${sourceName}: не найдено ни одной записи <Event>`);
    }
    return events;
}

function inventory(entries: VedAstroEntry[], label: string): void {
    const byTag = new Map<string, number>();
    const byNature = new Map<string, number>();
    for (const e of entries) {
        byTag.set(e.tag || "(пусто)", (byTag.get(e.tag || "(пусто)") ?? 0) + 1);
        byNature.set(e.nature || "(пусто)", (byNature.get(e.nature || "(пусто)") ?? 0) + 1);
    }
    console.error(`=== ${label}: ${entries.length} записей`);
    console.error(`  по Tag: ${[...byTag].map(([k, v]) => `${k}=${v}`).join(", ")}`);
    console.error(`  по Nature: ${[...byNature].map(([k, v]) => `${k}=${v}`).join(", ")}`);
}

async function main(): Promise<void> {
    const base = new URL("../database/source/vedastro/", import.meta.url);
    const read = async (file: string): Promise<string> =>
        await Deno.readTextFile(new URL(file, base));
    const write = async (file: string, data: string): Promise<void> =>
        await Deno.writeTextFile(new URL(file, base), data);

    const horoscope = parseXml(await read("HoroscopeDataList.xml"), "HoroscopeDataList.xml");
    const events = parseXml(await read("EventDataList.xml"), "EventDataList.xml");

    inventory(horoscope, "HoroscopeDataList");
    inventory(events, "EventDataList");

    await write("parsed_horoscope.json", JSON.stringify(horoscope, null, 1));
    await write("parsed_events.json", JSON.stringify(events, null, 1));
    console.error("OK: parsed_horoscope.json, parsed_events.json записаны");
}

if (import.meta.main) {
    main().catch((e) => {
        console.error(`parse_vedastro: ${e instanceof Error ? e.message : String(e)}`);
        Deno.exit(1);
    });
}
