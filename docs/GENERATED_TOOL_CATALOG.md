# Каталог инструментов King Orch (сгенерировано автоматически)

> ⚠️ ФАЙЛ СГЕНЕРИРОВАН скриптом `scripts/gen_docs.cjs`. **НЕ ПРАВИТЬ РУКАМИ** —
> правки будут перезаписаны при следующей генерации. Правь код, затем перегенерируй.

## Built-in инструменты (код)

- `todo_write` — Управление чек-листом задач агента. Используй для многошаговых задач, чтобы не терять план (он переживает сжатие контекста). Действия: add (нужен title), done/remove (по index или title), clear (очистить), list (показать).
- `todo_list` — Показать текущий чек-лист задач агента (что сделано, что осталось).
- `emit_signal` — Сохранить сигнал/маркер в сессию. Другие агенты, экстрактор и phase_router увидят его. Принимает key (имя сигнала) и value (произвольный JSON-объект с данными).
- `read_spill` — Дочитать полный результат большого инструмента, сохранённый в файл spills (локатор приходит в сообщении '[РЕЗУЛЬТАТ ИНСТРУМЕНТА сохранён в файл spills]'). Принимает path (путь к spill-файлу). Возвращает полное содержимое (обрезанное до 16К символов).

## Инструменты кодинга (Rust-ядро, infra/tools/ — SSOT)

- `read_file` [read-only] — Прочитать текстовый файл с номерами строк. path — путь к файлу (абсолютный или относительно корня проекта). offset — номер строки, с которой начать (1-based, по умолчанию 1). limit — сколько строк прочитать (по умолчанию 200). Чтение доступно по любому пути.
- `read_many_files` [read-only] — Прочитать несколько файлов одним вызовом. paths — массив путей (абсолютных или относительно корня проекта). Каждый файл выводится в блоке с заголовком. Экономит вызовы: вместо серии read_file.
- `write_file` [read+write] — Создать новый файл или перезаписать существующий. path — путь (абсолютный или относительно корня проекта). content — полное содержимое файла. Запись внутри корня разрешена автоматически; запись вне корня — запросит подтверждение пользователя. Для точечных правок используй edit_file, а не перезапись целиком.
- `edit_file` [read+write] — Точечная замена фрагмента файла по ТОЧНОМУ совпадению. path — путь к файлу; old_string — искомый фрагмент (должен совпадать 1:1, включая отступы); new_string — замена. replace_all — заменить все вхождения (по умолчанию только первое). Если old_string не найден — вернётся ошибка (тишина недопустима). Запись вне корня — запросит подтверждение.
- `grep` [read-only] — Поиск по регулярному выражению (regex) в файлах проекта с учётом .gitignore. pattern — регулярное выражение; path — папка или файл для поиска (по умолчанию корень проекта); max_results — лимит совпадений (по умолчанию 200); case_sensitive — учёт регистра (по умолчанию false). Вывод: файл:строка: текст.
- `glob` [read-only] — Найти файлы по маске (glob) с учётом .gitignore. pattern — маска, например 'src/**/*.ts' или '*.rs'; path — папка для поиска (по умолчанию корень проекта); max_results — лимит (по умолчанию 200). Возвращает список путей.
- `list_directory` [read-only] — Показать содержимое папки в виде дерева с учётом .gitignore. path — папка (по умолчанию корень проекта); depth — глубина обхода (по умолчанию 2). Чтение доступно по любому пути.
- `bash` [read+write] — Выполнить shell-команду в корне проекта (cwd — корень или указанная папка внутри корня). command — команда; timeout_sec — таймаут (по умолчанию 30, максимум 120); cwd — рабочая папка относительно корня. ЗАПРЕЩЕНЫ git-мутации (commit/push/pull/rebase/merge/checkout/reset и т.п.) и разрушительные команды. Результат — stdout+stderr. Запуск в корне авто-разрешён (логируется); вне корня — запросит подтверждение.
- `run_tests` [read+write] — Запустить тесты/сборку/линт по БЕЛОМУ СПИСКУ команд (без подтверждения пользователя — это безопасно, команды предопределены и выполняются в корне). command — одно из: npm_test, npm_build, npm_lint, npm_typecheck, cargo_test, cargo_check. filter — необязательный фильтр тестов (передаётся после команды). timeout_sec — таймаут (по умолчанию 120, максимум 300). Применимо для проверки, что твой код компилируется/тесты проходят.
- `lsp_get_definition` [read-only] — Найти определение символа (функции, переменной, типа) по позиции в файле через LSP-сервер (rust-analyzer). path — файл; line/character — позиция (0-based). Возвращает список локаций (файл:строка:колонка). Требует установленного rust-analyzer. Read-only.
- `lsp_get_references` [read-only] — Найти все ссылки/использования символа по позиции в файле через LSP-сервер (rust-analyzer). path — файл; line/character — позиция (0-based); include_declaration — включать ли объявление (по умолчанию true). Возвращает список локаций. Требует установленного rust-analyzer. Read-only.
- `lsp_get_diagnostics` [read-only] — Получить диагностику (ошибки, предупреждения) для файла через LSP-сервер (rust-analyzer). path — файл. Возвращает список [SEVERITY] строка:колонка сообщение. Требует установленного rust-analyzer. Read-only.

## MCP-серверы (Deno, src-tauri/mcp_servers/)

- **academic_search** — инструменты: `AcademicSearch`, `academic-search-mcp`
- **ast_analyzer** — инструменты: `analyze_file`, `ast-analyzer-mcp`, `search_code`, `trace_function`
- **ast_treesitter** — инструменты: `ast-map-mcp`, `generate_and_save_ast`
- **browser** — инструменты: `BrowserFetch`, `BrowserSearch`, `BrowserSession`, `browser`
- **deno_runner** — инструменты: `deno-runner-mcp`, `run_sandbox`
- **docs_fetcher** — инструменты: `FetchArticle`, `FetchGithubReadme`, `WebFetch`, `WebFetchBatch`, `docs-fetcher-mcp`
- **fs_read** — инструменты: `Glob`, `Grep`, `LS`, `Read`, `fs-read-mcp`
- **fs_write** — инструменты: `Write`, `fs-write-mcp`
- **github_search** — инструменты: `GithubSearch`, `github-search-mcp`
- **knowledge_api** — инструменты: `Weather`, `WikipediaSearch`, `knowledge-api-mcp`
- **local_rag** — инструменты: `index_directory`, `local-rag-mcp`, `search_code`
- **markdown_section_reader** — инструменты: `ReadSection`, `markdown-section-reader-mcp`
- **mcp_base** — инструменты: `MyTool`, `my-server`
- **search_cache** — инструменты: `_(инструменты не распознаны статически)_`
- **search_stats** — инструменты: `_(инструменты не распознаны статически)_`
- **searxng_search** — инструменты: `SearxngSearch`, `searxng-search-mcp`
- **time** — инструменты: `GetCurrentTime`, `time-mcp`
- **web_http** — инструменты: `_(инструменты не распознаны статически)_`
- **web_search** — инструменты: `WebSearch`, `web-search-mcp`
- **youtube_mcp** — инструменты: `get_chunk`, `prepare_video`, `youtube-mcp`
- **youtube_search** — инструменты: `YoutubeSearch`, `youtube-search-mcp`

