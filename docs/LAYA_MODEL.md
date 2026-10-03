# Laya (System-1) — где документация

Справочник по модели **перенесён в плагин**, вместе с кодом, который её выполняет:

```
D:\Projects\my-tauri-plugins\tauri-plugin-system1\docs\LAYA_MODEL.md
```

Причина переноса: инференс, рантайм ONNX и доставка модели живут в
`tauri-plugin-system1`, и документация обязана лежать рядом с кодом. Иначе второй
хост, взявший плагин, получил бы модель без описания её грабли
(собственный документ, `tauri-plugin-system1/docs/README.md`, остаётся).

## Что осталось важного для этого хоста

| | |
|---|---|
| Боевые критерии System-1 | `agents/psychotherapist/database/element_validation_rules_system1.yaml` |
| Критерии System-2 (LLM-валидатор) | `agents/psychotherapist/database/element_validation_rules.yaml` |
| Узел графа | `system1_validator`, файл правил задаётся в YAML: `rules_file:` |
| Адаптер вызова плагина | `src-tauri/src/infra/system1.rs` |
| Универсальный парсер правил | `src-tauri/src/domain/workflow_engine/system1_rules.rs` |
| Панель в настройках | `<system1-panel>`, см. `index.html` → `template-settings-lazy-panels` |
| Бенчмарк | `python tools\laya_probe.py --onnx --labels both` → **189/234 (80.8%)** |
| Эталонные кейсы | `test_cases/new_tests_for_validator/cases.yaml` |
| История экспериментов | `tasks/02.10.26 Laya Фаза 3 — пробитие потолка архитектурой.md` |

## Граница формата файла критериев

`tools/laya_probe.py` читает `elements` как отображение с **числовыми** ключами
`1..9` и сам строит идентификаторы `e1..e9`. Поэтому ключи `1..9` в боевом YAML
менять нельзя — это сломает базу 189/234.

Идентификаторы при этом объявляются в файле полем `id: "e1"`, а не выводятся из
позиции: `src-tauri/src/domain/workflow_engine/system1_rules.rs` требует `id` для
числовых ключей и падает с внятной ошибкой, если его нет. Так набор и имена
правил остаются данными графа, а не константой движка. Строковые ключи
(`elements: {soma: …}`) и список с явными `id` тоже поддерживаются.

Правка критериев без прогона стенда = правка вслепую (зафиксировано в
`docs/LAYA_MODEL.md` плагина, §8 п.7).