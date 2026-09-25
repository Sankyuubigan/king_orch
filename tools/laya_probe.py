from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from typing import Any

import yaml

DEFAULT_MODEL_ID = "convaiinnovations/laya-multilingual"
DEFAULT_MODEL_REVISION = "e4e9ddf21a7b1903b7acffd8814ad4307bf63a67"
ALFRED_MODEL_ID = "alfred361/laya-multilingual-typed-decisions"
ALFRED_MODEL_REVISION = "60ce5be491a7723df937b757e43b254a08898f8e"


def load_json(path: Path) -> dict[str, Any]:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError(f"Не удалось прочитать {path}: {error}") from error


def load_definitions(arch_path: Path) -> dict[int, str]:
    try:
        text = arch_path.read_text(encoding="utf-8")
    except OSError as error:
        raise RuntimeError(f"Не удалось прочитать {arch_path}: {error}") from error
    definitions: dict[int, str] = {}
    for line in text.splitlines():
        line = line.strip()
        for number in range(1, 10):
            if line.startswith(f"{number}."):
                definitions[number] = line[len(str(number)) + 1:].strip()
    if len(definitions) < 9:
        raise RuntimeError(f"В {arch_path} найдены не все 9 определений элементов")
    return definitions


def build_questions(rules_path: Path, arch_path: Path) -> dict[str, dict[str, Any]]:
    try:
        document = yaml.safe_load(rules_path.read_text(encoding="utf-8"))
    except (OSError, yaml.YAMLError) as error:
        raise RuntimeError(f"Не удалось прочитать {rules_path}: {error}") from error

    elements = document.get("elements") if isinstance(document, dict) else None
    if not isinstance(elements, dict):
        raise RuntimeError(f"В {rules_path} отсутствует раздел elements")
    definitions = load_definitions(arch_path)

    questions: dict[str, dict[str, Any]] = {}
    for number in range(1, 10):
        element = elements.get(number)
        if not isinstance(element, dict):
            raise RuntimeError(f"Нет правил для элемента e{number}")
        must_not = element.get("must_not_be") or []
        if isinstance(must_not, str):
            must_not = [must_not]
        
        parts = [
            f"Элемент {number} ({element.get('name', '')}): {element.get('evidence_test', '')}",
        ]
        must_have = element.get("must_have")
        if must_have:
            parts.append(f"Обязательно: {must_have}")
        if number == 3:
            formula = element.get("formula")
            if formula:
                parts.append(str(formula))
        if must_not:
            parts.append("Запрещено для true: " + "; ".join(map(str, must_not)) + ".")
        parts.append("Доказательство — только прямые слова пользователя. Если сомневаешься или данных нет — ответ false.")

        questions[f"e{number}"] = {
            "type": "noul",
            "instructions": " ".join(parts),
            "criteria": {
                "true": "Пользователь прямо и недвусмысленно подтвердил это своими словами.",
                "false": "В словах пользователя нет прямого подтверждения, есть запрет или это фоновый дискомфорт.",
            },
        }
    return questions


def check_budgets(agent: Any, questions: dict[str, dict[str, Any]], state: str, max_len: int, head_max_len: int) -> None:
    from laya.agent import Agent
    from laya.common import build_sequence

    problems = []
    for qid, qdef in questions.items():
        internal = Agent._to_internal(qdef)
        ids, markers = build_sequence(agent.tok, state, internal, max_len, head_max_len)
        full = agent.tok("choice question: " + internal["ins"], add_special_tokens=False)["input_ids"]
        kept = len(ids[1:markers[0] - 1])
        dropped = len(full) - kept
        print(f"{qid}: инструкция {len(full)} токенов, влезло {kept}, отрезано {dropped}")
        if dropped > 0:
            problems.append(qid)
    if problems:
        raise RuntimeError(f"Инструкции обрезаны токенизатором: {', '.join(problems)}. Укороти вопросы.")


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


def evaluate(
    fixture_dir: Path,
    rules_path: Path,
    arch_path: Path,
    model_dir: Path,
    model_id: str,
    model_revision: str,
    gemma_result_path: Path,
    device: str,
    output_path: Path,
) -> int:
    import laya

    validation = load_json(fixture_dir / "validation.json")
    try:
        state = (fixture_dir / "task.md").read_text(encoding="utf-8").strip()
    except OSError as error:
        raise RuntimeError(f"Не удалось прочитать вход из {fixture_dir / 'task.md'}: {error}") from error
    expected = validation["expected_signals"]["validator_report"]
    gemma = None
    if gemma_result_path.exists():
        gemma = load_json(gemma_result_path).get("signals", {}).get("validator_report")
    priority_false = set(validation["priority_false_elements"])
    questions = build_questions(rules_path, arch_path)

    started = time.perf_counter()
    agent = laya.load(str(model_dir), device=device)
    actual_device = str(agent.device)
    load_ms = round((time.perf_counter() - started) * 1000)

    try:
        check_budgets(agent, questions, state, max_len=2048, head_max_len=512)
    except Exception as e:
        print(f"ВНИМАНИЕ: {e}")

    started = time.perf_counter()
    result = agent.predict(state, questions, max_len=2048, head_max_len=512)
    inference_ms = round((time.perf_counter() - started) * 1000)

    actual: dict[str, bool] = {}
    report_questions: dict[str, Any] = {}
    for number in range(1, 10):
        key = f"e{number}"
        answer = result["answers"][key]
        if "noul" in answer:
            prob = answer["noul"]
            actual[key] = prob >= 0.5
            confidence = answer.get("confidence", abs(prob - 0.5) * 2)
            probabilities = {"true": prob, "false": 1.0 - prob}
        else:
            actual[key] = answer.get("choice") == "true"
            confidence = answer.get("confidence", 0.0)
            probabilities = answer.get("probabilities", {})

        report_questions[key] = {
            "actual": actual[key],
            "expected": expected[key],
            "probabilities": probabilities,
            "confidence": confidence,
        }

    matches = [key for key in actual if actual[key] == expected[key]]
    mismatches = [key for key in actual if actual[key] != expected[key]]
    priority_mismatches = sorted(priority_false.intersection(mismatches))
    gemma_mismatches = None
    if gemma is not None:
        gemma_mismatches = [key for key in actual if gemma.get(key) != expected[key]]
    payload = {
        "model_id": model_id,
        "model_revision": model_revision,
        "device": actual_device,
        "fixture": fixture_dir.name,
        "load_ms": load_ms,
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
        "gemma_mismatches": gemma_mismatches,
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")

    print(f"Модель: {model_id}@{model_revision}")
    print(f"Устройство: {actual_device}; загрузка: {load_ms} мс; проверка: {inference_ms} мс")
    print("элемент  ожидалось  Laya   Gemma-12B")
    for number in range(1, 10):
        key = f"e{number}"
        gemma_value = "—" if gemma is None else str(gemma.get(key)).lower()
        print(
            f"{key:<7} {str(expected[key]).lower():<9} "
            f"{str(actual[key]).lower():<6} {gemma_value}"
        )
    print(f"Совпало с эталоном: {len(matches)}/9")
    print(f"Ошибки Laya: {', '.join(mismatches) if mismatches else 'нет'}")
    if gemma_mismatches is None:
        print("Ошибки Gemma-12B: ещё нет результата нового прогона")
    else:
        print(f"Ошибки Gemma-12B: {', '.join(gemma_mismatches) if gemma_mismatches else 'нет'}")
    print(f"Приоритетные ошибки Laya e3/e6: {', '.join(priority_mismatches) if priority_mismatches else 'нет'}")
    print(f"Отчёт: {output_path}")
    return 1 if priority_mismatches else 0


def parse_args() -> argparse.Namespace:
    project_root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture-dir", type=Path, default=project_root / "test_cases/fixtures/psychotherapist_validator_e1_e9")
    parser.add_argument("--rules", type=Path, default=project_root / "agents/psychotherapist/database/element_validation_rules.yaml")
    parser.add_argument("--arch", type=Path, default=project_root / "agents/psychotherapist/database/neurosis_architecture.md")
    parser.add_argument("--alfred", action="store_true", help="Использовать alfred361/laya-multilingual-typed-decisions вместо базовой Laya")
    parser.add_argument("--model-dir", type=Path, default=None)
    parser.add_argument("--gemma-result", type=Path, default=project_root / "test/laya_probe/results/gemma_e1_e9.json")
    parser.add_argument("--output", type=Path, default=None)
    parser.add_argument("--device", default="cuda")
    parser.add_argument("--download-only", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    project_root = Path(__file__).resolve().parent.parent
    if args.alfred:
        model_id = ALFRED_MODEL_ID
        model_revision = ALFRED_MODEL_REVISION
        model_dir = args.model_dir or (project_root / "test/laya_probe/models/laya-multilingual-alfred")
        output_path = args.output or (project_root / "test/laya_probe/results/laya_alfred_e1_e9.json")
    else:
        model_id = DEFAULT_MODEL_ID
        model_revision = DEFAULT_MODEL_REVISION
        model_dir = args.model_dir or (project_root / "test/laya_probe/models/laya-multilingual-base")
        output_path = args.output or (project_root / "test/laya_probe/results/laya_base_e1_e9.json")

    try:
        download_model(model_dir, model_id, model_revision)
        if args.download_only:
            print(f"Модель скачана: {model_dir}")
            return 0
        return evaluate(
            args.fixture_dir,
            args.rules,
            args.arch,
            model_dir,
            model_id,
            model_revision,
            args.gemma_result,
            args.device,
            output_path,
        )
    except Exception as error:
        print(f"ОШИБКА: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
