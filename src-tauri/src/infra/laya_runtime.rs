//! Рантайм ONNX для Laya (System-1): где лежит `onnxruntime.dll`, как её
//! достать и как загрузить в процесс.
//!
//! ## Почему не линковка
//!
//! При фиче `download-binaries` крейт `ort` вшивал в `king_orch.exe` статик
//! `onnxruntime.lib` (~341 МБ) и добавлял в таблицу импортов PE жёсткую
//! ссылку на `DirectML.dll`. Загрузчик Windows (`LdrpLoadImportModule`)
//! разрешает импорты **до** `main()`, поэтому на машине, где этой DLL нет,
//! процесс падал с `STATUS_DLL_NOT_FOUND` — без окна, без `king_orch.log`,
//! без записи в `crash_dump.log`. Отсюда «программа не открывается, логов нет».
//!
//! Фича `load-dynamic` включает `ort-sys/disable-linking`: build-скрипт выходит
//! до строки `cargo:rustc-link-lib=DirectML`, импортов в бинаре не остаётся,
//! а `onnxruntime.dll` открыется в момент первого вызова валидатора.
//!
//! ## Версия DLL
//!
//! `ort` 2.0.0-rc.13 требует ONNX Runtime не ниже `ORT_API_VERSION` = 27,
//! то есть 1.27+. Берём официальную CPU-сборку Microsoft 1.28.0: в отличие от
//! pyke-бинарников она не тянет за собой DirectML и компилируется с baseline
//! SSE2 (MLAS выбирает AVX/AVX2-ядра в рантайме по CPUID).

use std::path::{Path, PathBuf};

use tauri_plugin_downloader::DownloadOptions;

/// Папка рантайма рядом с exe. Путь считаем через `current_exe()` — правило
/// cwd из AGENTS.md, а не `app.path().executable_dir()`.
const RUNTIME_DIR: &str = "laya";
const DLL_NAME: &str = "onnxruntime.dll";
const ZIP_NAME: &str = "onnxruntime-win-x64-1.28.0.zip";

/// Официальный релиз Microsoft, CPU-сборка: без DirectML-зависимостей.
const ZIP_URL: &str =
    "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-win-x64-1.28.0.zip";

/// Папка рантайма: `<exe>/laya`.
pub fn runtime_dir() -> PathBuf {
    exe_dir().join(RUNTIME_DIR)
}

/// Путь к `onnxruntime.dll`.
pub fn dll_path() -> PathBuf {
    runtime_dir().join(DLL_NAME)
}

/// Есть ли DLL на диске.
pub fn is_present() -> bool {
    dll_path().is_file()
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Скачать архив и распаковать из него только `onnxruntime.dll`, если её нет.
///
/// Из архива берётся один файл: остальное (~79 МБ заголовков, `.lib`,
/// метаданных) приложению не нужно и на диске не оседает.
///
/// Синхронная обёртка — вызывается из обработчика ноды валидатора. Сам
/// `download_blocking` умеет и внутри tokio-рантайма (`block_in_place`), и вне.
pub fn ensure_runtime() -> Result<PathBuf, String> {
    let dll = dll_path();
    if dll.is_file() {
        return Ok(dll);
    }

    let dir = runtime_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("не создать каталог {}: {}", dir.display(), e))?;

    let zip_path = dir.join(ZIP_NAME);
    log::info!(
        "[laya] onnxruntime.dll отсутствует, скачиваю ONNX Runtime: {} -> {}",
        ZIP_URL,
        zip_path.display()
    );

    tauri_plugin_downloader::download_blocking(
        ZIP_URL,
        &zip_path,
        DownloadOptions {
            label: "ONNX Runtime (Laya)".into(),
            kind: "engine".into(),
            keep_partial: false,
            ..Default::default()
        },
        None,
    )
    .map_err(|e| {
        log::error!("[laya] загрузка ONNX Runtime не удалась: {}", e);
        format!("ошибка загрузки ONNX Runtime: {}", e)
    })?;

    extract_dll(&zip_path, &dll)?;
    let _ = std::fs::remove_file(&zip_path);

    let size = std::fs::metadata(&dll).map(|m| m.len()).unwrap_or(0);
    log::info!("[laya] ONNX Runtime готов: {} ({} байт)", dll.display(), size);
    Ok(dll)
}

/// Достать из zip единственный нужный файл `onnxruntime.dll`.
fn extract_dll(zip_path: &Path, dll: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("не открыть архив {}: {}", zip_path.display(), e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("архив не читается (повреждён или не zip): {}", e))?;

    let index = (0..archive.len())
        .find(|i| {
            archive
                .by_index(*i)
                .map(|f| f.name().ends_with(DLL_NAME))
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            log::error!("[laya] в архиве {} нет {}", zip_path.display(), DLL_NAME);
            format!("в архиве нет {}", DLL_NAME)
        })?;

    let mut entry = archive
        .by_index(index)
        .map_err(|e| format!("чтение записи архива: {}", e))?;
    let mut out = std::fs::File::create(dll)
        .map_err(|e| format!("не создать {}: {}", dll.display(), e))?;
    std::io::copy(&mut entry, &mut out).map_err(|e| {
        log::error!("[laya] распаковка {} не удалась: {}", DLL_NAME, e);
        format!("распаковка {}: {}", DLL_NAME, e)
    })?;
    Ok(())
}

/// Обеспечить наличие `onnxruntime.dll` и загрузить её в процесс.
///
/// Вызывать ДО любого другого вызова `ort`. Отсутствие DLL — это ошибка в лог
/// и возврат ошибки наружу, а не падение и не тихая работа без System-1.
pub fn init_environment() -> Result<(), String> {
    let dll = ensure_runtime()?;
    if !dll.is_file() {
        log::error!(
            "[laya] ONNX Runtime не найден: {} — валидатор System-1 (Laya) недоступен. \
             Файл ставится установкой движка в <exe>/laya/onnxruntime.dll.",
            dll.display()
        );
        return Err(format!("onnxruntime.dll не найден по пути {}", dll.display()));
    }

    let builder = ort::init_from(&dll).map_err(|e| {
        log::error!("[laya] не удалось загрузить ONNX Runtime {}: {}", dll.display(), e);
        format!("не удалось загрузить ONNX Runtime {}: {}", dll.display(), e)
    })?;

    if !builder.commit() {
        log::error!(
            "[laya] окружение ONNX Runtime не создалось (commit() == false), DLL: {}",
            dll.display()
        );
        return Err("окружение ONNX Runtime не создалось".to_string());
    }

    log::info!("[laya] ONNX Runtime загружен: {}", dll.display());
    Ok(())
}