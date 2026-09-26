# Курируемые источники по бесплатным LLM API

Baseline собран 2026-09-26. ЗВЁЗДЫ И ДАТЫ НИЖЕ — ЭТО ТОЧКА ОТСЧЁТА, А НЕ ИСТИНА.
Перед любым ответом перепроверяй живость каждого репозитория инструментом.
Метрика живости — ТОЛЬКО `pushed_at` (дата последнего коммита).
Поле `updated_at` меняется от звёзд и Issues — для оценки живости НЕ годится.

## Tier 1 — эталон, проверять всегда

- https://github.com/mnfst/awesome-free-llm-apis
  ⭐ 8409 | pushed 2026-08-21 | CC0. README генерится из `data.json` через GitHub Actions.
  Был ежедневный refresh; на 2026-09-26 коммиты шли пачками 19-21 августа, дальше тишина.
  Огромный пул провайдеров, есть колонки «модель / контекст / лимит». Эталон по широте.
- https://github.com/open-free-llm-api/awesome-freellm-apis
  ⭐ 3362 | pushed 2026-09-26 | Дейли-синк README с freellm.net, коммит буквально каждый день.
  Эталон по СВЕЖЕСТИ. Если mnfst замер — бери данные отсюда.

## Tier 2 — живые, вторичные

- https://github.com/guihuashaoxiang/FreeLLM-API-KeyHub — ⭐ 248 | pushed 2026-07-19. Hub с ключами, китайский уклон.
- https://github.com/amardeeplakshkar/awesome-free-llm-apis — ⭐ 172 | pushed 2026-08-16. Проверенные $0-модели, лимиты, OpenAI-совместимость. PR висят неделями — не жди апдейта.
- https://github.com/abbosaliboev/free-ai-bible — ⭐ 158 | pushed 2026-09-21. Обновляется регулярно.
- https://github.com/CYBIRD-D/FREE-LLM-API-Provider — ⭐ 109 | pushed 2026-09-23. Живой.
- https://github.com/pacocartones/free-llm-api-hub — ⭐ 48 | pushed 2026-09-07. Датасет по провайдерам.
- https://github.com/mvalentsev/awesome-free-ai-coding — ⭐ 33 | pushed 2026-09-26. Угол — бесплатные ИИ-тулы для кодинга.

## Tier 3 — слабые, брать только если нет альтернатив

- https://github.com/nherx/free-llm-api-resources — ⭐ 30 | pushed 2026-09-26.
  ВНИМАНИЕ: формально жив, по сути полумёртвый — 2 содержательных коммита (февраль→май 2026),
  «свежий» пуш = правка одной строки README. Помечен как «наследник» несуществующего
  cheahjs/free-llm-api-resources. Используй только как запасной вариант и помечай в ответе.
- https://github.com/ma-pony/awesome-free-llm-api — ⭐ 5 | pushed 2026-08-14. Молодой, малозвёздный.

## Дискавери: как находить новые репозитории

- Страница топика (рабочая, 46 репо): https://github.com/topics/free-llm-api
- Поиск через инструмент: `GithubSearch` с `query: "topic:free-llm-api"`, `type: "repositories"`, `sort: "stars"` и отдельно `sort: "updated"`.

## Чёрный список — НЕ ИСПОЛЬЗОВАТЬ (проверено 2026-09-26)

- `cheahjs/free-llm-api-resources` — **HTTP 404, репозиторий не существует.** Ссылки на него в интернете (в т.ч. с якорем #opencode-zen) — галлюцинация. Не пытайся его открыть и не ссылайся.
- `bradAGI/awesome-free-inference` — ⭐ 15, последний push 2026-03-23, 6 месяцев тишины, PR #1 висит с 2026-04-06 без ответа. Мёртв.
- `github.com/topics/free-ai-api` — **HTTP 404**, страница топика не рендерится. Ловушка: search API находит 46 репо, а страница отдаёт 404.
- `github.com/topics/free-ai-apis` — HTTP 200, но **репозиториев ноль** — пустая страница.
- `0xzr/freellmpool` — это Python-код (пул бесплатных инстансов), не список API. Как источник данных бесполезен.
- `Terafitzsing/freepik-ai-lab` — генератор изображений, к LLM API отношения не имеет.
