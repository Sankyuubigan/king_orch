# Контракты сигналов `emit_signal` (SSOT + правило генерации)

> **Критическое правило (нарушение = баг «тихо упал, в чат никто не ответил»):**
> **Сигнальный агент ОБЯЗАН генерировать ответ под грамматикой/JSON-schema своего
> контракта. Никогда не отключай грамматику для сигнальных агентов и не добавляй
> `default`-ветку в маршрутизатор как «защиту» — это костыль, маскирующий корень.**

## Что такое контракт сигнала

Единый источник правды (SSOT) — `agents/<команда>/signals/root.schema.json`:

```json
{ "agents": { "<agent_id>": { "key": "<сигнальный key>", "value": { "<JSON Schema value>" } } } }
```

Агент вызывает `emit_signal` как инструмент **внутри своего ответа**. Конверт:

```json
{ "tool": "emit_signal", "arguments": { "key": "<key>", "value": { ... } } }
```

- Свободные рассуждения агента живут в **think-блоке** гибридной GBNF
  (`verdict_contract`-стиль, Method 3) и попадают в сессию как `thought`-сообщения —
  в конверте поля `thought` НЕТ.
- `arguments.value` — **защищённая структура** по схеме контракта (имена полей, `enum`,
  `const`, `required`, `additionalProperties:false`).

## Разделение размышлений и структуры (один запрос)

Двухфазный режим (Phase 1 свободные размышления → Phase 2 JSON) удалён 17.10.2026
(см. `tasks/17.10.26 Удаление двухфазного режима.md`). Теперь всё происходит в одном
запросе:

1. **Hybrid GBNF** (`build_signal_envelope_grammar`) — грамматика
   `think-block envelope-json`: модель размышляет в `</think>`-блоке (лимит
   `think_budget_chars` из грамматики), потом выдаёт строгий конверт.
2. **Автономный Phase-2 JSON-only retry** (`build_signal_envelope_json_only_grammar`,
   `orchestrator/mod.rs` «Phase 2: JSON-only fallback») — если за ход сигнал не был
   доставлен, оркестратор делает единственный повторный вызов уже БЕЗ think-блока
   (`enable_thinking=false`), под грамматикой только envelope-json.

Поэтому поле `thought` внутри конверта не нужно: рассуждения живут в think-блоке
отдельного канала. Грамматика ограничивает **только форму** сигнала (имена/типы/допустимые
значения полей). Это тот же паттерн, что OpenAI `tool_calls` / llama.cpp `json_schema`.

llama.cpp на лету компилирует JSON Schema (Draft 7) в GBNF (модуль
`common/json-schema-to-grammar.cpp`), маскируя на каждом шаге недопустимые токены.
Результат: модель **физически не может** выдать не то имя поля и не то значение (не из
`enum`). Источник: официальный README llama.cpp (`--json-schema`), DeepWiki «Grammar
and Structured Output», и устоявшаяся практика constrained decoding (гарантирует 100%
валидный вывод, тогда как «просто попроси в промпте JSON» падает 5–15% времени).

## Как это проводится в коде (НЕ ломать)

- `domain/signals.rs`:
  - `load_signal_contract()` — грузит контракт агента из `root.schema.json`.
  - `build_signal_envelope_schema()` — собирает строгую JSON-schema **обёртки**
    `emit_signal` из контракта (с `additionalProperties:false`, `const` ключей,
    `required`). Без `$ref` (сломаны в llama.cpp, #8073).
  - `validate_signal_value()` — честная валидация `value` против обязательных полей
    контракта. Возвращает явную ошибку на retry, если форма искажена.
- `domain/orchestrator/mod.rs` (`run_agent_node`):
  - Для сигнального агента (есть контракт) ставится
    `engine.set_grammar(GrammarSpec { gbnf: Some(build_signal_envelope_grammar(contract)) })`.
    Метод строит hybrid GBNF: `think-block envelope-json` (Method 3 из `docs/gbnf.md`).
    `disable_reasoning` НЕ включается — модель думает в `<think>`.
    Ветка `else { None }` для сигнальных агентов **удалена** — отключать грамматику
    запрещено.
- `domain/orchestrator/dispatch.rs` (`execute_tool_call`, ветка `emit_signal`):
  - если `value` не прошёл `validate_signal_value()` — возвращается явная ошибка модели
    на retry (честно, не тихо).

## Разбор бага (кейс: модель исказила имя поля сигнала)

**Симптом:** в чат никто не ответил, лог обрывается на `signal_router → нет target`,
движок остановлен.

**Корень:** оркестратор ставил сигнальным агентам `set_grammar(None)`, поэтому модель
генерировала **без ограничения** и выдала неправильное имя поля (напр. `status` вместо
`e1`). `signal_router` читает нужное поле → не находит → `target: None` → граф
завершается, ни один `message`-узел не выполняется → в `ctx.messages`
не добавляется ни одного `message` → workflow-режим возвращает пустоту.

**Почему `default`-ветка — костыль (не делать):** это маскирует корень (модель отпустили
без грамматики) и зашывает «безопасное» поведение, которое может быть неверным. Правильно
— **запретить искажение формы на этапе генерации** (грамматика), а если оно всё же
проскочит, **честно вернуть ошибку** (`validate_signal_value`), а не молча маршрутизировать
в обход.

## Чек-лист при добавлении нового сигнального агента

1. Добавить запись в `signals/root.schema.json` (`key` + `value`-схема, обязательные поля
   в `required`, `additionalProperties:false`).
2. Убедиться, что `build_signal_envelope_schema` покрывает схему (без `$ref`).
3. В графе `signal_router` читает ровно те поля, что в контракте (SSOT — не дублировать
   строковые литералы полей в YAML, если можно вывести из контракта).
4. НЕ добавлять `default` к `signal_router` «на всякий случай».
5. Запустить тесты `domain/signals.rs` (`validate_signal_value_*`, `envelope_schema_*`).
