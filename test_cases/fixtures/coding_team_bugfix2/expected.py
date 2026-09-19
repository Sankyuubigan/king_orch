"""Хранилище пользовательских сессий.

Создаётся по одному экземпляру SessionStore на регион (eu, us).
Состояние изолировано внутри экземпляра (self.*), а не на уровне класса.
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