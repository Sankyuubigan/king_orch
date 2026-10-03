# Фикстуры стенда System-1 (Laya)

## Что здесь лежит

| Файл | Роль | Куда едет |
|---|---|---|
| `cases.yaml` | 26 кейсов с эталонными `expected` по 9 элементам | стенд `tools/laya_probe.py` |

Больше в этом каталоге боевых файлов нет.

## Где лежат критерии (SSOT)

Критерии System-1 **не** здесь — они перенесены в `agents/`, потому что только
`agents/` попадает в инсталлер (`tauri.conf.json` → `bundle.resources`), и нода
`System1Validator` читает их от каталога агентов:

| Модель | Файл критериев | Формат |
|---|---|---|
| **System-1 (Laya)** | `agents/psychotherapist/database/element_validation_rules_system1.yaml` | `question_type: noul`, `true_criteria` / `false_criteria`, `threshold` |
| System-2 (LLM-валидатор 12B) | `agents/psychotherapist/database/element_validation_rules.yaml` | `formula`, `must_have`, `must_not_be`, `evidence_test`, `anti_patterns` |

Это **не** дубль и не конфликт: два разных контракта входа для двух разных моделей.

Исторически файл System-1 назывался
`test_cases/new_tests_for_validator/element_validation_rules_prod_noul.yaml` и был
перенесён в `agents/` без изменения содержимого. Имя `prod_noul` встречается в
справочнике модели и в именах файлов результатов
(`test/laya_probe/results/laya_onnx_prod_noul_both_task*.json`) как историческая
метка прогона — это не путь к файлу критериев.

**Базовая цифра** на текущей паре (критерии из `agents/`, кейсы из `cases.yaml`):
**189/234 (80.8%), FP 34, FN 11**
(`test/laya_probe/results/summary_onnx_system1_both.json`).

## Где справочник по модели

`docs/LAYA_MODEL.md` — это **указатель**. Сам справочник по модели (грабли,
формат вопросов, перестановки, история замеров) перенесён в плагин:
`D:\Projects\my-tauri-plugins\tauri-plugin-system1\docs\LAYA_MODEL.md`.

## Экспериментальные YAML уехали в `test/laya_probe/experiments/`

Критерии отклонённых гипотез (score-рубрики v1/v2/v3, `e3_nocr`, `variants`,
`cascade_rules`, `e3_pole_experiment`) и 7 одноразовых сборщиков перенесены в
`test/laya_probe/experiments/`. Разбор каждого — в справочнике плагина
(§14, §15, §15.7, §17, §21).

⚠️ **`test/` в `.gitignore`** — эти файлы **не версионируются**. Отрицательные
результаты, на которые ссылается документация, воспроизводимы только на этой
машине. Скрипты и YAML, нужные для повторного замера, берутся из
`tasks/02.10.26 Laya Фаза 3 — пробитие потолка архитектурой.md` (Фаза 12
описывает стенд 4 перестановок) и справочника плагина.

## Правило

Новый файл критериев **не добавляется** в `agents/psychotherapist/database/`,
пока не пройден гейт и не записан вердикт в справочнике плагина. Иначе набор
критериев перестаёт быть единственным источником правды, а отклонённые варианты
начинают выглядеть как действующие конфигурации.