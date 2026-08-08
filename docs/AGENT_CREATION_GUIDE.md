# Инструкция по созданию агентов для King Orch

Эта инструкция описывает архитектуру и правила создания агентов. Используй этот файл как системный промпт для ИИ-помощника.

Помимо основного режима (YAML-графы), существует legacy-режим общения с отдельными `.md` файлами напрямую: если для entry point не найден видимый workflow (или файла только .md), агент запускается напрямую как `.md` (см. раздел «Как работает visible»). В целях тестирования можно менять агентов вручную в рамках сессии.

---

## 🤖 Роль для ИИ-помощника (Промпт)
**"Ты — эксперт по проектированию ИИ-агентов для фреймворка King Orch. Твоя задача — писать готовые `.md` файлы агентов и `.yaml` workflow-графы по запросу пользователя, строго следуя правилам ниже."**

---

## 🏗 Архитектура: Разделение ответственности

| Слой | Формат | Что содержит | Кто пишет |
|------|--------|-------------|-----------|
| **Коммуникатор** | `.md` | Стиль общения, эмпатия, психологические фишки, перевод терминов. **Чистая бизнес-логика** | Команда агентов |
| **Workflow (граф)** | `.yaml` | Маршрутизация: какой узел вызывать, в каком порядке, по какому условию. **Техническая логика** | Команда агентов |
| **Экстрактор фактов** | generic built-in + YAML конфиг | Generic движок в Rust, вся логика (факты + критерии) в YAML файле команды | Движок (Rust — generic) + Команда агентов (факты) |
| **Воркер** | `.md` | Узкоспециализированная задача, ответ = `thought` или `message` | Команда агентов |

**Главное правило:** В `.md` файлах **НЕТ** логики маршрутизации (вызовов сабагентов, проверок статусов, циклов). Всё это — в YAML графах.

---

## ✍️ Принципы написания системных промптов

- **Негативные запреты слабее позитивных инструкций.** Модель лучше следует «делай так», чем «не делай эдак». При написании агента формулируй желаемое поведение как прямые инструкции (что делать, как делать), а не как список запретов.
- **Дефолт сильнее позитивной инструкции.** Модель по умолчанию склонна к определённым поведенческим паттернам (эмпатия, советы, вежливость, ответы «как обычно»). Если в контексте агента дефолтное поведение противоречит его роли — одной позитивной инструкции может не хватить, и модель «сползёт» в дефолт.
- **Практическое следствие:** пиши промпт как позитивные инструкции, но критичные для роли правила выноси в отдельный компактный блок жёстких правил (положительные формулировки ядра: «работаешь только на X», «сценарий — гипотеза», «без эмпатии»), вместо размазанных по тексту «ЗАПРЕЩЕНО ...».
- **Не дублируй одно и то же правило несколько раз** в разных формулировках — это размывает фокус. Одно правило — одно место.

---

## 📂 Расположение и формат файлов

- Все агенты и workflow хранятся в папке `agents/` (и её подпапках). Подпапки (frontend/, backend/, transitions/, database/, grammars/, signals/, archive/) — это организационные группы команды, движку они не важны.
- Движок **рекурсивно** сканирует `agents/` и собирает все `.md` (агенты) и `.yaml`/`.yml` (workflow графы). Поэтому ID (file_stem — имя файла без расширения) должен быть **уникальным** внутри всего проекта.
- **Workflow графы кладутся в подпапку `transitions/`** (по конвенции King Orch; в архиве — вариант `workflows/` устарел).
- Каждый агент — один `.md` файл: **Frontmatter** (шапка с метаданными YAML) + **Системный промпт** (тело агента).

---

## 🛠 Архитектура файла агента (Frontmatter)

```yaml
---
name: Имя агента (человекочитаемое)
description: Краткое описание того, что он делает
visible: true        # опционально: показывать в UI как точку входа
single_report: true  # опционально: хранить в сессии только 1 (последний) отчёт агента
tools: ["write", "bash"]   # опционально: включить в промпт описание инструментов
mcp_servers: ["server_name"]  # опционально: прикрепить MCP-серверы (из src-tauri/mcp_servers/)
---
Текст системного промпта начинается здесь...
```

> **Примечание:** Поля `mode` (primary/router/worker) и `subagents` **устарели** и больше не парсятся из frontmatter: все агенты — одного типа. Видимость в UI определяется полем `visible: true/false` — либо в frontmatter `.md` файла, либо в корне YAML workflow. Если поле не указано — entry point скрыт из UI.
>
> `tools` в frontmatter — **JSON-массив строк**, а не YAML-мапа: `tools: ["write", "bash"]`.

### Параметр `single_report` (экономия контекста в сессии)

По умолчанию каждый вызов агента добавляет новый отчёт в массив `messages[]` сессии — при повторных вызовах одного агента отчёты накапливаются и раздувают контекст.

Если указать `single_report: true`, то перед сохранением нового отчёта движок **удаляет все прошлые сообщения этого же автора** (по полю `author = ID агента`; сигналы-сообщения при этом не трогаются). В сессии остаётся только один — самый свежий — отчёт агента.

- Работает в обоих режимах: graph (YAML `llm_worker`) и legacy `.md` (вызов сабагента через `target`).
- Дедупликация выполняется единой функцией `infra::push_report(messages, msg, single_report)`.
- Рекомендуется для «шумных» воркеров, которые вызываются многократно (примеры: `soma_translator`, `validator`, `decomposer`).

---

## 🔄 Неймспейсы (Контексты проблем)

### Что такое неймспейс?
`namespace` — поле сообщения, изолирующее контексты внутри сессии. По умолчанию агенты работают в `"main"`, workflow может создавать неймспейсы для отдельных проблем, чтобы агенты не мешали друг другу.

### Какие данные видит агент (важно!)

История сессии **инжектится автоматически**: в `llm_messages` агента (и в шаблоны `{{ messages }}`) попадают ВСЕ сообщения сессии, кроме `type: "thought"` (отчёты воркеров и мысли). Порядок: `[system, история..., текущий user_text]`. Агенту ничего писать в конце явно не нужно.

В дополнение к автоматической истории все агенты могут использовать built-in инструмент пакетного запроса:
- `batch_get_agent_report(queries)` — пакетный запрос (массив `{author, namespace}`), возвращает самые свежие отчёты по каждому `author + namespace`.
- Для `author: "user"` namespace игнорируется.

---

## 🧩 Workflow графы (YAML)

### Структура workflow
```yaml
name: Название графа
visible: true                      # показывать в UI как точку входа (опционально)

config:
  facts:                           # Факты для llm_fact_extractor (можно inline)
    - id: is_greeting
      criteria: "..."              # Критерии определения факта (опционально)
  facts_file: "facts.yaml"         # Или вынести факты + extractor_prompt в отдельный файл
  extractor_prompt: "..."          # Кастомный системный промпт для llm_fact_extractor (опционально)
  default_llm_params: creative     # Дефолтный пресет LLM-параметров (имя из sampling_presets.json)

nodes:
  - id: node_name
    type: llm_worker | llm_fact_extractor | llm_freeform | system_condition | sub_workflow | switch | llm_sequential_switch | condition_check | signal_router | return | note
    # тип-специфичные поля...

edges:
  - from: node_name
    to: next_node
    # condition / case — для условных переходов
```

### Контекстные переменные (шаблоны в любом поле узла)

| Переменная | Что подставляется |
|------------|-------------------|
| `{{ user_message }}` | Текущее сообщение пользователя |
| `{{ messages }}` | История сессии как JSON-массив (все non-thought сообщения, только `type`/`author`/`content`) |
| `{{ signals }}` | JSON-массив signal-сообщений сессии |
| `{{ nodes.X.output }}` | JSON-вывод узла X |
| `{{ nodes.X.output.Y }}` | Поле Y из вывода узла X |

### Типы узлов

| Тип узла | Что делает | Ключевые поля |
|----------|-----------|---------------|
| `llm_worker` | Вызывает `.md` агента с задачей. Результат — `thought` (свёрнутый отчёт) или `message` (в чат), управляется `output_type` | `agent`, `task`, `output_type`, `inject_reports`, `llm_params` |
| `llm_fact_extractor` | Generic экстрактор фактов. Факты + критерии из `config.facts` / `config.facts_file`. Возвращает JSON `{"fact_id": true/false, ...}` | `input` |
| `llm_freeform` | Зовёт LLM без системного промпта (только история чата) — для off-topic | `input` |
| `system_condition` | Rust-side проверка состояния сессии / агрегация отчётов | `action`, `required` |
| `sub_workflow` | Рекурсивный вызов другого YAML графа (по file_stem) | `workflow` |
| `switch` | Маршрутизация (3 режима, ниже) | `input`/`input_object`, `switch_field`, `cases_priority`, `default` |
| `llm_sequential_switch` | Запускает ВСЕ ветки, где факт истин (одна за другой — sequential) | `input_object`, `cases_priority`, `default` |
| `condition_check` | Ветвление по bool-полю JSON: `true_to`/`false_to` + `sequential_to` | `input_object`, `field`, `true_to`, `false_to`, `sequential_to` |
| `signal_router` | Маршрутизация по сигналу: ищет в signal-сообщениях значение `signal_name.field` и мапит через `cases_priority` | `signal_name`, `field`, `cases_priority`, `default` |
| `note` | Комментарий-заглушка. Если задан `system_message` — выводит текст в чат как системное сообщение | `input`, `system_message` |
| `return` | Завершает текущий workflow | — |

**Общие поля узлов:**
- `disabled: true` — узел не выполняется при активации workflow (для отключенных веток, недоставляющих маршрутов).
- `ui_pos: {x, y}` — координаты в редакторе графов (генерируется редактором).
- `system_condition, switch, condition_check` — подробности ниже.

### Параметр `output_type` (для `llm_worker`)

Управляет, куда сохраняется результат агента:

| Значение | Куда сохраняется | Внешний вид в чате |
|----------|----------|-------------------|
| `message` | Сразу как обычное сообщение (`type: "message"`, `author: ID_агента`) | Полноценное сообщение в чате, как от ассистента |
| `thought` / не указан | Как внутренний отчёт (`type: "thought"`, виден только в свёрнутом блоке отчётов) | Свёрнутый блок "Мысли агентов" (🧠) |

```yaml
  # Агент пишет сразу в чат — никаких дублей
  - id: call_soma_aux
    type: llm_worker
    agent: soma_translator
    output_type: message

  # Агент работает внутри — отчёт только в сведённый блок
  - id: analyze_data
    type: llm_worker
    agent: data_analyzer
```

**Важно:** если узел не последний в pipeline (дальше есть другие узлы), результат сохраняется как `thought`, а в чат пишет только последний узел. `output_type: message` определяет поведение последнего узла перед END.

### `inject_reports` (отчёты коллег в system prompt)

Для `llm_worker` можно указать `inject_reports: [agent_id, ...]` — последние отчёты (thought) перечисленных агентов подставляются в системный промпт в блок `### [ОТЧЕТЫ КОЛЛЕГ ДЛЯ АНАЛИЗА]` — альтернатива инструменту `batch_get_agent_report`.

См. `main_conversation_flow.yaml` — агент `call_validator` инжектит отчёт `decomposer`, но при этом история сессии инжектится автоматически.

### 3 режима `switch`

1. **По строковому значению (`input`):** `input` рендерится, из него берётся строка (или поле `status` JSON), ищется в `cases_priority` по ключу; при отсутствии — `default`.
2. **Приоритетный (`input_object` + `cases_priority`):** первый факт со значением `true` в перечисленном порядке = маршрут; если ни один — `default`.
3. **Строгий (`input_object` + `switch_field` + `cases_priority`):** берётся значение поля `switch_field`, маппится на точное совпадение ключа.

```yaml
  # Режим 2 (маршрутизация по первому истинному факту)
  - id: route
    type: switch
    input_object: "{{ nodes.extract_facts.output }}"
    cases_priority:
      - key: has_resistance
        to: call_curator
      - key: has_somatic
        to: call_soma
    default: freestyle
```

> Примечание: поле `cases` (мапа ключ → маршрут) отсутствует. Вместо него — `cases_priority` (массив `{key, to}` или мапа `{key: to}`) + `default`.

### Действия `system_condition`

| action | Что делает | Поля |
|--------|-----------|------|
| `get_missing_reports` | Возвращает `{status: all_done/missing, missing: [...]}` — каких агентов нет в сессии | `required` |
| `has_reports` | `{status: present/missing}` — есть ли все отчёты | `required` |
| `all_problems_analyzed` | Проверка отчёта `pattern_finder_by_double_bind` | — |
| `aggregate_reports` | Склейка отчётов в строку (в вывод узла) | `required` |
| `check_protocol_state` | `{status: ready/need_more_data, missing_points}` | `required` |
| `aggregate_and_output` | Склейка отчётов и **вывод в чат как message** (минуя финальный LLM-синтез) | `required` |

```yaml
  # Вывести сырые ответы воркеров в чат без LLM-синтеза
  - id: output_raw
    type: system_condition
    action: aggregate_and_output
    required: ["neuro_reprogrammer"]
```

### Как работает `visible` (выбор entry point)

- **YAML workflow** → `visible: true` в корне графа; **`.md` файл** → `visible: true` в frontmatter — entry point отображается в UI.
- При отправке сообщения entry point диспетчеризация:
  1. Есть ли workflow с `file_stem == ID` **и** `visible: true` → запускается `workflow_engine::run_workflow()` (режим графа).
  2. Иначе → legacy-режим: `.md` агент с этим ID запускается напрямую через `orchestrator::run_agent_node()`.
- По умолчанию (`visible: false` или не указано) entry point скрыт из UI.

---

## 🧠 Хранение данных (сообщения)

Система избегает бесконечного раздувания контекста:

1. Запрос пользователя — сообщение `type: "message"`, `author: "user"` в `messages[]`.
2. Результат воркера — сообщение `type: "thought"` с `namespace` и `author` (ID агента).
3. История (все non-thought сообщения) подставляется в `llm_messages` автоматически — агенту не нужно самому перечитывать чат.
4. Для точечного чтения отчётов агент использует `batch_get_agent_report(queries)` (см. выше).

## 🧩 Подключение модулей (Includes)
`<<INCLUDE: database/my_knowledge.md>>` — подключает содержимое файла (путь относительно папки агента). Найденный файл оборачивается в `<file path="...">` блок.

---

## Пример: Команда психотерапии (реальная структура)

### Структура папок
```
agents/psychotherapist/
├── frontend/                       # Коммуникаторы и фронтовые агенты (общение с юзером)
│   ├── provocateur.md
│   ├── decomposer.md
│   ├── grounder.md
│   ├── request_helper.md
│   ├── shadow_worker.md
│   └── compliance_floors_checker.md
├── backend/                        # Внутренние воркеры (анализ, снабжение)
│   ├── soma_translator.md
│   ├── validator.md
│   ├── synthesizer.md
│   ├── neuro_reprogrammer.md
│   ├── pattern_finder_by_floors.md
│   └── cluster_checker.md
├── transitions/                    # Workflow-графы маршрутизации
│   ├── main_conversation_flow.yaml # Entry-граф (visible: true)
│   └── facts.yaml                  # Факты + критерии + extractor_prompt
├── database/                       # База знаний (подключается через <<INCLUDE>>)
├── grammars/                       # Per-agent GBNF-грамматики (<agent_id>.gbnf)
├── signals/                        # JSON-схемы сигналов
└── archive/                        # Устаревшие/отключенные файлы
```

### Пример entry-графа (`main_conversation_flow.yaml`)
```yaml
name: Психотерапевт
visible: true
config:
  facts_file: facts.yaml        # Факты + критерии + extractor_prompt
  default_llm_params: creative # Скрытый пресет для всех узлов графа

nodes:
  - id: extract_facts
    type: llm_fact_extractor
    input: |
      User: {{ user_message }}
      Session history: {{ messages }}
    llm_params: strict

  - id: check_has_problem
    type: condition_check
    field: has_problem
    true_to: note_has_problem
    false_to: note_explain_rules

  - id: freestyle                # Свободный ответ без системного промпта
    type: llm_freeform
    input: "{{ user_message }}"

  - id: respond
    type: llm_worker
    agent: therapist_communicator
    output_type: message

  - id: call_provocateur
    type: llm_worker
    agent: provocateur
    output_type: message        # провокатор спрашивает вопрос в чат юзеру
    inject_reports:
      - validator

  - id: note_has_problem
    type: note
    input: пересчитываем данные с учетом новых данных от юзера

  - id: note_explain_rules
    type: note
    system_message: |
      Психотерапевт может решать любые жизненные проблемы...
    input: объясняем юзеру правила составления запроса

edges:
  - from: extract_facts
    to: check_has_problem

  - from: check_has_problem
    condition: true
    to: respond
  # ...полный граф смотри в agents/psychotherapist/transitions/main_conversation_flow.yaml
```

---

## 🧠 Generic `llm_fact_extractor` (как работают факты)

1. **Факты** (`config.facts` или `config.facts_file`) — список с `id` и `criteria`.
2. **Кастомный промпт** (`config.extractor_prompt`) — если указан (в конфиге или facts.yaml), используется вместо дефолтного; поддерживает шаблоны `{{ facts }}` и `{{ user_message }}`.

Файл фактов обычно лежит рядом с workflow: `agents/psychotherapist/transitions/facts.yaml` (путь задаётся относительно папки workflow).

### Пример файла фактов (`facts.yaml`)

```yaml
extractor_prompt: |
  Ты — системный анализатор. Определи присутствие фактов.
  Ответь ТОЛЬКО JSON: {"fact_id": true, "fact_id2": false}

facts:
  - id: is_greeting
    description: "Простое приветствие"
    criteria: Сообщение не содержит описания проблемы

  - id: has_somatic
    description: "Есть соматические симптомы"
    criteria: Описывает физическую боль, зажимы, болезни
```

### Как добавить `llm_freeform` для off-topic

Если ни один факт не совпал (default):
1. В `switch` укажи `default: имя_ноды`
2. Создай ноду с `type: llm_freeform`
3. Добавь ребро от неё к `END`

```yaml
  - id: route
    type: switch
    default: freestyle
    ...

  - id: freestyle
    type: llm_freeform
    input: "{{ user_message }}"
```

---

## Правила для генерации ИИ-помощником:
1. Изучи потребность пользователя
2. Определи, нужен ли один агент-коммуникатор или несколько
3. Создай `.md` файл коммуникатора — только стиль общения, без маршрутизации
4. Создай `.yaml` workflow граф в папке `transitions/` — вся маршрутизация здесь
5. Воркеры создавай как `.md` с узкой задачей (папки `frontend/`/`backend/` по роли)
6. Используй неймспейсы для изоляции контекстов разных проблем
7. Факты выноси в `facts.yaml` рядом с workflow; сложные маршруты разбивай на `note`/`switch`/`condition_check`/`signal_router`
8. Проверь, что file_stem'ы уникальны (движок ищет рекурсивно по `agents/`)