from __future__ import annotations

import argparse
import json
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml

if sys.stdout.encoding != "utf-8":
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

DEFAULT_MODEL_ID = "convaiinnovations/laya-multilingual"
DEFAULT_MODEL_REVISION = "e4e9ddf21a7b1903b7acffd8814ad4307bf63a67"
ALFRED_MODEL_ID = "alfred361/laya-multilingual-typed-decisions"
ALFRED_MODEL_REVISION = "60ce5be491a7723df937b757e43b254a08898f8e"
ONNX_MODEL_ID = "mizchi/laya-multilingual-onnx"

PROJECT_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CASES = PROJECT_ROOT / "test_cases/new_tests_for_validator/cases.yaml"
# Критерии (тексты вопросов e1..e9) живут отдельно от кейсов, но рядом с ними:
# docs/LAYA_MODEL.md §6.1.
DEFAULT_RULES = (
    PROJECT_ROOT / "test_cases/new_tests_for_validator"
    / "element_validation_rules_prod_noul.yaml"
)
PROD_RULES = PROJECT_ROOT / "agents/psychotherapist/database/element_validation_rules.yaml"

ELEMENTS = tuple(f"e{number}" for number in range(1, 10))
# Приоритетные элементы — те, где ложный элемент у клиента дороже всего:
# e3 катастрофа, e6 триггер, e8 искажение мировоззрения, e9 идол.
# Кейс может переопределить список полем `priority_false_elements`.
DEFAULT_PRIORITY_FALSE = ("e3", "e6", "e8", "e9")

# Человеческие записи вместо true/false — YAML их не булев, приводим сами.
TRUTHY = {"да", "yes", "y", "true", "1", "истина"}
FALSY = {"нет", "no", "n", "false", "0", "ложь"}


def load_json(path: Path) -> dict[str, Any]:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError(f"Не удалось прочитать {path}: {error}") from error


def as_bool(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        text = value.strip().lower()
        if text in TRUTHY:
            return True
        if text in FALSY:
            return False
    raise RuntimeError(f"Ожидалось true/false (или да/нет), получено: {value!r}")


@dataclass
class Case:
    """Кейс из cases.yaml.

    `skip` непустой -> кейс не заполнен и в прогон не идёт (см. load_cases).
    """

    id: str
    prompt: str
    note: str
    expected: dict[str, bool]
    priority_false: set[str]
    skip: str = ""

    @property
    def ready(self) -> bool:
        return not self.skip


def load_cases(path: Path) -> list[Case]:
    """Читает единый файл кейсов.

    Пустой prompt или неполный expected — это не ошибка, а «кейс ещё не заполнен»:
    такой кейс пропускается, чтобы можно было держать в файле заготовки. Ошибками
    считаем только битый YAML, отсутствие файла и неуникальные id.
    """
    try:
        document = yaml.safe_load(path.read_text(encoding="utf-8"))
    except (OSError, yaml.YAMLError) as error:
        raise RuntimeError(f"Не удалось прочитать {path}: {error}") from error

    if not isinstance(document, dict):
        raise RuntimeError(f"{path.name}: ожидался словарь с ключом cases")
    raw_cases = document.get("cases")
    if not isinstance(raw_cases, list):
        raise RuntimeError(f"{path.name}: нет списка cases")
    if not raw_cases:
        raise RuntimeError(f"{path.name}: список cases пуст")

    cases: list[Case] = []
    seen: set[str] = set()
    for position, raw in enumerate(raw_cases, start=1):
        if not isinstance(raw, dict):
            raise RuntimeError(f"{path.name}: кейс #{position} — не словарь")

        case_id = str(raw.get("id") or "").strip()
        if not case_id:
            raise RuntimeError(f"{path.name}: у кейса #{position} нет id")
        if case_id in seen:
            raise RuntimeError(f"{path.name}: повторяющийся id '{case_id}'")
        seen.add(case_id)

        prompt = str(raw.get("prompt") or "").strip()
        note = str(raw.get("note") or "").strip()

        priority_raw = raw.get("priority_false_elements")
        if priority_raw is None:
            priority_false = set(DEFAULT_PRIORITY_FALSE)
        else:
            if not isinstance(priority_raw, list):
                raise RuntimeError(
                    f"{path.name}: priority_false_elements у '{case_id}' — не список"
                )
            unknown = {str(item) for item in priority_raw} - set(ELEMENTS)
            if unknown:
                raise RuntimeError(
                    f"{path.name}: неизвестные элементы в priority_false_elements "
                    f"у '{case_id}': {sorted(unknown)}"
                )
            priority_false = {str(item) for item in priority_raw}

        expected: dict[str, bool] = {}
        expected_raw = raw.get("expected")
        if isinstance(expected_raw, dict) and expected_raw:
            try:
                expected = {
                    key: as_bool(expected_raw[key])
                    for key in ELEMENTS
                    if key in expected_raw
                }
            except RuntimeError as error:
                raise RuntimeError(f"{path.name}: '{case_id}' — {error}") from error

        skip = ""
        if not prompt:
            skip = "prompt пустой"
        elif not expected:
            skip = "expected не заполнен"
        else:
            missing = [key for key in ELEMENTS if key not in expected]
            if missing:
                skip = f"expected заполнен не полностью (нет {', '.join(missing)})"

        cases.append(
            Case(
                id=case_id,
                prompt=prompt,
                note=note,
                expected=expected,
                priority_false=priority_false,
                skip=skip,
            )
        )
    return cases


def load_thresholds(rules_path: Path) -> dict[str, float]:
    try:
        document = yaml.safe_load(rules_path.read_text(encoding="utf-8"))
    except Exception:
        return {}
    elements = document.get("elements", {}) if isinstance(document, dict) else {}
    thresholds = {}
    for num in range(1, 10):
        elem = elements.get(num, {}) if isinstance(elements, dict) else {}
        if isinstance(elem, dict) and "threshold" in elem:
            try:
                thresholds[f"e{num}"] = float(elem["threshold"])
            except (ValueError, TypeError):
                pass
    return thresholds


def build_questions(
    rules_path: Path,
    labels: str = "both",
) -> tuple[dict[str, dict[str, Any]], dict[str, str]]:
    """Собирает 9 вопросов из файла критериев.

    `labels` управляет подписями вариантов (документированная грабли #156):
      - `bool`       — подписи `true`/`false` (запрещены моделью, оставлено для контроля);
      - `neutral`    — подписи `A`/`B`, где A = «да»;
      - `neutral_rev`— подписи `A`/`B`, где A = «нет» (проверка на позиционное смещение).

    Возвращает (вопросы, qid -> подпись, означающая «да»).
    """
    try:
        document = yaml.safe_load(rules_path.read_text(encoding="utf-8"))
    except (OSError, yaml.YAMLError) as error:
        raise RuntimeError(f"Не удалось прочитать {rules_path}: {error}") from error

    elements = document.get("elements") if isinstance(document, dict) else None
    if not isinstance(elements, dict):
        raise RuntimeError(f"В {rules_path} отсутствует раздел elements")

    questions: dict[str, dict[str, Any]] = {}
    true_labels: dict[str, str] = {}
    for number in range(1, 10):
        element = elements.get(number)
        if not isinstance(element, dict):
            raise RuntimeError(f"Нет правил для элемента e{number}")

        name = element.get("name", "")

        if "question" in element and "true_criteria" in element:
            ins = element["question"]
            crit_true = element["true_criteria"]
            crit_false = element["false_criteria"]
        else:
            must_not = element.get("must_not_be") or []
            if isinstance(must_not, str):
                must_not = [must_not]
            must_not_str = "; ".join(map(str, must_not)) if must_not else ""
            evidence = element.get("evidence_test", "")
            if must_not_str:
                ins = f"Описан ли в тексте элемент «{name}»? Исключения (НЕ является данным элементом): {must_not_str}."
            else:
                ins = f"Описан ли в тексте элемент «{name}»?"
            crit_true = evidence
            crit_false = "Признак отсутствует или нет прямого подтверждения"

        qid = f"e{number}"
        qtype = element.get("question_type", "choice")
        if qtype == "noul":
            # Штатное решение авторов для ловушки #156 (docs §4.1): смысл остаётся
            # в ключах true/false, но модели показываются нейтральные подписи.
            # Режим both = перестановка этих подписей.
            if labels in ("bool",):
                raise RuntimeError(
                    "noul с булевыми подписями — это задокументированная ловушка (#156); "
                    "используй --labels neutral/neutral_rev/both"
                )
            noul_labels = {"true": "A", "false": "B"} if labels in ("neutral", "both") else {"true": "B", "false": "A"}
            questions[qid] = {
                "type": "noul",
                "instructions": ins,
                "criteria": {"true": crit_true, "false": crit_false},
                "labels": noul_labels,
            }
            true_labels[qid] = "true"
            continue

        if labels == "bool":
            criteria, true_label = {"true": crit_true, "false": crit_false}, "true"
        elif labels == "neutral":
            criteria, true_label = {"A": crit_true, "B": crit_false}, "A"
        elif labels == "neutral_rev":
            criteria, true_label = {"A": crit_false, "B": crit_true}, "B"
        else:
            raise RuntimeError(f"Неизвестный режим подписей: {labels}")

        questions[qid] = {
            "type": element.get("question_type", "choice"),
            "instructions": ins,
            "criteria": criteria,
        }
        true_labels[qid] = true_label
    return questions, true_labels


def download_model(model_dir: Path, model_id: str, model_revision: str) -> None:
    from huggingface_hub import snapshot_download

    required = ("model.safetensors", "rl_agent_config.json", "tokenizer/tokenizer.json")
    if all((model_dir / name).exists() for name in required):
        return
    model_dir.parent.mkdir(parents=True, exist_ok=True)
    snapshot_download(
        repo_id=model_id,
        revision=model_revision,
        local_dir=model_dir,
        allow_patterns=[
            "model.safetensors",
            "rl_agent_config.json",
            "tokenizer/*",
            "encoder/*",
            "README.md",
        ],
    )


OPTION_TOKEN_LIMIT = 48


def report_option_lengths(tok: Any, questions: dict[str, dict[str, Any]]) -> int:
    """Печатает текст вариантов ровно в том виде, в каком его увидит модель.

    Библиотека молча режет каждый вариант до лимита токенов
    (docs/LAYA_MODEL.md §2.1), поэтому обрезанный вариант — это не тот текст,
    который мы написали в критериях. Возвращает число перерезанных вариантов.
    """
    from laya.agent import Agent
    from laya.common import render_options

    cut = 0
    for qid, qdef in questions.items():
        for opt in render_options(Agent._to_internal(qdef)):
            full = tok(opt, add_special_tokens=False)["input_ids"]
            n = len(full)
            if n > OPTION_TOKEN_LIMIT:
                cut += 1
                seen = tok(opt, add_special_tokens=False, truncation=True,
                           max_length=OPTION_TOKEN_LIMIT)["input_ids"]
                print(f"  {qid}: {n:>3} токенов -> модель видит {len(seen)} "
                      f"(обрезано {n - len(seen)})")
                print(f"        ВИДИТ: {tok.decode(seen)}")
                print(f"        ПОТЕРЯНО: {tok.decode(full[len(seen):])}")
            else:
                print(f"  {qid}: {n:>3} токенов (целиком)")
    return cut


def evaluate_case(
    agent: Any,
    case: Case,
    rules_path: Path,
    model_id: str,
    model_revision: str | None,
    gemma_result_path: Path | None,
    output_path: Path,
    labels: str = "bool",
) -> dict[str, Any]:
    expected = case.expected
    priority_false = case.priority_false

    gemma = None
    if gemma_result_path and gemma_result_path.exists():
        gemma = load_json(gemma_result_path).get("signals", {}).get("validator_report")

    orders = ("neutral", "neutral_rev") if labels == "both" else (labels,)
    if not getattr(agent, "_lengths_printed", False):
        probe_questions, _ = build_questions(rules_path, labels=orders[0])
        print(f"\n--- Тексты вариантов как их видит модель (лимит {OPTION_TOKEN_LIMIT} токенов) ---")
        report_option_lengths(agent.tok, probe_questions)
        agent._lengths_printed = True
    started = time.perf_counter()
    thresholds = load_thresholds(rules_path)
    runs: list[dict[str, Any]] = []
    for order in orders:
        questions, true_labels = build_questions(rules_path, labels=order)
        result = agent.predict(case.prompt, questions, max_len=2048, head_max_len=512)
        per_order: dict[str, Any] = {}
        for key in ELEMENTS:
            answer = result["answers"][key]
            true_label = true_labels[key]
            # answer_confidence = max(вероятности) — единственная откалиброванная
            # метрика (docs/LAYA_MODEL.md §4.7). `confidence` у choice — это
            # 1 - энтропия/log(k), с другим масштабом; для маршрутизации не годится.
            confidence = answer.get("answer_confidence")
            if confidence is None:
                confidence = answer.get("confidence", 0.0)
            if "noul" in answer:
                prob = answer["noul"]
                per_order[key] = {"p_true": prob, "ans_conf": confidence, "raw": answer.get("noul")}
            else:
                probs = answer.get("probabilities", {})
                prob = float(probs.get(true_label, 0.0))
                per_order[key] = {"p_true": prob, "ans_conf": confidence, "raw": answer.get("choice")}
        runs.append(per_order)
    inference_ms = round((time.perf_counter() - started) * 1000)

    actual: dict[str, bool] = {}
    report_questions: dict[str, Any] = {}
    for key in ELEMENTS:
        # Усреднение по перестановкам снимает позиционное смещение (docs §4.2):
        # порядок вариантов меняет до 6 из 9 ответов, поэтому один проход не доверяем.
        samples = [run[key]["p_true"] for run in runs]
        prob_true = sum(samples) / len(samples)
        confidence = max(samples + [1.0 - s for s in samples])
        thr = thresholds.get(key, 0.5)
        actual[key] = prob_true >= thr
        probabilities = {"true": prob_true, "false": 1.0 - prob_true}
        verdicts = [run[key]["raw"] for run in runs]
        stable = len(set(str(v) for v in verdicts)) == 1

        report_questions[key] = {
            "actual": actual[key],
            "expected": expected[key],
            "probabilities": probabilities,
            "answer_confidence": confidence,
            "choice": verdicts[0],
            "per_order": [
                {"labels": order, "p_true": run[key]["p_true"], "raw": run[key]["raw"]}
                for order, run in zip(orders, runs)
            ],
            "order_stable": stable,
        }

    matches = [key for key in actual if actual[key] == expected[key]]
    mismatches = [key for key in actual if actual[key] != expected[key]]
    priority_mismatches = sorted(priority_false.intersection(mismatches))

    payload = {
        "model_id": model_id,
        "model_revision": model_revision,
        "device": str(agent.device),
        "case": case.id,
        "rules_file": rules_path.name,
        "labels": labels,
        "inference_ms": inference_ms,
        "expected": expected,
        "gemma_12b": gemma,
        "actual": actual,
        "questions": report_questions,
        "accuracy": len(matches) / len(actual),
        "matches": matches,
        "mismatches": mismatches,
        "priority_false_elements": sorted(priority_false),
        "priority_mismatches": priority_mismatches,
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")

    print(f"\n=================== КЕЙС {case.id} ({rules_path.name}, labels={labels}) ===================")
    print(f"Инференс Laya: {inference_ms} мс | Точность: {len(matches)}/9 ({len(matches)/9*100:.1f}%)")
    print("Элемент  Ожидалось  Laya (prob)      ans_conf  Match")
    for key in ELEMENTS:
        exp_str = str(expected[key]).lower()
        act_str = str(actual[key]).lower()
        prob_true = report_questions[key]["probabilities"]["true"]
        conf = report_questions[key]["answer_confidence"]
        status = "OK " if actual[key] == expected[key] else "ERR"
        print(f"{key:<8} {exp_str:<10} {act_str:<5} (p={prob_true:.3f})   {conf:.3f}     {status}")
    print(f"Совпадения: {', '.join(matches) if matches else 'нет'}")
    print(f"Ошибки: {', '.join(mismatches) if mismatches else 'нет'}")
    if priority_mismatches:
        print(f"Ошибки в приоритетных элементах: {', '.join(priority_mismatches)}")

    return payload


def print_summary(payloads: list[dict[str, Any]], skipped: list[Case]) -> None:
    if not payloads:
        print("\nНи один кейс не прогнан: проверь, что в cases.yaml заполнены prompt и expected")
        return
    total_matches = sum(len(payload["matches"]) for payload in payloads)
    total_elements = 9 * len(payloads)

    print("\n=================== ИТОГ ПО ФАЙЛУ КЕЙСОВ ===================")
    print("Кейс     Точность   Время   Ошибки                        Приоритетные")
    for payload in payloads:
        accuracy = len(payload["matches"])
        errors = ", ".join(payload["mismatches"]) or "-"
        priority = ", ".join(payload["priority_mismatches"]) or "-"
        print(f"{payload['case']:<8} {accuracy}/9       {payload['inference_ms']:>5} мс  "
              f"{errors:<30} {priority}")
    print(f"\nВсего: {total_matches}/{total_elements} "
          f"({total_matches / total_elements * 100:.1f}%) на {len(payloads)} кейсах")

    # Как часто ошибаемся по каждому элементу — видно слабые критерии.
    per_element = {
        key: sum(1 for payload in payloads if key in payload["mismatches"]) for key in ELEMENTS
    }
    worst = sorted(per_element.items(), key=lambda item: -item[1])
    print("Ошибки по элементам: " + ", ".join(f"{key}={count}" for key, count in worst))

    if skipped:
        print("\nПропущено (кейс не заполнен):")
        for case in skipped:
            print(f"  {case.id}: {case.skip}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--cases",
        type=Path,
        default=DEFAULT_CASES,
        help="Единый YAML-файл с кейсами (по умолчанию test_cases/new_tests_for_validator/cases.yaml)",
    )
    parser.add_argument(
        "--case",
        default="",
        help="Прогнать только перечисленные кейсы через запятую (по умолчанию — все)",
    )
    parser.add_argument("--rules", type=Path, default=None, help="Путь к файлу критериев (по умолчанию noul-критерии)")
    parser.add_argument("--use-original-rules", action="store_true", help="Использовать оригинальный element_validation_rules.yaml")
    parser.add_argument("--alfred", action="store_true", help="Использовать alfred361/laya-multilingual-typed-decisions")
    parser.add_argument("--onnx", action="store_true", help="Использовать ONNX модель mizchi/laya-multilingual-onnx")
    parser.add_argument("--model-dir", type=Path, default=None)
    parser.add_argument("--device", default="cuda")
    parser.add_argument(
        "--tokens-only",
        action="store_true",
        help="Печатает только отчёт по длинам вариантов, без инференса и записи JSON",
    )
    parser.add_argument(
        "--labels",
        choices=("bool", "neutral", "neutral_rev", "both"),
        default="both",
        help="Подписи вариантов: bool (true/false — перехватывают ответ, только для контроля), neutral (A=да), neutral_rev (перестановка), both (усреднение по двум перестановкам)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()

    if args.onnx:
        model_id = ONNX_MODEL_ID
        model_revision = None
        model_dir = args.model_dir or (PROJECT_ROOT / "test/laya_probe/models/laya-multilingual-onnx")
        tag = "onnx"
    elif args.alfred:
        model_id = ALFRED_MODEL_ID
        model_revision = ALFRED_MODEL_REVISION
        model_dir = args.model_dir or (PROJECT_ROOT / "test/laya_probe/models/laya-multilingual-alfred")
        tag = "alfred"
    else:
        model_id = DEFAULT_MODEL_ID
        model_revision = DEFAULT_MODEL_REVISION
        model_dir = args.model_dir or (PROJECT_ROOT / "test/laya_probe/models/laya-multilingual-base")
        tag = "base"

    if args.rules:
        rules_path = args.rules
    elif args.use_original_rules:
        rules_path = PROD_RULES
    else:
        rules_path = DEFAULT_RULES

    try:
        cases = load_cases(args.cases)
        wanted = {item.strip() for item in args.case.split(",") if item.strip()}
        if wanted:
            unknown = wanted - {case.id for case in cases}
            if unknown:
                raise RuntimeError(
                    f"В {args.cases.name} нет кейсов: {', '.join(sorted(unknown))}"
                )
            cases = [case for case in cases if case.id in wanted]
        ready = [case for case in cases if case.ready]
        skipped = [case for case in cases if not case.ready]
        for case in skipped:
            print(f"SKIP {case.id}: {case.skip}")
        print(f"Кейсов в файле: {len(cases)} | к прогону: {len(ready)} | пропущено: {len(skipped)}")

        if args.onnx:
            from huggingface_hub import snapshot_download
            if not (model_dir / "model.onnx").exists():
                snapshot_download(repo_id=model_id, local_dir=model_dir)
        else:
            download_model(model_dir, model_id, model_revision)

        if args.tokens_only:
            from transformers import AutoTokenizer
            tok_dir = model_dir / "tokenizer" if (model_dir / "tokenizer").exists() else model_dir
            tok = AutoTokenizer.from_pretrained(tok_dir)
            orders = ("neutral", "neutral_rev") if args.labels == "both" else (args.labels,)
            probe_questions, _ = build_questions(rules_path, labels=orders[0])
            print(f"\n--- Тексты вариантов как их видит модель (лимит {OPTION_TOKEN_LIMIT} токенов) ---")
            report_option_lengths(tok, probe_questions)
            return 0

        started = time.perf_counter()
        if args.onnx:
            import laya.onnx_agent
            onnx_file = model_dir / "model.onnx"
            agent = laya.onnx_agent.ONNXAgent(str(model_dir), onnx_path=str(onnx_file))
            agent.device = "onnx"
        else:
            import laya
            agent = laya.load(str(model_dir), device=args.device)
        load_ms = round((time.perf_counter() - started) * 1000)
        print(f"Модель {model_id} загружена за {load_ms} мс (устройство: {agent.device})")

        # slug из имени файла: element_validation_rules[_variant].yaml
        # Раньше тег определялся по подстроке "test" в имени, из-за чего файлы
        # вроде element_validation_rules_noul.yaml помечались как "orig" и
        # перезаписывали результаты друг друга.
        rules_tag = rules_path.stem.replace("element_validation_rules", "").strip("_-") or "prod"
        # Набор кейсов — часть тега: два разных файла кейсов (например, сплит-тест
        # соматики против основного набора) писали в одни и те же файлы и молча
        # затирали друг друга. Стандартный набор помечается пустым суффиксом,
        # чтобы его старые имена файлов оставались валидными.
        cases_stem = args.cases.stem
        cases_tag = "" if cases_stem == DEFAULT_CASES.stem else f"_{cases_stem}"
        out_tag = f"{tag}_{rules_tag}_{args.labels}{cases_tag}"

        results_dir = PROJECT_ROOT / "test/laya_probe/results"
        payloads: list[dict[str, Any]] = []
        for case in ready:
            out_file = results_dir / f"laya_{out_tag}_{case.id}.json"
            gemma_file = results_dir / f"gemma_{case.id}.json"
            payloads.append(
                evaluate_case(
                    agent,
                    case,
                    rules_path,
                    model_id,
                    model_revision,
                    gemma_file if gemma_file.exists() else None,
                    out_file,
                    labels=args.labels,
                )
            )

        print_summary(payloads, skipped)
        if payloads:
            summary = {
                "model_id": model_id,
                "model_revision": model_revision,
                "device": str(agent.device),
                "cases_file": args.cases.relative_to(PROJECT_ROOT).as_posix(),
                "rules_file": rules_path.name,
                "labels": args.labels,
                "cases_total": len(cases),
                "cases_run": len(payloads),
                "cases_skipped": [case.id for case in skipped],
                "total_matches": sum(len(payload["matches"]) for payload in payloads),
                "total_elements": 9 * len(payloads),
                "errors_per_element": {
                    key: sum(1 for payload in payloads if key in payload["mismatches"])
                    for key in ELEMENTS
                },
                "results": [
                    {
                        "case": payload["case"],
                        "accuracy": payload["accuracy"],
                        "matches": payload["matches"],
                        "mismatches": payload["mismatches"],
                        "priority_mismatches": payload["priority_mismatches"],
                        "inference_ms": payload["inference_ms"],
                    }
                    for payload in payloads
                ],
            }
            summary_path = results_dir / f"summary_{out_tag}.json"
            summary_path.write_text(
                json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8"
            )
            print(f"Сводка: {summary_path.relative_to(PROJECT_ROOT)}")
        return 0
    except Exception as error:
        print(f"ОШИБКА: {error}", file=sys.stderr)
        import traceback
        traceback.print_exc()
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
