# Laya (System-1) — где документация

Справочник по модели **лежит в плагине**, вместе с кодом, который его выполняет:

```
D:\Projects\cross-app-plugins\tauri-plugin-system1\docs\LAYA_MODEL.md
```

Причина: инференс, рантайм ONNX и доставка модели живут в `tauri-plugin-system1`,
и документация обязана лежать рядом с кодом — иначе второй хост, взявший плагин,
получил бы модель без описания её грабель (там же, `docs/README.md` плагина).

История замеров 08.10.26 и 10.10.26 (System-2 вместо Лайи, воронка 9 элементов,
иерархическая воронка, гейты G0–G5) — тоже в справочнике плагина, §24–§27.

## Статус интеграции в этом хосте (king_orch_3)

**Плагин System-1 отключён 17.10.2026** (коммит `5bc87e9`): из хоста удалены
адаптер `src-tauri/src/infra/system1.rs`, парсер правил
`src-tauri/src/domain/workflow_engine/system1_rules.rs`, тип узла
`system1_validator` (nodes.rs), панель настроек и зависимости
(Cargo.toml/package.json). В боевых графах ноды `validate_system1` нет.

## Что осталось в этом репозитории

| | |
|---|---|
| Критерии System-1 (для стенда) | `agents/psychotherapist/experiments/element_validation_rules_system1.yaml` |
| Критерии System-2 (LLM-валидатор) | `agents/psychotherapist/database/element_validation_rules.yaml` |
| Экспериментальные валидаторы | `agents/psychotherapist/experiments/` (`validator_surface/facade/core/flat.md`, `validator_layers.yaml`, `fact_consolidator.md`) |
| Бенчмарк | `python tools\laya_probe.py --onnx --labels both` → **189/234 (80.8%)** |
| Эталонные кейсы | `test_cases/new_tests_for_validator/cases.yaml` |
| История экспериментов | `tasks/02.10.26 Laya Фаза 3 — пробитие потолка архитектурой.md` и соседние задачи 24.09–11.10 |
