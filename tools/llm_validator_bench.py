"""Стенд для LLM-валидатора: одиночный агент, без графа, плюс время.

Зачем. Laya мерится на 26 кейсах и даёт 189/234. У валидатора есть только старые
прогоны `gemma_task{1,2}.json`, и там он вернул девять `false` на обоих кейсах —
потому что агент не получил текст пациента (в отчётах он писал «Пожалуйста,
предоставьте сообщения пользователя»). То есть валидатор на этих прогонах не
работал, и сравнивать с ним нельзя.

Этот стенд делает ровно то, чего не сделал старый прогон: подаёт агенту текст
кейса как сообщение пользователя и меряет ВРЕМЯ вместе с вердиктами.

Что воспроизведено из продакшна, чтобы замер был честным:
  * `<<INCLUDE:>>` раскрывается ровно как в `agent_manager.rs::process_includes`:
    файл подставляется как `<file path=...><file_content>...</file_content></file>`;
  * промпт агента берётся из `agents/psychotherapist/backend/validator.md`
    без правок — его правила, фильтр Шага 0 и требование цитаты;
  * текст кейса идёт сообщением пользователя (роль user), а не «задачей»;
  * параметры выборки — из `app_config.json` для этой модели, чтобы не подбирать
    их под себя.

Время меряется на стороне клиента (wall-clock одного запроса) — это тот же путь,
которым ждёт пользователь в приложении.

Запуск из корня проекта:
    python tools\\llm_validator_bench.py --case task3,task6
    python tools\\llm_validator_bench.py --all
Результат: test/laya_probe/results/llm_validator_bench.json
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

from tools.laya_probe import ELEMENTS, DEFAULT_CASES, load_cases  # noqa: E402

RESULTS = ROOT / "test/laya_probe/results"
AGENT_MD = ROOT / "agents/psychotherapist/backend/validator.md"
APP_CONFIG = Path.home() / "AppData/Roaming/com.kingorch.app/app_config.json"
# Пресеты выборки лежат рядом с exe приложения: узел валидатора в фикстуре объявлен
# как `default_llm_params: strict`, то есть замерять надо на НЁМ, а не на
# пользовательских параметрах модели из app_config (temperature 0.5 и т.п.).
SAMPLING_PRESETS = Path("D:/Programs/nildencorp/King Orch/sampling_presets.json")
# Движок ставит пользователь отдельно (docs/LAYA_MODEL.md §1): ищем в его data_dir.
ENGINE_ROOT = Path("D:/Programs/nildencorp/KingOrchData/llamacpp")
INCLUDE_RE = re.compile(r"<<INCLUDE:\s*(.+?)\s*>>", re.S)
TIMEOUT_LOAD_S = 900


def log(message: str, end: str = "\n") -> None:
    print(message, end=end, flush=True)


# --------------------------------------------------------------------------- промпт


def process_includes(base_dir: Path, content: str) -> str:
    """Дословный порт `agent_manager.rs::process_includes` (строки 109-120)."""

    def replace(match: re.Match[str]) -> str:
        rel_path = match.group(1).strip()
        full_path = base_dir / rel_path
        try:
            file_content = full_path.read_text(encoding="utf-8")
        except OSError:
            return (
                f'\n<error>Файл {rel_path} не найден по пути {full_path}</error>\n'
            )
        return (
            f'\n<file path="{rel_path}">\n'
            f"<file_content>\n{file_content}\n</file_content>\n</file>\n"
        )

    return INCLUDE_RE.sub(replace, content)


def build_system_prompt(agent_md: Path) -> str:
    raw = agent_md.read_text(encoding="utf-8")
    resolved = process_includes(agent_md.parent, raw)
    # YAML-фронтматтер — это метаданные профиля агента, а не текст системного
    # промпта: модель не должна видеть `name:` и `tools:`.
    if resolved.startswith("---"):
        parts = resolved.split("---", 2)
        if len(parts) == 3:
            resolved = parts[2].lstrip("\n")
    return resolved


def swap_criteria(prompt: str, agent_md: Path, criteria_path: Path) -> str:
    """Заменяет содержимое `<file>` с критериями на другой файл.

    Точечная замена нужна для честного сравнения с Laya: она работает на
    `element_validation_rules_prod_noul.yaml`, а валидатор по умолчанию видит
    `database/element_validation_rules.yaml`. Форматы разные, поэтому разрыв
    в вердиктах нельзя приписывать модели, пока критерии не совпадают.

    Подменяется ровно один блок — тот, чей `path` заканчивается на
    `element_validation_rules.yaml`. Остальные INCLUDE (в частности
    `neurosis_architecture.md`) не трогаются.
    """
    criteria_text = criteria_path.read_text(encoding="utf-8")
    needle = "element_validation_rules.yaml"

    pattern = re.compile(
        r'(<file path="[^"]*' + re.escape(needle) + r'">\n)'
        r'<.*?file_content>\n.*?\n</file_content>\n(</file>)',
        re.S,
    )
    replaced, count = pattern.subn(
        lambda m: m.group(1) + "" + criteria_text + "\n" + m.group(2),
        prompt,
    )
    if count != 1:
        raise ValueError(
            f"ожидался 1 блок с {needle}, заменено {count}; "
            "структура INCLUDE в агенте изменилась"
        )
    return replaced


# --------------------------------------------------------------------------- движок


def find_llama_server() -> Path | None:
    if not ENGINE_ROOT.exists():
        return None
    candidates = sorted(ENGINE_ROOT.glob("backends/*/*/build/bin/llama-server.exe"), reverse=True)
    return candidates[0] if candidates else None


def load_preset(name: str) -> dict[str, Any]:
    """Пресет выборки приложения (`strict` по умолчанию — как в узле валидатора)."""
    try:
        presets = json.loads(SAMPLING_PRESETS.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {"temperature": 0.0, "top_k": 0, "top_p": 1.0,
                "min_p": 0.0, "repetition_penalty": 1.1}
    preset = presets.get(name)
    return preset if isinstance(preset, dict) else presets.get("strict", {})


def pick_model(explicit: str) -> str:
    if explicit:
        return explicit
    config = json.loads(APP_CONFIG.read_text(encoding="utf-8"))
    return config.get("last_model") or (config.get("models") or [""])[0]


def wait_health(port: int, proc: subprocess.Popen, deadline: float) -> bool:
    url = f"http://127.0.0.1:{port}/health"
    while time.time() < deadline:
        if proc.poll() is not None:
            log(f"    движок умер с кодом {proc.returncode}")
            return False
        try:
            with urllib.request.urlopen(url, timeout=5) as response:
                if response.status == 200:
                    return True
        except (urllib.error.URLError, OSError):
            time.sleep(2)
    return False


def start_engine(server: Path, model: str, port: int, ctx: int,
                 gpu_layers: int = 999) -> subprocess.Popen:
    # -ngl обязателен: без него llama.cpp держит модель на CPU и генерирует
    # ~1 токен/с (пробный прогон: 568 токенов за 9 минут). Приложение работает
    # на CUDA-бэкенде, поэтому и стенд обязан мерить на GPU.
    cmd = [
        str(server), "-m", model, "--port", str(port), "-c", str(ctx),
        "-ngl", str(gpu_layers), "-np", "1", "--no-webui", "--jinja",
        "--host", "127.0.0.1",
    ]
    log("    " + " ".join(cmd))
    return subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


# --------------------------------------------------------------------------- запрос


SYSTEM_PATH = str(ROOT / "test/laya_probe")
if SYSTEM_PATH not in sys.path:
    sys.path.insert(0, SYSTEM_PATH)




GRAMMAR_FILE = ROOT / "test/laya_probe/results/rust_exact_grammar.gbnf"


def load_grammar() -> str:
    """Грамматика сигнала `validator` — воспроизведённая из `signals.rs`.

    Файл собирается `test/laya_probe/reproduce_rust_grammar.py` из строк
    формата самого `signals.rs` (результат лексической разборки + faithful
    `format!`), и проверен движком b9967: принимается, отдаёт валидный конверт.

    ВАЖНО: не использовать `print_signal_grammar.py` — там строки формата
    перенабраны вручную, теряются `{{`/`}}` и обратные слеши, и получается
    невалидная грамматика (движок отвечает HTTP 400).
    """
    if not GRAMMAR_FILE.exists():
        raise FileNotFoundError(
            f"Нет грамматики: {GRAMMAR_FILE}\n"
            "Сгенерируй: python test/laya_probe/reproduce_rust_grammar.py"
        )
    return GRAMMAR_FILE.read_text(encoding="utf-8").strip()


def bound_thinking(grammar: str, think_chars: int) -> str:
    """Жёстко ограничивает рассуждение В САМОЙ грамматике.

    Зачем это в грамматике, а не через `max_tokens`. `max_tokens` — это
    аварийный клапан: модель сначала тратит бюджет, и только потом его обрубает.
    На task16 12B уходила в цикл рассуждения и сожгла 121 секунду / 6000 токенов,
    не ответив ничего. Для продакшна это хуже всего варианта: время потрачено,
    сигнала нет.

    GBNF позволяет ограничить повтор: `[^<]{0,N}`. После исчерпания N символов
    грамматика форсирует `</think>`, и модель физически не может продолжать
    рассуждать — потолок не «наступает», а не даёт начаться.

    N задаётся в СИМВОЛАХ, не в токенах: у llama.cpp нет токенного счётчика в
    грамматике. Соотношение для русского текста — примерно 3 символа на токен,
    калибровка замерена прогоном (см. §18 документации).
    """
    if think_chars <= 0:
        return grammar
    bounded = '"<think>" [^<]{0,%d} "</think>"' % think_chars
    out, count = re.subn(r'"<think>" \[\^<\]\* "</think>"', bounded, grammar)
    if count != 1:
        raise ValueError(
            "не нашлась строка think-block — грамматика изменилась, "
            f"проверь {GRAMMAR_FILE}"
        )
    return out


def post_chat(port: int, system: str, user: str, params: dict[str, Any],
              timeout: int, max_tokens: int, grammar: str = "") -> dict[str, Any]:
    """Один запрос к агенту.

    `max_tokens` обязателен: без него llama-server генерирует бесконечно
    (`n_predict = -1` в настройках движка), и стенд висит. Прогон с
    `n_predict = -1` уже показал 529+ токенов на task1 без остановки.
    """
    payload = {
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "temperature": params.get("temperature", 0.0),
        "top_p": params.get("top_p", 1.0),
        "top_k": params.get("top_k", 0),
        "min_p": params.get("min_p", 0.0),
        "repeat_penalty": params.get("repetition_penalty", 1.1),
        "max_tokens": max_tokens,
        "seed": 20261003,
        "stream": False,
        "cache_prompt": False,
    }
    if grammar:
        # Грамматика сигнала — ровно та, что строит приложение
        # (`signals.rs::build_signal_envelope_grammar`, порт в
        # `test/laya_probe/print_signal_grammar.py`). Без неё модель пишет
        # рассуждение до потолка генерации и не доходит до JSON: в первом
        # прогоне так не ответили 9 из 26 кейсов.
        payload["grammar"] = grammar
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/chat/completions",
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


JSON_OBJ_RE = re.compile(r"\{[^{}]*\}", re.S)


def extract_report(text: str) -> dict[str, bool] | None:
    """Достаёт e1..e9 из ответа. Модель может обернуть их в emit_signal."""
    for candidate in JSON_OBJ_RE.findall(text):
        try:
            data = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        node: Any = data
        if isinstance(node, dict) and "validator_report" in node:
            node = node["validator_report"]
        if not isinstance(node, dict):
            continue
        keys = {k.lower(): v for k, v in node.items()}
        if all(f"e{n}" in keys for n in range(1, 10)):
            return {f"e{n}": bool(keys[f"e{n}"]) for n in range(1, 10)}
    return None


# --------------------------------------------------------------------------- main


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", default="", help="путь к .gguf (по умолчанию last_model из конфига)")
    parser.add_argument("--case", default="", help="через запятую; по умолчанию все готовые кейсы")
    parser.add_argument("--all", action="store_true", help="прогнать все 26 кейсов")
    parser.add_argument("--port", type=int, default=8731)
    parser.add_argument("--ctx", type=int, default=8192)
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--max-tokens", type=int, default=1600,
                        help="потолок генерации; без него движок генерирует бесконечно")
    parser.add_argument("--preset", default="strict",
                        help="пресет выборки приложения (sampling_presets.json)")
    parser.add_argument("--gpu-layers", type=int, default=999,
                        help="сколько слоёв выгрузить на GPU; 0 = CPU")
    parser.add_argument("--no-grammar", action="store_true",
                        help="без грамматики сигнала (для контроля: сколько кейсов "
                             "не доходит до JSON без неё)")
    parser.add_argument("--think-chars", type=int, default=0,
                        help="жёсткий потолок рассуждения в символах, зашитый "
                             "в грамматику (0 = не ограничивать). Это "
                             "продакшн-бюджет: без него модель может уйти в цикл "
                             "и сжечь всё окно, не ответив ничего")
    parser.add_argument("--criteria", type=Path, default=None,
                        help="файл критериев, которым ПОДМЕНЯЕТСЯ "
                             "database/element_validation_rules.yaml. Нужен, "
                             "чтобы сравнивать с Laya на одних и тех же "
                             "критериях (Laya работает на prod_noul). "
                             "Не указан — берутся боевые критерии из аген��а")
    parser.add_argument("--out", type=Path, default=RESULTS / "llm_validator_bench.json")
    args = parser.parse_args()

    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:
        pass

    cases = [case for case in load_cases(DEFAULT_CASES) if case.ready]
    if args.case:
        wanted = [item.strip() for item in args.case.split(",") if item.strip()]
        cases = [case for case in cases if case.id in wanted]
    if not cases:
        log("Нет кейсов к прогону")
        return 1

    model = pick_model(args.model)
    params = load_preset(args.preset)
    system = build_system_prompt(AGENT_MD)
    if args.criteria:
        criteria_path = args.criteria if args.criteria.is_absolute() else ROOT / args.criteria
        if not criteria_path.exists():
            log(f"Файл критериев не найден: {criteria_path}")
            return 1
        before = len(system)
        system = swap_criteria(system, AGENT_MD, criteria_path)
        log(f"Критерии подменены на {criteria_path.name} "
            f"({before} -> {len(system)} символов промпта)")

    server = find_llama_server()
    if server is None:
        log(f"llama-server.exe не найден под {ENGINE_ROOT}")
        return 1
    model_file = Path(model)
    if not model_file.exists():
        log(f"Модель не найдена: {model}")
        return 1

    log(f"Агент: {AGENT_MD.relative_to(ROOT)}")
    log(f"Системный промпт: {len(system)} символов (INCLUDE раскрыт)")
    log(f"Модель: {model_file.name}")
    log(f"Пресет: {args.preset} -> temperature={params.get('temperature')} "
        f"top_p={params.get('top_p')} top_k={params.get('top_k')} "
        f"repeat_penalty={params.get('repetition_penalty')} max_tokens={args.max_tokens}")
    grammar = "" if args.no_grammar else bound_thinking(load_grammar(), args.think_chars)
    if grammar:
        log(f"Грамматика сигнала: включена ({len(grammar)} символов, "
            f"{GRAMMAR_FILE.name}, потолок рассуждения "
            f"{args.think_chars if args.think_chars else 'НЕ ОГРАНИЧЕН'})")
    else:
        log("Грамматика сигнала: ВЫКЛЮЧЕНА (контрольный режим)")

    log(f"Движок: {server}")
    log(f"Кейсов: {len(cases)}\n")

    log("Запуск движка (загрузка модели может занять минуты)...")
    started_engine = time.perf_counter()
    proc = start_engine(server, str(model_file), args.port, args.ctx, args.gpu_layers)
    if not wait_health(args.port, proc, time.time() + TIMEOUT_LOAD_S):
        log("Движок не поднялся — прогон невозможен")
        return 1
    log(f"Движок готов за {time.perf_counter() - started_engine:.1f} с\n")

    payload: dict[str, Any] = {
        "agent_file": str(AGENT_MD.relative_to(ROOT)),
        "model": str(model_file),
        "model_name": model_file.name,
        "sampling": {"preset": args.preset,
                     **{k: params.get(k) for k in
                        ("temperature", "top_p", "top_k", "min_p", "repetition_penalty")},
                     "max_tokens": args.max_tokens},
        "grammar": "on" if grammar else "off",
        "engine": str(server),
        "engine_load_s": round(time.perf_counter() - started_engine, 1),
        "n_cases": len(cases),
        "cases": [],
    }

    try:
        for index, case in enumerate(cases, start=1):
            log(f"[{index}/{len(cases)}] {case.id} ...", end="")
            started = time.perf_counter()
            try:
                response = post_chat(args.port, system, case.prompt, params,
                                   args.timeout, args.max_tokens, grammar)
            except urllib.error.HTTPError as error:
                elapsed = time.perf_counter() - started
                body = ""
                try:
                    body = error.read().decode("utf-8", errors="replace")[:600]
                except Exception:
                    pass
                log(f" ОТКАЗ СЕРВЕРА HTTP {error.code} за {elapsed:.1f} с: {body}")
                payload["cases"].append({
                    "case": case.id, "ok": False,
                    "error": f"HTTP {error.code}", "error_body": body,
                    "elapsed_s": round(elapsed, 2),
                    "expected": {e: case.expected[e] for e in ELEMENTS},
                    "report": None,
                })
                continue
            except (urllib.error.URLError, OSError, TimeoutError) as error:
                elapsed = time.perf_counter() - started
                log(f" ОШИБКА {elapsed:.1f} с: {error}")
                payload["cases"].append({
                    "case": case.id, "ok": False, "error": str(error),
                    "elapsed_s": round(elapsed, 2),
                    "expected": {e: case.expected[e] for e in ELEMENTS},
                    "report": None,
                })
                continue

            elapsed = time.perf_counter() - started
            choice = (response.get("choices") or [{}])[0]
            message = choice.get("message") or {}
            content = message.get("content") or ""
            reasoning = message.get("reasoning_content") or ""
            # finish_reason отличает «модель закончила сама» от «движок оборвал»
            # — без него неразобранный кейс не диагностируется.
            finish_reason = choice.get("finish_reason")
            # peg-gemma4 с reasoning_format=deepseek кладёт рассуждение в отдельное
            # поле, а `content` оставляет пустым (проверено пробным прогоном:
            # content='' при completion_tokens=1200). Поэтому ищем в обоих.
            report = (
                extract_report(content)
                or extract_report(reasoning)
                or extract_report(json.dumps(message, ensure_ascii=False))
            )

            correct = 0
            fp = fn = 0
            if report:
                for element in ELEMENTS:
                    expected = case.expected[element]
                    got = report[element]
                    if got == expected:
                        correct += 1
                    elif got:
                        fp += 1
                    else:
                        fn += 1

            usage = response.get("usage") or {}
            completion = usage.get("completion_tokens") or 0
            # Модель может исчерпать потолок генерации, не дойдя до сигнала:
            # тогда content пуст, а весь текст лежит в reasoning_content.
            # Это не «медленно», а «не ответил» — помечаем отдельно, иначе
            # такой кейс молча зачтётся как «быстрый и неверный».
            hit_cap = completion >= args.max_tokens
            log(f" {elapsed:.1f} с | токенов {completion}"
                f"{' (ПОТОЛОК)' if hit_cap else ''} | верно {correct}/9"
                f" | FP {fp} FN {fn}{'' if report else ' | ОТВЕТ НЕ РАЗОБРАН'}")
            payload["cases"].append({
                "case": case.id,
                "ok": bool(report),
                "elapsed_s": round(elapsed, 2),
                "hit_token_cap": hit_cap,
                "completion_tokens": completion,
                "correct": correct,
                "fp": fp,
                "fn": fn,
                "report": report,
                "expected": {e: case.expected[e] for e in ELEMENTS},
                "prompt_tokens": usage.get("prompt_tokens"),
                "raw_head": content[:400],
                # Хвост нужен, чтобы понять, ПОЧЕМУ ответ не распарсился.
                # task12B генерировал 4136 токенов, не дошёл до потолка, и JSON
                # не появился — по началу ответа это не диагностируется.
                "raw_tail": content[-400:],
                "finish_reason": finish_reason,
                "raw_reasoning_head": reasoning[:400],
                "raw_message_keys": sorted(message),
            })
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            proc.kill()

    ok_cases = [row for row in payload["cases"] if row["ok"]]
    total = sum(row["correct"] for row in ok_cases)
    times = sorted(row["elapsed_s"] for row in payload["cases"])
    payload["summary"] = {
        "parsed": len(ok_cases),
        "unparsed": len(payload["cases"]) - len(ok_cases),
        "correct_total": total,
        "possible_total": len(ok_cases) * 9,
        "fp": sum(row["fp"] for row in ok_cases),
        "fn": sum(row["fn"] for row in ok_cases),
        "time_min_s": round(times[0], 2) if times else None,
        "time_median_s": round(times[len(times) // 2], 2) if times else None,
        "time_mean_s": round(sum(times) / len(times), 2) if times else None,
        "time_max_s": round(times[-1], 2) if times else None,
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")

    summary = payload["summary"]
    log("\n" + "=" * 70)
    log(f"РАЗОБРАНО {summary['parsed']}/{payload['n_cases']}, "
        f"не разобрано {summary['unparsed']}")
    if summary["parsed"]:
        log(f"ВЕРНО {summary['correct_total']}/{summary['possible_total']} = "
            f"{summary['correct_total'] / summary['possible_total'] * 100:.1f}%  "
            f"FP {summary['fp']}  FN {summary['fn']}")
    log(f"ВРЕМЯ на кейс: мин {summary['time_min_s']} с, "
        f"медиана {summary['time_median_s']} с, среднее {summary['time_mean_s']} с, "
        f"макс {summary['time_max_s']} с")
    log(f"Результат: {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())