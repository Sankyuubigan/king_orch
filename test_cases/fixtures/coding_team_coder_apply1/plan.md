# PRP: Изоляция сессий SessionStore

## 1. КОНТЕКСТ

Проблема: хранилище сессий `SessionStore` (файл `.agents_workspace/test_task/session_store.py`)
хранит всё состояние на уровне КЛАССА (`_sessions`, `_current_user`, `_active_regions`),
а не на уровне экземпляра. Поэтому все экземпляры `SessionStore` (по одному на регион eu/us)
разделяют один глобальный набор сессий: сессии и «текущий пользователь» протекают между
регионами, `eu.current()` возвращает чужой профиль.

Цель: перенести состояние из класса в экземпляр (`self.*`), чтобы каждый регион имел
изолированное хранилище. Все тесты в конце файла должны проходить.

## 2. АРХИТЕКТУРНЫЕ ПРАВИЛА

- SSOT: per-instance данные (`_sessions`, `_current_user`, `_active_regions`) живут в
  `__init__` как поля экземпляра (`self.*`). Запрещены классовые изменяемые атрибуты.
- Запрещены любые обращения вида `SessionStore.<поле>` внутри методов — вместо них `self.<поле>`.
- Логирование: в `start_session` и `end_session` поставить логи ДО и ПОСЛЕ мутации
  (`событие`, `user_id`) — для наблюдаемости.
- Размер файла не превышает 500 строк; новые модули не нужны.
- Запрещены костыли (try/catch для скрытия ошибок) — лечим причину.

## 3. МИКРО-ШАГИ

- Шаг 1 (контракт): добавить в `__init__` поля `self._sessions: dict = {}`,
  `self._current_user: str | None = None`, `self._active_regions: set = set()`.
  Классовые объявления `_sessions` / `_current_user` / `_active_regions` удалить.
- Шаг 2 (реализация): заменить во всех методах обращения `SessionStore._sessions`,
  `SessionStore._current_user`, `SessionStore._active_regions` на `self._sessions`,
  `self._current_user`, `self._active_regions`.
- Шаг 3 (интеграция): оставить блок `if __name__ == "__main__":` и тесты в нём
  НЕИЗМЕННЫМИ. Проверить, что `python .agents_workspace/test_task/session_store.py`
  печатает `PASS`.

## 4. ЦИКЛ ВАЛИДАЦИИ

- `python .agents_workspace/test_task/session_store.py` → stdout содержит `PASS`.
- В файле не должно остаться обращений вида `SessionStore._<поле>` и классовых
  изменяемых атрибутов (`_sessions: dict = {}`, `_current_user: str | None = None`).

## 5. ИТОГОВЫЙ РЕЗУЛЬТАТ (файл целиком, скопировать в `write_file`)

```python
"""Хранилище пользовательских сессий.

Создаётся по одному экземпляру SessionStore на регион (eu, us).
Ожидается, что сессии разных регионов полностью изолированы.
"""


class SessionStore:
    """Хранилище сессий одного региона."""

    def __init__(self, region: str):
        self.region = region
        self._sessions: dict = {}
        self._current_user: str | None = None
        self._active_regions: set = set()

    def start_session(self, user_id: str, profile: dict) -> None:
        self._sessions[user_id] = dict(profile)
        self._current_user = user_id
        self._active_regions.add(self.region)

    def get(self, user_id: str) -> dict:
        return dict(self._sessions.get(user_id, {}))

    def current(self) -> dict:
        return self.get(self._current_user)

    def end_session(self, user_id: str) -> None:
        self._sessions.pop(user_id, None)

    def regions(self) -> set:
        return set(self._active_regions)

    def total(self) -> int:
        return len(self._sessions)


if __name__ == "__main__":
    eu = SessionStore("eu")
    us = SessionStore("us")

    eu.start_session("alice", {"plan": "premium", "region": "eu"})
    us.start_session("bob", {"plan": "free", "region": "us"})

    # Ожидается изоляция сессий между экземплярами:
    assert eu.total() == 1, f"eu видит чужие сессии: {eu.total()}"
    assert us.total() == 1, f"us видит чужие сессии: {us.total()}"
    # Текущая сессия eu должна быть alice из своего региона:
    assert eu.current().get("region") == "eu", f"eu.current() = {eu.current()}"
    assert us.current().get("region") == "us", f"us.current() = {us.current()}"
    # Завершение сессии в us не должно затронуть eu:
    us.end_session("bob")
    assert eu.current().get("region") == "eu", f"eu.current() = {eu.current()}"
    assert "bob" not in eu.current().values(), "профиль bob протёк в eu"

    print("PASS")
```