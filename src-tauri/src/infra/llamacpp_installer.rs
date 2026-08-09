//! Установка движка llamacpp: скачивание полного релиза llama.cpp
//! (архив `llama-<tag>-bin-win-<variant>-x64.zip`) с GitHub.
//!
//! ВАЖНО (новая архитектура): движок — это ОТДЕЛЬНЫЙ ПРОЦЕСС `llama-server.exe`,
//! который приложение запускает по HTTP (см. infra::llm). Приложение больше
//! НЕ линкует llama.cpp (нет PE-импортов, нет DLL рядом с exe) — поэтому нужен
//! полный архив движка, а не только CUDA runtime.
//!
//! Несколько бекендов могут быть установлены ОДНОВРЕМЕННО (как в Jan):
//! `backends/<variant>/` — каждый вариант живёт в своей подпапке со своим
//! `engine_meta.json`. Переключение между ними мгновенное, без перекачивания.
//! Выбор юзера хранится в app_config.json (`engine_variant`), "auto" = подбор
//! по GPU (см. gpu_detector::required_cuda_gen).

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

/// Варианты движка llama.cpp в ассетах релизов ggml-org/llama.cpp.
pub const VARIANT_CUDA: &str = "cuda-12.4";
/// Актуальная сборка CUDA 13.x (мажорная версия может отличаться в новых релизах —
/// реальное имя записывается в engine_meta.json по имени скачанного ассета).
pub const VARIANT_CUDA13: &str = "cuda-13.3";
pub const VARIANT_CPU: &str = "cpu";
pub const VARIANT_VULKAN: &str = "vulkan";
/// ROCm для Windows — только современные AMD (RX 6000/7000+). На старых AMD
/// (RX 5xx/Vega/RDNA1) не работает — им нужен Vulkan.
pub const VARIANT_HIP: &str = "hip-radeon";
/// Значение конфига «подобрать автоматически по видеокарте».
pub const VARIANT_AUTO: &str = "auto";

const LLAMA_CPP_API: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest";
const METADATA_FILE: &str = "engine_meta.json";

/// Семейство варианта движка — по нему проверяется совместимость с GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineFamily {
    Cpu,
    Cuda12,
    Cuda13,
    Vulkan,
    Hip,
}

impl EngineFamily {
    pub fn from_variant(variant: &str) -> EngineFamily {
        let v = variant.to_lowercase();
        if v.starts_with("cuda-13") {
            EngineFamily::Cuda13
        } else if v.starts_with("cuda-12") || v.contains("cuda") {
            EngineFamily::Cuda12
        } else if v.starts_with("vulkan") {
            EngineFamily::Vulkan
        } else if v.starts_with("hip") {
            EngineFamily::Hip
        } else {
            EngineFamily::Cpu
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EngineFamily::Cpu => "CPU",
            EngineFamily::Cuda12 => "CUDA 12.x",
            EngineFamily::Cuda13 => "CUDA 13.x",
            EngineFamily::Vulkan => "Vulkan",
            EngineFamily::Hip => "HIP/ROCm",
        }
    }

    /// GPU-семейство (модель оффлоудится в VRAM при запуске)
    pub fn is_gpu(self) -> bool {
        !matches!(self, EngineFamily::Cpu)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EngineMeta {
    pub tag: String,
    /// Вариант движка: "cpu", "cuda-12.4" или "cuda-13.x" (фактический, из имени ассета)
    #[serde(default)]
    pub variant: String,
    pub installed_at: String,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// Папка движка по умолчанию — рядом с exe (в папке установки программы)
pub fn default_dir(exe_dir: &Path) -> PathBuf {
    exe_dir.join("llamacpp")
}

/// Папка со всеми установленными бекендами: `<llamacpp_dir>/backends`
pub fn backends_dir(dir: &Path) -> PathBuf {
    dir.join("backends")
}

/// Папка конкретного варианта бекенда: `<llamacpp_dir>/backends/<variant>`
pub fn variant_dir(dir: &Path, variant: &str) -> PathBuf {
    backends_dir(dir).join(variant)
}

/// Все известные варианты бекенда (для дропдауна в настройках)
pub fn all_variants() -> Vec<String> {
    vec![
        VARIANT_CPU.to_string(),
        VARIANT_CUDA.to_string(),
        VARIANT_CUDA13.to_string(),
        VARIANT_VULKAN.to_string(),
        VARIANT_HIP.to_string(),
    ]
}

/// Известен ли вариант (не "auto" и есть в списке)
pub fn is_known_variant(variant: &str) -> bool {
    all_variants().iter().any(|v| v == variant)
}

/// Автоопределение варианта по GPU (логика как в Jan):
/// Blackwell → cuda-13.x; NVIDIA с драйвером CUDA 13+ (R580+) → cuda-13.x
/// (сборка содержит ядра sm_75..sm_120, включая RTX 40xx); остальные NVIDIA
/// (драйвер CUDA 12+) → cuda-12.4; иначе CPU.
pub fn select_variant() -> String {
    use crate::infra::gpu_detector::{detect_gpu, required_cuda_gen, CudaGen};
    let gpu = detect_gpu();
    match required_cuda_gen(&gpu) {
        Some(CudaGen::Cuda13) => VARIANT_CUDA13.to_string(),
        Some(CudaGen::Cuda12) => VARIANT_CUDA.to_string(),
        None => VARIANT_CPU.to_string(),
    }
}

/// Итоговый вариант по предпочтению юзера: "auto"/None → подбор по GPU,
/// явный известный вариант → как есть, неизвестный → авто.
pub fn resolve_variant(pref: Option<&str>) -> String {
    match pref {
        Some(p) if !p.is_empty() && p != VARIANT_AUTO && is_known_variant(p) => p.to_string(),
        _ => select_variant(),
    }
}

/// Человекочитаемое описание варианта для UI (подсказка в дропдауне)
pub fn variant_note(variant: &str) -> &'static str {
    match variant {
        VARIANT_CPU => "Работает на любом компьютере, без видеокарты",
        VARIANT_CUDA => "NVIDIA GTX 10xx — RTX 40xx (драйвер CUDA 12+)",
        VARIANT_CUDA13 => "NVIDIA с драйвером CUDA 13+ (R580+). Работает на RTX 40xx и 50xx; для 50xx обязателен",
        VARIANT_VULKAN => "Любые видеокарты: AMD, Intel, NVIDIA (через Vulkan)",
        VARIANT_HIP => "Только современные AMD: RX 6000/7000 (ROCm). На старых AMD (RX 5xx, Vega, RX 5700) не работает — выберите Vulkan",
        _ => "",
    }
}

/// Описание варианта для дропдауна в настройках
#[derive(Serialize, Clone)]
pub struct VariantInfo {
    pub id: String,
    pub label: String,
    pub note: String,
    /// Этот вариант подобрал бы авто-режим на текущей машине
    pub recommended: bool,
    pub installed: bool,
}

/// Список вариантов для дропдауна + установлен ли каждый на диске
pub fn available_variants(dir: &Path) -> Vec<VariantInfo> {
    let auto = select_variant();
    let installed = list_installed_variants(dir);
    all_variants()
        .into_iter()
        .map(|id| VariantInfo {
            label: variant_label(&id).to_string(),
            note: variant_note(&id).to_string(),
            recommended: id == auto,
            installed: installed.iter().any(|v| v == &id),
            id,
        })
        .collect()
}

pub fn variant_label(variant: &str) -> &'static str {
    match variant {
        VARIANT_CPU => "CPU (процессор)",
        VARIANT_CUDA => "CUDA 12.x (NVIDIA)",
        VARIANT_CUDA13 => "CUDA 13.x (NVIDIA, драйвер 580+)",
        VARIANT_VULKAN => "Vulkan (любая видеокарта)",
        VARIANT_HIP => "HIP / ROCm (AMD RX 6000/7000+)",
        VARIANT_AUTO => "Авто (рекомендуется)",
        _ => "Вариант (неизвестный)",
    }
}

/// Имена ассетов движка в релизах llama.cpp менялись:
/// - старый формат: llama-<tag>-bin-win-<variant>-x64.zip (до b10275)
/// - новый формат: cudart-llama-bin-win-<variant>-x64.zip (с b10275,
///   единый универсальный архив со всеми бэкендами + CUDA runtime)
fn asset_name_candidates(tag: &str, variant: &str) -> Vec<String> {
    vec![
        format!("llama-{}-bin-win-{}-x64.zip", tag, variant),
        format!("cudart-llama-bin-win-{}-x64.zip", variant),
    ]
}

/// Фактический вариант из имени ассета:
/// "llama-b10278-bin-win-cuda-13.3-x64.zip" → "cuda-13.3",
/// "cudart-llama-bin-win-cuda-12.4-x64.zip" → "cuda-12.4",
/// "llama-b10278-bin-win-cpu-x64.zip" → "cpu"
fn variant_from_asset_name(name: &str) -> Option<String> {
    let stem = name.strip_suffix("-x64.zip")?;
    let idx = stem.rfind("-win-")?;
    let v = &stem[idx + 5..];
    if v.is_empty() {
        return None;
    }
    Some(v.to_string())
}

/// Поиск ассета по семейству (cuda-12 / cuda-13 / vulkan / hip): мажорная версия
/// CUDA в имени может отличаться от ожидаемой (например cuda-13.4 вместо cuda-13.3).
fn find_asset_by_family<'a>(release: &'a GitHubRelease, family: EngineFamily) -> Option<(&'a GitHubAsset, String)> {
    let needle = match family {
        EngineFamily::Cuda13 => "-win-cuda-13",
        EngineFamily::Cuda12 => "-win-cuda-12",
        EngineFamily::Vulkan => "-win-vulkan",
        EngineFamily::Hip => "-win-hip",
        EngineFamily::Cpu => return None,
    };
    for asset in &release.assets {
        if !asset.name.ends_with("-x64.zip") {
            continue;
        }
        if asset.name.contains(needle) {
            let actual = variant_from_asset_name(&asset.name)
                .unwrap_or_else(|| match family {
                    EngineFamily::Cuda13 => "cuda-13".to_string(),
                    EngineFamily::Cuda12 => "cuda-12".to_string(),
                    EngineFamily::Vulkan => "vulkan".to_string(),
                    EngineFamily::Hip => "hip-radeon".to_string(),
                    EngineFamily::Cpu => "cpu".to_string(),
                });
            return Some((asset, actual));
        }
    }
    None
}

fn find_engine_asset<'a>(release: &'a GitHubRelease, variant: &str) -> Option<(&'a GitHubAsset, String)> {
    let tag = &release.tag_name;
    let candidates = asset_name_candidates(tag, variant);
    for name in &candidates {
        if let Some(asset) = release.assets.iter().find(|a| a.name == *name) {
            return Some((asset, variant.to_string()));
        }
    }
    // Фолбэк по маске: могло измениться форматирование имени
    if let Some(asset) = release.assets.iter().find(|a| {
        a.name.starts_with(&format!("llama-{}-bin-win-{}", tag, variant))
            && a.name.ends_with("-x64.zip")
    }) {
        return Some((asset, variant.to_string()));
    }
    // Фолбэк по семейству: сменилась минорная версия CUDA (cuda-13.3 → cuda-13.4)
    if let Some(hit) = find_asset_by_family(release, EngineFamily::from_variant(variant)) {
        return Some(hit);
    }
    // CPU-вариант в новых релизах не публикуется: как последний шанс для
    // CPU-машин подходит универсальный cudart-архив (запустится без GPU).
    if variant == VARIANT_CPU {
        if let Some(asset) = release.assets.iter().find(|a| a.name == "cudart-llama-bin-win-cuda-12.4-x64.zip") {
            return Some((asset, VARIANT_CUDA.to_string()));
        }
    }
    None
}

pub fn meta_path(dir: &Path, variant: &str) -> PathBuf {
    variant_dir(dir, variant).join(METADATA_FILE)
}

/// Установлен ли конкретный вариант бекенда: главный бинарь на месте
pub fn is_installed(dir: &Path, variant: &str) -> bool {
    variant_dir(dir, variant).join("llama-server.exe").exists()
}

/// Метаданные установленного варианта (None если не установлен)
pub fn installed_meta(dir: &Path, variant: &str) -> Option<EngineMeta> {
    if !is_installed(dir, variant) {
        return None;
    }
    let data = fs::read_to_string(meta_path(dir, variant)).ok()?;
    serde_json::from_str(&data).ok()
}

/// Какие варианты бекенда реально установлены на диске
pub fn list_installed_variants(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(backends_dir(dir)) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // Только папки с engine_meta.json (внутренние папки чужих архивов игнорируем)
        if path.join(METADATA_FILE).exists() {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// Установлен ли ХОТЯ БЫ один вариант бекенда
pub fn has_any_installed(dir: &Path) -> bool {
    !list_installed_variants(dir).is_empty()
}

/// Миграция старого формата (бинарь лежал в корне <llamacpp_dir>, meta в корне)
/// → новый: backends/<variant>/. Возвращает вариант, в который перенесён движок.
pub fn migrate_legacy_layout(dir: &Path) -> Result<Option<String>, String> {
    let root_exe = dir.join("llama-server.exe");
    if !root_exe.exists() {
        return Ok(None);
    }
    let root_meta_path = dir.join(METADATA_FILE);
    let variant = fs::read_to_string(&root_meta_path)
        .ok()
        .and_then(|data| serde_json::from_str::<EngineMeta>(&data).ok())
        .map(|m| m.variant)
        .filter(|v| is_known_variant(v))
        .unwrap_or_else(|| VARIANT_CPU.to_string());

    // Уже перенесено ранее — просто убираем дубль из корня
    let target = variant_dir(dir, &variant);
    if is_installed(dir, &variant) {
        let _ = fs::remove_file(&root_exe);
        let _ = fs::remove_file(&root_meta_path);
        return Ok(Some(variant));
    }

    fs::create_dir_all(&target).map_err(|e| format!("Не удалось создать {}: {}", target.display(), e))?;
    let entries = fs::read_dir(dir).map_err(|e| format!("Ошибка чтения {}: {}", dir.display(), e))?;
    let mut moved = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // Не трогаем папку backends и файл скачивания
        if name == "backends" || name == "engine.zip" {
            continue;
        }
        let dest = target.join(name);
        if dest.exists() {
            continue;
        }
        if fs::rename(&path, &dest).is_ok() {
            moved += 1;
        }
    }
    if moved == 0 {
        return Err("Не удалось перенести файлы движка в новый формат.".to_string());
    }
    Ok(Some(variant))
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("king-orch-app/1.0")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("Ошибка создания HTTP-клиента: {}", e))
}

async fn fetch_latest_release(client: &reqwest::Client) -> Result<GitHubRelease, String> {
    let resp = client
        .get(LLAMA_CPP_API)
        .send()
        .await
        .map_err(|e| format!("Ошибка запроса GitHub API: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("GitHub API вернул HTTP {}", status));
    }
    resp.json::<GitHubRelease>()
        .await
        .map_err(|e| format!("Ошибка парсинга ответа GitHub: {}", e))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path).map_err(|e| format!("Ошибка чтения файла: {}", e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("Ошибка чтения: {}", e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hex: String = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect();
    Ok(hex)
}

/// Скачивание ассета с докачкой (HTTP Range) и прогрессом
async fn download_asset<L: Fn(String), P: Fn(u64, u64)>(
    client: &reqwest::Client,
    asset: &GitHubAsset,
    dest_zip: &Path,
    on_log: &L,
    on_progress: &P,
) -> Result<(), String> {
    let part_path = dest_zip.with_extension("zip.part");
    let resume_from = fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);

    let mut req = client.get(&asset.browser_download_url);
    if resume_from > 0 {
        req = req.header("Range", format!("bytes={}-", resume_from));
        on_log(format!("Докачка архива с {} МБ...", resume_from / 1024 / 1024));
    } else {
        on_log(format!(
            "📥 Скачивание {} ({} МБ)...",
            asset.name,
            asset.size / 1024 / 1024
        ));
    }

    let resp = req.send().await.map_err(|e| format!("Ошибка загрузки: {}", e))?;
    let status = resp.status();
    let total = asset.size;

    let mut file = if status == reqwest::StatusCode::PARTIAL_CONTENT && resume_from > 0 {
        fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&part_path)
            .map_err(|e| format!("Ошибка создания файла: {}", e))?
    } else {
        if status != reqwest::StatusCode::OK {
            return Err(format!("Сервер вернул HTTP {}", status));
        }
        fs::File::create(&part_path).map_err(|e| format!("Ошибка создания файла: {}", e))?
    };

    let mut downloaded = resume_from;
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Ошибка приёма данных: {}", e))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Ошибка записи на диск: {}", e))?;
        downloaded += chunk.len() as u64;
        if total > 0 {
            on_progress(downloaded, total);
        }
    }
    drop(file);

    if total > 0 && downloaded < total {
        let _ = fs::remove_file(&part_path);
        return Err(format!("Загрузка прервалась: скачано {} из {} байт", downloaded, total));
    }

    fs::rename(&part_path, dest_zip).map_err(|e| format!("Ошибка финализации файла: {}", e))?;
    on_log(format!("✅ Скачано: {} МБ", downloaded / 1024 / 1024));
    Ok(())
}

/// Извлечение ВСЕГО содержимого архива в dest_dir (с подпапками, например backends/)
fn extract_all<L: Fn(String)>(zip_path: &Path, dest_dir: &Path, on_log: &L) -> Result<u32, String> {
    on_log("📦 Распаковка движка llama.cpp...".to_string());
    let data = fs::read(zip_path).map_err(|e| format!("Ошибка чтения zip: {}", e))?;
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|e| format!("Ошибка чтения zip: {}", e))?;

    let mut count = 0u32;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Ошибка чтения zip-записи: {}", e))?;
        let name = entry.name().replace('\\', "/");
        let clean = name.trim_start_matches('/');
        if clean.is_empty() || clean.ends_with('/') {
            continue;
        }
        let out_path = dest_dir.join(clean);
        // Защита от path traversal: файл обязан остаться внутри dest_dir
        if !out_path.starts_with(dest_dir) {
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Ошибка создания папки {}: {}", parent.display(), e))?;
        }
        let mut out_file = fs::File::create(&out_path)
            .map_err(|e| format!("Ошибка создания {}: {}", out_path.display(), e))?;
        std::io::copy(&mut entry, &mut out_file)
            .map_err(|e| format!("Ошибка распаковки {}: {}", clean, e))?;
        count += 1;
    }
    Ok(count)
}

/// Удаление содержимого папки движка (перед распаковкой новой версии)
fn clear_dir(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// Поиск `llama-server.exe` под папкой движка. Часть архивов (Jan-формат,
/// cudart-архивы b10275+) кладёт бинарь в подпапку `backends/<tag>/<variant>/build/bin/`,
/// а наш движок ожидает его в корне папки варианта. Возвращает папку с бинарём.
fn find_server_dir(root: &Path) -> Option<PathBuf> {
    let mut queue: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(cur) = queue.pop() {
        let Ok(entries) = fs::read_dir(&cur) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                queue.push(path);
            } else if path.file_name().map(|n| n == "llama-server.exe").unwrap_or(false) {
                return Some(cur);
            }
        }
    }
    None
}

/// Подъём содержимого вложенной папки с llama-server.exe в корень папки варианта
/// (если архив имел структуру backends/<tag>/<variant>/build/bin/...).
fn lift_server_files(variant_root: &Path, on_log: &dyn Fn(String)) -> Result<(), String> {
    if variant_root.join("llama-server.exe").exists() {
        return Ok(());
    }
    let Some(src) = find_server_dir(variant_root) else {
        return Err("После распаковки llama-server.exe не найден — архив повреждён или изменил структуру.".to_string());
    };
    if src == variant_root {
        return Ok(());
    }
    on_log(format!("📁 Архив имел вложенную структуру — поднимаю файлы из {}", src.display()));
    let entries = fs::read_dir(&src).map_err(|e| format!("Ошибка чтения {}: {}", src.display(), e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let dest = variant_root.join(&name);
        if dest.exists() {
            continue;
        }
        fs::rename(&path, &dest).map_err(|e| format!("Ошибка переноса {}: {}", name, e))?;
    }
    // Чистим опустевшие подпапки старой структуры (backends/<tag>/<variant>/build)
    let mut cur = src.clone();
    while cur != *variant_root {
        let _ = fs::remove_dir(&cur);
        cur = match cur.parent() {
            Some(p) => p.to_path_buf(),
            None => break,
        };
    }
    Ok(())
}

/// Установка (или обновление) варианта бекенда llamacpp: полный архив llama-server.
/// Ставится в `backends/<variant>/`, остальные установленные варианты не трогаются.
pub async fn install<L: Fn(String), P: Fn(u64, u64)>(
    dir: &Path,
    variant: &str,
    on_log: L,
    on_progress: P,
) -> Result<EngineMeta, String> {
    let target = variant_dir(dir, variant);
    fs::create_dir_all(&target).map_err(|e| format!("Не удалось создать папку {}: {}", target.display(), e))?;

    // Удаляем старые файлы варианта ДО скачивания (в середине нельзя: clear_dir
    // удалил бы сам скачанный engine.zip, лежащий внутри target).
    clear_dir(&target);

    on_log(format!("🔄 Вариант «{}»: получение информации о последнем релизе llama.cpp...", variant));
    let client = http_client()?;
    let release = fetch_latest_release(&client).await?;

    let asset = find_engine_asset(&release, variant).ok_or_else(|| {
        format!(
            "В релизе {} не найден ассет движка: {}",
            release.tag_name,
            asset_name_candidates(&release.tag_name, variant).join(" или ")
        )
    })?;
    let actual_variant = asset.1.clone();
    if actual_variant != variant {
        on_log(format!(
            "ℹ️ Точный вариант {} в релизе не найден — используется {} (совместим).",
            variant, actual_variant
        ));
    }

    let zip_path = target.join("engine.zip");
    download_asset(&client, asset.0, &zip_path, &on_log, &on_progress).await?;

    if let Some(digest) = &asset.0.digest {
        let expected = digest.strip_prefix("sha256:").unwrap_or(digest);
        let actual = sha256_file(&zip_path)?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = fs::remove_file(&zip_path);
            return Err(format!(
                "Контрольная сумма не совпала! Ожидалось {}, получено {}. Загрузка повреждена.",
                expected, actual
            ));
        }
        on_log("✅ Контрольная сумма SHA-256 подтверждена".to_string());
    }

    let file_count = extract_all(&zip_path, &target, &on_log)?;
    let _ = fs::remove_file(&zip_path);

    // Jan-архивы и cudart-архивы имеют вложенную структуру — поднимаем бинарь наверх
    lift_server_files(&target, &on_log)?;

    let meta = EngineMeta {
        tag: release.tag_name.clone(),
        variant: actual_variant.clone(),
        installed_at: chrono_now(),
    };
    let data = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    fs::write(meta_path(dir, variant), data).map_err(|e| format!("Ошибка записи метаданных: {}", e))?;

    on_log(format!(
        "✅ Бекенд llama.cpp установлен: {} (вариант {}). Распаковано файлов: {}.",
        release.tag_name, actual_variant, file_count
    ));
    Ok(meta)
}

/// Проверка наличия обновления конкретного варианта (только проверка, не установка)
pub async fn check_update<L: Fn(String)>(dir: &Path, variant: &str, on_log: L) -> Result<Option<String>, String> {
    let meta = match installed_meta(dir, variant) {
        Some(m) => m,
        None => return Ok(None),
    };
    let client = http_client()?;
    let release = fetch_latest_release(&client).await?;
    if release.tag_name != meta.tag {
        on_log(format!(
            "🔄 Доступно обновление бекенда llama.cpp ({}): {} → {}",
            variant, meta.tag, release.tag_name
        ));
        Ok(Some(release.tag_name))
    } else {
        on_log(format!("Бекенд llama.cpp актуален ({}): {}", variant, meta.tag));
        Ok(None)
    }
}

/// Удаление конкретного варианта бекенда (освобождает ~300-500 МБ)
pub fn remove<L: Fn(String)>(dir: &Path, variant: &str, on_log: &L) -> Result<(), String> {
    let target = variant_dir(dir, variant);
    if !target.exists() {
        return Ok(());
    }
    clear_dir(&target);
    let _ = fs::remove_dir(&target);
    on_log(format!(
        "🗑️ Бекенд «{}» удалён. Установите его заново, чтобы пользоваться этим режимом.",
        variant_label(variant)
    ));
    Ok(())
}

fn chrono_now() -> String {
    let now = std::time::SystemTime::now();
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_from_asset_name_parses_old_and_new_formats() {
        assert_eq!(
            variant_from_asset_name("llama-b10278-bin-win-cuda-13.3-x64.zip").unwrap(),
            "cuda-13.3"
        );
        assert_eq!(
            variant_from_asset_name("cudart-llama-bin-win-cuda-12.4-x64.zip").unwrap(),
            "cuda-12.4"
        );
        assert_eq!(
            variant_from_asset_name("llama-b10278-bin-win-cpu-x64.zip").unwrap(),
            "cpu"
        );
        assert_eq!(variant_from_asset_name("llama-b10278-bin-win-cuda-13.3-x64.zip.asc"), None);
    }

    #[test]
    fn family_from_variant() {
        assert_eq!(EngineFamily::from_variant("cpu"), EngineFamily::Cpu);
        assert_eq!(EngineFamily::from_variant("cuda-12.4"), EngineFamily::Cuda12);
        assert_eq!(EngineFamily::from_variant("cuda-13.3"), EngineFamily::Cuda13);
        assert_eq!(EngineFamily::from_variant("cuda-13.7"), EngineFamily::Cuda13);
        assert_eq!(EngineFamily::from_variant("vulkan"), EngineFamily::Vulkan);
        assert_eq!(EngineFamily::from_variant("hip-radeon"), EngineFamily::Hip);
        assert_eq!(EngineFamily::from_variant(""), EngineFamily::Cpu);
        assert!(EngineFamily::Vulkan.is_gpu());
        assert!(EngineFamily::Hip.is_gpu());
        assert!(!EngineFamily::Cpu.is_gpu());
    }

    #[test]
    fn resolve_variant_respects_user_preference() {
        // Явный известный вариант — возвращается как есть
        assert_eq!(resolve_variant(Some("vulkan")), "vulkan");
        assert_eq!(resolve_variant(Some("hip-radeon")), "hip-radeon");
        assert_eq!(resolve_variant(Some("cpu")), "cpu");
        // auto / None / неизвестный — автоопределение по GPU
        assert_eq!(resolve_variant(Some("auto")), select_variant());
        assert_eq!(resolve_variant(None), select_variant());
        assert_eq!(resolve_variant(Some("")), select_variant());
        assert_eq!(resolve_variant(Some("opencl")), select_variant());
        assert!(is_known_variant("vulkan"));
        assert!(is_known_variant("hip-radeon"));
        assert!(!is_known_variant("opencl"));
        assert!(!is_known_variant("auto"));
    }

    #[test]
    fn variant_labels_and_notes_are_user_friendly() {
        assert_eq!(variant_label("auto"), "Авто (рекомендуется)");
        assert_eq!(variant_label("hip-radeon"), "HIP / ROCm (AMD RX 6000/7000+)");
        // Подсказка для HIP предупреждает про старые AMD
        assert!(variant_note("hip-radeon").contains("старых AMD"));
        assert!(variant_note("vulkan").contains("Любые видеокарты"));
        // CUDA 13.x — не «только RTX 50xx»: подходит для любых NVIDIA с драйвером 580+
        assert_eq!(variant_label("cuda-13.3"), "CUDA 13.x (NVIDIA, драйвер 580+)");
        assert!(variant_note("cuda-13.3").contains("RTX 40xx и 50xx"));
        assert!(variant_note("cuda-13.3").contains("580"));
        assert!(variant_note("cuda-12.4").contains("GTX 10xx"));
    }

    fn release_with(names: &[&str]) -> GitHubRelease {
        GitHubRelease {
            tag_name: "b10278".to_string(),
            assets: names
                .iter()
                .map(|n| GitHubAsset {
                    name: n.to_string(),
                    browser_download_url: format!("https://example.com/{}", n),
                    size: 1000,
                    digest: None,
                })
                .collect(),
        }
    }

    #[test]
    fn finds_asset_by_cuda13_family_when_minor_differs() {
        // Релиз содержит cuda-13.7, а мы запросили cuda-13.3 — должен найтись
        let release = release_with(&[
            "llama-b10278-bin-win-cuda-13.7-x64.zip",
            "llama-b10278-bin-win-cpu-x64.zip",
        ]);
        let (asset, actual) = find_engine_asset(&release, VARIANT_CUDA13).unwrap();
        assert_eq!(asset.name, "llama-b10278-bin-win-cuda-13.7-x64.zip");
        assert_eq!(actual, "cuda-13.7");
    }

    #[test]
    fn exact_match_keeps_requested_variant() {
        let release = release_with(&["llama-b10278-bin-win-cuda-13.3-x64.zip"]);
        let (_asset, actual) = find_engine_asset(&release, VARIANT_CUDA13).unwrap();
        assert_eq!(actual, "cuda-13.3");
    }

    #[test]
    fn cpu_falls_back_to_universal_cudart_archive() {
        let release = release_with(&["cudart-llama-bin-win-cuda-12.4-x64.zip"]);
        let (asset, actual) = find_engine_asset(&release, VARIANT_CPU).unwrap();
        assert_eq!(asset.name, "cudart-llama-bin-win-cuda-12.4-x64.zip");
        assert_eq!(actual, "cuda-12.4");
    }

    #[test]
    fn no_asset_returns_none() {
        let release = release_with(&["llama-b10278-bin-win-vulkan-x64.zip"]);
        assert!(find_engine_asset(&release, VARIANT_CUDA13).is_none());
    }

    #[test]
    fn finds_vulkan_asset_by_family() {
        let release = release_with(&["llama-b10331-bin-win-vulkan-x64.zip"]);
        let (asset, actual) = find_engine_asset(&release, VARIANT_VULKAN).unwrap();
        assert_eq!(asset.name, "llama-b10331-bin-win-vulkan-x64.zip");
        assert_eq!(actual, "vulkan");
    }

    #[test]
    fn finds_hip_asset_by_family() {
        let release = release_with(&["llama-b10331-bin-win-hip-radeon-x64.zip"]);
        let (asset, actual) = find_engine_asset(&release, VARIANT_HIP).unwrap();
        assert_eq!(asset.name, "llama-b10331-bin-win-hip-radeon-x64.zip");
        assert_eq!(actual, "hip-radeon");
    }

    #[test]
    fn per_variant_layout_and_migration() {
        let tmp = std::env::temp_dir().join(format!("kingorch_inst_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Старый формат: бинарь и meta в корне
        fs::write(tmp.join("llama-server.exe"), "bin").unwrap();
        fs::write(
            tmp.join("engine_meta.json"),
            r#"{"tag":"b10275","variant":"cuda-12.4","installed_at":"0"}"#,
        )
        .unwrap();

        // Миграция → backends/cuda-12.4/
        assert_eq!(migrate_legacy_layout(&tmp).unwrap().as_deref(), Some("cuda-12.4"));
        assert!(is_installed(&tmp, "cuda-12.4"));
        assert!(!tmp.join("llama-server.exe").exists());
        assert_eq!(installed_meta(&tmp, "cuda-12.4").unwrap().variant, "cuda-12.4");
        assert_eq!(list_installed_variants(&tmp), vec!["cuda-12.4".to_string()]);
        assert!(has_any_installed(&tmp));
        // Повторная миграция — no-op
        assert_eq!(migrate_legacy_layout(&tmp).unwrap(), None);

        // Вторая миграция при отсутствии старого формата
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn lift_server_files_moves_nested_binaries() {
        let tmp = std::env::temp_dir().join(format!("kingorch_lift_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let nested = tmp.join("backends/b9967/win-cuda-13-common_cpus-x64/build/bin");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("llama-server.exe"), "bin").unwrap();
        fs::write(nested.join("ggml-cuda.dll"), "dll").unwrap();

        let log_cb = |_: String| {};
        lift_server_files(&tmp, &log_cb).unwrap();
        assert!(tmp.join("llama-server.exe").exists());
        assert!(tmp.join("ggml-cuda.dll").exists());
        assert!(!nested.join("llama-server.exe").exists());

        let _ = fs::remove_dir_all(&tmp);
    }
}
