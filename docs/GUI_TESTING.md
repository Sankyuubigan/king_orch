# 🖥️ GUI-тестирование King Orch внешним ИИ-агентом

> **Как это работает:** UI King Orch — веб-страница внутри WebView2 (Chromium). При dev-запуске
> через `build.bat` приложение открывает CDP-порт `9222`, и внешний агент (opencode и т.п.)
> может читать DOM, кликать, вводить текст и делать скриншоты — без «зрения».
> Полный гайд для Tauri-проектов: `global_ai_docs/desktop_rust_tauri/gui_testing.md`.

## Запуск

1. Запусти приложение: `build.bat`. Порт включает **конфиг-оверрайд** в `build.cjs`
   (`app.windows[].additionalBrowserArgs: "--remote-debugging-port=9222 --remote-allow-origins=*"`).
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

## Основные селекторы (стабильные id из index.html)

| Что | Селектор |
|-----|----------|
| Новая сессия | `#btn-new-session` |
| Вкладки: Чат / Студия / Настройки / Логи | `#tab-chat`, `#tab-agent-studio`, `#tab-settings`, `#tab-logs` |
| Поле ввода чата | `#chat-input` |
| Отправить | `#btn-send` (Ctrl+Enter), стоп — `#btn-stop` |
| Выбор агента / модели | `#agent-select`, `#model-select` |
| История чата | `#chat-history` (сообщения `.message`) |
| Вкладки студии: Граф / Тест агентов | `#subtab-graph`, `#subtab-ai-test` |
| Граф: открыть / сохранить | `#btn-open-workflow`, `#btn-save-workflow`, контейнер `#graph-container` |
| Настройки: проверка обновлений | `#btn-check-update`, `#btn-install-update` |
| Параметры генерации | `#temp-slider`, `#topk-slider`, `#topp-slider`, `#minp-slider`, `#reppen-slider`, `#prespen-slider`, `#btn-reset-params` |
| Тема / формат промпта | `#theme-select`, `#prompt-format-select` |
| Логи | `#log-view` (textarea), сохранить `#btn-save-logs` |
| Модал подтверждения | `#confirm-overlay`, `#confirm-btn-yes`, `#confirm-btn-no` |

## Процесс тестирования

1. Подключись (`snapshot -i`) и изучи UI.
2. Выполни сценарий (клики/ввод/select); после каждого шага — новый снапшот.
3. Проверяй результат: текст в `#chat-history`, состояние кнопок (disabled/visible), содержимое `#log-view`.
4. При необходимости — скриншот `npx agent-browser --cdp 9222 screenshot`.
5. В отчёте указывай: что нажато, что ожидалось, что получено.

## Ограничения

- Тестируется **живое** приложение: используй отдельную тестовую сессию, не трогай UI во время
  генерации LLM/скачивания (жди через `wait`).
- Окно приложения должно быть открыто (WebView2 троттлит свёрнутые окна).
- Порт 9222 — только в dev-запуске (build.bat); у установленных пользователей его нет.