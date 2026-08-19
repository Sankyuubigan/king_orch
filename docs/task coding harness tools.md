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

