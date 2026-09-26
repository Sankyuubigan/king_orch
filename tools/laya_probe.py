from __future__ import annotations

import argparse
import json
import sys
import time
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


def load_json(path: Path) -> dict[str, Any]:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError(f"Не удалось прочитать {path}: {error}") from error


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


def evaluate_task(
    agent: Any,
    task_num: int,
    fixture_dir: Path,
    rules_path: Path,
    model_id: str,
    model_revision: str,
    gemma_result_path: Path | None,
    output_path: Path,
    labels: str = "bool",
) -> dict[str, Any]:
    task_file = fixture_dir / f"task{task_num}.md"
    val_file = fixture_dir / f"validation{task_num}.json"
    
    validation = load_json(val_file)
    state = task_file.read_text(encoding="utf-8").strip()
    expected = validation["expected_signals"]["validator_report"]
    priority_false = set(validation.get("priority_false_elements", ["e3", "e6"]))
    
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
    runs: list[dict[str, Any]] = []
    for order in orders:
        questions, true_labels = build_questions(rules_path, labels=order)
        result = agent.predict(state, questions, max_len=2048, head_max_len=512)
        per_order: dict[str, Any] = {}
        for number in range(1, 10):
            key = f"e{number}"
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
    for number in range(1, 10):
        key = f"e{number}"
        # Усреднение по перестановкам снимает позиционное смещение (docs §4.2):
        # порядок вариантов меняет до 6 из 9 ответов, поэтому один проход не доверяем.
        samples = [run[key]["p_true"] for run in runs]
        prob_true = sum(samples) / len(samples)
        confidence = max(samples + [1.0 - s for s in samples])
        actual[key] = prob_true >= 0.5
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
        "task": f"task{task_num}",
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

    print(f"\n=================== ТЕСТ task{task_num} ({rules_path.name}, labels={labels}) ===================")
    print(f"Инференс Laya: {inference_ms} мс | Точность: {len(matches)}/9 ({len(matches)/9*100:.1f}%)")
    print("Элемент  Ожидалось  Laya (prob)      ans_conf  Match")
    for number in range(1, 10):
        key = f"e{number}"
        exp_str = str(expected[key]).lower()
        act_str = str(actual[key]).lower()
        prob_true = report_questions[key]["probabilities"]["true"]
        conf = report_questions[key]["answer_confidence"]
        status = "OK " if actual[key] == expected[key] else "ERR"
        print(f"{key:<8} {exp_str:<10} {act_str:<5} (p={prob_true:.3f})   {conf:.3f}     {status}")
    print(f"Совпадения: {', '.join(matches) if matches else 'нет'}")
    print(f"Ошибки: {', '.join(mismatches) if mismatches else 'нет'}")

    return payload


def parse_args() -> argparse.Namespace:
    project_root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture-dir", type=Path, default=project_root / "test_cases/fixtures/psychotherapist_validator_e1_e9")
    parser.add_argument("--rules", type=Path, default=None, help="Путь к файлу критериев (по умолчанию test или orig)")
    parser.add_argument("--use-original-rules", action="store_true", help="Использовать оригинальный element_validation_rules.yaml")
    parser.add_argument("--alfred", action="store_true", help="Использовать alfred361/laya-multilingual-typed-decisions")
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
    project_root = Path(__file__).resolve().parent.parent
    
    if args.alfred:
        model_id = ALFRED_MODEL_ID
        model_revision = ALFRED_MODEL_REVISION
        model_dir = args.model_dir or (project_root / "test/laya_probe/models/laya-multilingual-alfred")
        tag = "alfred"
    else:
        model_id = DEFAULT_MODEL_ID
        model_revision = DEFAULT_MODEL_REVISION
        model_dir = args.model_dir or (project_root / "test/laya_probe/models/laya-multilingual-base")
        tag = "base"

    if args.rules:
        rules_path = args.rules
    elif args.use_original_rules:
        rules_path = project_root / "agents/psychotherapist/database/element_validation_rules.yaml"
    else:
        rules_path = args.fixture_dir / "element_validation_rules_test.yaml"

    try:
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

        import laya
        
        started = time.perf_counter()
        agent = laya.load(str(model_dir), device=args.device)
        load_ms = round((time.perf_counter() - started) * 1000)
        print(f"Модель {model_id} загружена за {load_ms} мс (устройство: {agent.device})")

        # slug из имени файла: element_validation_rules[_variant].yaml
        # Раньше тег определялся по подстроке "test" в имени, из-за чего файлы
        # вроде element_validation_rules_noul.yaml помечались как "orig" и
        # перезаписывали результаты друг друга.
        rules_tag = rules_path.stem.replace("element_validation_rules", "").strip("_-") or "prod"
        out_tag = f"{tag}_{rules_tag}_{args.labels}"

        for task_num in (1, 2):
            out_file = project_root / f"test/laya_probe/results/laya_{out_tag}_task{task_num}.json"
            gemma_file = project_root / f"test/laya_probe/results/gemma_task{task_num}.json"
            evaluate_task(
                agent,
                task_num,
                args.fixture_dir,
                rules_path,
                model_id,
                model_revision,
                gemma_file if gemma_file.exists() else None,
                out_file,
                labels=args.labels,
            )
        return 0
    except Exception as error:
        print(f"ОШИБКА: {error}", file=sys.stderr)
        import traceback
        traceback.print_exc()
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
