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










Ключевые находки (сентябрь 2026)
1. tools + кастомный grammar НЕЛЬЗЯ совмещать в llama.cpp
HTTP 400: "Cannot use custom grammar constraints with tools."

В llama.cpp ровно один слот грамматики на сэмплер. Когда передаёшь tools, llama-server сам генерирует внутреннюю lazy-GBNF из JSON Schema и занимает этот слот. Кастомная грамматика перезаписывает её → битва → llama.cpp отклоняет запрос.

Источник: Discussion #22408 (апрель 2026), aldehir (collaborator llama.cpp): "Even if composition were allowed, a user-defined grammar would need to be written very carefully to avoid matching content intended for the tool call grammar."

2. Native tools API уже использует GBNF внутри
Когда передаёшь tools в /v1/chat/completions:

Чат-шаблон инжектит JSON Schema тулов в промпт (модель видит тулы)
llama-server автоматически генерирует lazy-GBNF из JSON Schema
Модель выдаёт tool_call → llama.cpp парсит
Цитата aldehir: "Most models use GBNF grammar to ensure proper tool calls." — но это внутренняя автоматическая GBNF, а не ручная.

3. Qwen3-8B — бенчмарки (BFCL V4)
Метрика	Qwen3-8B	Вердикт
Non-Live AST (простой 1-ход)	87.58%	Хорошо
Live	80.53%	OK
Multi-Turn	41.75%	Плохо
Web Search	12.0%	Катастрофа
Memory	14.62%	Катастрофа
Вердикт: Для простого одноразового tool-call — работает. Для агентного сценария (multi-turn) — ненадёжно.

4. tool_choice: "required" — активные баги (авг-сен 2026)
#27217: Не применяется к шаблонам с supports_preserve_reasoning → молча генерирует до max_tokens
#27767: Qwen3 шаблон + enable_thinking:false → required не работает, детерминированно при temp=0
#26737: XML-style шаблоны → HTTP 500
5. Лучшие практики из community (distilled)
Используй нативный tools API — llama-server сам генерирует грамматику и инжектит промпт
Не смешивай tools + grammar — llama.cpp отклоняет
--jinja по умолчанию (since b7170) — правильный чат-шаблон
GBNF для чистого structured output (не tool calling) — fact extractor, signal contracts
LLGuidance — новый движок встроенный в llama.cpp, быстрее GBNF, лучше JSON Schema покрытие, есть готовые рецепты для Qwen-3
Не пересили грамматику — aldehir: "grammar constraining isn't a silver bullet. Sometimes you can overly constrain a model and it starts producing incorrect output."
6. Агентные фреймворки для локальных моделей
Фреймворк	Подход
llama-cpp-agent (deprecated)	GBNF grammar-first
ToolAgents (successor)	Native tools API через llama-server
OpenCursor (6k★)	Native tools API, сам спавнит llama-server
AtomicBot/atomic-agent	GBNF grammar
hermes-agent	Native tools API

# Дополнение (сентябрь 2026): репо-исследование сабагентами + реальный кейс qwen3.8-9b

## 7. Как референсные репы обрабатывают сломанный JSON аргументов (подтверждено кодом)

### 7.1 opencode (anomalyco) — repair через error-конверт, НЕ regex-cпасение
- `packages/opencode/src/session/llm.ts` — `experimental_repairToolCall(failed)`:
  - случаи case-fix (Write_File → write_file) — чинится;
  - иначе сломанные arguments ВЫБРАСЫВАЮТСЯ и подменяются на гарантированно-валидный JSON `{"tool": "<name>", "error": "<msg>"}` скрытого тула `invalid` (`tool/invalid.ts`), который в `activeTools` не отдаётся модели.
  - Результат: модель получает structured error как результат тула и обязана переписать вызов. Никакого regex-спасения частично-распарсенного arguments.
- `session/request.ts`/`retry.ts` — ретраи только на 429/5xx/timeout; JSON-ошибки НЕ ретраятся автоматически.
- Fallback-парсер `provider/transform.ts` парсит нативные tool_calls от провайдера — не свободный текст.
- `sanitizeSurrogates` в transform.ts — заменяет битые UTF-16 суррогаты на U+FFFD (полезно для нас при декодировании).

### 7.2 deepseek-harness — никогда не парсит текстовый JSON
- Нет текстового JSON-конверта для тулов вообще: только нативный канал `tool_calls`.
- `packages/core/agent-loop/src/tool-calls.ts` — `parseArguments(raw)`:
  - `try { JSON.parse(raw) } catch { return raw }` — невалидный JSON НЕ роняет, а сохраняет как raw-строку; пустой raw → `{}`.
  - Финальный гейт — JSON-schema валидация (`packages/core/tools/src/schema.ts`): `validate()` → `ToolArgsError` → результат тула `isError: true` → модель чинит args в ТОМ ЖЕ ходе (same-turn retry).
  - `ToolNotFoundError` (UNKNOWN_TOOL) — тоже model-visible error.
- Messages-протокол (`llm-deepseek/src/protocols/messages/translate.ts`): на `message_stop` для finish≠max_tokens делает `JSON.parse(arguments)`; неудача → `LlmError MALFORMED_RESPONSE` → retry-каскад. При max_tokens — проверка пропускается.
- `llm/src/assembler.ts`: на `max_tokens` finish все незавершённые tool-call блоки ДРОПАЮТСЯ (не исполняются).
- PTC-режим (`packages/core/tools/src/ptc.ts`): автортулы заменяются одним `run_code`, а код передаётся **escaped JSON-строкой** в arguments — контент с кавычками/переводами строк НЕ угрожает валидности внешнего JSON (это то, как надо передавать большие content-поля).

### 7.3 hermes-agent — репорабельны только тривиальные поломки
- `agent/message_sanitization.py` `_repair_tool_call_arguments(raw, tool_name)` — единственный repair, стадии:
  0. `json.loads(raw, strict=False)` — принимает ЖИВЫЕ control-символы (реальные `\n`/`\t`) внутри строк → это САМЫЙ частый кейс локальных моделей;
  1-3. хвостовые запятые, добор `}`/`]` по балансу скобок, срез лишних закрывающих (≤50 итераций);
  4. экранирование control-символов 0x00–0x1F как `\uXXXX` (только их);
  last resort: `"{}"` + WARNING (лог ограничен 100KB).
- **КРИТИЧНО: ни одна стадия не чинит неэкранированные внутренние кавычки** — они нерепорабельны → `"{}"`.
- Streaming (`_assemble_tool_calls`): провал репора → если нет finish_reason — весь ход становится partial-stream stub (тул НЕ исполнен, retry); если finish_reason есть → `length`-транкация → стандартный ретрай.
- Non-streaming (`tool_executor.py` `_parse_tool_arguments`): `json.loads` БЕЗ репора; не-dict → аргументы скипаются, тул НЕ исполняется, результат `{"error": "Invalid tool arguments", "message": "Tool arguments must be a valid JSON object; tool was not executed."}` с типом `invalid_tool_arguments` → модель сама перепишет.
- History sanitizer: уже записанные битые аргументы сессии → `"{}"` + маркер-результат «tool call arguments were corrupted in this session and have been dropped».
- `write_file` НЕ исполняется с половинчатым content — модель обязана повторить с валидным JSON.

## 8. Веб-исследование: превентив vs куратив (конкретика llama.cpp)
1. **Превентив (гарантия)**: llama.cpp строит GBNF из JSON Schema тулов автоматически на non-streamed пути (`/v1/chat/completions` + tools, без stream). `grammars/json.gbnf` — правило строки `char ::= [^"\\\x7F\x00-\x1F] | [\\] (...)` — неэкранированная `"` внутри строки **не эмиттируема**: `if __name__ == "__main__":` физически не может появиться как есть.
2. **Native FC конверт**: с `--jinja` + supported chat template llama.cpp эмитит `"arguments": "{\"path\":...}"` — arguments как JSON-СТРОКУ, разделители однозначны. Форматы: Llama 3.x, Hermes 2/3, Qwen 2.5/Coder, Mistral Nemo, Firefunction v2, Command R7B, DeepSeek R1 (WIP), Generic fallback.
3. **Куратив — лишь вторичен**: OpenAI spec прямо: "The model does not always generate valid JSON... Validate the arguments in your code before calling your function." Лимит: streamed tool args в ряде билдов обходят grammar (#20352, #20359); крупные схемы ломают combined grammar (#25923 — большой maxLength/пустые объекты → rejected GBNF → весь tool grammar ломается). Вывод: держать схемы тулов компактными, cappить maxLength/maxItems в `content`.
4. **Ретрай-с-фидбеком — стандарт**: 1 попытка репора → ошибка модели → модель переписывает → затем отказ. `safe_args_parse()` (common/chat.cpp) — полный JSON.parse, при неудаче передаёт raw-строку без обрезки; сервер НЕ усекает (обрезка у нас — артефакт нашего regex).
5. **Production для 3.8B + llama.cpp**: grammar-constrained native tool call + arguments-as-string + НЕ стримить tool-ходы (или валидировать после стрима).

## 9. Наш кейс: qwen3.8-9b `{"name":..., "arguments":{...}}` (формат OpenAI в тексте)
Наблюдение в e2e `coding_team_coder_apply1` (test.bat, TEST_MODEL_PATH): модель пишет ```json конверт в текст,
```
{"name": "write_file", "arguments": {"path": "...", "content": "\"\"\"<python со скобками и кавычками>\"\"\""}}
```
Диагноз (сентябрь 2026):
- `serde_json::from_str` ВСЕГО блока падает (в `content` — литеральные символы, которые Rust-`serde_json` в строке не принимает, либо неэкранированная кавычка);
- fallback-regex `"arguments"\s*:\s*(\{.*?\})` — не-жадный, режет по ПЕРВОМУ `}` внутри `content` (Python `{eu.total()}`) → `arguments: null` → write_file: «параметр path обязателен» × 3 → стоп.
Выводы для нашего кода:
- `\{.*?\}` — известный анти-паттерн: заменить на string/escape-aware сканнер вложенности (как в extract_json_block);
- литеральные control-символы (живой `\n` в строке) — репорабельны пре-пассом (паттерн hermes strict=false → re-escape);
- неэкранированная кавычка внутри значения — НЕрепорабельна: строго по экосистеме → отдать модели error-результат и переписать в том же ходе (а не 3-фейла-и-стоп);
- нативный путь `gen.tool_calls` у нас в плагине есть, но для этой модели llama-server вернул EOS-текст без native tool_calls → текстовый parse остаётся основным мостом (признано: текст → фенс → наш парсер).