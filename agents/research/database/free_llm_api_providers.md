# Бесплатные LLM API: проверенные провайдеры

> **ЧТО ЭТОТ ФАЙЛ.** Провайдеры как таковые: кто работает, какие у него лимиты, нужен ли
> телефон или карта, к чему привязана квота. Здесь же профиль ограничений пользователя,
> уровни доказательности, правила ответа и чёрный список провайдеров.
>
> **ЧЕГО ЗДЕСЬ НЕТ.** Каталога репозиториев-источников, которым можно доверять, и правил
> проверки их живости. Это отдельный файл — `free_llm_api_sources.md`.
>
> **НЕ ПУТАТЬ:** «провайдер» = API-сервис, куда идут запросы (Groq, Mistral).
> «Источник» = GitHub-репозиторий со списком провайдеров (`mnfst/awesome-free-llm-apis`).

Верификация выполнена **2026-10-13**. Каждый факт помечен уровнем доказательства.
Даты проверки относятся к 2026-10-13, лимиты меняются — перепроверяй перед выдачей.

## УРОВНИ ДОКАЗАТЕЛЬНОСТИ (обязателен в ответах)

- **[ЛИЧНО]** — подтверждено самим пользователем личным опытом. Приоритет высший, не оспаривай.
- **[ДОКИ]** — есть прямая цитата из официальной документации с URL.
- **[ПРОВЕРЕНО]** — проверено живым запросом (код ответа, заголовки rate-limit).
- **[НЕ ПРОВЕРЕНО]** — данных нет. Так и пиши в ответе, выдумывать запрещено.

---

## ПРОФИЛЬ ПОЛЬЗОВАТЕЛЯ (жёсткий фильтр отбора)

Это не «хотелки», а жёсткие ограничения окружения. Провайдер, не проходящий фильтр, НЕ предлагается.

1. **Один IP-адрес** на все запросы. Трафик идёт через Vercel-релей в США.
   Vercel-релей нужен ТОЛЬКО для обхода региональных блокировок. Он НЕ умножает число IP.
   Ротации IP нет. Значит всё, что лимитирует ПО IP клиента, бесполезно при любом числе аккаунтов.
2. **Телефон только +7 (Россия).** Иностранные и виртуальные номера получить нельзя.
3. **Много аккаунтов**: Google, GitHub, почты. Это РЕСУРС, его надо тратить грамотно.
4. **Никаких банковских карт и никакого внесения денег.** Ни «триал $5», ни «положи $10».
5. **VPN** есть, но годится только чтобы зайти на сайт и получить ключ. Не для запросов.

## ГЛАВНЫЙ КРИТЕРИЙ ЩЕДРОСТИ

Прирост идёт от умножения **RPD × число аккаунтов**. Значит важна **суточная квота RPD на один ключ**,
а не RPM и не «универсальность».

**Второй по важности вопрос: к чему привязан лимит** — к аккаунту/организации/проекту (хорошо, ротируем
аккаунтами) или к IP клиента (бесполезно, ротация невозможна).

---

## РАБОЧИЕ ПРОВАЙДЕРЫ (проходят фильтр)

### 1. Mistral AI — [ЛИЧНО] + [ДОКИ]

- **Телефон НЕ требуется.** Пользователь лично получил ключ без номера.
  Подтверждено документацией: https://docs.mistral.ai/getting-started/quickstarts/studio/activate-and-generate-api-key —
  «Free mode: API access is enabled by default with **no credit card required**. Usage and rate limits apply.»
  В индексе help-центра (проверено 2026-09-29, ~50 статей) статей про phone/SMS-верификацию НОЛЬ.
- **Карта: не требуется.**
- **Единица лимита — ОРГАНИЗАЦИЯ**, не ключ и не IP. Ключи внутри одной организации квоту не суммируют.
  https://help.mistral.ai/en/articles/698531 (дата статьи 12.08.2026): «They are set at the organization
  level and apply across all your workspaces». Три лимита: RPS, токенов/мин, токенов/мес.
- **[НЕ ПРОВЕРЕНО] числовые лимиты Free-плана** — Mistral их не публикует, отсылает к
  `admin.mistral.ai/plateforme/limits` за логином. Не выдумывай цифры.
- **Модель бесплатно:** Mistral Medium 3.5 (128B) и другие в free-режиме.
- Ротация: несколько API-ключей в одном аккаунте НЕ даёт больше суммы. Реальный прирост — несколько
  организаций/аккаунтов.

### 2. Groq — [ДОКИ]

Лучшее найденное сочетание «много RPD + без карты + не по IP».

- **1 000 RPD** на организацию на большинстве моделей; 250 RPD у compound/compound-mini.
  Источник: https://console.groq.com/docs/rate-limits (снимок 2026-10-10), Free Plan, модели
  `openai/gpt-oss-120b`, `openai/gpt-oss-20b`, `qwen/qwen3.8-27b`: 30 RPM / 1 000 RPD / 8K TPM / 200K TPD.
- **Единица лимита — организация, НЕ пользователь и НЕ IP.** Дословно: «Rate limits apply at the
  organization level, not individual users.» Cached tokens не считаются.
- Скорость: LPU, 300-600 tok/s — самый быстрый из бесплатных.
- **[ЛИЧНО]/[НЕ ПРОВЕРЕНО]:** требование карты для Groq Free документами НЕ подтверждено — доки молчат.
  Не утверждай, что карта не нужна, со ссылкой на доки. Если это критично — проверяй в консоли.
- Ротация: N аккаунтов × 1 000 RPD. Линейный прирост.

### 3. Google Gemini (AI Studio) — [ДОКИ] + [ЛИЧНО]

Единственный провайдер, где мультиаккаунтинг работает **принципиально**, и где лимит не по IP.

- **Лимит привязан к ПРОЕКТУ, не к ключу и не к IP.** [ДОКИ]
  https://ai.google.dev/gemini-api/docs/rate-limits: «Rate limits are applied **per project, not per API key**.
  Requests per day (RPD) quotas reset at **midnight Pacific time**».
  https://docs.cloud.google.com/docs/quotas/overview: «Within a Google Cloud project, quotas are shared
  across all applications and IP addresses» — то есть IP вообще не участвует в разделении квоты.
- **Несколько ключей в одном проекте квоту НЕ суммируют.** [ДОКИ]
  https://docs.cloud.google.com/docs/billing: «The cumulative usage from all keys within a project counts
  toward that project's spend cap». Вывод: лить через разные ключи одного проекта — бессмысленно.
- **Реальная RPD — [ЛИЧНО] ~20 запросов в сутки на модель.** Пользователь замерил это на своём аккаунте.
- **ОТКУДА ВЗЯЛОСЬ «1 500 RPD» — [ДОКИ] ЭТО УСТАРЕВШАЯ ЦИФРА.**
  Снимок https://ai.google.dev/gemini-api/docs/rate-limits от 2025-05-30, секция Free Tier:
  Gemini 2.0 Flash — 15 RPM / 1 000 000 TPM / **1 500 RPD**; Flash-Lite — 30 RPM / 1 500 RPD.
  Это квота моделей 2025 года. Google снял эти таблицы из документации (текущая страница публикует
  только spend-лимиты и Batch-таблицы) и сослалась на AI Studio.
  **Никогда не выдавай 1 500 RPD за текущую квоту Gemini.**
- **Мультиаккаунтинг:** каждый новый Google-проект = своя квота. Несколько проектов на аккаунт + несколько
  аккаунтов = линейный прирост.
- **[НЕ ПРОВЕРЕНО]** числовой лимит проектов на один Google-аккаунт — механизм подтверждён, дефолт не вычитан.
- **Карта: не требуется. Телефон: не требуется.**
- ⚠️ **Free-тайр недоступен в EU/UK/Швейцарии** — там действуют платные data-terms. Vercel-релей в США
  это обходит.
- ⚠️ Промпты с free-таира могут использоваться Google для улучшения продуктов.

### 4. Cloudflare Workers AI — [ДОКИ]

- **300 RPM на аккаунт на модель. RPD-лимита НЕТ ВООБЩЕ** — только минутные.
  [ДОКИ] https://developers.cloudflare.com/workers-ai/platform/limits/ (обновл. 2026-09-17):
  Text Generation — 300 RPM «per account, per model».
- **Привязка к аккаунту, не к IP** — ротация аккаунтов работает.
- **Карта для бесплатного Workers-тарифа в тексте документации не требуется.**
- 10 000 нейронов/сутки — отдельная квота, общая на всё Workers AI, сброс в 00:00 UTC.
- ⚠️ **5 моделей исключены из бесплатных** и требуют prepaid-кредиты Workers:
  `@cf/moonshotai/kimi-k2.6`, `@cf/moonshotai/kimi-k2.7-code`, `@cf/zai-org/glm-5.2`,
  `@cf/deepseek-ai/deepseek-v4-flash-0731`, `@cf/deepseek-ai/deepseek-v4-pro-0813`.
  Это ровно тот «долив», который запрещён — эти модели не предлагай.

### 5. Pollinations AI — [ПРОВЕРЕНО]

- **Работает вообще БЕЗ ключа и без регистрации.** [ПРОВЕРЕНО] живой запрос
  `POST https://gen.pollinations.ai/v1/chat/completions` (model `openai-fast`, без заголовка авторизации)
  → **200 OK**, ответ `x-model-used: gpt-5-nano-2025-08-07`.
- **Лимиты [ДОКИ]** https://github.com/pollinations/pollinations/blob/master/APIDOCS.md:
  Anonymous — **1 запрос в 15 секунд**, базовые модели, без регистрации.
  Seed (бесплатная регистрация) — **1 запрос в 5 секунд**, стандартные модели.
- **[НЕ ПРОВЕРЕНО]** к чему именно привязан анонимный лимит. Ключа нет — идентификатор только IP,
  значит почти наверняка по IP. Поэтому **не рассчитывай на ротацию аккаунтов**, но и не отбрасывай:
  ~4 800 запросов в сутки даже с одного IP.
- Единственный из списка, где текст + изображения + видео + аудио под одним ключом. Есть MCP-сервер.

---

## ЧЁРНЫЙ СПИСОК (НЕ ПРЕДЛАГАТЬ)

### NVIDIA NIM — ⛔ [ЛИЧНО] + [ПРОВЕРЕНО] + [ДОКИ]

- **+7 (Россия) не проходит верификацию.** [ЛИЧНО] пользователь: при регистрации через VPN потребовали
  телефон США и другой страны. [ДОКИ] подтверждено на форуме NVIDIA, десятки тредов:
  https://forums.developer.nvidia.com/t/cannot-verify-russian-phone-number-for-nim-api-key/375644,
  https://forums.developer.nvidia.com/t/manual-account-verification-request-russia-7-not-available-in-sms-list/370531,
  https://forums.developer.nvidia.com/t/account-access-verification-update/360900/19 —
  ответ от 29.04.2026: «I cannot complete the phone verification step because my country of residence
  (Russia) is not listed in the SMS verification options.»
- **Блокировка сайта — гео/легальная, не бот и не rate-limit.** [ПРОВЕРЕНО]
  `integrate.api.nvidia.com/v1/models` → **HTTP 451** (Unavailable For Legal Reasons) с `Server: awselb/2.0`
  — это AWS ELB, а не Akamai. [ЛИЧНО] пользователь видел Akamai-403 на build.nvidia.com
  (Reference #18.85257e68..., errors.edgesuite.net). Оба отказа — гео/легальные.
- **Обхода нет:** аккаунт всё равно не создать без верификации телефона. Vercel-релей и VPN не помогают.

### Cerebras — ⛔ [ДОКИ]

Ошибка данных в источниках. Два свежих репо, включая ежедневно обновляемый
`open-free-llm-api/awesome-freellm-apis`, числят его в «Permanent Free Tiers» с колонкой «Credit Card: No».
Это НЕВЕРНО.

- [ДОКИ] https://inference-docs.cerebras.ai/support/rate-limits (снимок 2026-10-06):
  «New accounts receive $5 in free credits **after adding a verified payment method**. These credits expire
  30 days after they're granted... **If you skip adding a payment method at sign-up, Playground and API
  access remain inactive until you do.**»
  На вопрос «Is there a permanently free tier?» — дословно: «**No.** Cerebras doesn't currently offer a
  no-cost tier that renews automatically.»
- **Карта обязательна + кредиты горят за 30 дней.** Отпадает по двум пунктам сразу.

### Kilo Code gateway — ⛔ [ЛИЧНО] + [ДОКИ]

- [ДОКИ] https://kilo.ai/docs/gateway/authentication (снято 2026-10-13) дословно:
  «The gateway allows unauthenticated access for free models only. **Anonymous requests are identified by
  IP address** and are subject to rate limiting (**200 requests per hour per IP**).»
- **Лимит строго по IP** → при одном IP ротация бессмысленна. [ЛИЧНО] пользователь подтвердил.
- [ПРОВЕРЕНО] с IP субагента `/api/gateway/v1/models` → 403, чат с `:free` моделями → 403.

### OpenCode Zen — ⛔ [ЛИЧНО] + [ДОКИ]

- Free-модели существуют, но помечены «**during its free period**» — то есть временные, не постоянные.
  https://opencode.ai/docs/zen, раздел Privacy: «Big Pickle: **During its free period**, collected data may
  be used to improve the model» — то же для MiMo-V2.6-Flash Free, MiMo-V2.5 Free, Ling 3.0 Flash Fin Free.
- Требует биллинг-данных: https://opencode.ai/docs/zen — «You sign in to OpenCode Zen, **add your billing
  details**, and copy your API key».
- [ПРОВЕРЕНО] `/zen/v1/models` 200, но чат с free-моделью → **403 FreeTierError**.

### OVHcloud AI Endpoints — ⛔ [ЛИЧНО] + [ПРОВЕРЕНО]

- [ЛИЧНО] регистрация на eu.ovhcloud.com требует нероссийский номер для верификации личности.
  [ПРОВЕРЕНО] анонимный каталог `endpoints.ai.cloud.ovh.net` прямо отдаёт:
  `"rate_limit_api":[{"rate_limit_unit":"req/m","rate_limit_value":400}],
  "rate_limit_api_anonymous":[{"rate_limit_unit":"req/m","rate_limit_value":2}]`.
  Живой запрос `POST .../v1/chat/completions` без ключа → 200 OK с заголовками
  `ratelimit-limit: 2`, `ratelimit-remaining: 1`, `ratelimit-reset: 21`.
- **Аноним = по IP, 2 req/min.** Ключ не идентифицирует — привязка к IP следует конструктивно.
  Отпадает: и телефон, и IP-лимит.

### OpenRouter — ⛔ [ЛИЧНО] + [ДОКИ]

- [ДОКИ] https://openrouter.ai/docs/api_reference/limits (снимок 2026-09-16) дословно:
  «If you have purchased less than 10 credits, you're limited to **50 :free model requests per day**.
  If you purchase at least 10 credits, your daily limit is increased to **1 000** :free model requests per day.»
- [ЛИЧНО] подтверждено: для роста лимита надо закинуть $10. Ровно то, что запрещено.
- ⚠️ Справедливости ради: лимит у OpenRouter **не привязан к IP** — там дословно «Making additional
  accounts or API keys will not affect your rate limits, as we govern capacity globally». Ротация аккаунтов
  не помогает, потому что 50 RPD идут в глобальный пул, а не в ваш аккаунт.
- [ПРОВЕРЕНО] `/api/v1/models` → 403 с IP субагента.

### ModelScope и SiliconFlow — ⛔ [ДОКИ]

- [ДОКИ] https://modelscope.ai/docs/model-service/API-Inference/limits: «Your ModelScope account must first
  bind to an Alibaba Cloud account. Additionally, to prevent abuse, the corresponding cloud account must have
  completed **real-name verification** before you can use API-Inference normally».
- SiliconFlow: [ПРОВЕРЕНО] API жив (401 без ключа), но [НЕ ПРОВЕРЕНО] принимается ли +7.
  Не предлагай, пока не подтверждено.

### GitHub Models — ⛔ [ДОКИ] ЗАКРЫТ ПОЛНОСТЬЮ

- [ДОКИ] https://docs.github.com/en/github-models (RU-версия несёт тот же текст):
  «GitHub Models has been retired. **As of July 30, 2026**, GitHub Models has been fully retired. The
  playground, model catalog, **inference API**, and bring your own key (BYOK) are **no longer available to
  any customer**.» Раздел «Where to go next» предлагает Azure AI Foundry — другого продукта, не переезд.
- [ПРОВЕРЕНО] `models.inference.ai.azure.com` → **NXDOMAIN** (висячий CNAME на
  `modelmesh-nexus-global-main.trafficmanager.net` с таргетом Status 3) — сигнатура снятого endpoint.
  `models.github.ai` отвечает 200 «OK» на ЛЮБОЙ путь и метод — catch-all-заглушка, не API.
  `github.com/marketplace/models` → **404**.

---

## НЕ РЕШЕНО / ПРОВЕРИТЬ ПРИ СЛЕДУЮЩЕЙ СЕССИИ

Не выдавай за факт, указывай честно «проверить не удалось»:

- Числовые лимиты free-тарифа **Mistral** (RPS / токенов в месяц) — не опубликованы, только в консоли.
- Требуется ли карта для **Groq Free** — документация молчит.
- Числовой лимит проектов на Google-аккаунт — механизм подтверждён, дефолт не вычитан.
- К чему привязан лимит **LLM7.io** (док не говорит; free-токен 1/сек, 60/мин, 250/час,
  100 000 токенов за 24 ч — https://docs.llm7.io/limits).
- **SiliconFlow** — принимается ли +7.
- Ни по одному провайдеру в мире официальные доки не описывают IP-лимитирование явно. Выводы о привязке
  делаются конструктивно (наличие/отсутствие идентификатора в запросе), а не из цитаты.
- Модельные списки и лимиты меняются часто — Groq уже убивал llama-3.3-70b-versatile и
  llama-3.1-8b-instant 16.08.2026.

## ПРОВАЙДЕРЫ БЕЗ ПОДТВЕРЖДЁННОГО FREE-ТАРИФА (не предлагать)

Проверены 2026-10-13, публичного free-тира не подтверждено либо API недоступен:
SambaNova (free-тира нет в списке моделей), Aion Labs (нет free в списке), AI21 Labs (403),
Agnes AI (сайт 200, API 404), Ollama Cloud (нет подтверждённого free), Chutes.ai (API 404),
glhf.chat (HTTP 526, origin не отвечает), Kluster AI (сайт недоступен).
Любой из них требует отдельной проверки перед рекомендацией — по умолчанию не предлагать.

---

## ПРАВИЛА ОТВЕТА АГЕНТА

1. Сначала определи, вписывается ли задача в профиль пользователя. Если нужен телефон не из РФ,
   карта или внесение денег — **сразу говори «не подходит» и почему**, не предлагай.
2. **Считай прирост формулой `RPD × число аккаунтов` и называй её явно в ответе.**
   Количество моделей у провайдера — не аргумент. RPM вторичен: 1 000 RPD при 30 RPM выигрывает
   20 RPD при 100 RPM.
3. **Отдельно проверяй привязку лимита и называй её в ответе:**
   - аккаунт / организация / проект → ротация нескольких аккаунтов ЛИНЕЙНО поднимает потолок;
   - IP клиента → при единственном IP пользователя ротация бессмысленна, сколько бы аккаунтов
     ни завести. Такой провайдер бесполезен, даже если лимит выглядит щедрым.
4. Числа без уровня доказательства не выдавай за факт. Помечай [НЕ ПРОВЕРЕНО] явно.
5. Прямо скажи, если мимо лимита придётся залезать в Cloudflare/OpenRouter/Kilo ради ещё одной порции.
6. Не предлагай провайдеров из чёрного списка даже «на всякий случай».
7. Не повторяй числа из списков-агрегаторов как факт, если они противоречат официальным докам.
   Проверено 2026-10-13: агрегаторы отнесли Cerebras к постоянному free-таиру с колонкой
   «Credit Card: No», хотя официальные доки требуют карту и кредиты сгорают за 30 дней.
   Та же колонка «Credit Card?» в других списках — не гарантия.
