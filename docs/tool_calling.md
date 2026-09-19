Для tool-calling агентов

Использовать нативный tools API:
1. Передавать tools[] в запросе к llama-server
2. Не передавать кастомный grammar
3. Убедиться что --jinja включён и chat template правильный (chatml fallback)
4. tool_choice: "required" если нужен принудительный вызов
5. Retry при пустом/текстовом ответе (как hermes).





Сравнение подходов
hermes-agent
Механика: Native FC (tools в API) + application guards (loop detection, dedup blockers, stale write refusal)
Модели: GPT-4, Claude, DeepSeek, локальные через OpenAI-compatible endpoint
Сила: Не требует per-agent грамматик. Один код → все модели. Retry при пустом ответе.
Слабость: Если модель упорно выдаёт прозу — только retry + стоп. Нет физического ограничения.
deepseek-harness
Механика: Native FC (zod → JSON Schema → tools) + fs-observation-policy (read-before-write) + sandbox fencing + loop guards
Модели: DeepSeek-V3/R1 (600B+), Claude, GPT-4
Сила: Самая чистая архитектура. Читай-перед-пиши policy. Sandbox. Без грамматик.
Слабость: Рассчитана на мощные модели. Нет grammar-level safety net.
opencode
Механика: Native FC + text fallback parser (transform.ts 70KB) + retry
Модели: Любые через provider interface
Сила: Fallback-парсер для моделей без native FC. Retry.
Слабость: Fallback-парсер всё равно полагается на модель выдать JSON в тексте.



Исследование: инструменты записи файлов в 3 референсных репах
1. hermes-agent (NousResearch)
Механика: Нативный OpenAI function-calling (tools параметр в chat completion). Пишущий тул — write_file_tool(path, content, task_id) в tools/file_tools.py:153. Параметры (path, content) напрямую маппятся в JSON Schema для модели.

Удержание модели от прозы: НЕ грамматикой. Вместо этого — application-level guards:

read_dedup / consecutive_read_block — блокирует повторные чтения
patch_failure_tracker — считает неудачные попатчи и в конце скажет «перечитай файл»
stale_overwrite_blocker — отказывает в записи если файл менялся
loop_tool_validation.py, turn_tool_round.py — обнаруживают повторные tool-вызовы и стопают
Ключевой момент: hermes полагается на то, что llama.cpp сервер возвращает нативные tool_calls в choices[0].message.tool_calls. Если модель выдала прозу вместо tool_call — это обрабатывается как «пустой ответ» / итерация + retry. Нет грамматического ограничения.

2. deepseek-harness (DeepSeek AI)
Механика: Аналогично — нативный tool calling через zod-схемы тулов. Пакет packages/fs/tool-fs определяет модельные тулы read, write, edit. packages/core/tools — реестр + guarded execution pipeline.

Удержание:

fs-observation-policy — read-before-edit guard (нельзя писать файл, который не читали)
fs-sandbox — sandbox fencing по per-call sandbox mode
tool-str-replace-editor — альтернативный интерфейс редактирования (str_replace вместо diff)
Loop guards в packages/guard — advisory repeat-call reminders
Ключевой момент: dsh рендерит JSON Schema тулов в системный промпт, модель вызывает их нативно. Если модель вернула текст вместо tool call — нет grammar-level recovery, только модельный retry на уровне агент-лупа.

3. opencode (anomalyco)
Механика: Effect-Schema тулы (packages/opencode/src/tool/write.ts), описание в write.txt (инжектится в промпт). Параметры: content: Schema.String, filePath: Schema.String. Основной путь — нативный provider function-calling.

Fallback-парсер: packages/opencode/src/provider/transform.ts (70KB) содержит текстовый fallback для провайдеров без native tool calling — парсит JSON-блоки из текста модели. session/retry.ts обрабатывает retry на пустых/сломанных ответах.