"""Сплит-тест соматики: та же совокупность кейсов с вырезанными телесными фразами.

Зачем: гипотеза «соматика зашумляет System-1 и внимание модели тратится впустую»
должна быть проверена замером, а не интуицией. Сравниваем один и тот же файл
критериев на двух входах; разница в счёте и есть эффект.

Как устроено:
- режутся ТОЛЬКО предложения с телесным перечислением (а не пересказываются) —
  протокол «только хирургическое вырезание», docs/LAYA_MODEL.md, дополнение к §11.4;
- эталонные метки `expected` копируются как есть и проверяются на идентичность:
  если после правки текста доказательная база элемента исчезает, метка становится
  недостоверной, и такой кейс в тест не годится;
- если фраза для вырезания не найдена — падаем с ошибкой, а не молча режем пустоту.

Запуск из корня проекта:
    python tools\\somatic_ablation.py            # создать test\\laya_probe\\cases_no_somatic.yaml
    python tools\\laya_probe.py --onnx --labels both --cases test\\laya_probe\\cases_no_somatic.yaml
"""
from __future__ import annotations

import sys
from pathlib import Path

import yaml

PROJECT_ROOT = Path(__file__).resolve().parent.parent
SOURCE = PROJECT_ROOT / "test_cases/new_tests_for_validator/cases.yaml"
TARGET = PROJECT_ROOT / "test/laya_probe/cases_no_somatic.yaml"

# Предложения, которые убираем. Ключ — id кейса, значение — точный текст
# предложения: при несовпадении падаем, чтобы кейс не «вырезался молча».
CUTS: dict[str, tuple[str, ...]] = {
    # жалоба на тонус и ноги, без психологической функции
    "task3": (
        "У меня совсем нет сил, с ног валюсь.",
        "Постоянно хочется прилечь отдохнуть.",
    ),
    # энергия, сонливость, механика сна; остаётся грусть и страх капитуляции
    "task6": (
        "Совсем нет сил.",
        "Спал достаточно, но после пробуждения состояние полного отсутствия сил.",
        "Постоянно хочется прилечь на кровать, но уснуть не могу, так как спать не хочу, хотя в голове тупая сонливость.",
    ),
    # симптом в перечне и механика сна; зависимость и условие остаются
    "task8": (
        "бессонница",
        "сон критически рушится",
    ),
    # энергия, бессонница, отключки; остаётся опора на бездействие и страх
    "task10": (
        "По утрам нет сил встать, ночью мучает бессонница, а днем вырубает в спонтанные отключки, падаю без сил.",
    ),
}


def apply_cuts(case_id: str, prompt: str) -> str:
    result = prompt
    for fragment in CUTS.get(case_id, ()):
        if fragment not in result:
            raise RuntimeError(
                f"{case_id}: фраза для вырезания не найдена, кейс бы остался неизменным: "
                f"{fragment!r}"
            )
        result = result.replace(fragment, "", 1)
    return "\n".join(line for line in result.splitlines() if line.strip())


def main() -> int:
    document = yaml.safe_load(SOURCE.read_text(encoding="utf-8"))
    cases = document["cases"]

    for case in cases:
        case_id = str(case["id"])
        case["prompt"] = apply_cuts(case_id, str(case["prompt"]))
        case["note"] = f"{case['note']} [вариант без соматики]"

    # Эталоны обязаны совпасть с исходником: иначе мы измеряем не соматику,
    # а смену разметки.
    original = {str(c["id"]): c["expected"] for c in yaml.safe_load(SOURCE.read_text(encoding="utf-8"))["cases"]}
    changed = [
        str(c["id"]) for c in cases if c["expected"] != original[str(c["id"])]
    ]
    if changed:
        raise RuntimeError(f"Эталоны разошлись с исходником в кейсах: {', '.join(changed)}")

    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(
        yaml.safe_dump(document, allow_unicode=True, sort_keys=False, width=100),
        encoding="utf-8",
    )

    removed = sum(len(v) for v in CUTS.values())
    print(f"Записано: {TARGET.relative_to(PROJECT_ROOT)}")
    print(f"Вырезано фрагментов: {removed} в кейсах: {', '.join(sorted(CUTS))}")
    print("Эталоны проверены: совпадают с cases.yaml")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())