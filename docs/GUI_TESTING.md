# 🖥️ GUI-тестирование King Orch внешним ИИ-агентом

> **Как это работает:** UI King Orch — веб-страница внутри WebView2 (Chromium). При dev-запуске
> через `build.bat` приложение открывает CDP-порт `9222`, и внешний агент (opencode и т.п.)
> может читать DOM, кликать, вводить текст и делать скриншоты — без «зрения».
> Полный гайд для Tauri-проектов: `global_ai_docs/desktop_rust_tauri/gui_testing.md`.

## Запуск

1. Запусти приложение: `build.bat`. Порт включает **конфиг-оверрайд**: `app.windows[].additionalBrowserArgs: "--remote-debugging-port=9222 --remote-allow-origins=*"` (берётся из `.build-config.json`/`tauri-dev-override.json`, логика в `tauri-build-toolkit`).
   ⚠️ env-переменная `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` НЕ работает (wry игнорирует её — задаёт свои аргументы).
2. Проверь, что порт жив: `Invoke-RestMethod http://127.0.0.1:9222/json/version`.
3. ⚠️ Агенту: не запускать `cmd /c build.bat` синхронно (виснет, пока открыто приложение) —
   через `Start-Process` с редиректом вывода в `test/build_out.txt` (правило в `desktop_rust_tauri/rules.md` §9).

## Подключение агента

```powershell
npx agent-browser --cdp 9222 snapshot -i   # интерактивные элементы с ref'ами (@e1, @e2...)
npx agent-browser --cdp 9222 click @e3
npx agent-browser --cdp 9222 fill @e2 "текст"
npx agent-browser --cdp 9222 select @e1 "опция"
npx agent-browser --cdp 9222 press Enter
npx agent-browser --cdp 9222 wait 1000
npx agent-browser --cdp 9222 get text body
npx agent-browser --cdp 9222 screenshot app.png
```

Фолбэк (если CLI не распознаёт WebView2): скрипт на puppeteer-core (уже есть в проекте как
npm-зависимость MCP-сервера `browser`) — `puppeteer.connect({ browserURL: "http://127.0.0.1:9222" })`,
`page.click("#btn-send")` и т.д.

## Основные селекторы

> Таблица сверена с кодом 13.10.26 (`index.html`, `src/controllers/tabs.ts`,
> `src/utils/chat-dom.ts`, `src/controllers/studio-page.ts`). Прежняя версия описывала
> ids, которых в проекте больше нет (`#tab-chat`, `#subtab-graph`, `#chat-input`,
> `#log-view`, `#btn-check-update`).

### Каркас рабочей области

| Что | Селектор |
|-----|----------|
| Полоса вкладок | `#workspace-tabs`, кнопки разделов `.workspace-tab-menu-toggle` (в `#workspace-tab-actions`) |
| Кнопка вкладки | `.workspace-tab[data-tab-id="<id вкладки>"]`, подпись `.workspace-tab-label`, закрыть `.workspace-tab-close` |
| Кнопка «＋» новая вкладка | `.workspace-tab-add` |
| Контейнер страниц | `#workspace-content`; страницы чата/веба — внутри `#chat-tab-slot` (`.tab-page`) |
| Статичные разделы | `#view-sessions`, `#view-settings`, `#view-logs`, `#view-engines` |

**Видимость страницы** — инвариант: в `#workspace-content` активна ровно одна
`.view.active` (`TabController.activate` сам пишет в лог, если их оказалось больше).

### Студия агентов (раздел `agent-studio`)

Разметки в `index.html` нет — она в `<template id="template-agent-studio">`, и каждый
экземпляр вкладки получает свои id с префиксом `s-<ключ вкладки>-`. Поэтому адресуем
экземпляры по **суффиксу** id (`[id$="-…"]`), а не по точному id.

| Что | Селектор |
|-----|----------|
| Все открытые экземпляры студии | `#workspace-content [id$="view-agent-studio"]` |
| Суб-вкладки | `[id$="-subtab-graph"]`, `[id$="-subtab-ai-test"]`, `[id$="-subtab-coding-test"]` |
| Панели суб-вкладок | `[id$="-view-sub-graph"]`, `[id$="-view-sub-ai-test"]`, `[id$="-view-sub-coding-test"]` |
| Граф: открыть / сохранить / отмена | `[id$="-btn-open-workflow"]`, `[id$="-btn-save-workflow"]`, `[id$="-btn-undo"]` |
| Холст и сайдбар | `[id$="-graph-container"]`, `[id$="-graph-sidebar"]` (открыт — класс `open`) |
| Ноды графа (drawflow) | `#node-<id ноды>` — **в пределах своего экземпляра**, id нод у разных графов совпадают |

### Чат

Страница чата строится в JS, поэтому адресуется **классами**, а не ids.

| Что | Селектор |
|-----|----------|
| Поле ввода | `.chat-input` (Ctrl+Enter — отправить) |
| Отправить / стоп | `.chat-input-controls-right .btn-primary`, `.chat-input-controls-right .btn-danger` |
| Выбор агента / модели | `.agent-select`, `.model-select` |
| История | `.chat-history` (сообщения `.message`) |

### Настройки и диалоги

| Что | Селектор |
|-----|----------|
| Разделы настроек | `.settings-nav-btn[data-section="sessions|agent-studio|logs|engines"]` |
| Параметры генерации | `#temp-slider`, `#topk-slider`, `#topp-slider`, `#minp-slider`, `#reppen-slider`, `#prespen-slider`, `#btn-reset-params` |
| Тема | `#theme-select` |
| Модал подтверждения | `#confirm-overlay`, `#confirm-btn-yes`, `#confirm-btn-no` |
| Всплывающее окно обновления | `#update-popup`, `#btn-update-now`, `#btn-update-later` |
| Обновления и логи | веб-компоненты плагина `<about-updates-panel>` и `<logs-panel>` (внутри `#view-logs`), **не** `#log-view` |

## Оформление студии — проверять классами, а не id

Студия множится по вкладкам, поэтому оформление привязано к **классам**
(`.graph-toolbar`, `.graph-layout`, `.graph-container`, `.graph-sidebar`,
`.graph-detail-content`, `.sub-view.studio-scroll`). Правило по id в `styles.css`
никогда не сработает: id элементов студии всегда с префиксом экземпляра. Это
проверяется тестом `test/studio-styles.test.ts`.

## Процесс тестирования

1. Подключись (`snapshot -i`) и изучи UI.
2. Выполни сценарий (клики/ввод/select); после каждого шага — новый снапшот.
3. Проверяй результат: текст в `.chat-history`, состояние кнопок (disabled/visible), логи
   на вкладке «Логи».
4. При необходимости — скриншот `npx agent-browser --cdp 9222 screenshot`.
5. В отчёте указывай: что нажато, что ожидалось, что получено.

## Ограничения

- Тестируется **живое** приложение: используй отдельную тестовую сессию, не трогай UI во время
  генерации LLM/скачивания (жди через `wait`).
- Окно приложения должно быть открыто (WebView2 троттлит свёрнутые окна).
- Порт 9222 — только в dev-запуске (build.bat); у установленных пользователей его нет.