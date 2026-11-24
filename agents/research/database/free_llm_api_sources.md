# Бесплатные LLM API и раздачи: источники и мониторинг

> **ЧТО ЭТОТ ФАЙЛ.** Где искать новые бесплатные LLM API, следить за возвратом купонов ZCode
> и ловить жирные временные акции: официальные каналы, трекеры раздач, Telegram, Reddit,
> GitHub-репозитории (Tier 1–3) и чёрный список фейков.
>
> **СВЯЗАННЫЕ ФАЙЛЫ:**
> - Актуальный ТОП-рейтинг, профиль пользователя и правила отбора: `free_llm_coding_ranking.md`
> - Полные досье всех провайдеров, харнессов и чёрный список худших API: `free_llm_api_providers.md`

**Последнее обновление: 2026-11-24.** Добавлен блок живого мониторинга раздач/купонов
(`aipromonow.com`, `freeaiapi.org`, `llm24.net`, `wool-gathering`, `t.me/AIUngated`, X/Reddit ZCode)
и перепроверены Tier 1 репозитории.
Метрика живости GitHub-репо — **ТОЛЬКО `pushed_at`** (дата последнего коммита; `updated_at`
меняется от звёзд/Issues и не считается).

## ⚠️ ГЛАВНАЯ ЛОВУШКА АГРЕГАТОРОВ

Даже свежие и звёздные списки систематически содержат **критичные ошибки**:
- **Ложное «Credit Card: No»:** `open-free-llm-api/awesome-freellm-apis` числил **Cerebras** в
  «Permanent Free, без карты», хотя доки Cerebras прямо требуют карту и не имеют постоянного free.
- **Ложный «возобновляемый / годный для кодинга»:** в списках висят **`Gonka`** (на деле
  одноразовые 3M на `gate.joingonka.ai` или 1.2M/мес на брокере + лаги) и **`AIHubMix`** (10 вызовов
  без оплаты, а с $1 — всего 1M TPD).
- **Слепая зона GitHub-списков:** Tier 1 репозитории ведут только *постоянные бесплатные API* и
  **почти не ловят временные раздачи и ad-supported харнессы** (ZCode 300M, AMD Token Lab, Capy,
  free-pi, packet.ai).

**Правило:** агрегаторы дают только *наводку*. Карта, `+7`, сброс квоты и реальный TPD всегда
сверяются с первоисточником и живым тестом.

---

## 1. МОНИТОРИНГ ЖИВЫХ РАЗДАЧ И КУПОНОВ (первый эшелон!)

### Официальный мониторинг ZCode / Z.ai (возврат 300M / Weekend Build)
- **X (Twitter) ZCode и Z.ai:** `https://x.com/zcode_ai` и `https://x.com/Zai_org` — первоисточник
  всех анонсов *Weekend Build* и *Global Build*.
- **Reddit `r/ZaiGLM` (сортировка New):** `https://www.reddit.com/r/ZaiGLM/` — главный живой пульс
  (исчезновение/возврат купонов, падения серверов всплывают в течение часа).
- **GitHub баг-трекер ZCode:** `https://github.com/zai-org/feedback/issues` — диагностика сбоев
  выдачи (`getManualClaimPlanPreviews`), капчи Aliyun и антифрода `405 / 3012`.
- **Официальный Discord Z.ai:** ссылка в шапке `https://z.ai` / `https://zcode.z.ai` + релизноуты
  ZCode там же.

### Трекеры акций, промо-кредитов и новых бесплатных квот
- **`https://aipromonow.com`** — ⭐ **лучший трекер именно временных раздач и халяв.** Есть
  **RSS-лента** и голосование сообщества *Still works / Expired now* (именно здесь быстрее всего
  видно статус ZCode 300M/500M, AMD Token Lab, Capy, packet.ai, Privatemode).
- **`https://freeaiapi.org`** — живой трекер бесплатных LLM API, кодинг-агентов и релеев
  (отслеживает `100milionseek`, OpenCode, AMD Token Lab и др.).
- **`https://llm24.net`** — агрегатор бесплатных и промо AI-шлюзов/моделей.
- **`https://github.com/jiuyuan233/wool-gathering`** — китайский авто-агрегатор всех акций и
  купонов Zhipu/DeepSeek/Kimi/Alibaba/Tencent (ежедневные обновления, проверено 2026-11-23).

### Telegram и комьюнити
- **`https://t.me/AIUngated` (AI Ungated)** — живой русскоязычный Telegram-канал с практическими
  тестами бесплатных LLM API, AI-роутеров и реальных лимитов (разборы также выходят на Хабре).
- **Reddit-ветки по бесплатному кодингу:** `https://www.reddit.com/r/ZaiGLM/`,
  `https://www.reddit.com/r/opencode/`, `https://www.reddit.com/r/LocalLLaMA/`.

---

## 2. GITHUB-РЕПОЗИТОРИИ: Tier 1 (эталон по постоянным API)

- **`https://github.com/mnfst/awesome-free-llm-apis`**
  ⭐ ~11 900+ | активность PR/коммитов подтверждена в ноябре 2026 (~204 коммита) | CC0.
  Структурированный **`data.json`** (`raw.githubusercontent.com/mnfst/awesome-free-llm-apis/main/data.json`)
  и подробные footnotes со ссылками на первоисточники.
- **`https://github.com/open-free-llm-api/awesome-freellm-apis`**
  ⭐ ~5 000+ | свежая индексация **2026-11-21** (~254 коммита).
  Организация `open-free-llm-api` (**НЕ путать** с фейк-клоном `freellms/*`). Ежедневный синк с
  `freellm.net`, быстро вычищает мёртвых провайдеров, но колонку «Credit Card?» всегда перепроверять.
- **`https://github.com/nejib1/Free-LLM`**
  Свежий живой список бесплатных LLM API (обновлён **2026-11-17**), полезен как дополнительная сверка.

## 3. Tier 2 — вторичные (baseline 2026-11-01, перепроверять `pushed_at`)

- `https://github.com/guihuashaoxiang/FreeLLM-API-KeyHub` — ⭐ 248 | pushed 2026-07-19. Китайский уклон.
- `https://github.com/amardeeplakshkar/awesome-free-llm-apis` — ⭐ 172 | pushed 2026-08-16.
  ⚠️ Завышает/искажает квоты (пишет Groq 14 400 RPD, OpenRouter 200 RPD) — цифры не брать без сверки.
- `https://github.com/abbosaliboev/free-ai-bible` — ⭐ 158 | pushed 2026-09-21. Много устаревших
  моделей и триальных «$5 credit».
- `https://github.com/CYBIRD-D/FREE-LLM-API-Provider` — ⭐ 109 | pushed 2026-09-23.
- `https://github.com/pacocartones/free-llm-api-hub` — ⭐ 48 | pushed 2026-09-07.
- `https://github.com/mvalentsev/awesome-free-ai-coding` — ⭐ 33 | pushed 2026-09-26. Бесплатные ИИ-тулы для кодинга.

## 4. Tier 3 — слабые (только если нет альтернатив)

- `https://github.com/nherx/free-llm-api-resources` — ⭐ 30 | pushed 2026-09-26 (по сути полумёртвый,
  косметические коммиты; позиционируется как наследник удалённого `cheahjs`).
- `https://github.com/ma-pony/awesome-free-llm-api` — ⭐ 5 | pushed 2026-08-14.

## 5. Дискавери: как находить новые репозитории

- Страница топика: `https://github.com/topics/free-llm-api`
- Поиск через GitHub API/поиск: `topic:free-llm-api` (отдельно `sort:stars` и `sort:updated`).

## 6. ЧЁРНЫЙ СПИСОК ИСТОЧНИКОВ — НЕ ИСПОЛЬЗОВАТЬ

- `cheahjs/free-llm-api-resources` — **HTTP 404, удалён.** Любые ссылки на него — галлюцинация.
- **`freellms/*` — SEO-спам (`freellm` клон).** Владелец-юзер за 25 минут 09–10.08.2026 создал
  пустые репо под рекламу `freellms.org`:
  - `freellms/free-llm-api-keys` — 1 коммит, пустой рекламный README.
  - `freellms/awesome-freellm-apis` — **фейковый клон** настоящей `open-free-llm-api/awesome-freellm-apis`,
    протухшие данные (мертвый GitHub Models висит спустя месяцы).
- `bradAGI/awesome-free-inference` — мёртв (последний push 2026-03-23).
- `github.com/topics/free-ai-api` (HTTP 404) и `github.com/topics/free-ai-apis` (0 репо).
- `0xzr/freellmpool` — Python-код, не каталог провайдеров.
- `Terafitzsing/freepik-ai-lab` — генератор картинок, не LLM API.