Профессиональный подход к оценке кодовых LLM строится на приоритете **объективных метрик (исполняемые тесты, pass@1, diff-exact-match) и только затем — LLM-as-judge для аспектов, которые тестами не покрываются** (читаемость, стиль, адекватность изменений). Судья-LLM не обязателен и не должен быть единственным арбитром: как показывают исследования, LLM-судьи подвержены позиционному смещению (предпочитают ответ, идущий первым в промпте), смещению по длине (болтливость = «лучше») и синофантии, поэтому результат, не подтверждённый выполнением кода, недостоверен【turn1search16】【turn1search19】. Для задач рефакторинга и баг-фиксов золотой стандарт — это SWE-bench-подобная связка: эталонный патч + FAIL_TO_PASS (тесты, которые должны начать проходить) + PASS_TO_PASS (тесты, которые не должны сломаться)【turn0search16】【turn0search2】.

Ниже — конкретная методика, заточенная под ваш сценарий: ~8B GGUF, 16 ГБ VRAM, всё в GPU.

## 1. Профессиональный стек измерения (что использовать и почему)

| Метрика | Как измеряется | Источник / инструмент |
|---|---|---|
| **pass@1 (функциональная корректность)** | Процент задач, где сгенерированный патч/код проходит FAIL_TO_PASS и не ломает PASS_TO_PASS | SWE-bench harness, EvalPlus (HumanEval+/MBPP+ с усиленными тестами)【turn0search2】【turn0search16】【turn0search10】【turn0search11】 |
| **Exact / unified-diff match** | Строковое сравнение с gold patch или `git diff` | Aider refactor-benchmark, SWE-Refactor【turn0search11】【turn0search12】 |
| **Edit-format success** | Доля ответов, распарсенных в валидный edit (diff/search-replace) — ловит «lazy coding» | Aider leaderboard【turn0search13】【turn0search11】 |
| **Prompt eval (PP, tok/s)** | Пропускная способность префилла | `llama-bench` (тест `pp512`), поле `timings.prompt_per_second` в `llama-server`【turn1search6】【turn1search8】【turn1search3】 |
| **Token gen (TG, tok/s)** | Авторегрессионная генерация | `llama-bench` (`tg128`), поле `timings.predicted_per_second`【turn1search8】【turn1search3】 |
| **TTFT (time to first token)** | Время до первого токена | `stream=true` + таймстамп первого chunk из `llama-server` |
| **VRAM footprint** | Пик `nvidia-smi` во время прогона | мониторинг через `pynvml` |
| **KV-cache capacity (без квантизации)** | Максимальный `--ctx-size` до OOM при `-ngl 99 -ctk f16 -ctv f16` | бинарный поиск по `--ctx-size` |
| **LLM-judge score (доп.)** | Парная или рубричная оценка по схеме | GPT-4/Claude с swap-positions калибровкой【turn1search16】 |

## 2. Архитектура тестового конвейера

Профессиональная связка строится как пайплайн с разделением ролей, чтобы судья никогда не запускал тесты сам (это роль детерминированного evaluator'а):

```
                 ┌────────────────────┐
 tasks.jsonl ──▶ │  launcher.py       │  читает задачи, итерирует модели
                 └────────┬───────────┘
                          ▼
                 ┌────────────────────┐  llama-server --n-gpu-layers -1
                 │  runner.py         │  OpenAI-совместимый /v1/chat/completions
                 │  (stream + timings)│  собирает PP/TG/TTFT/usage
                 └────────┬───────────┘
                          ▼
                 ┌────────────────────┐
                 │  extractor.py      │  достаёт код/diff из ответа
                 └────────┬───────────┘
                          ▼
                 ┌────────────────────┐  изолированный venv/docker
                 │  evaluator.py      │  применяет патч, гоняет pytest
                 │  (objective)       │  FAIL_TO_PASS / PASS_TO_PASS
                 └────────┬───────────┘
                          ▼
                 ┌────────────────────┐  только для рефакторинга/ревью
                 │  judge.py          │  LLM-as-judge по рубрике
                 │  (subjective, opt.)│  + swap-positions для калибровки
                 └────────┬───────────┘
                          ▼
                 ┌────────────────────┐
                 │  reporter.py       │  CSV/JSON/HTML сводка
                 └────────────────────┘
```

Разделение критично: **судья оценивает, но не исполняет**. Исполняет `evaluator.py` через `pytest` в изолированном окружении — это единственный способ получить детерминированный pass/fail【turn0search6】【turn0search2】.

## 3. Запуск llama.cpp: как уложить 8B в 16 ГБ VRAM

Команда запуска сервера для полностью GPU-resident инференса:

```bash
llama-server \
  -m model-Q8_0.gguf \
  -ngl 99 \                  # все слои в GPU (32 у Llama-3-8B)
  -c 16384 \                # ctx-size — главный потребитель VRAM после весов
  -ctk f16 -ctv f16 \        # KV-кэш без квантизации (требование ТЗ)
  --host 0.0.0.0 --port 8080 \
  -j 4 \                     # параллельные слоты для батчей
  --cont-batching \
  --cache-reuse 256
```

Ключевые параметры для вашего железа【turn0search1】【turn0search3】【turn0search4】【turn1search7】:

- `-ngl 99` (или `-1`) — оффлоад всех 32 слоёв; иначе часть уходит в CPU и TG падает в разы.
- `-ctk f16 -ctv f16` — KV-кэш в fp16 без квантизации; именно это вы и хотите «измерить в объёме без квантизации». Альтернатива `q8_0`/`q4_0` для KV даёт экономию, но вы её исключаете по условию.
- `-c 16384` — размер контекста. Для Llama-3-8B (32 слоя, GQA 8 KV-heads, head_dim 128) fp16 KV-кэш — это примерно `2 × 32 × 8 × 128 × 2 байта × ctx ≈ 128 КБ × ctx`. Вес Q8_0 ~8 ГБ, веса + KV-кэш 16K ≈ ~10–11 ГБ — в 16 ГБ помещается с запасом; 32K уже на грани. Точную границу ищут бинарным поиском по `--ctx-size` до OOM.

**Бенчмарк чистой скорости** через `llama-bench` (без серверных накладных):

```bash
llama-bench -m model.gguf -ngl 99 -p 512,2048,8192 -n 128 -ctk f16 -ctv f16 -o csv
```

Вывод даёт `pp512`, `pp2048`, `tg128` в tok/s — эти числа сравнимы между моделями и квантизациями【turn1search8】【turn1search6】.

Для измерения в реальном инференсе runner должен использовать **стриминговый режим** `llama-server` и парсить поле `timings` из completion-ответа (или `usage`), которое содержит `prompt_n`, `prompt_per_second`, `predicted_n`, `predicted_per_second` — это официальные метрики движка, а не ваши таймеры на коленке【turn1search3】【turn1search4】.

## 4. Структура набора задач (tasks.jsonl)

Каждая задача — самодостаточный кейс с детерминированной проверкой:

```json
{
  "id": "refactor-001",
  "category": "refactor",
  "repo": "tiny_shop_py",
  "language": "python",
  "instruction": "Вынеси логику расчёта скидки в отдельную функцию `apply_discount(items, coupon)`, сохранив публичный API класса `Cart`.",
  "files": {"cart.py": "<source>"},
  "tests": {
    "fail_to_pass": ["test_discount_edge_zero", "test_discount_negative_items"],
    "pass_to_pass": ["test_add_item", "test_total", "test_remove_item"]
  },
  "gold_patch": "diff --git a/cart.py ...",
  "expected_diff_summary": "extract method apply_discount, no behavior change",
  "judge_rubric": {
    "behavior_preserved": 1,
    "single_responsibility": 1,
    "no_dead_code": 1,
    "naming": 1
  },
  "max_tokens": 2048,
  "temperature": 0.0
}
```

Рекомендуемое распределение по категориям и эталонные датасеты под каждую:

| Категория | Что тестирует | Эталонный датасет | Метрика |
|---|---|---|---|
| **Bug-fix** | Локализация + исправление дефекта | SWE-bench Lite (300 задач), Defects4J | pass@1 (FAIL_TO_PASS + PASS_TO_PASS)【turn0search16】【turn0search9】 |
| **Refactor (behavior-preserving)** | Реструктуризация без изменения поведения | SWE-Refactor, Aider refactor-benchmark (133 Exercism), RefactorBench | PASS_TO_PASS сохранность + rubric judge【turn0search12】【turn0search11】【turn0search10】 |
| **Code review** | Поиск дефекта по диффу, ревью PR | CodeReviewerBench, SWE Atlas-Refactoring | recall найденных багов + judge【turn0search14】 |
| **Diff format compliance** | Умение выдать валидный search-replace/unified diff | Aider edit-format тесты | edit-format success %【turn0search13】 |

Тщательнее формируйте **pass_to_pass** — именно оно ловит «рефакторинг, который что-то молча сломал». В SWE-bench это ключевая защита от ложных «успехов»【turn0search16】.

## 5. Скелет раннера на Python

```python
# runner.py — вызов модели и сбор таймингов
import time, requests, json, statistics

def run_task(server_url, task, model_tag):
    t0 = time.perf_counter()
    first_tok_ts = None
    chunks = []

    with requests.post(
        f"{server_url}/v1/chat/completions",
        json={
            "model": model_tag,
            "messages": [
                {"role": "system", "content": "Return ONLY a unified diff. No prose."},
                {"role": "user", "content": build_prompt(task)},
            ],
            "temperature": 0.0,
            "max_tokens": task["max_tokens"],
            "stream": True,
            "stream_options": {"include_usage": True},
        },
        stream=True, timeout=600,
    ) as r:
        for line in r.iter_lines():
            if not line or not line.startswith(b"data: "):
                continue
            payload = line[6:]
            if payload == b"[DONE]":
                break
            obj = json.loads(payload)
            delta = obj["choices"][0].get("delta", {})
            if delta.get("content"):
                if first_tok_ts is None:
                    first_tok_ts = time.perf_counter()
                chunks.append(delta["content"])
            # финальный chunk с usage/timings от llama-server
            if obj.get("timings"):
                timings = obj["timings"]

    text = "".join(chunks)
    return {
        "output": text,
        "ttft_ms": (first_tok_ts - t0) * 1000 if first_tok_ts else None,
        "pp_tps": timings.get("prompt_per_second"),
        "tg_tps": timings.get("predicted_per_second"),
        "prompt_tokens": timings.get("prompt_n"),
        "gen_tokens": timings.get("predicted_n"),
        "wall_ms": (time.perf_counter() - t0) * 1000,
    }
```

`build_prompt` должен включать исходные файлы + инструкцию + требование формата вывода. Для bug-fix добавляйте стектрейс/issue-описание, для review — дифф. Температуру держите на 0.0 — это делает pass@1 осмысленным и сравнимым; при необходимости усредняйте по seed 0/1/2 для устойчивости (как в EvalPlus)【turn0search11】.

## 6. Измерение KV-кэша без квантизации

Отдельный скрипт `probe_kv.py`:

```python
# Бинарный поиск максимального ctx-size до OOM, всё в GPU, KV=f16
import subprocess

def fits(ctx):
    p = subprocess.Popen([
        "llama-server", "-m", MODEL, "-ngl", "99",
        "-c", str(ctx), "-ctk", "f16", "-ctv", "f16",
        "--port", "0",
    ])
    # ждём загрузки или OOM по stderr
    ...
    return loaded_ok

lo, hi = 2048, 65536
while lo + 1024 < hi:
    mid = (lo + hi) // 2
    if fits(mid): lo = mid
    else: hi = mid
print("max_kv_f16 =", lo)
```

Параллельно снимайте `nvidia-smi` для подтверждения, что веса + KV действительно легли в 16 ГБ и нет spill-over в RAM (когда KV не помещается в VRAM, llama.cpp молча кладёт часть на CPU — это убивает TG)【turn0search3】【turn1search2】.

## 7. LLM-as-judge: нужна ли он, и как не испортить оценку

**Вывод: обязателен только для рефакторинга/ревью, где нет однозначного теста; для баг-фиксов избыточен.** Конкретно:

- **Bug-fix**: объективная метрика pass@1 (FAIL_TO_PASS + PASS_TO_PASS) полностью покрывает «исправил или нет». Судья добавит шум, не точность【turn0search16】.
- **Refactor**: PASS_TO_PASS гарантирует сохранение поведения, но не говорит, стал ли код лучше. Здесь LLM-judge по рубрике (single-responsibility, no dead code, naming, idiomaticity) уместен.
- **Code review**: без судьи метрику построить сложно; используйте recall найденных дефектов по gold-списку + judge как доп.

Правила корректного LLM-judge (иначе оценки пострадают):

1. **Парная схема с swap-positions**: для каждой пары моделей прогоняйте сравнение дважды, меняя порядок ответов в промпте. Если вердикт инвертируется — помечайте как «bias-tie» и не учитывайте. Это канонический способ гашения position bias【turn1search16】【turn1search19】.
2. **Point-wise рубрика вместо пары, где можно**: просите судью дать целочисленную оценку 1–5 по каждому критерию отдельно (а не «какой лучше»). Меньше подвержено length bias.
3. **Фиксируйте длину ответов**: судьи систематически предпочитают более длинные ответы (verbosity bias) — это документированный артефакт【turn1search17】【turn1search18】. Добавляйте в промпт судьи явное указание «не оценивай длину» и нормализуйте.
4. **Калибровка по эталону**: возьмите 20 задач с известными gold-патчами, прогоните судью на gold vs намеренно испорченный патч — судья должен стабильно ставить gold выше. Если нет — поменяйте промпт/модель.
5. **Судья ≠ тестируемая модель**: используйте модель иного семейства и более крупную (GPT-4o / Claude), не одну из сравниваемых, иначе синофантия и общие систематические слепоты совпадут【turn1search18】.
6. **Температура судьи = 0**, несколько прогонов с усреднением; сообщайте `agreement_rate` между прогонами (repetition stability)【turn1search16】.

Калибровочный чек-лист для запуска бенчмарка:

```
[ ] Docker/venv изоляция pytest-исполнителя (код модели может делать import os)
[ ] temperature=0, fixed seed, 3 прогона для усреднения pass@1
[ ] -ngl 99 + проверка nvidia-smi, что KV не свалился в RAM
[ ] PASS_TO_PASS включён в каждую задачу рефакторинга
[ ] LLM-judge: swap-positions on, verbosity-notice in prompt, judge ≠ тестируемая модель
[ ] Детерминированный порядок задач, фиксированный system prompt во всех прогонах
[ ] Логирование полных ответов для пост-анализа регрессий
[ ] Версии llama.cpp и GGUF-файла зафиксированы в отчёте
```

## 8. Отчёт и интерпретация

Финальная сводка на модель — таблица с агрегатами: `pass@1 (bug-fix)`, `pass_to_pass_rate (refactor)`, `edit_format_success`, `pp_tps`, `tg_tps`, `ttft_ms`, `vram_peak_gb`, `max_kv_f16`, `judge_score_avg`, `judge_swap_disagreement_%`. Сортировка по композитному рангу или по pass@1 — последний самый корректный сигнал, остальные — контекст【turn0search13】【turn0search11】. Для значимого сравнения 8B-моделей между собой держите ~50–100 задач на категорию: на меньшем числе pass@1 шумит сильнее, чем реальные различия между близкими моделями.

Практический итог: запустите связку `llama-server (OpenAI API, -ngl 99, f16 KV) → runner.py → pytest evaluator (pass@1 / PASS_TO_PASS) → опц. LLM-judge со swap-калибровкой → CSV-репорт`. Это ровно тот слой абстракции, на котором работают SWE-bench, Aider и EvalPlus, адаптированный под локальный 16 ГБ инференс и требование полностью GPU-resident KV без квантизации【turn0search2】【turn0search11】【turn0search10】【turn1search6】.







Готовых наборов задач под бенчмаркинг кодовых LLM довольно много, и для ваших категорий (bug-fix, refactor, code-review) полностью с нуля придумывать задания не нужно — имеет смысл стартовать с готовых репозиториев и датасетов на HuggingFace, а собственный мини-сет собирать лишь для специфичных стектрейсов/доменов, которых нет в публичных. Ниже — каталог проверенных источников, разбор применимости под локальный 8B на 16 ГБ VRAM, и каркас загрузчика.

## Сравнительная карта готовых наборов

| Набор | Тип задач | Объём | Формат / метрика | Репо / датасет | Годен для локального 8B (16 ГБ) |
|---|---|---|---|---|---|
| **HumanEvalFix** (из HumanEvalPack) | Bug-fix функций | 164 | buggy_solution + canonical + test, pass@1 | `bigcode/humanevalpack` (config `python`)【turn1search0】【turn1search2】【turn0search5】 | ✅ Идеально, контекст короткий |
| **EvalPlus HumanEval+ / MBPP+** | Генерация/исправление, усиленные тесты | 164 / 378 | test (80× и 35× больше тестов), pass@1 | `evalplus/evalplus` (GitHub), `evalplus/humanevalplus`, `evalplus/mbppplus`【turn0search15】【turn0search17】【turn0search16】【turn0search18】 | ✅ Короткий контекст |
| **BugsInPy** | Реальные баги в Python-проектах | 493 | коммиты до/после, воспроизводимо через framework | `soarsmu/BugsInPy`【turn1search9】【turn1search6】 | ⚠️ Подмножество, контекст фрагмента |
| **SWE-bench Lite** | Bug-fix в реальном репо | 300 (или 534) | FAIL_TO_PASS + PASS_TO_PASS, gold patch | `SWE-bench/SWE-bench_Lite` (HF)【turn0search0】【turn0search1】【turn0search4】 | ⚠️ Подмножество, контекст может выходить за 16–32K |
| **SWE-bench Verified** | Bug-fix, экспертно верифицированные | 500 | то же | `SWE-bench/SWE-bench_Verified` (HF)【turn0search2】 | ⚠️ Аналогично |
| **Aider refactor-benchmark** | Рефакторинг (133 Exercism + крупные файлы) | 133 + крупные файлы из 9 репо | edit-format success + pass всех unit-тестов | `Aider-AI/refactor-benchmark`【turn0search0】【turn0search1】 | ✅ Хорошо для рефакторинга |
| **RefactorBench** (Microsoft) | Мультифайловый рефакторинг | 100 | AST unit-тесты, behavior-preserving | `microsoft/RefactorBench`【turn0search12】【turn0search10】 | ⚠️ Контекст нескольких файлов — проверять влезание |
| **BigCodeBench** | Сложные задачи с библиотеками (с нуля) | 1140 | pass@1 по ~5.6 тестам на задачу | `bigcode/bigcodebench` (HF), `bigcode-project/bigcodebench` (GitHub)【turn0search5】【turn0search7】【turn0search9】 | 🚫 Вне вашего ТЗ (это генерация с нуля), но можно переформатировать в "доработать" |
| **Code Review Bench (Martian)** | Code review PR | 50 PR × 5 репо | golden comments, precision/recall/F1 | `withmartian/code-review-benchmark`【turn0search15】【turn0search17】 | ✅ Подходит для ревью |
| **Defects4J / GitBug-Java** | Java-баги | 854 / 199 | репродьюсимая среда, pass-to-pass | `rjust/defects4j`, `gitbugactions/gitbug-java`【turn0search11】【turn1search7】 | ⚠️ Только если Java в скоупе |

## Что выбрать под локальный 8B / 16 ГБ VRAM

Для инференса на 8B-модели с контекстом 16–32K (= предел KV-кэша f16 на 16 ГБ) рекомендуется такой стартовый микс, от лёгкого к тяжёлому:

- **HumanEvalFix (164)** — самый лёгкий вход, контекст функции 50–200 токенов, эталон по bug-fix. Содержит поля `buggy_solution`, `canonical_solution`, `bug_type`, `failure_symptoms`, `test` — всё для pass@1 из коробки【turn1search2】.
- **Aider refactor-benchmark** — единственный из публичных, который заточен именно под редактирование существующего кода и измеряет edit-format success (способность выдать валидный diff/search-replace), что критично для 8B-моделей, склонных к «ленивому кодингу»【turn0search0】.
- **BugsInPy** — для перехода от синтетических к реальным багам; берите ~50 инстансов, по которым коммит-фикс локализован в одном-двух файлах, чтобы уложиться в контекст【turn1search9】.
- **SWE-bench Lite** — только после того, как первые три откалиброваны. Требует Docker-окружения и pull снапшотов репозиториев; для 8B берите подмножество ~50 инстансов с короткими issue-описаниями и небольшими diff'ами, иначе контекст + стектрейс не влезут в 32K【turn0search1】【turn0search3】.
- **Code Review Bench (Martian)** — готовый набор из 50 PR с проверенными людьми комментариями, который можно гонять на локальной 8B без тяжёлой инфраструктуры【turn0search15】.

RefactorBench от Microsoft красив, но 100 многофайловых задач на крупных open-source репо — это агрессивный сценарий, где 8B почти наверняка уткнётся в лимит контекста; используйте как «верхнюю границу» сложности, а не как основной корпус【turn0search10】【turn0search12】.

## Кураторские awesome-репозитории для навигации

Если нужны не только перечисленные, а весь ландшафт — три проверенных указателя:

- `codefuse-ai/Awesome-Code-LLM` — курируемый список датасетов и бенчмарков для кодовых LLM【turn1search11】.
- `BenchGecko/awesome-llm-benchmarks` — раздел Code Generation с актуальными ссылками【turn1search12】.
- `tongye98/Awesome-Code-Benchmark` — обзорная классификация кодовых бенчмарков【turn0search18】.
- `FudanSELab/awesome-software-engineering-research` — датасеты баг-фиксов (TSSB-3M, SSB-9M и др.) для крупномасштабных экспериментов【turn0search7】.

## Загрузчик датасетов (стартовый каркас)

Ниже скрипт, который тянет несколько ключевых наборов и приводит их к единой схеме `tasks.jsonl`, совпадающей с той, что обсуждалась для раннера в предыдущем ответе:

```python
# fetch_datasets.py
from datasets import load_dataset
import json, pathlib

OUT = pathlib.Path("tasks"); OUT.mkdir(exist_ok=True)

def dump_jsonl(rows, name):
    p = OUT / f"{name}.jsonl"
    with p.open("w") as f:
        for r in rows: f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"{name}: {len(rows)} tasks -> {p}")

# 1) HumanEvalFix из HumanEvalPack (config=python)
ds = load_dataset("bigcode/humanevalpack", "python", split="test")
rows = [{
    "id": f"hefix-{r['task_id']}",
    "category": "bugfix",
    "language": "python",
    "instruction": "Исправь баг в функции. Верни только исправленную функцию.",
    "files": {"solution.py": r["buggy_solution"]},
    "tests": {"fail_to_pass": extract_tests(r["test"], r["entry_point"]),
              "pass_to_pass": []},
    "bug_type": r.get("bug_type"),
    "failure_symptoms": r.get("failure_symptoms"),
    "max_tokens": 512,
    "temperature": 0.0,
} for r in ds]
dump_jsonl(rows, "humanevalfix")

# 2) EvalPlus HumanEval+ и MBPP+ (для расширения и перекрёстной проверки)
from evalplus.data import get_human_eval_plus, get_mbpp_plus
for name, getter in [("humaneval_plus", get_human_eval_plus),
                     ("mbpp_plus", get_mbpp_plus)]:
    data = getter()
    rows = [{
        "id": f"{name}-{k}",
        "category": "codegen",
        "language": "python",
        "instruction": v["prompt"],
        "files": {},
        "tests": {"fail_to_pass": v["test"], "pass_to_pass": []},
        "max_tokens": 1024, "temperature": 0.0,
    } for k, v in data.items()]
    dump_jsonl(rows, name)

# 3) Aider refactor-benchmark — клонируем репо и берём упражнения
import subprocess, os
if not (OUT.parent / "refactor-benchmark").exists():
    subprocess.run(["git", "clone", "--depth", "1",
        "https://github.com/Aider-AI/refactor-benchmark",
        str(OUT.parent / "refactor-benchmark")], check=True)
# Парсинг exercism-упражнений — см. README репозитория Aider для формата

# 4) Code Review Bench (Martian) — клон
if not (OUT.parent / "code-review-benchmark").exists():
    subprocess.run(["git", "clone", "--depth", "1",
        "https://github.com/withmartian/code-review-benchmark",
        str(OUT.parent / "code-review-benchmark")], check=True)
```

`extract_tests` — вспомогательная функция, достающая assert'ы из `test`-поля HumanEvalPack; формат там уже близок к исполняемому, обычно хватает `entry_point`-обёртки【turn1search2】【turn1search4】.

## Когда и как делать свой набор

Готовых датасетов хватает для bug-fix и базового рефакторинга. Собственный набор оправдан в трёх случаях:

1. **Специфичный домен/стек** — например, ваш внутренний фреймворк, pytest-фикстуры, конкретные библиотеки, которых нет в публичных наборах.
2. **Стектрейс-формат** — публичные наборы дают issue-описание или голую функцию; если вы хотите тестировать модель именно на разборе стектрейса, придётся сгенерировать стектрейсы самим (запустить намеренно сломанный код, снять traceback).
3. **Поведение на длинных контекстах** — если хотите давить именно на 16–32K контекст, готовые наборы либо слишком короткие (HumanEvalFix), либо слишком тяжёлые (SWE-bench). Промежуточный слой придётся собирать вручную.

Шаблон одной задачи собственного датасета, повторяющий схему публичных:

```json
{
  "id": "custom-bugfix-001",
  "category": "bugfix",
  "language": "python",
  "instruction": "Найди и исправь баг. Верни только unified diff.",
  "context": {
    "stacktrace": "Traceback (most recent call last):\n  File \"app.py\", line 42, in <module>\n    total = apply_discount(cart.items, coupon)\n  File \"cart.py\", line 28, in apply_discount\n    return sum(i * (1 - coupon) for i in items)\nTypeError: unsupported operand type(s)",
    "files": {"cart.py": "<source with bug>"}
  },
  "tests": {
    "fail_to_pass": ["test_discount_zero", "test_discount_negative"],
    "pass_to_pass": ["test_add", "test_remove"]
  },
  "gold_patch": "diff --git a/cart.py ...",
  "max_tokens": 1024,
  "temperature": 0.0
}
```

Ключевое правило при формировании собственного набора — **FAIL_TO_PASS и PASS_TO_PASS должны быть написаны до прогона модели**, иначе метрика pass@1 становится субъективной. Для bug-fix берёте сломанный код, пишете тест, который падает на баге и проходит на фиксе (FAIL_TO_PASS), и тесты, которые проходят и до и после (PASS_TO_PASS) — ровно так, как устроен SWE-bench【turn0search1】【turn0search16】.

## Сколько задач достаточно для значимых метрик

pass@1 на 8B-моделях шумит сильно; эмпирические ориентиры из практики исследовательских команд и устройства публичных бенчмарков:

- **Минимум для калибровки раннера**: 20 задач на категорию — хватает, чтобы поймать грубые отличия между моделями (например, одна стабильно ломает синтаксис, другая нет).
- **Значимое сравнение моделей**: 50–100 задач на категорию — статистически различимая разница в pass@1 между близкими 8B-моделями.
- **Полноценный бенчмарк**: 200+ на категорию (как в SWE-bench Lite / Verified)【turn0search1】【turn0search2】.

Для первого прогона на вашем железе рекомендуется: 164 HumanEvalFix + 50 Aider refactor + 20 Martian code-review = ~234 задач, что уложится в один вечер инференса на 8B при `tg ~50 tok/s` и среднем `gen_tokens ~800`.

## Чего нет в публичных и придётся делать самим

- **Длинные стектрейсы с multi-file локализацией** — готовых наборов мало; RefactorBench частично закрывает, но тянет за собой полные репозитории【turn0search12】.
- **Тесты на утечку контекста** — последовательные правки с накоплением сессии; публичных нет, собираются из своих логов разработки.
- **Языковая специфика кроме Python/Java** — для Go/Rust/TS готовых bug-fix наборов заметно меньше; в `awesome-software-engineering-research` есть TSSB-3M/SSB-9M, но это Python-коммиты【turn0search7】.

Стартовая точка: клонировать `fetch_datasets.py` выше, прогнать 164 HumanEvalFix на раннере из предыдущего ответа, получить pass@1/PP/TG по одной модели — это калибровочный baseline. Дальше подключать Aider-refactor и BugsInPy подмножеством, и только после стабилизации метрик переходить к SWE-bench Lite и собственным стектрейс-задачам.