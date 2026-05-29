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
| `chat.rs` | `chat_request`, `stop_processing` | Главный цикл чата |

### Подслой 5.2: Домен (`src-tauri/src/domain/`)

**Дверь:** `domain/mod.rs` — реэкспортирует `run_chat`, `AgentProfile`, `load_agents`, `build_l0_manifest`, `ParsedOrchestratorResponse`

| Файл | Зона ответственности |
|------|---------------------|
| `orchestrator/mod.rs` | Главный цикл: парсинг ответа LLM, вызов сабагентов/инструментов, рекурсивный `run_agent_node()` |
| `orchestrator/prompt.rs` | Сборка системного промпта, рендер состояния сессии |
| `orchestrator/runtime.rs` | Загрузка и запуск MCP-серверов |
| `parsers.rs` | Распаковка JSON от LLM, очистка think-тегов |
| `agent_manager.rs` | Парсинг .md файлов агентов, обработка INCLUDE |

### Подслой 5.3: Инфраструктура (`src-tauri/src/infra/`)

**Дверь:** `infra/mod.rs` — реэкспортирует типы и функции всех инфра-модулей

| Файл | Зона ответственности |
|------|---------------------|
| `llm.rs` | Работа с llama-cpp-2, токенизация, генерация, сэмплирование, чтение GGUF |
| `config.rs` | Структуры AppConfig/ModelParams, чтение/запись конфига, каталог моделей |
| `session_manager.rs` | Чтение/запись JSON-файлов сессий, миграция старых форматов |
| `mcp_client.rs` | JSON-RPC клиент для MCP-серверов через stdin/stdout |
| `downloader.rs` | Скачивание .gguf файлов с прогрессом |

---

## 📊 Карта зависимостей

```
main.rs
  └→ api/ (дверь: api/mod.rs)
       ├→ domain/ (дверь: domain/mod.rs)
       │    └→ infra/ (дверь: infra/mod.rs)
       └→ infra/ (дверь: infra/mod.rs)

Frontend:
main.ts
  └→ controllers/ (дверь: controllers/index.ts)
       ├→ ui/ (дверь: ui/index.ts)
       ├→ services/ (дверь: services/index.ts)
       └→ utils/ (дверь: utils/index.ts)
```

**Запрещённые импорты:**
- ❌ `api/chat.rs → crate::infra::llm::ChatMessage` (кишки)
- ✅ `api/chat.rs → crate::infra::ChatMessage` (через дверь)
- ❌ `controllers/chat.ts → ../ui/render` (кишки)
- ✅ `controllers/chat.ts → ../ui` (через дверь)