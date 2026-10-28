/**
 * Поиск файла натальных данных .jhd среди вложений пользователя.
 *
 * Контракт тулов графа: один argv-токен — плоский JSON строк:
 *   {"attachments": "[{\"file_name\":..., \"file_path\":..., \"origin\":...}]"}
 * (аргумент — строка с JSON-массивом: движок резолвит `{{ nodes.X.output.attachments }}`
 * в строку, как orientations у таро). Значение может быть и реальным массивом.
 *
 * Выход: {"found": true|false, "path": "...", "file_name": "..."}
 * found=false — НЕ ошибка (exit 0): маршрутизацию делает condition_check графа.
 * Приоритет: вложение текущего запроса ("current") перед историей ("history");
 * среди равных — самое свежее (история идёт от старых к новым).
 */

function fail(message: string): never {
    console.error(`find_jhd_attachment: ${message}`);
    Deno.exit(1);
}

interface AttachmentRef {
    file_name?: string;
    file_path?: string | null;
    origin?: string;
}

function main() {
    const rawArgs = Deno.args[0];
    if (rawArgs === undefined) fail("не переданы аргументы (ожидается JSON со списком attachments)");
    let parsed: unknown;
    try {
        parsed = JSON.parse(rawArgs);
    } catch {
        fail(`аргументы не JSON: ${rawArgs.slice(0, 120)}`);
    }

    // Поле attachments может прийти строкой (шаблонная подстановка) или массивом.
    let list: unknown = parsed;
    if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) {
        const obj = parsed as Record<string, unknown>;
        if (!("attachments" in obj)) fail("в аргументах нет поля attachments");
        list = obj.attachments;
    }
    if (typeof list === "string") {
        const trimmed = list.trim();
        if (trimmed === "" || trimmed === "[]" || trimmed === "null") {
            console.log(JSON.stringify({ found: false, path: "", file_name: "" }));
            return;
        }
        try {
            list = JSON.parse(trimmed);
        } catch {
            fail(`поле attachments — не JSON-массив: ${trimmed.slice(0, 120)}`);
        }
    }
    if (!Array.isArray(list)) fail("attachments обязан быть массивом");

    const refs = list as AttachmentRef[];
    const isJhd = (name: string) => name.toLowerCase().endsWith(".jhd");
    const candidates = refs.filter(
        (r) => typeof r.file_name === "string" && isJhd(r.file_name) && typeof r.file_path === "string" && r.file_path,
    );
    // Приоритет текущего запроса; при равенстве — последний в списке (свежее).
    const chosen =
        [...candidates].reverse().find((r) => r.origin === "current") ??
        [...candidates].reverse().find((r) => r.origin === "history") ??
        [...candidates].reverse()[0];

    if (!chosen) {
        console.log(JSON.stringify({ found: false, path: "", file_name: "" }));
        return;
    }
    console.log(
        JSON.stringify({
            found: true,
            path: chosen.file_path,
            file_name: chosen.file_name,
        }),
    );
}

main();
