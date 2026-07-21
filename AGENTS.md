# 📜 Правила проекта King Orch


> **⚠️ ПРИОРИТЕТ №1 (НАРУШЕНИЕ = КРИТИЧЕСКАЯ ОШИБКА)**

### 00. Глобальная документация (читать ОБЯЗАТЕЛЬНО)
> **Перед любой работой — прочитать и учитывать документацию из:**
> 1. `D:\Projects\docusaurus-starter\docs\Sega Mega Note\Моя картотека\software\настройки\global_ai_docs` — глобальные правила и контекст проекта.
>
> **А также всегда проверять локальную документацию в папке `docs/` текущего проекта.**

### 0. Конфиг приложения
- Путь: `C:\Users\user\AppData\Roaming\com.kingorch.app\app_config.json`
- Содержит список `.gguf` моделей, последнюю использованную модель (`last_model`), параметры (`model_params`), настройки контекста.

>
> ### 1. Запрет на удаление кеша C++ компиляции
> **ЗАПРЕЩЕНО удалять `target/debug/build/llama-cpp-sys-2-*` или `target/debug/build/llama-cpp-2-*`.**
> Эти директории содержат скомпилированные C++ объектные файлы llama.cpp. Полная перекомпиляция с нуля занимает >20 минут (`llama-cpp-sys-2` собирает CUDA через nvcc и компилирует ~200 C++ файлов).
>
> **Если сборка падает с ошибкой линковки `.obj` файлов:**
> 1. Найди конкретный битый `.obj`/`.dir` (например, `ggml-cuda/Release/`)
> 2. Удали ТОЛЬКО его, а не всю папку `build/*`
> 3. Запусти `test.bat` — cargo дособерёт только удалённые файлы
>
> ### 2. Расположение тестов
> **Тестовые файлы (`.test.ts`) — ТОЛЬКО в `test/`. Запрещено создавать тесты внутри папки `src/`.**

## 1. Архитектура

### Стек
- **Backend**: Rust (Tauri 2.0) — 3-слойная архитектура: API → Домен → Инфра
- **Frontend**: TypeScript + Vite — контроллеры + двери (index.ts)
- **Модели**: только локальные `.gguf` файлы (llama.cpp через llama-cpp-2)
- **Агенты**: Markdown файлы (`.md`) с YAML frontmatter

### Трёхслойная архитектура бэкенда
```
api/      — Tauri-команды (тонкий слой, делегирует в domain/infra)
domain/   — Бизнес-логика (оркестратор, парсеры, агенты)
infra/    — Инфраструктура (LLM, сессии, MCP, конфиг)
```
- `api/` импортирует из `domain/` и `infra/` ТОЛЬКО через их двери (`mod.rs`)
- `domain/` импортирует из `infra/` ТОЛЬКО через дверь (`mod.rs`)
- `infra/` не импортирует другие слои

### Принципы
- **Изоляция контекста**: каждый агент видит только то, что ему нужно
- **Единый источник правды**: Массив `messages[]` в JSON-файле сессии. Все результаты агентов хранятся как сообщения с `namespace` и `agent_name`.
- **Единый стейт (фронтенд)**: Запрещено дублировать переменные в контроллерах. Всё в `Store`.
- **Разделение бизнес-логики и маршрутизации**: `.md` файлы агентов содержат ТОЛЬКО бизнес-логику (стиль общения, правила представления). Вся маршрутизация (вызов сабагентов, проверка статусов, циклы) — в `.yaml` workflow графах. Классификация контекста — built-in в `workflow_engine/intent_classifier.rs`.

## 2. Сессия и сообщения

### Формат сообщения
```rust
pub struct ChatMessage {
    pub id: Option<String>,          // "msg_0", "msg_1"...
    #[serde(rename = "type")]
    pub msg_type: String,            // "message" | "thought"
    pub content: String,
    pub namespace: Option<String>,   // "main", "problem_1"...
    pub sub_calls: Option<Vec<SubCall>>,
    pub author: Option<String>,      // "user", "system", или ID агента
}
```

Все данные сессии — массив `messages[]`. Результаты воркеров сохраняются как `type: "thought"` с указанием `author` (ID агента).

### Получение данных агентами

**Режим графа (YAML workflow):** ВСЯ история non-thought сообщений (`type: "message"`) из `messages[]` автоматически inject'ится в `llm_messages` каждого `llm_worker` (`orchestrator/mod.rs:287`, цикл по `messages`, пропускающий только `thought`). Автор графа НЕ обязан писать `{{ messages }}` — история переписки видна всем агентам графа по умолчанию. 

**Legacy .md режим:** Все non-thought сообщения из `messages[]` автоматически inject'ятся как отдельные `ChatMessage` в `llm_messages` перед текущим user_text. Порядок: `[system, msg_1, ответ_1, msg_2, ответ_2, ..., current_user_text]`. Отдельные thought-сообщения в историю не попадают.

В дополнение к автоматической истории агенты также могут использовать built-in инструмент пакетного запроса:
```json
{"thought": "...", "tool": "batch_get_agent_report", "arguments": {"queries": [{"author": "soma_translator", "namespace": "problem_1"}, {"author": "user", "namespace": "main"}]}}
```

## 3. Логика вызова сабагентов (Backend)

За вызовы отвечает **domain-слой** (`domain/orchestrator/`):
- `mod.rs` — цикл обработки: парсинг ответа LLM, вызов сабагентов/инструментов
- `prompt.rs` — сборка системного промпта
- `runtime.rs` — загрузка MCP-серверов

## 4. Система агентов

### Архитектура: Бизнес-логика vs Маршрутизация

**Главное правило:** Маршрутизация и бизнес-логика **строго разделены**.

| Что | Где | Формат | Пример |
|-----|-----|--------|--------|
| **Бизнес-логика** | `.md` файлы агентов | Markdown | `therapist_communicator.md` |
| **Маршрутизация** | `.yaml` файлы в `workflows/` | YAML граф | `main_conversation_flow.yaml` |
| **Классификация контекста** | Built-in в `workflow_engine/` | Rust | `intent_classifier.rs` |

### Вызов инструментов (MCP)
```json
{"thought": "...", "tool": "имя", "arguments": {...}}
```

## 5. Код-стайл

### TypeScript (frontend)
- Контроллеры НЕ импортируют друг друга — общение через шину (EventBus)
- UI-компоненты — чистые функции, без стейта
- `invoke()` к Tauri инкапсулирован в Сервисах (где есть сервисная обёртка)

### Markdown (агенты)
- Frontmatter YAML между `---`
- Include-макрос: `<<INCLUDE: database/file.md>>`

## 6. Сессии и хранение

- Сессии хранятся как JSON в `app_data_dir/sessions/`
- Единственный источник правды — массив `messages[]` в JSON-файле
- Draft — автосохранение с debounce 500ms

## 7. MCP-серверы

- Расположение: `src-tauri/mcp_servers/`
- Формат: `.cjs` (Node.js) или `.ts` (Deno)
- Протокол: JSON-RPC 2.0 через stdin/stdout
- Базовый фреймворк: `mcp_base.cjs`

## 8. Сборка и тестирование (специфика King Orch)

> ⚠️ **ЗАПРЕЩЁН прямой вызов `cargo build` / `cargo test` / `cargo check`**
> (см. `global_ai_docs/desktop_rust_tauri/rules.md:29`)
> Только через `build.bat` и `test.bat` — они сами находят VS (vswhere) и вызывают `vcvarsall.bat`.

**Перед любой Rust-командой — убедиться, что sccache-сервер запущен:**
```powershell
sccache --start-server        # если ещё не запущен
```

### Rust-сборка (только так!)
```powershell
cmd /c "cd /d `"D:\Projects\king_orch_3`" && build.bat 2>&1"
```

### Rust-тесты (только так!)
```powershell
cmd /c "cd /d `"D:\Projects\king_orch_3`" && test.bat 2>&1"
```

### Фронтенд-сборка и тесты
```powershell
npm run build
npm test            # все тесты vitest
```

#### Правила написания тестов frontend
- **Окружение**: `jsdom` (без layout-движка). `getBoundingClientRect()` возвращает нули, `offsetHeight` = 0
- **Мокание DOM-метрик**: Использовать `vi.spyOn(el, "getBoundingClientRect").mockReturnValue(...)`
- **Мокание Tauri API**: `vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))`
- **Доступ к private-методам**: через `(ctrl as any).methodName()`

**Специфические технические особенности сборки King Orch:**
- Если `cargo build` падает с `fatal error C1034: windows.h` — VS-окружение не настроено, запусти `build.bat` или `test.bat` один раз.
- Если сборка падает с CUDA/nvcc — CUDA 12.9+ несовместима с `llama-cpp-sys-2` 0.1.146. Временно отключи CUDA в Cargo.toml (`features = ["mtmd"]` вместо `["cuda", "mtmd"]`).

### Правила для тестов (важно!)
- **Таймаут на любые тесты — не более 2 минут (120 000ms) (`timeout: 120000` в tool call).**
- **LLM-тесты (с реальной моделью) — только `#[ignore]`.** Запускать строго через `cargo test -- --ignored` по явной просьбе пользователя.