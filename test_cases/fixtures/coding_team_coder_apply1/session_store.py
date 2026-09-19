"""Хранилище пользовательских сессий.

Создаётся по одному экземпляру SessionStore на регион (eu, us).
Ожидается, что сессии разных регионов полностью изолированы.
"""


class SessionStore:
    """Хранилище сессий одного региона."""

    _sessions: dict = {}
    _current_user: str | None = None
    _active_regions: set = set()

    def __init__(self, region: str):
        self.region = region

    def start_session(self, user_id: str, profile: dict) -> None:
        SessionStore._sessions[user_id] = dict(profile)
        SessionStore._current_user = user_id
        SessionStore._active_regions.add(self.region)

    def get(self, user_id: str) -> dict:
        return dict(SessionStore._sessions.get(user_id, {}))

    def current(self) -> dict:
        return self.get(SessionStore._current_user)

    def end_session(self, user_id: str) -> None:
        SessionStore._sessions.pop(user_id, None)

    def regions(self) -> set:
        return set(SessionStore._active_regions)

    def total(self) -> int:
        return len(SessionStore._sessions)


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