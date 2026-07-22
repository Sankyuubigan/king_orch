# План: Tauri Updater fix + Lazy MCP binary download

## Часть 1: Исправление автообновления (Tauri Updater Error 404)

### Проблема

При проверке обновлений в настройках:
- Нет обновления → "У вас актуальная версия" (OK)
- Есть обновление → `❌ Ошибка проверки обновлений: Download request failed with status: 404 Not Found`

**Корень:** `gh release create` нормализует пробелы в именах файлов → `King Orch` → `King.Orch`. Но `release.cjs` генерирует `latest.json` на основе локального имени файла (с пробелами → `%20`). URL下载 ведёт в пустоту.

Дополнительно: endpoint в `tauri.conf.json` использует редирект GitHub Releases (`/releases/latest/download/`), который иногда отдаёт 404.

### 1.1 tauri.conf.json — смена endpoint
**Файл:** `src-tauri/tauri.conf.json`
```patch
- "https://github.com/Sankyuubigan/king_orch/releases/latest/download/latest.json"
+ "https://raw.githubusercontent.com/Sankyuubigan/king_orch/main/latest.json"
```
Причина: raw.githubusercontent.com — статический файл из ветки main, без редиректов.

### 1.2 release.cjs — исправление URL ассетов
**Файл:** `release.cjs`

**Проблема:** `release.cjs:185` генерирует URL через `encodeURIComponent(targetForUpdater)`, где `targetForUpdater` — локальное имя файла с пробелами (`King Orch_...`). GitHub при загрузке через `gh release create` заменяет пробелы на точки → файл на GitHub называется `King.Orch_...`. Итого: URL с `%20` → 404.

**Решение:** После публикации релиза (шаг 6) — запрашивать GitHub API за реальными именами ассетов и перезаписывать `latest.json`.

Между шагом 5.5 (коммит latest.json) и шагом 6 (публикация релиза) добавить логику, а после шага 6 — перезаписать URL:

```javascript
// После gh release create — получить реальные имена ассетов из API
console.log('\n🔄 Обновление URL в latest.json по реальным именам ассетов...');
try {
    const apiResponse = execSync(
        `gh api repos/Sankyuubigan/king_orch/releases/tags/${tag} --jq ".assets[] | .name + \":\" + .browser_download_url"`,
        { encoding: 'utf8', cwd: scriptDir }
    ).trim();

    let latestJsonText = fs.readFileSync(latestJsonPath, 'utf8');
    const latestJson = JSON.parse(latestJsonText);

    for (const line of apiResponse.split('\n')) {
        const [assetName, downloadUrl] = line.split(':');
        if (!assetName || !downloadUrl) continue;
        const fullUrl = downloadUrl.startsWith('http')
            ? downloadUrl
            : `https://${downloadUrl.substring(downloadUrl.indexOf('github.com'))}`;

        // Обновить URL в platforms если имя файла совпадает с тем что в latest.json
        for (const [platform, info] of Object.entries(latestJson.platforms)) {
            const oldFileName = decodeURIComponent(info.url.split('/').pop());
            const apiBaseName = assetName.replace(/\.(sig|exe|zip)$/i, '');
            const localBaseName = oldFileName.replace(/\.(sig|exe|zip)$/i, '');
            if (apiBaseName === localBaseName || apiBaseName.replace(/\./g, ' ') === localBaseName) {
                info.url = fullUrl;
            }
        }
    }

    fs.writeFileSync(latestJsonPath, JSON.stringify(latestJson, null, 2), 'utf8');
    console.log('✅ latest.json обновлён с реальными URL ассетов.');

    // Перекоммитить и запушить обновлённый latest.json
    execSync('git add latest.json', { stdio: 'inherit', cwd: scriptDir });
    execSync('git commit --amend --no-edit', { stdio: 'inherit', cwd: scriptDir });
    execSync('git push origin main --force-with-lease', { stdio: 'inherit', cwd: scriptDir });
    console.log('✅ Исправленный latest.json запушен.');
} catch (e) {
    console.error('⚠️ Не удалось обновить URL ассетов:', e.message);
}
```

### 1.3 release.cjs — коммит latest.json в main (уже есть, оставить)
**Файл:** `release.cjs` (шаг 5.5)
Текущий код коммита/пуша latest.json в main — оставить как есть.

### 1.4 Фикс текущего latest.json для v26.7.26
**Файл:** `latest.json`
Заменить `%20` на `.` в URL для немедленного починки обновления для текущих пользователей:
```patch
- "url": "https://github.com/Sankyuubigan/king_orch/releases/download/v26.7.26/King%20Orch_26.7.26_x64-setup.exe"
+ "url": "https://github.com/Sankyuubigan/king_orch/releases/download/v26.7.26/King.Orch_26.7.26_x64-setup.exe"
```

---

## Часть 2: Динамическое скачивание MCP-бинарников (bins)

### 2.1 Cargo.toml
**Файл:** `src-tauri/Cargo.toml`
Добавить зависимость:
```toml
zip = "2"
```
Нужен для распаковки Deno (.zip архив). PowerShell Expand-Archive НЕ используем,
так как на многих ПК политики выполнения запрещают скрипты PS.

### 2.2 infra/downloader.rs — универсальная download_binary
**Файл:** `src-tauri/src/infra/downloader.rs`
Добавить функцию `download_binary`:
- скачивает файл по URL в save_path
- эмитит прогресс через `app.emit("download_binary_progress", ...)`
- поддерживает распаковку `.zip` (флаг extract_zip)
- не проверяет GGUF-магию (в отличие от существующей `download_model`)
- после скачивания делает chmod +x на unix

### 2.3 НОВЫЙ infra/bin_downloader.rs
**Файл:** `src-tauri/src/infra/bin_downloader.rs`

Содержит:
- `ensure_bins(app: &tauri::AppHandle, bins_dir: &Path)` — проверяет и качает недостающие
- Хардкод URL'ов для x86_64 Windows:
  - Node.js: `https://nodejs.org/dist/v22.0.0/win-x64/node.exe`
  - Deno: `https://github.com/denoland/deno/releases/download/v2.2.8/deno-x86_64-pc-windows-msvc.zip`
  - yt-dlp: `https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe`
- Конфигурация target/OS определяется через `std::env::consts`

### 2.4 infra/mod.rs
**Файл:** `src-tauri/src/infra/mod.rs`
```patch
+ pub mod bin_downloader;
+ pub use bin_downloader::ensure_bins;
```

### 2.5 domain/orchestrator/runtime.rs
**Файл:** `src-tauri/src/domain/orchestrator/runtime.rs`

В `resolve_runtime_and_args`:
- перед возвратом пути node/deno, если файл не существует:
  - вызвать `super::super::infra::bin_downloader::ensure_bins(??? )` — нужно прокинуть AppHandle или bins_dir

Текущая сигнатура:
```rust
pub fn resolve_runtime_and_args<L: Fn(String) + Clone + Send + Sync>(log_cb: L, script_path: &Path) -> (PathBuf, Vec<String>)
```
Нужно добавить параметр `bins_dir: &Path`:
```rust
pub fn resolve_runtime_and_args<L: Fn(String) + Clone + Send + Sync>(log_cb: L, script_path: &Path, bins_dir: &Path) -> (PathBuf, Vec<String>)
```

В `load_mcp_servers`:
- передать `bins_dir` в `resolve_runtime_and_args` (нужно получать его заранее)

**Вариант без инжекции AppHandle:** bins_dir вычисляется из `directories_next` или константой из конфига.
Поскольку `resolve_runtime_and_args` работает внутри `spawn_blocking`, проще всего вычислять bins_dir прямо там:
```rust
let bins_dir = std::path::PathBuf::from("nonexistent"); // fallback
if let Ok(data_dir) = std::env::var("APPDATA") {
    bins_dir = PathBuf::from(data_dir).join("com.kingorch.app").join("bins");
}
```
(на Windows) или через `dirs::data_dir()` — добавить крейт `directories` или использовать штатный `app.path()`, но внутри spawn_blocking его нет.

**Лучшее решение: передавать bins_dir как параметр**, который вычисляется в `api/chat.rs` (где есть `&app`).

### 2.6 api/chat.rs
**Файл:** `src-tauri/src/api/chat.rs`
В `chat_request`:
```rust
let bins_dir = app.path().app_data_dir().unwrap_or_default().join("bins");
```
передавать через дополнительные параметры в `domain::run_chat`, который прокинет в `load_mcp_servers`.

### 2.7 src/controllers/chat.ts
**Файл:** `src/controllers/chat.ts`
В `bindTauriEvents`:
```typescript
listen("download_binary_progress", (event) => {
  const { tool, phase } = event.payload as { tool: string; phase: string };
  this.logToGUI(`📥 ${tool}: ${phase}`);
});
listen("binary_downloaded", (event) => {
  const { tool } = event.payload as { tool: string };
  showToast(`${tool} установлен`, "success");
});
```

---

## Файлы для изменения

| # | Файл | Тип | Описание |
|---|------|-----|----------|
| 1 | `src-tauri/tauri.conf.json` | modify | updater endpoint → raw.githubusercontent |
| 2 | `release.cjs` | modify | запрос API за реальными именами ассетов + коммит latest.json |
| 3 | `latest.json` | modify | заменить %20 на . в URL (срочный фикс v26.7.26) |
| 4 | `src-tauri/Cargo.toml` | modify | добавить zip dependency |
| 5 | `src-tauri/src/infra/downloader.rs` | modify | добавить `download_binary` |
| 6 | `src-tauri/src/infra/bin_downloader.rs` | **NEW** | ensure_bins + hardcoded URLs |
| 7 | `src-tauri/src/infra/mod.rs` | modify | экспорт bin_downloader |
| 8 | `src-tauri/src/domain/orchestrator/runtime.rs` | modify | lazy download перед node/deno |
| 9 | `src-tauri/src/api/chat.rs` | modify | bins_dir вычисление + проброс |
| 10 | `src/controllers/chat.ts` | modify | слушать `download_binary_progress` |
