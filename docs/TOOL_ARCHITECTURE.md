# Архитектура тулов King Orch (SSOT)

> Этот файл — **единственный источник правды** о том, где живут тула и куда класть новую.
> Остальные доки на него ссылаются, а не дублируют. Каталог конкретных тулов —
> [GENERATED_TOOL_CATALOG.md](GENERATED_TOOL_CATALOG.md) (автоген).

## Слои (L0–L5)

| Слой | Путь | Что там | Потребители |
|---|---|---|---|
| **L0** | `my-tauri-plugins/tools/agent-tools/` | CLI/MCP для ИИ-кодеров внешних харнессов: codemap, websearch, consult, code. Не входит в поставку King Orch | ZCode/OpenCode/Claude Code |
| **L1** | `my-tauri-plugins/{code,excel,diag}-tools/` + `json-store/` (и будущие lib-крейты) | Tauri-free SSOT-ядра: code, excel, diag, json-store | code-tools ест И хост И agent-tools; остальные — хост |
| **L2** | `my-tauri-plugins/tauri-plugin-*` | Движки с Tauri/OS-зависимостями: image-engine, ytdlp, downloader, llama-engine, logs… | Хост (и любые другие Tauri-проекты) |
| **L3** | `src-tauri/src/infra/tools/` | Трейт `Tool` + built-ins + тонкие адаптеры над L1/L2 + нишевые тула | Только King Orch |
| **L4** | `agents/<agent>/tools/*.ts` | Бизнес-математика конкретного агента; исполняется нодой `tool` через Deno (`workflow_engine/custom_tool.rs`) | Конкретный агент |
| **L5** | `src-tauri/mcp_servers/*.ts` | Deno MCP-серверы (JSON-RPC stdio): внешний сервис/протокол | Боевые агенты через `mcp_servers:` frontmatter |

**Правило нейтральности L0–L2** (репо `my-tauri-plugins`): см. README репо, раздел
«⚖️ Нейтральность репозитория». Ни имени, ни путей, ни env продуктов-потребителей
(включая King Orch) в именах крейтов, коде, env и доках. Имя — по capability.

### Исторические нарушения нейтральности в my-tauri-plugins (список к чистке)

Ниже — нарушения правила, ревизия конца 09.11.26: **кодовые нарушения (имена крейтов, env,
temp-префиксы, док-комменты тронутых файлов) исправлены и верифицированы компилятором обоих
репо** (cargo test плагинов + test.bat хоста — exit 0). Остаток — функциональные привязки
(константы API, тест-инфраструктура): механически не чинятся, вынос в параметры — отдельное
решение владельца. Всё новое пишется только нейтрально (core_rules.md §1.12).

| Нарушение | Где (факт) | Статус |
|---|---|---|
| ~~Продуктовый акроним `ko-*` в именах крейтов~~ | было: `ko-code-tools`/`ko-excel-tools`/`ko-diag-tools`/`ko-json-store` (пакеты, Cargo.toml обоих репо, use-статементы) | ✅ Ф4 09.11.26: git mv + perl-свип; cargo test плагинов и test.bat хоста — exit 0 |
| ~~env `KING_ORCH_DATA_ROOT`~~ | было: `json-store/src/roots.rs` | ✅ Ф4: → `JSON_STORE_DATA_ROOT` |
| ~~env `KING_ORCH_BINS_DIR` (websearch)~~ | было: `agent-tools/src/tools/websearch/store.rs` + e2e-тесты | ✅ Шаг 1b Ф2: цепочка `AGENT_TOOLS_DATA_DIR` → `%APPDATA%\agent-tools` → tmpdir; agent-tools 454 теста зелёные |
| ~~temp-префиксы `king_orch_dl_` / `king_orch_chrome_dl_`~~ | было: downloader `process_dl.rs`/`chrome_cdp.rs` | ✅ Ф4: → `dl_` / `chrome_dl_` |
| ~~env system1 `KING_ORCH_SYSTEM1_THREADS` / `KING_ORCH_DATA_DIR`~~ | было: system1 `runtime.rs`/`inference.rs` | ✅ Ф4: → `SYSTEM1_THREADS` / `SYSTEM1_DATA_DIR` (хост эти env не сеттит) |
| ~~Док-упоминания «King Orch» в тронутых файлах~~ | codemap, agent-tools README/src, websearch, code/excel/diag/json-store, downloader, cloud-routers, PLUGIN_STANDARD, build-toolkit/llama-engine доки | ✅ Ф4: ~50 строк perl-свипом |
| Функциональные привязки (НЕ проза — механически не чинятся) | image-engine `HOST_APP_IDENTIFIER="com.kingorch.app"` / `HOST_DATA_DIR_NAME="KingOrchData"`; llama-engine/cloud-routers `LEGACY_HOST_DIRS`; llamacpp_installer kingorch_* temp-префиксы; system1 paths.rs `join("KingOrchData")`; about-updates rollback-URL релиза; image-engine bench-пути `../../king_orch_3/test/bench` (5 файлов) | ⏳ вынос в параметры API — отдельное решение владельца (не в фазах 2–3) |

## Decision tree: куда класть новую тулу

Проверять **строго сверху вниз**, не перескакивать:

1. **Нужна хосту И другим харнессам (≥2 потребителя)?** → **L1** lib-крейт в
   `my-tauri-plugins`, имя по capability (без продуктовых акронимов), жёстко
   Tauri-free (Tauri-зависимость = отказ — agent-tools бинарь без Tauri).
2. **Нужен GUI/OS/движок (графика, аудио, видео, окна)?** → **L2** `tauri-plugin-*`.
3. **Нишевая capability продукта King Orch?** → **L3** `infra/tools`: built-in или
   тонкий адаптер над L1/L2 (формат/SSOT — в ядре, адаптер без логики).
4. **Бизнес-математика одного агента, правка без пересборки Rust?** → **L4**
   `agents/<agent>/tools/*.ts` (чистые функции; общая нескольким — `agents/_shared/tools/`).
5. **Внешний сервис/протокол, которого нет нативно?** → **L5** `mcp_servers/`.
   **НОВЫЙ MCP-сервер — только по явному решению владельца**, если нативный путь
   (L1–L4) не подходит.

## Запрет межслойного дублирования

Одна возможность = одна реализация. Урок-прецедент (удалены 09.11.26): MCP-серверы
`fs_read`/`fs_write` дублировали нативные code-тулы `read_file`/`write_file` (L1),
`downloader` был слабее движка `tauri-plugin-downloader` (L2), `local_rag` — заброшенный
прототип, перекрытый нативным grep + `ast_analyzer.search_code`. Перед созданием тула —
проверь каталог и decision tree: если возможность уже есть в другом слое, **используй её,
а не копируй**.

## Осознанные размещения (спорные на первый взгляд — решено)

- `infra/tools/refmod_ops.rs` (warp/hue/splice) и `media_lora.rs` — нишевые операции
  продукта, живут в L3 по принципу «host = нишевое»; формат карточки RefMod — SSOT
  в плагине image-engine (L2).
- Субтитры YouTube (`prepare_video`/`get_chunk`): механика — SSOT в плагине
  tauri-plugin-ytdlp (L2, `engine::subtitles`: VTT-парсер/спонсорские/чанки,
  Tauri-free); хост — тонкий адаптер L3 (`infra/tools/youtube.rs`) + резолв
  каталога движка из конфига плагина. Паттерн «media → image-engine».
- Физические переносы между слоями не делаются без причины: слои легитимны, дублей в них нет.

## Известные ограничения (зафиксировано, решение отдельно)

- `agents/coder/task_planner.md`, `agents/coder/project_mapper.md` хардкодят дев-путь
  `D:\Projects\my-tauri-plugins\bin\agent-tools.exe` — на машине пользователя пути нет
  (agent-tools.exe не бандлится). Либо бандлить exe, либо убирать codemap-инструкции из
  продуктовых агентов — решение владельца.
- `markdown_section_reader` (L5) подключён только архивным агентам (archive/neuro_reprogrammer)
  — жив, но кандидатов на миграцию не имеет.
- Полная миграция MCP→натив (web_search/docs_fetcher/searxng_search/ast_analyzer/…) —
  отдельные проекты по мере решения владельца (см. tasks/261104_2149-05644410).

## Ссылки

- Каталог тулов: [GENERATED_TOOL_CATALOG.md](GENERATED_TOOL_CATALOG.md) (автоген из кода)
- Создание агентов и параметр `tools`: [AGENT_CREATION_GUIDE.md](AGENT_CREATION_GUIDE.md)
- Контракты CLI для кодеров (L0): глобальная дока `global_ai_docs/agent_tools.md`
- Правило нейтральности репо плагинов: `my-tauri-plugins/README.md`
