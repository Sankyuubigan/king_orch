# Подключение tauri-plugin-logs (единый лог вывода приложения)

Виджет «Логи» вынесен в переиспользуемый плагин `tauri-plugin-logs`
(репозиторий: `D:\Projects\my-tauri-plugins\tauri-plugin-logs`).
Плагин: один `log::Log` (включая host RSL при дебаге), файл `king_orch.log` рядом
с exe, dev-зеркало `test/last_logs.txt`, «Последние логи» в Store, опциональный
облачный репорт и аналитика Aptabase (поле `project`). Длинные записи автоматически
обрезаются до 2000 символов перед записью в лог и отправкой во вкладку «Логи».

## Шаги подключения

### 1. Rust-зависимость (`src-tauri/Cargo.toml`)
```toml
tauri-plugin-logs = { path = "../../my-tauri-plugins/tauri-plugin-logs" }
log = "0.4"
```

### 2. Конфиг (`src-tauri/tauri.conf.json` → `app.plugins.logs`)
```json
"plugins": {
  "logs": {
    "log_file_name": "king_orch.log",
    "last_logs": true,
    "reporting": {
      "enabled": true,
      "project": "<project-slug>",
      "app_key": "<aptabase-key>",
      "events": true
    }
  }
}
```

### 3. Капабилити (`src-tauri/capabilities/default.json`)
```json
"permissions": ["core:default", "logs:default"]
```

### 4. Инициализация в Rust (`src-tauri/src/main.rs` или `lib.rs`)
Плагин Wry-only: `init()` не-generic.
```rust
tauri::Builder::default()
    .plugin(tauri_plugin_logs::init())
    .setup(|app| {
        // выключить облако, пока не зарегистрирован app_key
        tauri_plugin_logs::set_reporting_enabled(false);
        tauri_plugin_logs::track_event("app_started", None);
        Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
```

### 5. Логирование на бэкенде
```rust
log::info!/warn!/error!("...")                    // локально
tauri_plugin_logs::track_event("имя", Some(json!({...})))  // локально + облако
```

### 6. Фронтенд (`package.json`)
```json
"dependencies": { "@my-tauri-plugins/plugin-logs": "file:../my-tauri-plugins/tauri-plugin-logs" }
```
```ts
import "@my-tauri-plugins/plugin-logs";           // side-effect: ловит ошибки UI
import { logFront, onLogMessage, clearLogs, setReportingEnabled } from "@my-tauri-plugins/plugin-logs";
```
Виджет: `<logs-panel></logs-panel>` (web-component), стили — в вашем style.css.

## Примечания
- `reporting.enabled` включается с рабочего места: до этого облако молчит, файл живёт всегда.
- Файл лога кладётся рядом с exe через `current_exe().parent()` (правило cwd, см. global_ai_docs).
- `test/last_logs.txt` — dev-зеркало: пишется, только если рядом с exe есть каталог `test/` (удобно проверять логи в CI).