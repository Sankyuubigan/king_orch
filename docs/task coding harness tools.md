Прошёл по всем 12 репозиториям, открыл README, докумен­тацию по инструментам и исходники там, где это удалось загрузить. Ниже — сводка находок, сравнительная таблица, разбор паттернов по категориям и план аудита/внедрения.

## Краткое резюме

Подходы у команд **принципиально разные** — от «всё есть плагин» (DeepSeek Harness) до минималистичного ядра из 4 инструментов (Mini-Agent) и «Claude Code‑совместимого ядра + MCP/скиллов» (OpenClaude, Kimi Code, Gemini CLI). **Утверждение, что плагин‑подход DeepSeek — лучший, некорректно как универсальная истина**: для production‑команды оптимальна **гибридная модель** — компактное ядро базовых инструментов для кода (read/write/edit/grep/glob/bash/task/plan/todo) + MCP/плагины/скиллы для всего остального. Важно: `zai-org/GLM-4.5` — это **веса модели, а не агент‑харнес**, инструментов кода там нет【turn0search15】; `ultraworkers/claw-code` — это «реконструкция архитектуры Claude Code», агент‑управляемый музейный экспонат, а не production‑фреймворк【turn0search10】【turn0search14】.

## Сравнительная таблица по 12 репозиториям

| Репозиторий | Язык | Архитектурный подход | Чтение/понимание кода | Расширяемость | Примечательный паттерн |
|---|---|---|---|---|---|
| **NousResearch/hermes-agent** | Python | Ядро + toolsets + skills + MCP, 47 встроенных инструментов | `read_file`, `search_files`, `patch`, `write_file`, `session_search`, LSP‑нет | Toolsets (core/composite/platform), плагины, external skill dirs | Prompt‑caching «священен» — всё, что мутирует контекст, вынесено за ядро【turn2search21】【turn10fetch0】【turn7search6】 |
| **anomalyco/opencode** | TS + Zig TUI | Ядро Build/Plan + плагины + MCP | `read`, `glob`, `grep`, `list`, `lsp`, `task` (subagent `@general`) | JS/TS плагины в `.opencode/plugins/`, npm‑пакеты | Разделение на Build (полный доступ) и Plan (read‑only) агентов; permission allow/deny/ask【turn0search5】【turn1search0】【turn1search2】 |
| **charmbracelet/crush** | Go | Ядро + MCP | `read/view`, `glob`, `grep`, **LSP‑enhanced** (контекст через Language Server) | MCP, отключаемые инструменты | LSP как первичный источник понимания структуры кода【turn0search10】【turn1search16】 |
| **QwenLM/Qwen-Agent** | Python | Фреймворк (LLM + Tools + Agents) | `code_interpreter`, `retrieval_qwen` (RAG), пользовательские функции | MCP‑конфиг, интегрированные инструменты, свои tools | Tool‑calling templates/парсеры инкапсулированы; есть отдельный `qwen-code` CLI с полным file‑toolset【turn0search15】【turn7search11】【turn0search16】 |
| **deepseek-ai/deepseek-harness** | TypeScript | **Всё есть плагин** (Cordis) | Инструменты — плагины в реестре `ctx.tools` | 56 service‑rows, 26 seam‑точек расширения | `docs/capability-seams.md` публикует точки вмешательства; регистрации — effects, откатываемые при unload; native `landlock-run` sandbox【turn0search0】【turn0search3】【turn5fetch0】【turn6find1】 |
| **xai-org/grok-build** | Rust, ~79 крейтов | Монолитный harness + MCP/skills/plugins/hooks/checkpoints | `xai-grok-tools` (terminal, file edit, search), `xai-grok-workspace` (FS, VCS, checkpoints) | MCP, skills, plugins, hooks | «Grok не доверяет Grok»: защита от прокрастинации, преждевременной декларации победы, галлюцинаций интерфейсов; авто‑подмена `grep`→`ripgrep`/`bfs`/`ugrep`【turn0search5】【turn7search1】【turn0search8】 |
| **ultraworkers/claw-code** | Rust→Python | Реконструкция архитектуры Claude Code | Базовый Claude Code‑toolset | — | «Agent‑managed museum exhibit»; не production, полезен как референс архитектуры CC【turn0search10】【turn0search14】 |
| **zai-org/GLM-4.5** | — | **Модель, не харнес** | — | — | Веса + quickstart; инструменты кода отсутствуют【turn0search15】 |
| **Gitlawb/openclaude** | TypeScript | Claude Code‑совместимое ядро + MCP/skills | `read`/`write`/`edit`, `grep`, `glob`, agents, tasks | MCP, slash commands, skills (`registry.json`), VS Code extension | ripgrep как зависимость; «runs anywhere, uses anything» — любой провайдер【turn0search20】【turn7search4】【turn0search23】 |
| **google-gemini/gemini-cli** | Node/TS | ReAct‑ядро + MCP | `list_directory`, `read_file`, `read_many_files`, `glob`, `search_file_content`, `replace`, `save_memory` | `settings.json`: `excludeTools`, `tools.core` allowlist, MCP | `rootDirectory`‑ограничение; read_many_files для батч‑чтения; mutators требуют подтверждения с diff【turn0search35】【turn1search8】【turn1search7】【turn11search4】 |
| **MiniMax-AI/Mini-Agent** | Python | Минималистичный цикл Perception→Thinking→Action→Feedback | `FileReadTool`, `FileWriteTool`, `BashTool`, `SessionNoteTool` | MCP, Claude Skills (15), Anthropic‑compatible API | Auto‑compaction контекста через суммаризацию; Session Note как persistent memory【turn0search25】【turn0search26】【turn3fetch2】【turn4fetch0】 |
| **MoonshotAI/kimi-code** | Single binary (Rust/Go) | Ядро + skills + MCP + AgentSwarm | `Read`, `Grep` (ripgrep), `Glob`, `ReadMediaFile`, `Agent`/`AgentSwarm` subagents | Skills, MCP, ACP (IDE), server/web | Plan Mode; фоновые/крон‑задачи (`TaskList`, `CronCreate`); `AskUserQuestion`; yolo/auto режимы【turn0search30】【turn4fetch0】 |

## Карточки по категориям

### 1. Инструменты чтения файлов — что стоит позаимствовать

- **`read_file` с пагинацией по строкам + offset/limit** — стандарт де‑факто (Gemini CLI, Qwen Code, Kimi Code). Критично для контекстного бюджета: модель читает только нужный диапазон【turn1search8】【turn4fetch0】.
- **`read_many_files` / batch‑read** (Gemini CLI) — один вызов читает несколько файлов сразу, сокращает round‑trips【turn1search7】.
- **`ReadMediaFile` / мультимодальный read** (Kimi Code) — чтение изображений, видео, PDF; `read_file` у Qwen Code сам определяет модальность и fallback на text‑extraction【turn4fetch0】【turn3fetch1】.
- **Структурированный read для `.ipynb`** (Qwen Code) — парсит JSON ноутбука в model‑readable вид с cell IDs, дальше `notebook_edit` правит по ID【turn3find1】.
- **Line numbers в выводе** — везде (Claude Code, Hermes, Gemini) для точного таргетинга edit‑операций.
- **Respect `.gitignore` + `rootDirectory` confinement** (Gemini CLI) — безопасность и чистота контекста【turn1search8】.

### 2. Понимание структуры кода — лучшие практики

- **LSP‑enhanced** (Crush) — вместо «grep по тексту» агент обращается к Language Server за definition/references/диагностикой. Это качественно другой уровень понимания, чем у grep‑only агентов【turn1search16】.
- **`glob` (по именам) vs `grep` (по содержимому)** — чёткое разделение, закреплённое в системных промптах (Claude Code, Kimi Code, Gemini CLI). Confusion этих инструментов — частый источник траты токенов【turn1search3】【turn4fetch0】.
- **`list_directory` с ignore‑patterns** (Gemini CLI) — компактный обзор структуры без чтения содержимого【turn1search8】.
- **Repo map / AGENTS.md** (Kimi Code `/init`, Hermes skills) — предгенерируемый скелет проекта, который агент держит в контексте вместо постоянного re‑exploration【turn4fetch0】.
- **`search_files` / `session_search`** (Hermes) — поиск по предыдущей сессии и по файлам в одном toolset【turn10fetch0】.
- **Checkpoints + VCS integration** (Grok Build `xai-grok-workspace`) — снапшоты состояния рабочей области для отката и self‑verification【turn7search1】.
- **Ripgrep как движок** — практически везде (OpenClaude, Kimi Code, Gemini CLI); Grok Build идёт дальше с авто‑подменой на `bfs`/`ugrep` с fallback【turn0search8】【turn7search4】.

### 3. Общие принципы, лайфхаки, паттерны

- **Read‑only vs mutator tools с разными permission‑уровнями** — Kimi Code: `Read`/`Grep`/`Glob` auto‑allow, `Write`/`Edit`/`Bash` require approval【turn4fetch0】. Это и безопасность, и контекстный бюджет (read‑tools дешевле).
- **Plan Mode** (Kimi Code `EnterPlanMode`/`ExitPlanMode`, OpenCode `Plan` agent, Gemini `exit_plan_mode`) — агент сначала исследует и составляет план, пользователь подтверждает, потом начинается mutation. Снижает cost of mistakes【turn4fetch0】【turn1search6】.
- **Subagent delegation** — Hermes `delegate_task`, Kimi Code `Agent`/`AgentSwarm`, Grok Build child agents, OpenCode `@general`, Gemini `Task`. Изолированный контекст для подзадач, parent получает только результат【turn10fetch0】【turn4fetch0】【turn0search10】.
- **TodoList / TodoWrite** — видимый to‑do list в контексте, чтобы агент не «забывал» подзадачи (Kimi Code, Gemini, Hermes, Qwen Code)【turn4fetch0】【turn1search7】.
- **Persistent memory** — Mini‑Agent `SessionNoteTool`, Hermes `memory` + Honcho plugin, Gemini `save_memory`. Кросс‑сессионная память вне основного контекста【turn4fetch0】【turn10fetch0】.
- **Auto‑compaction контекста** — Mini‑Agent и Kimi Code `/compact`: при приближении к лимиту токенов LLM суммаризует историю【turn0search26】【turn4fetch0】.
- **Sandboxing** — Grok Build и DeepSeek Harness имеют native sandbox (Landlock у dsh, bubblewrap у сообщества для Crush). Bash по умолчанию off или gated【turn0search9】【turn6find1】.
- **Prompt‑caching awareness** (Hermes) — любые изменения toolset/system prompt mid‑conversation инвалидируют кэш; ядро спроектировано так, чтобы не мутировать prefix【turn2search21】.
- **Defensive design «не доверяй модели»** (Grok Build) — промпты и архитектура исходят из того, что модель будет прокрастинировать, объявлять победу раньше времени, галлюцинировать tool‑интерфейсы; система ловит каждое из этих поведений【turn0search8】.
- **Skill‑система вместо раздувания ядра** — Hermes, OpenClaude, Kimi Code, Mini‑Agent: переиспользуемые скиллы (review, debug, test‑writing) загружаются из каталога, а не зашиваются в ядро【turn0search20】【turn0search23】【turn0search26】.

### 4. План аудита текущего проекта и внедрения

**Фаза 1 — Аудит (чек‑лист «что у нас есть»)**

Для каждого пункта отметьте: есть / нет / частично.

- Чтение: `read_file` с offset/limit, `read_many_files`, `list_directory`, мультимодальный read, structured notebook read.
- Поиск: `grep` (ripgrep), `glob`, `search_files` по содержимому с regex, поиск по истории сессий.
- Редактирование: `write_file`, `edit`/`replace` (exact match), `patch`, `notebook_edit`.
- Исполнение: `bash`/`shell` с timeout, sandboxing (Landlock/bubblewrap/docker), env‑passthrough.
- Понимание структуры: LSP‑интеграция, repo‑map/AGENTS.md, AST‑based tools (tree‑sitter).
- Оркестрация: subagent/delegate, Plan Mode, TodoList, AskUserQuestion.
- Память: persistent session note, cross‑session memory, auto‑compaction.
- Расширяемость: MCP‑client, plugin‑system, skill‑catalog, hooks.
- Безопасность: permission tiers (read‑only auto / mutator ask), rootDirectory confinement, `.gitignore` respect, secret‑redaction.
- Наблюдаемость: tool‑call logging, token accounting, checkpoint/rollback.

**Фаза 2 — Расширение (что добавить, в порядке приоритета)**

1. Выровнять базовый toolset под де‑факто стандарт: `read_file` (offset/limit + line numbers), `read_many_files`, `write_file`, `edit`/`replace`, `grep` (ripgrep), `glob`, `list_directory`, `bash`, `task`/`delegate`, `todo`, `plan`/`exit_plan_mode`. Это даёт совместимость с промптами Claude Code / Kimi Code / Gemini CLI.
2. LSP‑интеграция (как у Crush) — самый высокий ROI для понимания кода; references/definitions/diagnostics вместо grep‑эвристик.
3. Plan Mode + TodoList — дешёвая реализация, большой эффект на надёжность.
4. Subagent delegation с изолированным контекстом.
5. Auto‑compaction + persistent memory (SessionNote‑паттерн).
6. MCP‑client для внешних возможностей (GitHub, базы данных, web‑search) — не писать в ядре.
7. Sandboxing (Landlock на Linux, docker/ssh backend как у Hermes).
8. Skill‑каталог для переиспользуемых задач (review, test‑gen, refactor).

## Ответы на концептуальные вопросы

**«У них у всех одно и то же или разные подходы?»** — Разные. Выделяются три архитектурных модели:

1. **Plugin‑first** (DeepSeek Harness) — модель, инструменты, скиллы, сессии, UI и даже agent loop — swapable плагины на Cordis; 26 из 56 service‑rows — заявленные seam‑точки расширения【turn0search0】【turn5fetch0】.
2. **Core + plugins/MCP** (большинство: OpenCode, Crush, Kimi Code, Gemini CLI, Hermes, OpenClaude) — компактное ядро базовых code‑tools + MCP/плагины/скиллы для остального. Hermes дополнительно вводит toolsets (core/composite/platform) для конфигурации per‑platform【turn10fetch0】.
3. **Minimalist** (Mini‑Agent, Qwen‑Agent) — 3–4 базовых инструмента (file read/write, bash, session note), всё остальное — через MCP/скиллы/Code Interpreter【turn4fetch0】【turn0search26】.

**«Плагины DeepSeek — лучший подход, или инструменты должны быть в ядре?»** — Универсального ответа нет, но для production‑команды кодинг‑агентов оптимальна **гибридная модель с ядром**: базовые инструменты для работы с кодом (read/write/edit/grep/glob/bash/task/plan/todo) должны быть **в ядре**, потому что они (а) используются в каждом запросе — накладные расходы плагина не оправданы; (б) должны быть согласованы с prompt‑caching — частая смена toolset инвалидирует кэш (это явно принцип Hermes【turn2search21】); (в) требуют тесной интеграции с sandbox, diff‑viewer, permission‑system. Специфичные возможности (GitHub, Jira, базы данных, web‑search, конкретные доменные тулзы) — **в плагинах/MCP/скиллах**, потому что они (а) опциональны; (б) зависят от среды и провайдера; (в) их можно включать per‑project/per‑agent.

Чистый подход DeepSeek «всё есть плагин» элегантен для research/extensibility и публикации точек вмешательства (`capability-seams.md` — отличная практика, которую стоит скопировать), но в production несёт overhead: каждая загрузка/выгрузка плагина, больше поверхность атаки, сложнее гарантировать согласованность toolset ↔ prompt cache. DeepSeek Harness к тому же в developer preview с заявленными breaking changes【turn0search0】.

## Общие моменты

1. **Скопируйте паттерн `capability-seams`** — опубликуйте в репо таблицу «ядро / seam / bundle» для каждой возможности. Это документация, которая сразу показывает команде, где можно вмешиваться без чтения кода【turn5fetch0】.
2. **LSP как first‑class citizen** — если выбираете одну фишку для переноса, берите LSP‑интеграцию из Crush. Это качественный скачок в понимании кода по сравнению с grep‑only.
3. **Permission tiers с auto‑allow для read‑only** — паттерн Kimi Code: read‑tools не требуют подтверждения, mutators — требуют. Снижает трение и сохраняет безопасность【turn4fetch0】.
4. **Prompt‑cache discipline** — заимствуйте принцип Hermes: ядро не должно мутировать system prompt / toolset mid‑conversation. Любая фича, которая это нарушает, должна быть вынесена в subagent или plugin【turn2search21】.
5. **Defensive prompts** — изучите подход Grok Build «не доверяй модели»: промпты, которые ловят преждевременную победу и сужение задачи, полезнее, чем ещё один инструмент【turn0search8】.
6. **MCP как основной механизм расширения**, а не собственный plugin‑format — снижает lock‑in и даёт доступ к экосистеме; собственный формат оставьте только для того, что MCP не покрывает (hooks, transformers).
7. **Skill‑каталог вместо раздувания toolset** — переиспользуемые задачи (code review, test generation, refactor) оформляйте как skills с SKILL.md, а не как новые инструменты.
8. **Не тратьте ресурсы на клонирование claw-code** и не ждите инструментов от GLM-4.5 — первый это арт‑проект/референс, вторая это модель.

---

## 🧭 Общие рекомендации (с учётом вашего стека)

### 1. Базовые code-tools — на Rust, не на Deno/MCP

**Почему:** Rust-команды Tauri быстрее, безопаснее и лучше интегрируются с логгером (правило 2.5 — ошибки должны идти в UI). Deno/MCP добавляет лишний hop и сложность с таймаутами.

**Минимальный набор (реализовать как Tauri-команды):**

| Инструмент | Зачем | Важные детали |
|---|---|---|
| `read_file` | Чтение с пагинацией | `offset`/`limit` по строкам, номера строк в выводе, уважение `.gitignore` |
| `read_many_files` | Батч-чтение | Несколько путей за один вызов — экономит round-trips |
| `write_file` | Создание/перезапись | Только в пределах `root_dir` (конфайнмент) |
| `edit_file` / `replace` | Точные правки | По exact-match строки, с diff в лог |
| `grep` | Поиск по содержимому | Обёртка над `ripgrep` (уже в зависимостях) |
| `glob` | Поиск по именам | Быстрый обход дерева, без чтения содержимого |
| `list_directory` | Структура папок | С ignore-паттернами |
| `bash` | Исполнение команд | **Таймаут + sandbox + async-режим** (правило 9) |

**Архитектурный совет:** Вынеси эти инструменты в отдельный крейт `tools-core` в `src-tauri/`, за трейтом `Tool` — потом легко будет добавлять новые.

```rust
// src-tauri/src/tools/mod.rs
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value; // JSON Schema
    fn execute(&self, args: serde_json::Value) -> Result<ToolResult, ToolError>;
}
```

### 2. LSP-интеграция — самый высокий ROI для понимания кода

**Что взять из Crush:** Использование Language Server для получения definition/references/diagnostics вместо grep-эвристик.

**Как реализовать в твоём стеке:**
- **Rust-код:** Запускать `rust-analyzer` как sidecar-процесс, общение по LSP over stdio.
- **TS/Deno-код:** Использовать `tsserver` (через `typescript-language-server`).
- **Обёртка на Rust:** `lsp_get_definition`, `lsp_get_references`, `lsp_get_diagnostics`.

**Почему это важно:** GGUF-модели часто галлюцинируют при grep-поиске. LSP даёт точные позиции символов, что критично для `edit_file`.

### 3. Tool-calling для GGUF — два пути

**Путь A (если модель поддерживает function calling):**
- Использовать `llama-server` с параметром `--api-key` (для безопасности) и форматом OpenAI tool-calling.
- Модели: Qwen2.5-Instruct, Hermes-3-Llama-3.1, Llama-3.1-8B-Instruct (с поддержкой tools).

**Путь B (если модель не поддерживает):**
- Реализовать парсер на Rust, который извлекает tool-call из текстового вывода.
- Формат промпта: 
  ```
  Когда нужно вызвать инструмент, выведи:
  ```json
  {"tool": "read_file", "args": {"path": "src/main.rs", "offset": 10, "limit": 50}}
  ```
  ```
- Парсер ищет JSON-блоки и валидирует схему.

**Рекомендация:** Начни с Пути B (универсальнее), потом добавь поддержку A для совместимых моделей.

### 4. MCP-клиент — для внешних инструментов, не базовых

**Архитектура:**
- MCP-серверы на TS запускаются как sidecar-процессы (`deno run -A server.ts`).
- Rust-ядро имеет `McpClient`, который общается по stdio (JSON-RPC).
- Реестр MCP-серверов в конфиге (`mcp.json`).

**Что вынести в MCP (а не в Rust-ядро):**
- Web-search (keyless: DuckDuckGo HTML-парсер, Wikipedia API).
- GitHub integration (если нужен — но без ключей, только public repos).
- Доменные тулзы (специфичные для проекта).

**Важно:** MCP-серверы не должны иметь доступ к ФС проекта напрямую — только через Rust-команды (через Tauri API), иначе нарушится security model.

### 5. Память и контекст — критично для GGUF

**Проблема:** Локальные модели имеют ограниченный контекст (4k-32k токенов). Без управления контекстом агент быстро "забывает".

**Решения:**
- **Auto-compaction:** При приближении к лимиту токенов (например, 80% контекста) — отдельный запрос к `llama-server` на суммаризацию истории.
- **Persistent session note:** Файл `tasks/session_notes.md` (или отдельная папка `memory/`), куда агент записывает ключевые решения, прогресс.
- **Repo map:** Предгенерируемый скелет структуры проекта (через `glob` + `list_directory`), который агент держит в контексте вместо постоянного re-exploration.

### 6. Планирование и TodoList — интеграция с твоим правилом `tasks/`

У тебя уже есть правило 1.8 (план в `tasks/`). Сделай это инструментом:

```rust
// Инструмент todo_write
{
  "tool": "todo_write",
  "args": {
    "action": "add" | "complete" | "list",
    "task_id": "...",
    "text": "..."
  }
}
```

Этот инструмент должен читать/писать в `tasks/<дата> <название>.md` согласно твоему формату.

### 7. Безопасность и permission tiers

**Паттерн из Kimi Code:**
- `read_file`, `grep`, `glob`, `list_directory` — auto-allow (безопасны).
- `write_file`, `edit_file`, `bash` — требуют подтверждения (через UI Tauri).
- `bash` с определёнными командами (`cargo`, `git diff`) — auto-allow, с другими — ask.

**Реализация в Tauri:**
- При вызове мутатора — emit события в UI, пользователь подтверждает (кнопка "Разрешить"/"Запретить").
- Логировать все tool-callы во вкладку "Логи".

### 8. Анти-зависание — асинхронный bash с таймаутом

У тебя уже есть правило 9 (асинхронный запуск сборки). Обобщи это для `bash`-инструмента:

```rust
pub async fn execute_bash(cmd: String, timeout_secs: u64) -> Result<String, ToolError> {
    let output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        tokio::process::Command::new("cmd")
            .args(&["/c", &cmd])
            .output()
    ).await;

    match output {
        Ok(Ok(out)) => Ok(String::from_utf8_lossy(&out.stdout).to_string()),
        Ok(Err(e)) => Err(ToolError::ExecutionFailed(e.to_string())),
        Err(_) => Err(ToolError::Timeout),
    }
}
```

Для долгих процессов (сборка, тесты) — возвращать сразу `task_id`, потом опрашивать статус через `task_status`.

---

## 🏗️ Рекомендуемая архитектура

```
┌─────────────────────────────────────────┐
│           Tauri UI (React/Deno)          │
│  ┌─────────────┐  ┌──────────────────┐  │
│  │   Chat UI   │  │   Logs Tab       │  │
│  └─────────────┘  └──────────────────┘  │
└──────────────────┬──────────────────────┘
                   │ Tauri Commands
┌──────────────────▼──────────────────────┐
│         Rust Core (src-tauri)            │
│  ┌───────────────────────────────────┐  │
│  │         Agent Loop                │  │
│  │  ┌─────────┐  ┌────────────────┐  │  │
│  │  │ Tool    │  │ Tool-Calling   │  │  │
│  │  │ Registry│  │ Parser         │  │  │
│  │  └─────────┘  └────────────────┘  │  │
│  └───────────────────────────────────┘  │
│  ┌────────────┐ ┌────────────────────┐  │
│  │ Tools Core │ │ LSP Client         │  │
│  │ (FS, bash) │ │ (rust-analyzer)    │  │
│  └────────────┘ └────────────────────┘  │
│  ┌────────────────────────────────────┐ │
│  │ McpClient (stdio to TS servers)    │ │
│  └────────────────────────────────────┘ │
│  ┌────────────────────────────────────┐ │
│  │ LlamaServerClient (HTTP localhost) │ │
│  └────────────────────────────────────┘ │
└──────────────────┬──────────────────────┘
                   │ HTTP (localhost:random)
┌──────────────────▼──────────────────────┐
│      llama-server.exe (sidecar)         │
│      GGUF model in VRAM                 │
└─────────────────────────────────────────┘
```

---

## 📋 План внедрения (по приоритету)

### Фаза 1 — Базовый toolset (1-2 недели)
- [ ] Аудит текущих инструментов (что есть).
- [ ] Реализовать `read_file` с offset/limit + line numbers.
- [ ] Добавить `grep` (обёртка над ripgrep) и `glob`.
- [ ] Добавить `list_directory` с ignore-паттернами.
- [ ] Реализовать `bash` с таймаутом и async-режимом.
- [ ] Интегрировать логирование всех tool-callов в UI (правило 2.5).

### Фаза 2 — Tool-calling и agent loop (2-3 недели)
- [ ] Реализовать парсер tool-call из текста (если GGUF не поддерживает нативно).
- [ ] Или настроить `llama-server` на OpenAI tool-calling format.
- [ ] Добавить `write_file`, `edit_file` (с подтверждением через UI).
- [ ] Реализовать `todo_write`/`todo_read` (интеграция с `tasks/`).

### Фаза 3 — Понимание кода (2-3 недели)
- [ ] Интегрировать `rust-analyzer` как sidecar.
- [ ] Реализовать `lsp_get_definition`, `lsp_get_references`.
- [ ] Добавить `tsserver` для TS/Deno-кода.
- [ ] Сгенерировать repo map (через `glob` + `list_directory`).

### Фаза 4 — Расширяемость (2-3 недели)
- [ ] Реализовать `McpClient` на Rust (stdio JSON-RPC).
- [ ] Перенести web-search в MCP-сервер на TS (DuckDuckGo keyless).
- [ ] Добавить permission tiers (read-only auto, mutator ask).
- [ ] Реализовать auto-compaction контекста.

### Фаза 5 — Улучшения (опционально)
- [ ] Subagents (планировщик/кодер).
- [ ] Plan Mode (исследовать → план → подтверждение → execute).
- [ ] Persistent session note.
- [ ] Sandboxing (Landlock на Linux, ограничение путей на Windows).

---

## 💡 Личные советы

1. **Не пытайся повторить DeepSeek Harness** — у вас локальные модели, другая цель. Делай компактное ядро на Rust + MCP для расширений.

2. **LSP — убийца grep** для понимания кода. Если внедришь только одну фичу из этого списка, пусть будет LSP.

3. **Логируй каждый tool-call** — это спасёт при дебаге "почему агент сделал X". Формат: `[TOOL] read_file(path="src/main.rs", offset=10, limit=50) → 200 OK`.

4. **Тестируй на маленьких моделях** — Qwen2.5-7B-Instruct хорошо работает с tool-calling и поддерживает GGUF. Не нужен огромный контекст для базовых задач.

5. **Не зашивай инструменты в ядро намертво** — оставь трейт `Tool`, чтобы потом легко добавлять новые без перекомпиляции ядра.

6. **Для Deno/MCP-серверов** — используй `deno run -A --no-check` для скорости, и обязательно логируй stderr в `test/mcp_logs/`.

7. **Если модель галлюцинирует tool-call** — добавь в системный промпт жёсткое правило: "Если не уверен, нужен ли инструмент — сначала спроси у пользователя".

---



---

### 🚨 Главная проблема: Дыра в диспетчере инструментов (Rule 2.2)
Ты упомянул: *"в `primary_coder.md` есть `tools: {write: true, bash: true}`, но в Rust таких диспетчеров нет — фактически исполняются только `fs_*` через MCP"*.
Это прямое нарушение твоего же правила **2.2 (Запрет на ложь и молчаливые ошибки)**. Агент декларирует инструмент, LLM генерирует под него call, а диспетчер его не находит. Скорее всего, сейчас это приводит либо к молчаливому сбою, либо к скармливанию агенту "инструмент не найден", что ломает цикл.

**Решение:**
1. **Удалить декларации несуществующих тулов** из `.md` файлов агентов. Оставь только то, что реально есть.
2. **Или реализовать их** (рекомендую, см. ниже).

---

### 🏗️ Развилка 1: Где должны жить базовые code-tools? (Rust vs Deno/MCP)

Сейчас у вас `fs_read` и `fs_write` реализованы через MCP на Deno. Для прототипа это ок, но для production-агента это **плохая архитектура** по трём причинам:
1. **Производительность и Токены:** Чтение файла через Deno возвращает JSON, который гонится через stdio в Rust, парсится и скармливается LLM. Это медленно. Если LLM читает 10 файлов по 500 строк, stdio-накладные расходы съедят время.
2. **Логирование (Rule 2.5):** Ошибки чтения/записи файлов в Deno не попадут автоматически в твой кастомный Rust-логгер `TauriLogger`. Придётся дублировать логику в Deno, что нарушает SSOT.
3. **Безопасность (Rule 1.1 & 1.2):** Bash и FS write инструменты требуют жёсткого контроля (запрет git, temp-папки). Делать это в Deno сложнее, чем в Rust.

**Моя рекомендация:** Перенеси **базовые инструменты кодинга** (`read_file`, `write_file`, `edit_file`, `grep`, `bash`, `todo_write`) в Rust-ядро (`infra/tools/`). MCP-серверы на Deno оставь для **внешних интеграций** (web-search, GitHub API, специфичные доменные тулзы).

#### Как должен выглядеть Rust-диспетчер (правильный путь):
```rust
// src-tauri/src/infra/tools/mod.rs
pub fn execute_builtin_tool(name: &str, args: &serde_json::Value) -> Result<String, ToolError> {
    match name {
        "read_file" => tools::fs::read_file(args),
        "write_file" => tools::fs::write_file(args),
        "edit_file" => tools::fs::edit_file(args),
        "bash" => tools::shell::execute_bash(args), // С таймаутами!
        "todo_write" => tools::tasks::write_todo(args),
        // "lsp_get_definition" => tools::lsp::get_def(args), // Будущее
        _ => Err(ToolError::NotFound(name.into())),
    }
}
```

---

### 🛠️ Конкретные инструменты для Rust-ядра (Чеклист)

Если вы перенесёте тулы в Rust, реализуйте их сразу правильно, учитывая локальные модели:

#### 1. `read_file` (С пагинацией)
Локальные модели тупят на огромных файлах. Инструмент должен принимать `offset` и `limit`.
```rust
// Вывод ДОЛЖЕН содержать номера строк! Это критично для edit_file.
"  1 | use tauri::Manager;\n
  2 | \n
  3 | fn main() {"
```

#### 2. `grep` (Обёртка над ripgrep)
Не заставляй LLM писать regex. Пусть тул принимает `pattern` и `path`, а Rust внутри вызывает `rg --json`.
```json
{
  "tool": "grep",
  "arguments": {"pattern": "run_agent_node", "path": "src/domain"}
}
```

#### 3. `edit_file` (Exact Match)
Самый частый источник багов у агентов — кривые правки. Тул должен требовать точного совпадения `old_string` и заменять на `new_string`. Если `old_string` не найден — возвращать ошибку (не молчать!).

#### 4. `bash` (Уважающий Rule 9 и 1.5)
Реализуй его через `tokio::process::Command` с жёстким `timeout`.
```rust
pub async fn execute_bash(cmd: String, timeout_secs: u64) -> Result<String, ToolError> {
    // Парсим cmd, если видим "git commit" -> Block!
    if is_forbidden_git_op(&cmd) {
        return Err(ToolError::Forbidden("Git operations are blocked".into()));
    }
    // Запуск с таймаутом
    let output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        tokio::process::Command::new("cmd").args(&["/c", &cmd]).output()
    ).await;
    // ... логирование в TauriLogger
}
```

---

### 🧠 Развилка 2: Как улучшить понимание кода (LSP)

Так как у вас Tauri (Rust) + Deno (TS), использовать `grep` для поиска определений функций — это самоубийство для контекста локальной модели. Она утонет в мусоре.

**Рекомендация:** Добавь Rust-инструменты `lsp_get_definition` и `lsp_get_references`.
1. Запускай `rust-analyzer` как sidecar-процесс при старте приложения (как `llama-server`).
2. Для Deno/TS — используй встроенный Language Server из Deno.
3. Агент вызовет `lsp_get_definition(path="src/main.rs", line=10, char=5)`, и получит точный путь и строку, куда прыгнуть. Это сэкономит десятки тысяч токенов по сравнению с `grep`.

---

### 📝 Развилка 3: Интеграция `tasks/` (Rule 1.8)

У вас уже есть `todo_write` как opt-in тул. Свяжи его с правилом 1.8.
Инструмент `todo_write` на Rust должен **физически писать/читать** markdown-файлы в `tasks/`. Агент не должен держать план только в голове (в контексте LLM). Он вызывает тул, Rust читает `tasks/10.08.26 Переезд.md`, парсит чекбоксы, обновляет и возвращает статус агенту.

---

### 🚀 Развилка 4: Управление контекстом (Auto-Compaction)

Локальные модели (даже с 32k контекстом) быстро забиваются логами тулов (вывод `bash`, текст файлов).

**Что добавить в `orchestrator/mod.rs`:**
Перед вызовом LLM (строка ~694) проверяй длину истории (`messages`).
Если `total_tokens > 80% * ctx_size`:
1. Бери последние 5-10 сообщений.
2. Отправляй отдельный запрос к `llama-server` с промптом: "Суммаризуй эту историю, сохранив ключевые решения и пути к файлам".
3. Заменяй старые сообщения на `system: "Summary of previous steps: ..."` + последние 2-3 сообщения.

---

### 📋 Итоговый план действий (С учётом прототипа)

1. **Починить диспетчер (Bugfix):** В `dispatch.rs` сделай так, чтобы при вызове несуществующего `write` агент получал чёткую ошибку: `"Error: Tool 'write' is not available. Use 'fs_write' instead."`. Это уберёт "молчание".
2. **Перенести FS и Bash в Rust (Архитектура):** Вынеси `read_file`, `write_file`, `edit_file`, `bash` из Deno MCP в Rust-команды. Это закроет вопросы с логированием (Rule 2.5) и безопасностью (Rule 1.1).
3. **Доработать `read_file`:** Добавь нумерацию строк и параметры `offset`/`limit`. Это сразу повысить качество `edit_file`.
4. **Добавить `grep` тул в Rust:** Обёртка над `ripgrep`.
5. **LSP Sidecar:** Добавить запуск `rust-analyzer` в `infra/` и инструменты `lsp_*`.

----

ссылки на которые опиралась эта документация:
https://github.com/nousresearch/hermes-agent
https://github.com/anomalyco/opencode
https://github.com/charmbracelet/crush
https://github.com/QwenLM/Qwen-Agent
https://github.com/deepseek-ai/deepseek-harness
https://github.com/xai-org/grok-build
https://github.com/ultraworkers/claw-code
https://github.com/zai-org/GLM-4.5
https://github.com/Gitlawb/openclaude
https://github.com/google-gemini/gemini-cli
https://github.com/MiniMax-AI/Mini-Agent
https://github.com/MoonshotAI/kimi-code