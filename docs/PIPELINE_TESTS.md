# Pipeline Tests

Система тестирования workflow-пайплайнов (YAML графов) с fixture-папками.

## Структура fixture-папки

Каждый тест — отдельная папка в `test_cases/fixtures/<test_id>/`:

```
test_cases/fixtures/coding_team_bugfix1/
├── task.md          # Описание задачи для LLM (user_text)
├── buggy.py         # Исходный файл с багом (копируется в workspace)
├── expected.py      # Эталонный файл (для визуального сравнения)
└── validation.json  # Правила 3-уровневой валидации
```

### Формат validation.json

```json
{
  "workflow": "coding_team",
  "test_id": "coding_team_bugfix1",
  "model_path": "D:\\nn\\models\\llm\\uncen\\qwen3.8-9b\\Qwen3.8-9B-heretic-uncensored.i1-IQ4_NL.gguf",
  "expected_agents": ["primary_coder", "bug_analyst", "task_planner", "qa_diagnost"],
  "validate_l3_stdout_contains": ["PASS"],
  "validate_l3_no_stderr_contains": ["ERROR", "Traceback"],
  "timeout_sec": 300
}
```

## 3-уровневая валидация

| Уровень | Что проверяется | Механика |
|---------|----------------|----------|
| **L1 Structure** | Пайплайн отработал, агенты вызваны | `result.is_ok()`, `messages` не пуст, `expected_agents` присутствуют |
| **L2 File** | Файл изменён, баг исправлен | Сравнение файлов, проверка наличия исправления |
| **L3 Functional** | Код работает корректно | Запуск скрипта, проверка stdout/stderr |

## Запуск

### Из CLI

```bash
cd src-tauri
cargo test -- --ignored test_coding_team_bugfix_e2e
# или через test.bat:
test.bat "test_coding_team_bugfix_e2e -- --ignored"
```

### Из GUI

1. Студия агентов → вкладка "Тест агентов ИИ"
2. Выбрать режим "Pipeline test"
3. Выбрать тест из списка
4. Выбрать модель LLM
5. Нажать "Запустить тест"

## Отчёт

После прогона создаётся `test_cases/fixtures/<test_id>/report_<timestamp>.md` с:
- Итогом (ПРОЙДЕН / НЕ ПРОЙДЕН)
- Результатами по уровням
- Временем выполнения
- Использованной моделью
- preview сообщений агентов

## Архитектура кода

- **Загрузка + валидация**: `src-tauri/src/domain/pipeline_test.rs` (чистый Rust, без Tauri-зависимостей)
- **Tauri-команды**: `src-tauri/src/api/test.rs` (тонкий мост)
- **GUI**: `src/controllers/agent-test.ts` + `index.html` (подвыборка "Тест агентов ИИ")

## Добавление нового теста

1. Создать папку `test_cases/fixtures/<new_test_id>/`
2. Положить `task.md`, `buggy.py`, `expected.py`, `validation.json`
3. Для CLI-теста: добавить `#[test] #[ignore]` функцию в `pipeline_test.rs`
4. Для GUI: тест появится автоматически (сканирование fixtures/)

## Существующие тесты

### coding_team_bugfix1
- **Пайплайн:** Кодер
- **Задача:** Исправить баг в Python-функции `sum_list` (пропускает последний элемент)
- **Валидация:** L1 (структура) + L2 (файл исправлен) + L3 (код работает)
- **Запуск:** `test.bat "test_coding_team_bugfix_e2e -- --ignored"`

### psychotherapist_back_pain
- **Пайплайн:** Психотерапевт
- **Задача:** Запрос "болит поясница" — проверка пайплайна психосоматики
- **Ожидаемый результат:** Терминальный узел `data_collector` выдаёт ответ в чат
- **Валидация:** L1 (структура — вызваны `data_collector`, `soma_translator`, `decomposer`, `validator`) + проверка что последний ответ от `data_collector`
- **Запуск:** `test.bat "test_psychotherapist_back_pain_e2e -- --ignored"`
- **Описание пайплайна:**
  1. `extract_facts` — экстрактор фактов определяет `has_problem: true`, `has_somatic: true`
  2. `call_soma_translator` — расшифровка боли по НГМ
  3. `call_decomposer` — декомпозиция проблемы
  4. `call_validator` — валидация данных по 9 элементам
  5. `checking_insufficient_data` → `call_collector` — сбор недостающих данных
  6. `data_collector` — терминальный узел, отвечает пользователю

---

