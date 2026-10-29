/**
 * vedastro_classes — СЛУЖЕБНЫЙ скрипт (НЕ тул графа): классификация всех записей
 * VedAstro на применимость к предрасчёту астролога (правило 00.2: недетектируемое
 * правило инжектить нельзя — модель применит его вслепую).
 *
 * Каждая запись получает машинный вердикт + причину + очередь:
 *   computable | needs_ashtakavarga | needs_pair_chart | needs_gulika |
 *   needs_user_data | prose_only | out_of_profile;  queue: 1|2|3
 * (таксономия зафиксирована в tasks/29.10.26 Этап 2 …, «Решения ДО реализации»).
 *
 * Список computable-йог и даш-правил берётся ИЗ РЕЕСТРА vedastro_detect.ts
 * (единый источник правды) — расхождение классификатора и детекции невозможно.
 *
 * Запуск:
 *   deno run --no-check --no-config --allow-read --allow-write agents/astrologist/tools/vedastro_classes.ts
 * Вход:  ../database/source/vedastro/parsed_{horoscope,events}.json (от parse_vedastro.ts)
 * Выход: ../database/source/vedastro/classified_{horoscope,events}.json + опись в stderr.
 * Классификация ПОЛНАЯ: unmatched-запись = ошибка, а не тихий пропуск.
 */

import { COMPUTABLE_YOGAS, UNIVERSAL_DASHA_RULES, SUN_DASHA_RULES } from "./vedastro_detect.ts";

interface VedAstroEntry {
    name: string;
    nature: string;
    condition: string;
    description: string;
    tag: string;
}

export type Verdict =
    | "computable"
    | "needs_ashtakavarga"
    | "needs_pair_chart"
    | "needs_gulika"
    | "needs_user_data"
    | "prose_only"
    | "out_of_profile";

export interface ClassifiedEntry extends VedAstroEntry {
    verdict: Verdict;
    reason: string;
    queue: 1 | 2 | 3;
}

const comp = (queue: 1 | 2 | 3, reason: string): Omit<ClassifiedEntry, keyof VedAstroEntry> => ({
    verdict: "computable", reason, queue,
});
const prose = (reason: string): Omit<ClassifiedEntry, keyof VedAstroEntry> => ({
    verdict: "prose_only", reason, queue: 3,
});

/** Неконкретные квалификаторы условий («сильная/поражённая планета» и т.п.) — не формализуются. */
const VAGUE_QUALIFIERS = [
    "fortified", "afflicted", "strong", "weak", "well disposed", "well aspected",
    "favourably", "unfavourably", "anything to do", "moderately",
];

const PROSE_YOGA_REASONS: Record<string, string> = {
    PushkalaYoga: "«мощная планета» и «дом близкого друга» (дружба/сила) не формализованы",
    LakshmiYoga: "«lord of Lagna is powerful» — сила управителя лагны не формализована",
    KahalaYoga: "«strongly disposed» — сила управителя лагны не формализована",
    ParvataYoga:
        "квантор не определён текстом (сколько кендр занято благодетелями); классики расходятся в домах (6/8 против 7/8)",
    GauriYoga: "запутанный синтаксис условия — однозначное прочтение не гарантировано",
    BharathiYoga: "запутанный синтаксис условия — однозначное прочтение не гарантировано",
    MalikaYoga: "общее определение семейства маліка без конкретного дома — не отдельное правило",
    KarmaMalikaYoga:
        "результат в записи потерян источником («Definition: Results:» без текста); условие восстановимо по имени, но трактовать нечего",
};

const PLANET_NAMES = "Sun|Moon|Mars|Mercury|Jupiter|Venus|Saturn|Rahu|Ketu";
const SIGN_NAMES = "Aries|Taurus|Gemini|Cancer|Leo|Virgo|Libra|Scorpio|Sagittarius|Capricorn|Aquarius|Pisces";
const rePlanetInHouse = new RegExp(`^(${PLANET_NAMES})InHouse\\d+$`);
const rePlanetInSign = new RegExp(`^(${PLANET_NAMES})In(${SIGN_NAMES})$`);
const reSignRising = new RegExp(`^(${SIGN_NAMES})Rising$`);

function classifyHoroscope(e: VedAstroEntry): Omit<ClassifiedEntry, keyof VedAstroEntry> {
    if (e.tag === "Yoga") return classifyYoga(e);
    // Базовые связки «управитель дома в доме» — все 144 уже в БЗ v1 (SSOT, не дублируем).
    if (/^House\d+LordInHouse\d+$/.test(e.name)) {
        return comp(2, "2-я очередь: связка «управитель дома в доме»; применимость вычислима по имени, в БЗ не дублируется (SSOT: все 144 пары уже в БЗ v1)");
    }
    // Уточнения связок (Fortified/Afflicted/…).
    if (/^House\d+LordInHouse\d+[A-Z]/.test(e.name)) {
        const c = e.condition.toLowerCase();
        const vague = VAGUE_QUALIFIERS.find((k) => c.includes(k));
        if (vague) {
            return prose(`уточнение связки с неконкретным квалификатором («${vague}») — не формализуется`);
        }
        return comp(2, "2-я очередь: уточнение связки вычислимой природы (дома/соединения)");
    }
    if (rePlanetInHouse.test(e.name)) {
        return comp(2, "2-я очередь: трактовка планеты в доме — вне 1-й очереди этапа");
    }
    if (rePlanetInSign.test(e.name)) {
        return comp(2, "2-я очередь: трактовка планеты в знаке — вне 1-й очереди этапа");
    }
    if (reSignRising.test(e.name)) {
        return comp(3, "дубль БЗ v1 (файлы лагн database/lagnas/) — SSOT: не дублируем");
    }
    if (/^[A-Z][a-z]+RisingWithEvilPlanet$/.test(e.name)) {
        return comp(2, "2-я очередь: малефик в лагне; применимость вычислима (набор малефиков Марс/Сатурн/Раху/Кету)");
    }
    if (/^Lord2InHouse\d+(AndLord1InHouse\d+)?$/.test(e.name)) {
        return comp(2, "2-я очередь: положение 2-го управителя (включая обмен с 1-м)");
    }
    if (e.name.includes("Arudha")) {
        return prose("требует пад Арудхи — они не рассчитываются в предрасчёте");
    }
    if (e.name === "LordInTrine") {
        return prose("общедоктринальный тезис (управители трикон всегда благоприятны) без условия карты — не детектируемое правило");
    }
    if (e.name === "Lord2WithEvilInHouse") {
        return prose("дом в условии не указан — предикат не определяется записью");
    }
    if (e.tag === "Marriage") {
        if (e.name.includes("Friends")) {
            return prose("дружба/вражда планет (Naisargika Maitri) не формализована в предрасчёте");
        }
        return comp(2, "2-я очередь: правило брака по положениям — применимость вычислима");
    }
    // Прочие Personal-комбинации (соединения/аспекты в домах) — вычислимы.
    if (e.tag === "Personal") {
        const c = e.condition.toLowerCase();
        const vague = VAGUE_QUALIFIERS.find((k) => c.includes(k));
        if (vague) return prose(`условие с неконкретным квалификатором («${vague}») — не формализуется`);
        return comp(2, "2-я очередь: комбинация вычислимой природы (дома/соединения/аспекты)");
    }
    throw new Error(
        `classifyHoroscope: запись "${e.name}" (tag="${e.tag}") не сопоставлена ни одному правилу — классификация обязана быть полной`,
    );
}

function classifyYoga(e: VedAstroEntry): Omit<ClassifiedEntry, keyof VedAstroEntry> {
    if (e.name in COMPUTABLE_YOGAS) {
        return comp(1, "условие выражается через предрасчёт карты (предикат в vedastro_detect.ts)");
    }
    if (e.name.includes("Ashtakavarga")) {
        return {
            verdict: "needs_ashtakavarga",
            reason: "условие оперирует бинду Аштакаварги — они не рассчитываются",
            queue: 3,
        };
    }
    if (e.name === "VanchanaChoraBheethiYoga") {
        return { verdict: "needs_gulika", reason: "условие требует Гулику — она не рассчитывается", queue: 3 };
    }
    if (/^MahabhagyaYoga(Male|Female)$/.test(e.name)) {
        return { verdict: "needs_user_data", reason: "требует пола человека — в данных карты нет", queue: 3 };
    }
    if (e.name in PROSE_YOGA_REASONS) return prose(PROSE_YOGA_REASONS[e.name]);
    throw new Error(`classifyYoga: йога "${e.name}" не сопоставлена — классификация обязана быть полной`);
}

function classifyEvents(e: VedAstroEntry): Omit<ClassifiedEntry, keyof VedAstroEntry> {
    if (e.tag === "Gochara") {
        return comp(1, "транзит от Луны; дом-от-Луны вычисляется из transits (detectGochara)");
    }
    if (e.tag === "AshtakvargaGochara") {
        return { verdict: "needs_ashtakavarga", reason: "бинду Аштакаварги не рассчитываются", queue: 3 };
    }
    if (e.tag === "DasaSpecialRules") {
        if (UNIVERSAL_DASHA_RULES.includes(e.name) || e.name in SUN_DASHA_RULES) {
            return comp(1, "правило периодов: универсальное либо по состоянию Солнца (detectDashaRules)");
        }
        if (e.name === "ExaltedSunDasa") {
            return prose("«deep exaltation» (точная степень с орбисом) не считается; по dignity дублировало бы ElevatedSunDasa");
        }
        throw new Error(`classifyEvents: даш-правило "${e.name}" не сопоставлено`);
    }
    if (/^Dasa,PD\d$/.test(e.tag)) {
        return comp(3, "3-я очередь: шаблонный результат подпериода (имя кодирует связку), Nature пуст; объём не влезает в промпт");
    }
    if (e.tag === "Personal,Tarabala" || e.tag === "Personal,Chandrabala") {
        return prose("критерий силы (Strong/Middling/Weak) записью не раскрыт");
    }
    if (e.tag === "GocharaSummary") {
        return prose("сводка по гочаре, не правило применимости");
    }
    if (e.tag === "Astronomical" || e.tag === "Astronomical,YamaNumber") {
        return prose("справочное определение (астрономия/панчанга), не правило карты");
    }
    const MUHURTA_TAGS = [
        "Travel", "Agriculture", "Building", "BuyingSelling", "HairNailCutting",
        "Medical", "RulingConstellation,General", "General", "Marriage", "Personal",
    ];
    if (MUHURTA_TAGS.includes(e.tag)) {
        return {
            verdict: "out_of_profile",
            reason: "мухурта/панчанга (выбор времени действия) — вне профиля агента: натальная карта и периоды",
            queue: 3,
        };
    }
    throw new Error(
        `classifyEvents: запись "${e.name}" (tag="${e.tag}") не сопоставлена ни одному правилу — классификация обязана быть полной`,
    );
}

export function classifyFile(entries: VedAstroEntry[], kind: "horoscope" | "events"): ClassifiedEntry[] {
    return entries.map((e) => {
        const verdict = kind === "horoscope" ? classifyHoroscope(e) : classifyEvents(e);
        return { ...e, ...verdict };
    });
}

function inventory(entries: ClassifiedEntry[], label: string): void {
    const byVerdict = new Map<string, number>();
    const byQueue = new Map<string, number>();
    for (const e of entries) {
        byVerdict.set(e.verdict, (byVerdict.get(e.verdict) ?? 0) + 1);
        byQueue.set(`q${e.queue}`, (byQueue.get(`q${e.queue}`) ?? 0) + 1);
    }
    console.error(`=== ${label}: ${entries.length} записей`);
    console.error(`  вердикты: ${[...byVerdict].map(([k, v]) => `${k}=${v}`).join(", ")}`);
    console.error(`  очереди:  ${[...byQueue].map(([k, v]) => `${k}=${v}`).join(", ")}`);
}

async function main(): Promise<void> {
    const base = new URL("../database/source/vedastro/", import.meta.url);
    const read = async (file: string): Promise<VedAstroEntry[]> =>
        JSON.parse(await Deno.readTextFile(new URL(file, base)));
    const write = async (file: string, data: unknown): Promise<void> =>
        await Deno.writeTextFile(new URL(file, base), JSON.stringify(data, null, 1));

    const horoscope = await read("parsed_horoscope.json");
    const events = await read("parsed_events.json");

    const classifiedHoroscope = classifyFile(horoscope, "horoscope");
    const classifiedEvents = classifyFile(events, "events");

    inventory(classifiedHoroscope, "HoroscopeDataList (классификация)");
    inventory(classifiedEvents, "EventDataList (классификация)");

    await write("classified_horoscope.json", classifiedHoroscope);
    await write("classified_events.json", classifiedEvents);
    console.error("OK: classified_horoscope.json, classified_events.json записаны");
}

if (import.meta.main) {
    main().catch((e) => {
        console.error(`vedastro_classes: ${e instanceof Error ? e.message : String(e)}`);
        Deno.exit(1);
    });
}
