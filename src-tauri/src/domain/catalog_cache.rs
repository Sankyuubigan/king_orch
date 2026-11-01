//! Кэш каталога агентов/workflows для hot-путей (01.11.26).
//!
//! `get_prompt_preview` вызывается фронтендом на каждую паузу набора текста
//! (debounce 400 мс) и ДО кэша перечитывал с диска и парсил весь каталог
//! (115 .md + 22 .yaml) на каждый вызов — сотни ресканов в минуту на машине
//! юзера. Кэш с коротким TTL (3 сек) убирает churn: один полный рескан на
//! окно набора. Правки каталога подхватываются не позже TTL, а команда,
//! ПИШУЩАЯ в agents/ (`save_workflow`), инвалидирует кэш явно.
//!
//! Слой: domain (данные кэша — доменные типы; infra не может их держать,
//! а api не имеет права обходить двери). Сам кэш ничего не импортирует из
//! других слоёв — только соседние модули domain.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::agent_manager::{load_agents, AgentProfile};
use super::workflow_engine::parser::load_workflows;
use super::workflow_engine::WorkflowDef;

/// TTL кэша каталога. Короткий осознанно: правка .md/.yaml на диске видна
/// через ≤3 сек даже без явной инвалидации (backstop против забытой
/// инвалидации, не «долгий кэш»).
const CATALOG_TTL: Duration = Duration::from_millis(3000);

/// Снимок каталога: результаты `load_agents` + `load_workflows` на момент загрузки.
pub struct CatalogSnapshot {
    pub agents: Vec<AgentProfile>,
    pub workflows: Vec<WorkflowDef>,
}

static CATALOG_CACHE: Mutex<Option<(PathKey, Instant, Arc<CatalogSnapshot>)>> = Mutex::new(None);

/// Ключ кэша: путь каталога. Путь приходит от `find_agents_dir` (стабилен
/// внутри сессии), сравнение строковое — достаточно.
type PathKey = std::path::PathBuf;

/// Снимок каталога с TTL-кэшем: при живом кэше дискового чтения нет.
///
/// Ошибки НЕ кэшируются: `Err` уходит вызывающему, следующая попытка снова
/// читает диск (прежняя семантика `get_prompt_preview` сохранена).
pub fn catalog_snapshot(agents_dir: &Path) -> Result<Arc<CatalogSnapshot>, String> {
    let mut guard = CATALOG_CACHE
        .lock()
        .map_err(|e| format!("catalog cache lock poisoned: {}", e))?;
    if let Some((dir, loaded_at, snap)) = guard.as_ref() {
        if dir == agents_dir && loaded_at.elapsed() < CATALOG_TTL {
            return Ok(Arc::clone(snap));
        }
    }
    let agents = load_agents(agents_dir)?;
    let workflows = load_workflows(agents_dir)?;
    let snap = Arc::new(CatalogSnapshot { agents, workflows });
    *guard = Some((agents_dir.to_path_buf(), Instant::now(), Arc::clone(&snap)));
    Ok(snap)
}

/// Явная инвалидация (после записи в agents/). Lock-poison не роняет поток:
/// кэш просто останется до конца TTL.
pub fn invalidate_catalog_cache() {
    if let Ok(mut guard) = CATALOG_CACHE.lock() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Несуществующий каталог — НЕ ошибка, а легитимный пустой снимок
    /// (load_agents на отсутствующей папке возвращает Ok(vec![]) —
    /// agent_manager.rs:246; прежний get_prompt_preview получил бы то же
    /// самое без кэша). Повторный вызов консистентен.
    #[test]
    fn missing_dir_snapshot_is_empty() {
        let bad = Path::new("Z:/definitely/not/a/catalog");
        let snap = catalog_snapshot(bad).expect("пустой каталог — не ошибка");
        assert!(snap.agents.is_empty());
        assert!(snap.workflows.is_empty());
        let again = catalog_snapshot(bad).expect("повторный вызов — не ошибка");
        assert!(again.agents.is_empty());
    }

    #[test]
    fn invalidate_clears_entries() {
        invalidate_catalog_cache();
        assert!(CATALOG_CACHE.lock().unwrap().is_none());
    }
}
