# 🏗 Архитектура King Orch

Этот файл — навигационная карта для AI-агентов и разработчиков.

## Иерархия системы

Система состоит из **5 слоёв** (от верхнего к глубокому):

1. **Контроллеры** (Бизнес-логика UI)
2. **Сервисы** (Мосты к Tauri IPC)
3. **UI-компоненты** (Чистый рендеринг DOM)
4. **Утилиты** (Вспомогательные функции)
5. **Бэкенд (Rust)** — 3 подслоя с дверями:
   - 5.1. **API** (Tauri-команды) ← фасад `api/mod.rs`
   - 5.2. **Домен** (Оркестратор, Парсеры, Агенты) ← фасад `domain/mod.rs`
   - 5.3. **Инфраструктура** (LLM, Сессии, MCP, Сеть) ← фасад `infra/mod.rs`

**Ядро (Core)** — 3 файла (Store, EventBus, Main), пронизывающие фронтенд.

---

## 🚪 Принцип дверей

Каждый модуль имеет **файл-дверь** — единственную точку импорта для других модулей:

| Слой | Дверь (Фронтенд) | Дверь (Бэкенд) |
|------|:-:|:-:|
| UI | `src/ui/index.ts` | — |
| Сервисы | `src/services/index.ts` | — |
| Утилиты | `src/utils/index.ts` | — |
| Контроллеры | `src/controllers/index.ts` | — |
| API | — | `src-tauri/src/api/mod.rs` |
| Домен | — | `src-tauri/src/domain/mod.rs` |
| Инфра | — | `src-tauri/src/infra/mod.rs` |

**Правило:** Модуль может импортировать из другого модуля ТОЛЬКО через его дверь. Прямой импорт из «кишок» запрещён.

---

## 🧱 ЯДРО (Frontend Core)

### Store (`src/store.ts`)
Единый источник истины. Контроллеры мутируют стор напрямую.

### EventBus (`src/events.ts`)
Шина событий. Контроллеры НЕ импортируют друг друга — общение через `bus.emit()` / `bus.on()`.

### Main (`src/main.ts`)
Тонкий бутстраппер. Импортирует контроллеры через `./controllers` (дверь).

---

## ⚙️ СЛОЙ 1: КОНТРОЛЛЕРЫ

### ChatController (`src/controllers/chat.ts`)
Управление чатом, отправка LLM, рендеринг, автосохранение. Импортирует UI и сервисы через двери.

### SettingsController (`src/controllers/settings.ts`)
Конфигурация, параметры моделей, скачивание. Вызывает `invoke()` напрямую (нет сервисной обёртки).

### SessionController (`src/controllers/sessions.ts`)
Список сессий, удаление, переименование. Работает через `../services` (дверь).

### GraphController (`src/controllers/graph.ts`)
Визуализация workflow-графов на Drawflow. Загружает/сохраняет YAML workflow через Tauri-команды.

**Undo/Redo (Snapshot-based):** Перед каждой мутацией (`addNode`, `renameNode`, `toggleNodeDisabled`, `clearEditor`, `beginEdgeReconnect`, sidebar `change`-обработчики, закрытие сайдбара, начало Drawflow-native connection drag, начало перетаскивания одной ноды (mousedown на теле ноды), multi-drag, успешный edge reconnect) вызывается `saveCheckpoint()`, которая сохраняет глубокую копию `drawflow.Home.data` в `undoStack[]` (макс 50). Snapshots содержат только `.data` — zoom/pan viewport не участвует. Undo (Ctrl+Z) и Redo (Ctrl+Shift+Z) очищают редактор и пересоздают состояние через `editor.import()` с флагом `isRestoring: true` для подавления лишних событий.

**Dirty state:** Флаг `isDirty` — `true` при любой мутации (там же где snapshot), `false` после `handleSave()` или `handleOpen()`. После Undo/Redo сравнивается JSON текущего состояния с `pristineSnapshot` (слепок при последнем save/load) для корректного определения изменений. Визуально: зелёный/красный кружок рядом с именем файла в тулбаре.

**Node ID lifecycle:** Узлы Drawflow имеют единственный идентификатор — `data.id`, который совпадает с ключом в `drawflow.Home.data`. При переименовании ноды через сайдбар (`ge-node-id`) вызывается `renameNode(oldKey, newKey)`:
- Запись перемещается под новый ключ в хеше
- Обновляются все `conn.node` в `inputs[]`/`outputs[]` всех нод
- Обновляются target-ссылки в data динамических нод (`cases_priority[].to`, `default`, `sequential_to`, `true_to`, `false_to`)
- Обновляются DOM `id` и SVG CSS-классы соединений
- Вызывается `updateConnectionNodes` для перерисовки путей

В `handleSave()` добавлена валидация: если `edge.from` или `edge.to` не существует среди `nodes[].id` — показывается **красный тост** и `console.error` с JSON битых рёбер.

---

## 🌉 СЛОЙ 2: СЕРВИСЫ

### SessionService (`src/services/SessionService.ts`)
Единственный шлюз для Tauri-команд работы с сессиями.

### ModelService (`src/services/ModelService.ts`)
Параметры моделей и каталог. (Сейчас не используется контроллерами — SettingsController вызывает invoke напрямую.)

---

## 🎨 СЛОЙ 3: UI-КОМПОНЕНТЫ

Чистые функции. На входе — данные, на выходе — DOM-элементы.

- **Render** (`src/ui/render.ts`) — сообщения, мысли, сабагенты, инструменты
- **MessageMenu** (`src/ui/message-menu.ts`) — контекстное меню
- **ThoughtsBlock** (`src/ui/thoughts-block.ts`) — раскрывающийся блок мыслей
- **Confirm** (`src/ui/confirm.ts`) — модал подтверждения
- **Toast** (`src/ui/toast.ts`) — всплывающие уведомления

---

## 🛠 СЛОЙ 4: УТИЛИТЫ

- **Markdown** (`src/utils/markdown.ts`) — конвертация MD → HTML
- **Types** (`src/types.ts`) — глобальные TypeScript-интерфейсы, включая `AgentEntry` (id, name, description, entry_type, is_hidden)

---

## 🦀 СЛОЙ 5: БЭКЕНД (Rust)

### Подслой 5.1: API (`src-tauri/src/api/`)

**Дверь:** `api/mod.rs` — реэкспортирует команды и `AppState`

| Файл | Команды | Зона ответственности |
|------|---------|---------------------|
| `config.rs` | `get_config`, `set_config_value`, `set_last_model`, `set_theme`, `set_prompt_format` | Чтение/запись конфигурации |
| `sessions.rs` | `get_sessions`, `load_session`, `save_session`, `delete_session`, `rename_session`, `open_session_folder` | CRUD сессий |
| `models.rs` | `get_models_catalog`, `get_model_params`, `set_model_params`, `reset_model_params`, `add_model` | Параметры моделей и каталог |
| `agents.rs` | `get_agents` | Загрузка списка entry points (.md + YAML) |
| `graph.rs` | `get_workflow_graphs` | Чтение YAML workflow и возврат структуры графа для UI |
| `chat.rs` | `chat_request`, `stop_processing`, `get_prompt_preview`, `get_prompt_memory` | Главный цикл чата + Live-превью токенов/VRAM |

### Подслой 5.2: Домен (`src-tauri/src/domain/`)

**Дверь:** `domain/mod.rs` — реэкспортирует `run_chat`, `AgentEntry`, `AgentProfile`, `load_entry_points`, `build_system_prompt`, `load_agents`, а также workflow-контракт `load_workflows`, `find_workflow_by_stem`, `WorkflowDef`, `NodeType`

| Файл/Модуль | Зона ответственности |
|-------------|---------------------|
| `orchestrator/mod.rs` | Главный цикл: парсинг ответа LLM, вызов сабагентов/инструментов, рекурсивный `run_agent_node()`, built-in `get_agent_report`. В legacy .md режиме вся история non-thought сообщений inject'ится в `llm_messages` как отдельные ChatMessage. **Точка входа `run_chat()` автоматически определяет: запускать workflow YAML или legacy `run_agent_node()`** |
| `orchestrator/prompt.rs` | Сборка системного промпта, инструкция по `get_agent_report` вместо рендера состояния |
| `orchestrator/runtime.rs` | Загрузка и запуск MCP-серверов |
| `workflow_engine/mod.rs` | **Графовый движок маршрутизации.** Исполняет YAML-графы (workflows). Точка входа — `run_workflow()` |
| `workflow_engine/parser.rs` | Парсинг YAML workflow файлов, структуры `WorkflowDef`, `NodeDef`, `EdgeDef`, поиск по `file_stem` |
| `workflow_engine/nodes.rs` | Исполнение узлов графа: `llm_worker`, `llm_fact_extractor`, `system_condition`, `sub_workflow`, `switch`, `note` (pass-through), `return` |
| `workflow_engine/context.rs` | Контекст выполнения: проход `{{ template }}` переменных, хранение outputs узлов |
| `workflow_engine/fact_extractor.rs` | **Built-in** fact-экстрактор (не требует отдельного .md файла). Факты инжектятся runtime из YAML |
| `parsers.rs` | Распаковка JSON от LLM, очистка think-тегов |
| `agent_manager.rs` | Парсинг .md файлов агентов, обработка INCLUDE, загрузка entry points (`load_entry_points`) через `visible` поле. Парсит `single_report` (только 1 отчёт агента в сессии) в `AgentProfile` |

### Подслой 5.3: Инфраструктура (`src-tauri/src/infra/`)

**Дверь:** `infra/mod.rs` — реэкспортирует типы и функции всех инфра-модулей

| Файл | Зона ответственности |
|------|---------------------|
| `llm.rs` | Работа с llama-cpp-2, токенизация, генерация, сэмплирование, чтение GGUF, структура `ChatMessage` с полями `type`/`author`. Хелпер `push_report()` — дедупликация отчётов агента при `single_report` |
| `config.rs` | Структуры AppConfig/ModelParams, чтение/запись конфига, каталог моделей |
| `session_manager.rs` | Чтение/запись JSON-файлов сессий (единый массив `messages[]`) |
| `mcp_client.rs` | JSON-RPC клиент для MCP-серверов через stdin/stdout |
| `downloader.rs` | Скачивание .gguf файлов с прогрессом |

---

## 📊 Карта зависимостей

```
main.rs
  └→ api/ (дверь: api/mod.rs)
       ├→ domain/ (дверь: domain/mod.rs)
         │    ├→ workflow_engine/ (YAML графы — маршрутизация)
         │    │    ├→ parser.rs          — парсинг YAML, visible/find_workflow_by_stem
         │    │    ├→ nodes.rs           — типы узлов (llm_worker, switch, ...)
         │    │    ├→ context.rs         — контекст, template-переменные
         │    │    ├→ fact_extractor.rs  — built-in экстрактор фактов (без .md)
         │    │    └→ mod.rs             — run_workflow() — вход в граф
        │    ├→ orchestrator/           — run_agent_node() (для .md агентов)
        │    ├→ parsers.rs
        │    └→ agent_manager.rs       — загрузка .md + YAML, load_entry_points()
       └→ infra/ (дверь: infra/mod.rs)

Frontend:
main.ts
  └→ controllers/ (дверь: controllers/index.ts)
       ├→ ui/ (дверь: ui/index.ts)
       ├→ services/ (дверь: services/index.ts)
       └→ utils/ (дверь: utils/index.ts)
```

**Поток выполнения:**
```
User → Entry point (выбор в UI: .md с visible: true или YAML с visible: true)
         ↓
    run_chat() проверяет: есть ли YAML workflow с file_stem == agent_id?
         ↓
     [Да] → workflow_engine::run_workflow()
           │          ├→ llm_fact_extractor → fact_extractor.rs (built-in, возвращает JSON фактов)
           │          ├→ switch → приоритетная (cases_priority) или стандартная маршрутизация
           │          ├→ condition_check → бинарная проверка поля (true/false)
           │          ├→ sub_workflow → рекурсивный вызов другого YAML
           │          ├→ llm_worker → run_agent_node() для .md агента (история non-thought сообщений inject'ится автоматически, как в legacy)
           │          ├→ note → pass-through (визуальная заметка, не влияет на выполнение)
           │          └→ system_condition → Rust-side проверка (aggregate_and_output для вывода)
         │
    [Нет] → orchestrator::run_agent_node()
│          └→ вся история non-thought сообщений inject'ится в llm_messages
│             (автоматически — в workflow-режиме то же самое, {{ messages }} лишь дублирует её внутрь task)
```

### Live-превью токенов и VRAM

Счётчик под полем ввода (`controllers/chat.ts::updateTokenCounter`) показывает `токены / лимит (~МБ VRAM)` для выбранного entry point:

1. `get_prompt_preview(model_path, agent_id, message, history)` собирает сырую строку промпта:
   - **`.md`-агент** (`agent_id` найден в `load_agents`): системный промпт этого агента + история non-thought сообщений + текущее сообщение.
   - **Режим графа** (`agent_id` — это workflow): т.к. промпт выбирает узел графа, берётся **самый «тяжёлый» агент** — среди узлов `llm_worker` текущего графа выбирается агент с самым длинным системным промптом (`build_worst_agent_prompt`). Пиковая VRAM определяется одним LLM-вызовом (движок работает последовательно), поэтому worst-case = самый большой одиночный промпт. Sub-workflow узлы **не** раскрываются — считается только текущий граф. Граф без `llm_worker` → пустой системный промпт (учтётся только история + сообщение).
2. Фронт токенизирует строку (`countTokens`, HF-токенизатор).
3. `get_prompt_memory(...)` → `infra::llm::estimate_vram_mb`: `размер файла модели + KV-кэш`, где `effective_ctx = (prompt_tokens + max_gen + 128).min(context_size)`.

Прокси «по символам» для выбора худшего агента допустим для примерной оценки. В UI для графа в подсказке счётчика добавляется пометка «Оценка по самому тяжёлому агенту графа».

**Запрещённые импорты:**
- ❌ `api/chat.rs → crate::infra::llm::ChatMessage` (кишки)
- ✅ `api/chat.rs → crate::infra::ChatMessage` (через дверь)
- ❌ `controllers/chat.ts → ../ui/render` (кишки)
- ✅ `controllers/chat.ts → ../ui` (через дверь)