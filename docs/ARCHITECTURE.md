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
Визуализация workflow-графов. Загружает данные с бэкенда через `invoke("get_workflow_graphs")` и рендерит их через vis-network (canvas).

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
- **Types** (`src/types.ts`) — глобальные TypeScript-интерфейсы

---

## 🦀 СЛОЙ 5: БЭКЕНД (Rust)

### Подслой 5.1: API (`src-tauri/src/api/`)

**Дверь:** `api/mod.rs` — реэкспортирует команды и `AppState`

| Файл | Команды | Зона ответственности |
|------|---------|---------------------|
| `config.rs` | `get_config`, `set_config_value`, `set_last_model`, `set_theme`, `set_prompt_format` | Чтение/запись конфигурации |
| `sessions.rs` | `get_sessions`, `load_session`, `save_session`, `delete_session`, `rename_session`, `open_session_folder` | CRUD сессий |
| `models.rs` | `get_models_catalog`, `get_model_params`, `set_model_params`, `reset_model_params`, `add_model` | Параметры моделей и каталог |
| `agents.rs` | `get_agents` | Загрузка списка агентов |
| `graph.rs` | `get_workflow_graphs` | Чтение YAML workflow и возврат структуры графа для UI |
| `chat.rs` | `chat_request`, `stop_processing` | Главный цикл чата |

### Подслой 5.2: Домен (`src-tauri/src/domain/`)

**Дверь:** `domain/mod.rs` — реэкспортирует `run_chat`, `AgentProfile`, `load_agents`

| Файл/Модуль | Зона ответственности |
|-------------|---------------------|
| `orchestrator/mod.rs` | Главный цикл: парсинг ответа LLM, вызов сабагентов/инструментов, рекурсивный `run_agent_node()`, built-in `get_agent_report`. **Точка входа `run_chat()` автоматически определяет: запускать workflow YAML или legacy `run_agent_node()`** |
| `orchestrator/prompt.rs` | Сборка системного промпта, инструкция по `get_agent_report` вместо рендера состояния |
| `orchestrator/runtime.rs` | Загрузка и запуск MCP-серверов |
| `workflow_engine/mod.rs` | **Графовый движок маршрутизации.** Исполняет YAML-графы (workflows). Точка входа — `run_workflow()` |
| `workflow_engine/parser.rs` | Парсинг YAML workflow файлов, структуры `WorkflowDef`, `NodeDef`, `EdgeDef`, поиск `visible_agents` |
| `workflow_engine/nodes.rs` | Исполнение узлов графа: `llm_worker`, `llm_classifier`, `system_condition`, `sub_workflow`, `switch`, `return` |
| `workflow_engine/context.rs` | Контекст выполнения: проход `{{ template }}` переменных, хранение outputs узлов |
| `workflow_engine/intent_classifier.rs` | **Built-in** intent-классификатор (не требует отдельного .md файла). Статусы инжектятся runtime из YAML |
| `parsers.rs` | Распаковка JSON от LLM, очистка think-тегов |
| `agent_manager.rs` | Парсинг .md файлов агентов, обработка INCLUDE, загрузка YAML workflow, вычисление `is_hidden` из `visible_agents` |

### Подслой 5.3: Инфраструктура (`src-tauri/src/infra/`)

**Дверь:** `infra/mod.rs` — реэкспортирует типы и функции всех инфра-модулей

| Файл | Зона ответственности |
|------|---------------------|
| `llm.rs` | Работа с llama-cpp-2, токенизация, генерация, сэмплирование, чтение GGUF, структура `ChatMessage` с полями `type`/`author` |
| `config.rs` | Структуры AppConfig/ModelParams, чтение/запись конфига, каталог моделей |
| `session_manager.rs` | Чтение/запись JSON-файлов сессий (единый массив `messages[]`) |
| `migration.rs` | Миграция старых сессий: конвертация `role`/`agent_name` → `type`/`author`. Запускается при старте приложения |
| `mcp_client.rs` | JSON-RPC клиент для MCP-серверов через stdin/stdout |
| `downloader.rs` | Скачивание .gguf файлов с прогрессом |

---

## 📊 Карта зависимостей

```
main.rs
  └→ api/ (дверь: api/mod.rs)
       ├→ domain/ (дверь: domain/mod.rs)
       │    ├→ workflow_engine/ (YAML графы — маршрутизация)
       │    │    ├→ parser.rs          — парсинг YAML, visible_agents
       │    │    ├→ nodes.rs           — типы узлов (llm_worker, switch, ...)
       │    │    ├→ context.rs         — контекст, template-переменные
       │    │    ├→ intent_classifier.rs — built-in классификатор (без .md)
       │    │    └→ mod.rs             — run_workflow() — вход в граф
       │    ├→ orchestrator/           — legacy run_agent_node() (для worker)
       │    ├→ parsers.rs
       │    └→ agent_manager.rs       — загрузка .md + YAML, visible_agents
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
User → Primary Agent (выбор в UI)
         ↓
    run_chat() ищет workflow YAML с visible_agents = agent_id
         ↓
    [Найден] → workflow_engine::run_workflow()
         │          ├→ llm_classifier → intent_classifier.rs (built-in)
         │          ├→ switch → маршрутизация по status
         │          ├→ sub_workflow → рекурсивный вызов другого YAML
         │          ├→ llm_worker → run_agent_node() для .md агента
         │          └→ system_condition → Rust-side проверка
         │
    [Не найден] → orchestrator::run_agent_node() (legacy)
```

**Запрещённые импорты:**
- ❌ `api/chat.rs → crate::infra::llm::ChatMessage` (кишки)
- ✅ `api/chat.rs → crate::infra::ChatMessage` (через дверь)
- ❌ `controllers/chat.ts → ../ui/render` (кишки)
- ✅ `controllers/chat.ts → ../ui` (через дверь)