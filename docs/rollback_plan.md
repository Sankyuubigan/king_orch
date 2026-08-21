# План: откат к предыдущим версиям (rollback)

> Статус: реализован (см. код). План и риски сохранены для дальнейшего рефакторинга.

## Проблема
Текущий апдейтер (`tauri-plugin-updater`) умеет только «есть ли версия новее → скачать
`latest.json`». Откат к старой версии невозможен: нет ни списка релизов, ни способа
указать конкретную версию для установки.

## Решение (Вариант A — manifest-ассет)
1. `release.cjs` при каждом релизе дополнительно загружает `manifest.json` (формат,
   идентичный `latest.json`) ассетом в GitHub-релиз. URL предсказуем:
   `https://github.com/Sankyuubigan/king_orch/releases/download/v{version}/manifest.json`.
2. Backend (Rust): `get_release_history()` — список релизов через GitHub Releases REST API;
   `install_release(version)` — строит endpoint на `manifest.json` выбранной версии,
   `updater_builder().endpoints(...).version_comparator(|_, _| true)` → штатный пайплайн
   плагина (скачивание, проверка подписи, NSIS-установка, перезапуск).
3. Авто-бэкап `app_config.json` + `sessions/` в `rollback_backup/` перед откатом.
4. Frontend: секция «История версий» в настройках.
5. `tauri.conf.json`: `bundle.windows.allowDowngrades: true`.

## Почему это безопасно (проверено)
- Плагин умеет ставить любую версию через `version_comparator(|_, _| true)` (подтверждено
  исходниками `tauri-plugin-updater 2.10.1`, lib.rs/updater.rs).
- Все релизы подписаны одним ключом → проверка подписи пройдёт для любой старой версии.
- Установка/перезапуск идентичны текущему обновлению (`update_popup.ts`), включая
  `cleanup_before_exit` (корректное завершение дочерних процессов llama-server).
- Список версий грузится через GitHub API (1 запрос при открытии настроек); установка
  не бьёт в API вообще (манифест уже содержит всё нужное).

## Риски (требуют эмпирической проверки тестовым релизом)
1. **NSIS-установщик при даунгрейде в пассивном режиме** — читался шаблон `dev`-ветки Tauri
   (update-режим `/UPDATE` идёт в `reinst_done`, блокировка только при `allowDowngrades=false`
   + silent). Но проект собирается CLI 2.0.0, и именно его шаблон стоит на старых
   установщиках; шаблон вшит в бинарник (не читается). Проверяется только тестовым
   релизом N → N-1. Fallback: NSIS-hook (`installer_hooks`) или Вариант B (ручная установка).
2. **Нормализация имён ассетов** в `manifest.json` (правило 3.3: `King Orch` → `King.Orch`).
   Митигация: переиспользуется уже работающая логика генерации `latest.json`.
3. **Старые релизы без `manifest.json`** — откат на версии, выпущенные ДО внедрения, не
   сработает (нет ассета). Откат «на одну назад» заработает сразу после первого релиза
   с новой механикой.
4. **Совместимость схемы данных** — старая версия может не прочитать конфиг/сессии новее.
   Авто-бэкап сохраняет, но НЕ восстанавливает автоматически (ручной возврат файлов).
5. Мелочи: обрезка истории при >100 релизов (пагинация per_page=100); движок llama.cpp
   при откате остаётся новый (не ломает, но может предлагать «обновить движок»).

## Порядок реализации
1. `release.cjs` — генерация + `gh release upload manifest.json` (после create, до commit latest.json).
2. `src-tauri/src/infra/updater_rollback.rs` — `backup_before_rollback`.
3. `src-tauri/src/api/updater.rs` — `get_release_history`, `install_release`.
4. Регистрация: `infra/mod.rs`, `api/mod.rs`, `main.rs` (invoke_handler), `Cargo.toml` (`url`).
5. `tauri.conf.json` — `bundle.windows.allowDowngrades: true`.
6. Frontend: `index.html` (блок «История версий»), `settings.ts` (логика), `main.ts` (элементы).
7. `build.bat` — проверка компиляции.

> ЗАПРЕЩЕНО коммитить/пушить/релизить без явного разрешения пользователя.
