use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use regex::Regex;

use super::workflow_engine::parser::load_workflows;

/// Ошибка строгого резолва агента по ссылке.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentResolveError {
    /// Ссылка не соответствует ни одному агенту.
    ///
    /// `team` — команда, внутри которой искали (`None` — глобальный вызов).
    /// Сообщение обязано называть, что именно проверялось: «не найден» без
    /// деталей не даёт починить ссылку.
    NotFound { reference: String, team: Option<String> },
}

impl std::fmt::Display for AgentResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentResolveError::NotFound { reference, team } => match team {
                Some(team) => write!(
                    f,
                    "Агент '{}' не найден (искали '{}/{}', затем '{}')",
                    reference, team, reference, reference
                ),
                None => write!(f, "Агент '{}' не найден", reference),
            },
        }
    }
}

impl std::error::Error for AgentResolveError {}

/// Нормализация ссылки на агента: `\` → `/`, срез ведущих `./` и `/`.
fn normalize_agent_ref(reference: &str) -> String {
    let mut r = reference.trim().replace('\\', "/");
    loop {
        if let Some(rest) = r.strip_prefix("./") {
            r = rest.to_string();
        } else if let Some(rest) = r.strip_prefix('/') {
            r = rest.to_string();
        } else {
            break;
        }
    }
    r
}

/// ЕДИНСТВЕННЫЙ резолв агента по ссылке (26.11.2026, Паттерн B).
///
/// Правила — строгие, без угадывания:
/// * `team = Some(t)`: сначала `a.id == "{t}/{reference}"` (локальная ссылка
///   внутри команды: `frontend/shadow_worker`, `planner`), затем `a.id == reference`
///   (в графе уже передан полный ID или агент другой команды).
/// * `team = None`: только точное `a.id == reference`.
/// * Не найдено → [`AgentResolveError::NotFound`].
///
/// Запрещено: поиск по последнему сегменту пути и любой фоллбэк по `file_stem` —
/// именно такая «доверчивость» позволяла призраку из `archive/` отвечать в чате
/// вместо актуального агента.
pub fn resolve_agent<'a>(
    agents: &'a [AgentProfile],
    team: Option<&str>,
    reference: &str,
) -> Result<&'a AgentProfile, AgentResolveError> {
    let norm = normalize_agent_ref(reference);
    if let Some(team) = team {
        let scoped = format!("{}/{}", team, norm);
        if let Some(agent) = agents.iter().find(|a| a.id == scoped) {
            return Ok(agent);
        }
    }
    agents
        .iter()
        .find(|a| a.id == norm)
        .ok_or_else(|| AgentResolveError::NotFound {
            reference: norm,
            team: team.map(str::to_string),
        })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub mode: String,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default)]
    pub subagents: Vec<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub current_date: bool,
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Зрение: агент получает изображения текущего запроса в LLM-вызов.
    /// Дефолт `false` — vision не включается неявно «просто потому что прикрепили
    /// файл»: это поднимает расход VRAM, роняет llama-server на не-vision моделях
    /// и неявно связывает LLM-вызов с наличием вложений. Включается вручную
    /// во frontmatter (`vision: true`) только тем агентам, которым пиксели
    /// действительно нужны.
    #[serde(default)]
    pub vision: bool,
    /// Граница размышления в символах для сигнальных агентов (`think_budget:`).
    ///
    /// `None` — историческое поведение: think-блок грамматики не ограничен.
    /// Заданное значение зашивает границу в ФОРМАТ ответа
    /// ([`crate::domain::signals::build_signal_envelope_grammar_with_think_budget`]),
    /// то есть делает JSON-фазу структурно достижимой: модель, ушедшая в длинный
    /// разбор, не может выжечь весь `n_predict` и не вернуть сигнал вовсе
    /// (измерено: `tasks/09.10.26 …md` §7).
    #[serde(default)]
    pub think_budget_chars: Option<usize>,
    /// Агент объявляет, что ему не нужно свободное размышление: движок
    /// передаёт `enable_thinking: false`.
    ///
    /// Нужно агентам с жёсткой формой ответа, у которых размышление идёт
    /// ОТДЕЛЬНЫМ каналом (`--reasoning-format`): грамматика этот канал не
    /// ограничивает, поэтому весь `n_predict` может уйти в think, а конверт
    /// сигнала не появится (измерено: `tasks/09.10.26 …md` §7 — 0 токенов ответа
    /// при 2042 токенах думателя). Тот же режим дал 63/63 разобранных ответов у
    /// плоского валидатора (`tasks/08.10.26 …md` §5), против 17/63 с размышлением.
    ///
    /// Дефолт `false` — поведение всех существующих агентов не меняется.
    #[serde(default)]
    pub disable_reasoning: bool,
    /// Контракт оператора: агент ОБЯЗАН выполнить задачу через инструменты;
    /// текстовый финал без единого успешного вызова тула = провал задачи.
    ///
    /// Это решение АВТОРА агента (как `vision:`): у chat/репорт-агентов текст —
    /// легитимный результат, у тул-операторов (`video_maker`, `image_generator`, …)
    /// текст без вызова тула означает, что артефакт не создан. GUARD оркестратора
    /// (`dispatch.rs::retry_native_final_without_tools`) по этому флагу не принимает
    /// «финал-текст» как успех: ретрай с хинтом «вызови инструмент», после исчерпания —
    /// честная `AGENT_ERROR_PREFIX`-ошибка. Дефолт `false` — поведение всех
    /// существующих агентов без объявления не меняется.
    #[serde(default)]
    pub require_tool_call: bool,
    /// Предлагать ли агента в КАТАЛОГЕ (Настройки → «Агенты в чате» → «Доступные агенты»).
    ///
    /// Это решение АВТОРА агента, а не пользователя: `visible: false` убирает запись из
    /// правой колонки каталога, чтобы служебный/экспериментальный агент не мозолил глаза.
    /// Пользовательский выбор «что показывать в чате» — отдельный сигнал и живёт в конфиге
    /// (`agent_visibility`), см. [`AgentEntry`].
    ///
    /// На ЗАПУСК не влияет: диспетчеризация по `agent_id` видимость не проверяет — иначе
    /// скрытый агент остался бы «невидимым, но исполняемым» (то же правило, что для `archive/`).
    ///
    /// Дефолт `true`: агент без объявления виден ровно как раньше, поведение не меняется.
    #[serde(default = "default_visible")]
    pub visible: bool,
}

/// Отсутствие поля `visible` означает «виден»: скрывать агента нужно ЯВНО.
fn default_visible() -> bool {
    true
}

/// Единая точка входа в UI — может быть .md агентом или YAML графом.
///
/// Каталог НЕ фильтруется по видимости: показывать или нет entry point —
/// решение пользователя, хранится в конфиге (`agent_visibility`).
/// Здесь лежит только факт «что вообще существует на диске».
#[derive(Debug, Clone, Serialize)]
pub struct AgentEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub entry_type: String,
    pub folder: Option<String>,
    /// Путь относительно `agents/` (например `coder/primary_coder.md`).
    /// Позволяет UI показать, где лежит агент, и отличить воркер от точки входа.
    pub rel_path: String,
    /// ПОЛНЫЙ абсолютный путь файла на диске — для подсказки при наведении
    /// на агента в выпадающем списке чата: пользователь видит, какой фактически
    /// файл редактирует. Frontend сам его получить не может — он не знает
    /// `agents_dir` (в dev он в репозитории, в релизе — рядом с exe),
    /// поэтому путь приходит готовым. Канонизация выполняется на уровне
    /// базового каталога (`load_entry_points` → `display_abs_dir`): база
    /// раскрывается один раз, пути файлов строятся `join`'ом к ней —
    /// без мусорных `..`/`.`-сегментов и `\\?\`-приставки от Windows.
    pub source_path: String,
    /// Роль агента: `"graph"` — узел workflow-графа, `"agent"` — самостоятельный
    /// legacy-агент (в т.ч. корень `agents/*.md`, запускается напрямую).
    pub role: String,
    /// Предлагать ли запись в КАТАЛОГЕ UI (правая колонка «Доступные агенты»).
    ///
    /// У `.md` агента — frontmatter `visible:` (дефолт `true`), у YAML-графа — поле
    /// верхнего уровня `visible:` (дефолт `true`). Работает одинаково для обоих типов,
    /// поэтому поле лежит здесь, а не в профилях-потомках.
    ///
    /// Каталог отдаёт запись ВСЕГДА (даже скрытую): UI обязан уметь отличить
    /// «агента нет на диске» от «агент скрыт автором», иначе скрытие выглядело бы
    /// как удаление (см. `utils/agent-visibility.ts` — единственный предикат показа).
    pub visible: bool,
    /// Сколько групп графа РЕАЛЬНО схлопывается в Fused.
    ///
    /// Считается по тем же правилам, что и рантайм (явная инструкция либо
    /// авто-склейка ролей), поэтому UI показывает тумблер ровно там, где
    /// переключение имеет смысл: у `.md`-агента и графа без групп здесь ноль, и
    /// тумблер скрыт. Отдельного счёта «на стороне фронта» здесь быть не должно —
    /// он разошёлся бы с движком.
    #[serde(default)]
    pub fusable_groups: usize,
}

impl AgentProfile {
    /// Локальный ID агента — путь ОТНОСИТЕЛЬНО его команды (`folder`).
    ///
    /// `psychotherapist/frontend/shadow_worker` → `frontend/shadow_worker`,
    /// `coder/planner` → `planner`. Для агентов в корне `agents/` (`folder = None`)
    /// совпадает с [`Self::id`].
    ///
    /// Зачем: грамматики (`agents/<team>/grammars/<local_id>.gbnf`) и
    /// signal-контракты (`agents/<team>/signals/root.schema.json` → ключ агента)
    /// адресуются ВНУТРИ команды, поэтому ключом там служит локальный путь, а не
    /// глобальный ID (26.11.2026, Паттерн B).
    pub fn local_id(&self) -> &str {
        match self.folder.as_deref() {
            Some(folder) if !folder.is_empty() => self
                .id
                .strip_prefix(folder)
                .and_then(|rest| rest.strip_prefix('/'))
                .unwrap_or(&self.id),
            _ => &self.id,
        }
    }
}

/// Относительный путь файла внутри `agents/` через `/` (кроссплатформенно).
fn rel_path_of(path: &Path, agents_dir: &Path) -> String {
    path.strip_prefix(agents_dir)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// Канонический ID агента: относительный путь от корня `agents/` без `.md`,
/// разделитель `/` (26.11.2026, Паттерн B).
///
/// Почему путь, а не имя файла: раньше `id = file_stem()`, поэтому два файла с
/// одинаковым именем в разных папках давали ОДИН id, и молчаливый `.find()`
/// брал первый по порядку `read_dir`. Именно так старый
/// `agents/psychotherapist/archive/shadow_worker.md` (призрак от прошлого
/// обновления) побеждал актуальный `frontend/shadow_worker.md` в установленной
/// копии. Путь делает коллизию невозможной по построению: у файлов в разных
/// папках разные ID.
///
/// `agents/psychotherapist/frontend/shadow_worker.md` →
/// `psychotherapist/frontend/shadow_worker`.
fn canonical_agent_id(path: &Path, agents_dir: &Path) -> String {
    let rel = path.strip_prefix(agents_dir).unwrap_or(path);
    let without_ext = rel.with_extension("");
    without_ext
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// Первый сегмент относительного пути — команда (`coder`, `psychotherapist`…),
/// `None` для файлов в корне `agents/`.
fn team_folder_of(path: &Path, agents_dir: &Path) -> Option<String> {
    let rel = path.strip_prefix(agents_dir).ok()?;
    let parent = rel.parent()?;
    parent
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
}

/// Windows-canonicalize добавляет к дисковым путям приставку `\\?\`
/// (у сетевых — `\\?\UNC\`): в подсказке для пользователя это только шум.
fn strip_unc(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// Канонический абсолютный путь директории для построения путей записей.
///
/// `find_agents_dir` в dev возвращает путь с подъёмами (`target/<profile>/../../../agents`):
/// если именно его отдавать в подсказке, пути усеяны `..`-сегментами.
/// `canonicalize` раскрывает `.`/`..`/ссылки один раз — дальше все пути строятся
/// `join`'ом к чистой абсолютной базе и точек не содержат. Падение
/// `canonicalize` (каталог исчез между проверкой и вызовом, сеть) не фатально:
/// отдаём каталог как есть — записи всё равно построились бы с этим путём.
fn display_abs_dir(dir: &Path) -> PathBuf {
    match fs::canonicalize(dir) {
        Ok(canonical) => strip_unc(&canonical),
        Err(_) => dir.to_path_buf(),
    }
}

fn collect_md_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_md_files(&path, files);
            } else if path.extension().map_or(false, |e| e == "md") {
                files.push(path);
            }
        }
    }
}

fn process_includes(base_path: &Path, content: &str) -> String {
    let re = Regex::new(r"<<INCLUDE:\s*(.+?)\s*>>").unwrap();
    re.replace_all(content, |caps: &regex::Captures| {
        let rel_path = caps.get(1).unwrap().as_str().trim();
        let full_path = base_path.join(rel_path);
        if let Ok(file_content) = fs::read_to_string(&full_path) {
            format!("\n<file path=\"{}\">\n<file_content>\n{}\n</file_content>\n</file>\n", rel_path, file_content)
        } else {
            format!("\n<error>Файл {} не найден по пути {}</error>\n", rel_path, full_path.display())
        }
    }).to_string()
}

/// Профили агентов вместе с фактическим путём их `.md`-файла — для каталога
/// entry points. Восстановить путь по `folder + id` нельзя: файл бывает вложен
/// глубже одного каталога (`agents/media/backend/image_editor.md` → folder
/// =`media`, а реконструкция `media/<id>.md` указывает мимо файла).
///
/// Порядок обхода детерминирован (`sort`): `read_dir` не гарантирует порядок, а
/// от него раньше зависело, кто из двух одноимённых агентов победит в
/// молчаливом `.find()`. При канонических ID коллизия невозможна, но
/// детерминизм остаётся: он делает поведение воспроизводимым.
fn load_agents_with_files(agents_dir: &Path) -> Vec<(AgentProfile, PathBuf)> {
    if !agents_dir.exists() { return Vec::new(); }
    let mut md_files = Vec::new();
    collect_md_files(agents_dir, &mut md_files);
    md_files.sort();
    md_files
        .into_iter()
        .filter_map(|path| parse_agent_file(&path, agents_dir).map(|agent| (agent, path)))
        .collect()
}

pub fn load_agents(agents_dir: &Path) -> Result<Vec<AgentProfile>, String> {
    if !agents_dir.exists() { return Ok(Vec::new()); }
    let loaded = load_agents_with_files(agents_dir);
    assert_unique_agent_ids(&loaded)?;
    Ok(loaded.into_iter().map(|(agent, _)| agent).collect())
}

/// Страж коллизий ID: два агента с одинаковым ID (в нижнем регистре — NTFS
/// регистронезависима) означают, что резолв снова стал лотереей.
///
/// При канонических ID (путь от `agents/`) это невозможно по построению; ошибка
/// здесь означала бы баг в формировании ID, и тихо выбирать «первого» нельзя —
/// именно так призрак побеждал актуального агента.
fn assert_unique_agent_ids(loaded: &[(AgentProfile, PathBuf)]) -> Result<(), String> {
    let mut seen: std::collections::HashMap<String, &PathBuf> = std::collections::HashMap::new();
    for (agent, path) in loaded {
        let key = agent.id.to_ascii_lowercase();
        if let Some(first) = seen.get(&key) {
            return Err(format!(
                "Коллизия ID агентов: '{}' и '{}' дают одинаковый ID '{}'. \
                 ID — относительный путь от agents/ без .md, поэтому совпадение означает ошибку \
                 в формировании ID; молча выбрать одного нельзя.",
                first.display(),
                path.display(),
                agent.id
            ));
        }
        seen.insert(key, path);
    }
    Ok(())
}

fn parse_agent_file(path: &Path, agents_dir: &Path) -> Option<AgentProfile> {
    if let Ok(content) = fs::read_to_string(path) {
        let base_dir = path.parent().unwrap_or_else(|| Path::new(""));
        let processed_content = process_includes(base_dir, &content);
        if let Some(mut agent) = parse_agent_markdown(&processed_content) {
            // Канонический ID — путь от корня agents/ (см. canonical_agent_id).
            agent.id = canonical_agent_id(path, agents_dir);
            agent.folder = team_folder_of(path, agents_dir);
            return Some(agent);
        }
    }
    None
}

fn parse_agent_markdown(content: &str) -> Option<AgentProfile> {
    let text = content.trim_start_matches('\u{feff}').trim();
    if text.starts_with("---") {
        if let Some(end_idx) = text[3..].find("---") {
            let frontmatter = &text[3..end_idx + 3];
            let system_prompt = text[end_idx + 6..].trim().to_string();
            let mut name = String::new();
            let mut description = String::new();
            let mut current_date = false;
            let mut temperature: Option<f32> = None;
            let mut vision = false;
            let mut think_budget_chars: Option<usize> = None;
            let mut disable_reasoning = false;
            let mut require_tool_call = false;
            let mut visible = true;
            let mut mcp_servers = Vec::new();
            let mut tools = Vec::new();
            let frontmatter_lines: Vec<&str> = frontmatter.lines().collect();
            let mut i = 0;
            while i < frontmatter_lines.len() {
                let line = frontmatter_lines[i].trim();
                if line.starts_with("name:") { name = line["name:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string(); }
                else if line.starts_with("description:") { description = line["description:".len()..].trim().trim_matches('"').trim_matches('\'').trim().to_string(); }
                // `visible: false` убирает агента из каталога «Доступные агенты».
                // Неизвестное значение (опечатка `fales`, кавычки) НЕ считается
                // скрытием: скрывать нужно явно, иначе опечатка молча спрячет агента.
                else if line.starts_with("visible:") {
                    let raw = line["visible:".len()..].trim().trim_matches('"').trim_matches('\'');
                    match raw.parse::<bool>() {
                        Ok(flag) => visible = flag,
                        Err(_) => log::warn!(
                            "[agents] Поле `visible:` имеет нераспознанное значение '{}' — считаю агента видимым (что видно в списке чата, решает пользователь в конфиге `agent_visibility`)",
                            raw
                        ),
                    }
                }
                else if line.starts_with("current_date:") { current_date = line["current_date:".len()..].trim().parse().unwrap_or(false); }
                else if line.starts_with("temperature:") {
                    temperature = line["temperature:".len()..].trim().parse::<f32>().ok();
                }
                else if line.starts_with("vision:") {
                    vision = line["vision:".len()..].trim().parse().unwrap_or(false);
                }
                else if line.starts_with("think_budget:") {
                    // Ноль и мусор — как «бюджета нет»: грамматика остаётся
                    // исторической, без границы think.
                    think_budget_chars = line["think_budget:".len()..]
                        .trim()
                        .parse::<usize>()
                        .ok()
.filter(|budget| *budget > 0);
                }
                else if line.starts_with("disable_reasoning:") {
                    disable_reasoning = line["disable_reasoning:".len()..]
                        .trim()
                        .parse::<bool>()
                        .unwrap_or(false);
                }
                else if line.starts_with("require_tool_call:") {
                    require_tool_call = line["require_tool_call:".len()..]
                        .trim()
                        .parse::<bool>()
                        .unwrap_or(false);
                }
                else if line.starts_with("mcp_servers:") {
                    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(line["mcp_servers:".len()..].trim()) { mcp_servers = parsed; }
                }
                else if line.starts_with("tools:") {
                    // Предпочтение: JSON-массив имён (`tools: ["code_write"]`).
                    let inline = line["tools:".len()..].trim();
                    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(inline) {
                        tools = parsed;
                    } else {
                        // Robustness: legacy YAML-мапа `tools:\n  write: true\n  bash: true`.
                        // Сопоставляем булевы ключи с мета-наборами / именами тулов.
                        let mut j = i + 1;
                        while j < frontmatter_lines.len() && (frontmatter_lines[j].starts_with(' ') || frontmatter_lines[j].starts_with('\t')) {
                            let kv = frontmatter_lines[j].trim();
                            if let Some((k, v)) = kv.split_once(':') {
                                if let Ok(enabled) = v.trim().parse::<bool>() {
                                    if enabled {
                                        match k.trim() {
                                            "write" | "edit" => tools.push("code_write".to_string()),
                                            "read" => tools.push("code_read".to_string()),
                                            "bash" => tools.push("bash".to_string()),
                                            other => tools.push(other.to_string()),
                                        }
                                    }
                                }
                            }
                            j += 1;
                        }
                        i = j - 1;
                    }
                }
                i += 1;
            }
            if !name.is_empty() { return Some(AgentProfile { id: String::new(), name, description, system_prompt, mode: "worker".to_string(), mcp_servers, subagents: Vec::new(), folder: None, tools, current_date, temperature, vision, think_budget_chars, disable_reasoning, require_tool_call, visible }); }
        }
    }
    None
}

/// Каталог всех существующих entry points для UI: `.md` агенты + YAML-графы.
///
/// ВОЗВРАЩАЕТ ВСЁ, БЕЗ ФИЛЬТРАЦИИ ПО ВИДИМОСТИ — включая записи с `visible: false`.
/// Два независимых сигнала видимости, и оба обязаны быть видны UI:
/// * `visible` (это поле записи) — решение АВТОРА: предлагать ли агента в каталоге;
/// * `agent_visibility` в конфиге — решение ПОЛЬЗОВАТЕЛЯ: что показывать в списке чата.
///
/// Отфильтровать скрытое здесь означало бы соврать UI: «скрыт автором» стало бы
/// неотличимо от «агента нет на диске» (см. `utils/agent-visibility.ts`).
/// Сколько групп графа действительно схлопывается в Fused.
///
/// Критерий — тот же, что у рантайма: либо есть `fused_instruction`, либо роли
/// нод склеиваются (у каждой ноды агент или `input`). Группа с роут��ром внутри
/// сюда не попадает намеренно: она всё равно будет отклонена при запуске, и
/// показывать пользователю тумблер под неё — значит обещать то, что не случится.
fn fusable_groups_count(workflow: &crate::domain::workflow_engine::parser::WorkflowDef) -> usize {
    use crate::domain::workflow_engine::groups;
    workflow
        .groups
        .iter()
        .filter(|group| {
            let explicit = group
                .fused_instruction
                .as_deref()
                .map(str::trim)
                .is_some_and(|i| !i.is_empty());
            let stitchable = groups::check_stitchable(workflow, group).is_ok();
            let macro_shaped =
                groups::ensure_macro_possible(workflow, group).is_ok();
            (explicit || stitchable) && macro_shaped
        })
        .count()
}

pub fn load_entry_points(agents_dir: &Path) -> Vec<AgentEntry> {
    // Подсказка UI обязана быть ПОЛНЫМ путём без `..`-сегментов: базу
    // канонизируем один раз, все записи строятся от неё.
    let agents_dir = &display_abs_dir(agents_dir);
    let mut entries = Vec::new();

    // .md агенты — путь не реконструируется, а берётся фактический из скана
    for (a, path) in load_agents_with_files(agents_dir) {
        entries.push(AgentEntry {
            id: a.id,
            name: a.name,
            description: a.description,
            entry_type: "agent".to_string(),
            folder: a.folder.clone(),
            rel_path: rel_path_of(&path, agents_dir),
            source_path: path.to_string_lossy().to_string(),
            role: "agent".to_string(),
            visible: a.visible,
            fusable_groups: 0,
        });
    }

    // YAML графы
    if let Ok(workflows) = load_workflows(agents_dir) {
        for wf in &workflows {
            let path = PathBuf::from(&wf.parent_dir).join(format!("{}.yaml", wf.file_stem));
            // parent_dir бывает и относительным (тестовые фикстуры) — абсолютный
            // путь обязателен по контракту поля: якорим на agents_dir.
            let source_abs = if path.is_absolute() {
                path.clone()
            } else {
                agents_dir.join(&path)
            };
            entries.push(AgentEntry {
                // ID графа — тот же канонический вид, что у `.md` агентов: путь от
                // корня agents/ без расширения. Иначе выбор графа в UI и
                // legacy-фоллбэк «граф по имени файла» разъезжались бы с резолвом.
                id: canonical_agent_id(&path, agents_dir),
                name: wf.name.clone(),
                description: wf.description.clone().unwrap_or_default(),
                entry_type: "workflow".to_string(),
                folder: team_folder_of(&path, agents_dir),
                rel_path: rel_path_of(&path, agents_dir),
                source_path: source_abs.to_string_lossy().to_string(),
                role: "graph".to_string(),
                visible: wf.visible,
                fusable_groups: fusable_groups_count(wf),
            });
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(content: &str) -> AgentProfile {
        parse_agent_markdown(content).expect("агент должен распарситься")
    }

    #[test]
    fn tools_json_array_parsed() {
        let a = parse("---\nname: Test\ntools: [\"code_write\"]\n---\nbody\n");
        assert_eq!(a.tools, vec!["code_write"]);
    }

    #[test]
    fn tools_yaml_map_parsed_as_legacy_robustness() {
        // Legacy формат: YAML-мапа вместо JSON-массива.
        let a = parse("---\nname: Test\ntools:\n  write: true\n  bash: true\n  read: false\n---\nbody\n");
        assert_eq!(a.tools, vec!["code_write", "bash"], "write→code_write, bash→bash, read:false игнорируется");
    }

    #[test]
    fn tools_after_yaml_map_not_consumed() {
        // Ключи после блока мапы (например description ниже tools) парсятся корректно.
        let a = parse("---\nname: Test\ntools:\n  read: true\ndescription: d\n---\nbody\n");
        assert_eq!(a.tools, vec!["code_read"]);
        assert_eq!(a.description, "d");
    }

        #[test]
    fn vision_defaults_to_false_and_is_opt_in() {
        // Дефолт OFF: зрение не должно включаться неявно «просто потому что
        // юзер прикрепил файл» (это роняло llama-server на не-vision моделях).
        let default_agent = parse("---\nname: Default Agent\n---\nbody\n");
        assert!(!default_agent.vision, "зрение по умолчанию должно быть выключено");

        let explicit_off = parse("---\nname: Off\nvision: false\n---\nbody\n");
        assert!(!explicit_off.vision);

        let vision_agent = parse("---\nname: Vision Agent\nvision: true\n---\nbody\n");
        assert!(vision_agent.vision, "vision: true должен включать зрение");
    }

    #[test]
    fn temperature_parsed_from_frontmatter() {
        let a = parse("---\nname: Primary Coder\ntemperature: 0.1\n---\nbody\n");
        assert_eq!(a.temperature, Some(0.1));

        let b = parse("---\nname: Default Agent\n---\nbody\n");
        assert_eq!(b.temperature, None);
    }

    /// `visible: false` убирает агента из каталога «Доступные агенты»; отсутствие
    /// поля = виден. Мусор в значении НЕ скрывает агента: опечатка не должна тихо
    /// прятать рабочего агента от пользователя.
    #[test]
    fn visible_parsed_and_defaults_to_true() {
        let default_agent = parse("---\nname: Default Agent\n---\nbody\n");
        assert!(default_agent.visible, "без поля visible агент обязан быть видимым");

        let hidden = parse("---\nname: Hidden\nvisible: false\n---\nbody\n");
        assert!(!hidden.visible, "visible: false обязан скрывать агента из каталога");

        let shown = parse("---\nname: Shown\nvisible: true\n---\nbody\n");
        assert!(shown.visible);

        let junk = parse("---\nname: Junk\nvisible: fales\n---\nbody\n");
        assert!(junk.visible, "нераспознанное значение — не скрытие: агент остаётся видимым");
    }

    /// Граница размышления — опция агента, а не глобальная настройка: агент без
    /// объявления сохраняет историческое «think не ограничен».
    #[test]
    fn think_budget_parsed_from_frontmatter() {
        let layer = parse("---\nname: Layer\nthink_budget: 1400\n---\nbody\n");
        assert_eq!(layer.think_budget_chars, Some(1400));

        let plain = parse("---\nname: Plain\n---\nbody\n");
        assert_eq!(plain.think_budget_chars, None);

        // Ноль и мусор трактуются как «бюджета нет»: граница в 0 символов
        // заставила бы модель отвечать без размышления вовсе.
        let zero = parse("---\nname: Zero\nthink_budget: 0\n---\nbody\n");
        assert_eq!(zero.think_budget_chars, None);
        let junk = parse("---\nname: Junk\nthink_budget: много\n---\nbody\n");
        assert_eq!(junk.think_budget_chars, None);
    }

    /// Контракт оператора «обязан вызвать тул» — решение автора агента:
    /// без объявления поведение историческое (текстовый финал легален).
    #[test]
    fn require_tool_call_parsed_and_defaults_to_false() {
        let default_agent = parse("---\nname: Default Agent\n---\nbody\n");
        assert!(!default_agent.require_tool_call, "без поля контракт выключен");

        let off = parse("---\nname: Off\nrequire_tool_call: false\n---\nbody\n");
        assert!(!off.require_tool_call);

        let operator = parse("---\nname: Operator\nrequire_tool_call: true\n---\nbody\n");
        assert!(operator.require_tool_call, "require_tool_call: true обязан включать контракт");

        // Мусор — как отсутствие поля: контракт не должен включаться от опечатки.
        let junk = parse("---\nname: Junk\nrequire_tool_call: да\n---\nbody\n");
        assert!(!junk.require_tool_call);
    }

    /// Страж контракта тул-операторов: владелец media/excel/diag-тула ОБЯЗАН
    /// объявлять `require_tool_call: true` — его текстовый финал без вызова
    /// тула означает «артефакт не создан», и принимать его как успех — тихая
    /// подмена (баг №3, лог 01.11.26). Краснеет, когда новый оператор получает
    /// тул без объявления флага — автор обязан осознанно решить.
    #[test]
    fn tool_operators_declare_require_tool_call() {
        const OPERATOR_TOOLS: &[&str] = &[
            // media (infra/tools: media.rs, video.rs)
            "generate_image", "edit_image", "upscale_image", "generate_video",
            // excel (infra/tools/excel)
            "excel_inspect", "excel_read", "excel_apply", "excel_audit", "excel_render", "excel_convert",
            // diag (infra/tools/diag)
            "diag_snapshot", "diag_recorder_start", "diag_recorder_stop",
        ];
        let agents_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .join("agents");
        let agents = load_agents(&agents_dir).expect("агенты прочитаны");
        assert!(!agents.is_empty(), "агенты должны загрузиться из agents/");

        for agent in &agents {
            if let Some(tool) = agent.tools.iter().find(|t| OPERATOR_TOOLS.contains(&t.as_str())) {
                assert!(
                    agent.require_tool_call,
                    "агент '{}' ({}) владеет тулом-оператором '{}' без require_tool_call: true — \
                     финал текстом без вызова тула принимался бы как успех при несозданном артефакте",
                    agent.id, agent.name, tool
                );
            }
            // Встречное: флаг без единого тула — неисполнимый контракт (ловушка).
            if agent.require_tool_call {
                assert!(
                    !agent.tools.is_empty(),
                    "агент '{}' объявил require_tool_call: true без единого тула — контракт неисполним",
                    agent.id
                );
            }
        }

        // Ключевые операторы существуют и объявили флаг (тест не проходит тихо
        // при переименовании/удалении любого из них). ID — канонические пути.
        for id in [
            "video-gen/backend/video_maker",
            "image-gen/backend/image_generator",
            "image-gen/backend/image_editor",
            "image-gen/backend/image_upscaler",
            "excel/backend/excel_operator",
            "diagnostics/backend/diag_specialist",
        ] {
            let agent = resolve_agent(&agents, None, id)
                .unwrap_or_else(|e| panic!("{}", e));
            assert!(
                agent.require_tool_call,
                "оператор '{}' обязан объявлять require_tool_call: true",
                id
            );
        }
    }

    /// `find_agents_dir` в dev возвращает путь с подъёмами
    /// (`target/<profile>/../../../agents`): если его отдавать в подсказку,
    /// пути усеяны `..`. Контракт `source_path` — ПОЛНЫЙ путь без
    /// `..`-сегментов и без `\\?\`-приставки: фронт показывает его как есть.
    fn agents_dir_dotted() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .join("src-tauri")
            .join("..")
            .join("agents")
    }

    #[test]
    fn display_abs_dir_resolves_dot_segments() {
        let clean = display_abs_dir(&agents_dir_dotted());
        let shown = clean.to_string_lossy().to_string();
        assert!(clean.is_absolute(), "путь подсказки обязан быть абсолютным: {shown}");
        assert!(!shown.contains(".."), "подъёмы раскрыты: {shown}");
        assert!(!shown.starts_with(r"\\?\"), "UNC-приставка срезана: {shown}");
        assert_eq!(
            clean,
            display_abs_dir(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .join("agents")
            ),
            "раскрытие `..` совпадает с прямой папкой"
        );
    }

    #[test]
    fn entry_source_paths_are_full_clean_file_paths() {
        let dotted = agents_dir_dotted();
        assert!(dotted.exists(), "фикстура — настоящая папка агентов репозитория");
        let entries = load_entry_points(&dotted);
        assert!(!entries.is_empty(), "каталог агентов не пуст");
        for entry in &entries {
            let source = PathBuf::from(&entry.source_path);
            assert!(
                source.is_absolute(),
                "source_path обязан быть абсолютным: {}",
                entry.source_path
            );
            assert!(
                !entry.source_path.contains(".."),
                "без подъёмов в подсказке: {}",
                entry.source_path
            );
            assert!(
                !entry.source_path.starts_with(r"\\?\"),
                "без UNC-приставки: {}",
                entry.source_path
            );
            assert!(
                fs::metadata(&source).map(|m| m.is_file()).unwrap_or(false),
                "путь указывает на фактический файл: {}",
                entry.source_path
            );
            assert!(
                !entry.rel_path.starts_with('/') && !entry.rel_path.starts_with('\\'),
                "rel_path остаётся относительным: {}",
                entry.rel_path
            );
        }
    }

    /// Файлы отчётов и замеров лежат под `agents/`, потому что относятся к этому
    /// агенту, но агентами быть не должны — иначе появятся в списке агентов.
    ///
    /// Защита от тихой поломки: сегодня файл без заголовка `---` просто
    /// отбрасывается, но стоит кому-то дописать заголовок по привычке — и файл
    /// станет агентом. Тест ловит это на сборке, а не в интерфейсе.
    #[test]
    fn experiment_reports_are_not_agents() {
        let agents_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .join("agents");
        let loaded = load_agents(&agents_dir).expect("агенты прочитаны");
        for report in [
            "agents/psychotherapist/experiments/validator_tests.md",
            "agents/psychotherapist/archive/old_archive_info.md",
        ] {
            let stem = std::path::Path::new(report)
                .file_stem()
                .expect("имя файла")
                .to_string_lossy()
                .to_string();
            assert!(
                !loaded.iter().any(|agent| agent.id == stem),
                "файл результатов '{}' загрузился как агент `{}` — уберите заголовок `---`",
                report,
                stem
            );
        }
    }

    /// Страж развода бизнес-логики и тул-механики (03.11.26): `.md` агента —
    /// это роль/политика/формат ответа. МЕХАНИКА вызова тулов живёт в схемах
    /// тулов (движок рендерит их в [ДОСТУПНЫЕ ИНСТРУМЕНТЫ] / [ПРАВИЛА ВЫЗОВА
    /// ИНСТРУМЕНТОВ] / [NATIVE TOOL CALLING]) и в signal-контрактах
    /// (`agents/<команда>/signals/root.schema.json` — движок рендерит их в
    /// [ТРЕБОВАНИЕ СИГНАЛА] для облака, локально принуждает грамматикой).
    /// Форматы `{"thought" ...}`/`{"tool" ...}` в промптах — легаси-конверты,
    /// противоречащие нативному tool calling'у движка.
    #[test]
    fn agent_md_files_have_no_tool_mechanics() {
        let agents_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .join("agents");
        // Каталоги со справочниками/отчётами — не бизнес-логика агентов.
        let skip_dirs = ["archive", "database", "experiments", "source"];
        let mut checked = 0usize;
        let mut files: Vec<PathBuf> = Vec::new();
        let mut dirs: Vec<PathBuf> = vec![agents_dir.clone()];
        while let Some(d) = dirs.pop() {
            let entries = match fs::read_dir(&d) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if skip_dirs.contains(&name) {
                        continue;
                    }
                    dirs.push(p);
                } else if p.extension().and_then(|e| e.to_str()) == Some("md") {
                    files.push(p);
                }
            }
        }
        assert!(files.len() > 50, "каталог агентов прочитан ({} .md)", files.len());
        for f in &files {
            let text = fs::read_to_string(f).unwrap_or_default();
            // Тело без frontmatter: механика в YAML-шапке (`tools: ["emit_signal"]`)
            // — декларация capability, а не инструкция в промпте.
            let body = match text.find("\n---\n") {
                Some(i) if text.starts_with("---\n") => &text[i + 5..],
                _ => text.as_str(),
            };
            for (ln, line) in body.lines().enumerate() {
                // «конвертирует/сконвертировать» — легитимные слова; «конверт»
                // (конверта/конвертом/конверте/конвертов) — легаси-конверты.
                let konvert = line.contains("конверт") && !line.contains("конверти");
                let hit = line.contains("{\"thought\"")
                    || line.contains("{\"tool\"")
                    || line.contains("JSON-конверт")
                    || line.contains("СОХРАНЕНИЕ СИГНАЛ")
                    || konvert;
                assert!(
                    !hit,
                    "тул-механика в бизнес-логике {}:{} (строка {ln}): {} — формат вызова \
                     живёт в схеме тула/signal-контракте, движок прикрепляет его сам",
                    f.display(),
                    ln + 1,
                    line.trim()
                );
            }
            checked += 1;
        }
        assert!(checked > 50, "проверено файлов: {checked}");
    }

    // ────────────────────────────────────────────────────────────────────
    // Стражи канонических ID (26.11.2026, Паттерн B).
    //
    // Причина, по которой эти тесты существуют: в установленной копии в чате
    // психотерапевта отвечал призрак `archive/shadow_worker.md` вместо
    // актуального `frontend/shadow_worker.md`. Причина была не в одном файле,
    // а в трёх молчаливых допущениях сразу: `id = file_stem()`, `read_dir` без
    // сортировки и `.find()` без проверки единственности. Ниже — по стражу на
    // каждое, чтобы вернуть любую из них можно было только уронив сборку.
    // ────────────────────────────────────────────────────────────────────

    fn agents_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .join("agents")
    }

    /// ID агента — путь от `agents/` без расширения, а не имя файла.
    ///
    /// Проверяется на РЕАЛЬНОЙ раскладке: у каждого загруженного агента ID обязан
    /// совпадать с его путём, а `folder` — с первым сегментом. Тест ловит возврат
    /// к `file_stem` даже если кто-то «всего лишь» поменяет одну строку в
    /// `parse_agent_file`.
    #[test]
    fn agent_ids_are_canonical_relative_paths() {
        let root = agents_root();
        let agents = load_agents(&root).expect("агенты прочитаны");
        assert!(agents.len() > 50, "каталог агентов прочитан ({} шт.)", agents.len());

        for agent in &agents {
            let parts: Vec<&str> = agent.id.split('/').collect();
            assert!(
                !agent.id.is_empty(),
                "пустой ID: агент без пути не может быть загружен"
            );
            assert!(
                !agent.id.ends_with(".md"),
                "ID '{}' содержит расширение — канонизация потеряна",
                agent.id
            );
            assert!(
                !agent.id.contains('\\'),
                "ID '{}' содержит обратный слэш: разделитель обязан быть '/'",
                agent.id
            );
            // Корневые агенты (`agents/basic.md`) легальны: id = file_stem, folder = None.
            // У остальных первый сегмент пути — команда.
            if parts.len() == 1 {
                assert!(
                    agent.folder.is_none(),
                    "корневой агент '{}' не может иметь команду '{:?}'",
                    agent.id,
                    agent.folder
                );
            } else {
                assert_eq!(
                    agent.folder.as_deref(),
                    Some(parts[0]),
                    "folder '{}' не совпадает с первым сегментом ID '{}'",
                    agent.folder.as_deref().unwrap_or("<none>"),
                    agent.id
                );
            }
            // local_id — ID без префикса команды (у корневых агентов совпадает с id).
            let expected_local = if parts.len() == 1 { agent.id.clone() } else { parts[1..].join("/") };
            assert_eq!(
                agent.local_id(),
                expected_local,
                "local_id агента '{}' обязан быть его путём внутри команды",
                agent.id
            );
        }
    }

    /// Одноимённые файлы в разных подпапках — РАЗНЫЕ агенты.
    ///
    /// Прямой страж исходного бага: раньше оба файла давали id `shadow_worker`
    /// (по `file_stem`) и побеждал тот, который `read_dir` отдал первым.
    /// В репозитории живут `archive/shadow_worker_old_version.md` и
    /// `frontend/shadow_worker.md` — разные файлы, которые при старом правиле
    /// были бы разными id, а при совпадении имён стали бы лотереей.
    ///
    /// Проверяются оба свойства сразу: (1) призрак и актуальный агент — разные
    /// записи каталога; (2) одинаковый `local_id` у двух агентов возможен ТОЛЬКО
    /// при разных путях.
    #[test]
    fn same_filename_in_different_subdirs_never_collides() {
        let root = agents_root();
        let agents = load_agents(&root).expect("агенты прочитаны");

        // Реальный кейс инцидента: архивный и рабочий shadow-агент одной команды.
        let ghosts: Vec<&str> = agents
            .iter()
            .filter(|a| a.folder.as_deref() == Some("psychotherapist"))
            .filter(|a| a.local_id().starts_with("archive/shadow_worker"))
            .map(|a| a.id.as_str())
            .collect();
        let actual: Vec<&str> = agents
            .iter()
            .filter(|a| a.id == "psychotherapist/frontend/shadow_worker")
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(
            ghosts.len(),
            1,
            "ожидался ровно один архивный shadow-агент, найдено {:?}",
            ghosts
        );
        assert_eq!(
            actual.len(),
            1,
            "актуальный shadow_worker обязан быть ровно один, найдено {:?}",
            actual
        );
        assert_ne!(
            ghosts[0], actual[0],
            "архивный и рабочий агент обязаны иметь РАЗНЫЕ ID — иначе резолв снова лотерея"
        );

        // Общий инвариант: одинаковый local_id у разных агентов допустим ТОЛЬКО
        // при разных путях (иначе это и есть та самая коллизия).
        let mut by_local: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
        for a in &agents {
            by_local.entry(a.local_id()).or_default().push(&a.id);
        }
        for (local, ids) in by_local {
            let unique: std::collections::HashSet<&&str> = ids.iter().collect();
            assert_eq!(
                unique.len(),
                ids.len(),
                "local_id '{}' встречается дважды как один и тот же ID: {:?}",
                local,
                ids
            );
        }
    }

    /// Резолв строгий: нет фоллбэка по последнему сегменту пути.
    ///
    /// Именно «доверчивый» резолв (по file_stem, по basename) позволял призраку
    /// из `archive/` подхватиться вместо актуального агента. Ссылка, которой нет,
    /// обязана быть ОШИБКОЙ, а не «найдём что-нибудь похожее».
    #[test]
    fn resolve_agent_is_strict_and_team_scoped() {
        let root = agents_root();
        let agents = load_agents(&root).expect("агенты прочитаны");

        // Локальная ссылка внутри команды резолвится в агента ИМЕННО этой команды.
        let scoped = resolve_agent(&agents, Some("psychotherapist"), "frontend/shadow_worker")
            .expect("актуальный shadow_worker резолвится");
        assert_eq!(scoped.id, "psychotherapist/frontend/shadow_worker");

        // Голое имя без пути — не резолвится вовсе (нет фоллбэка по basename).
        let bare = resolve_agent(&agents, Some("psychotherapist"), "shadow_worker");
        assert!(
            bare.is_err(),
            "ссылка 'shadow_worker' не может резолвиться: резолв обязан требовать путь \
             (иначе призрак archive/ снова стал бы кандидатом)"
        );

        // Глобальный резолв по полному ID работает.
        let global = resolve_agent(&agents, None, "psychotherapist/frontend/shadow_worker")
            .expect("глобальный резолв по каноническому ID");
        assert_eq!(global.id, scoped.id);

        // Несуществующий агент — ошибка, называющая ссылку и команду.
        match resolve_agent(&agents, Some("psychotherapist"), "frontend/nope_agent") {
            Err(AgentResolveError::NotFound { reference, team }) => {
                assert_eq!(reference, "frontend/nope_agent");
                assert_eq!(team.as_deref(), Some("psychotherapist"));
            }
            Ok(a) => panic!("ожидалась ошибка, а резолв вернул '{}'", a.id),
        }
    }

    /// Нормализация ссылки: `\`, ведущие `./` и `/` снимаются.
    ///
    /// Windows-путь в YAML (`frontend\shadow_worker`) не должен молча не
    /// находиться: автор графа пишет в редакторе на Windows, и обратный слэш —
    /// нормальный ввод.
    #[test]
    fn agent_ref_normalization_handles_windows_paths() {
        let root = agents_root();
        let agents = load_agents(&root).expect("агенты прочитаны");
        for reference in [
            "frontend/shadow_worker",
            "frontend\\shadow_worker",
            "./frontend/shadow_worker",
            "/frontend/shadow_worker",
            "  frontend/shadow_worker  ",
        ] {
            let a = resolve_agent(&agents, Some("psychotherapist"), reference)
                .unwrap_or_else(|e| panic!("ссылка {:?} обязана резолвиться: {}", reference, e));
            assert_eq!(a.id, "psychotherapist/frontend/shadow_worker");
        }
    }

    /// Каталог UI: дублей ID нет.
    ///
    /// `load_entry_points` — то, что видит пользователь. Если два агента дадут
    /// одинаковый ID, в списке будет один пункт, а запустится «какой-то» из двух.
    #[test]
    fn entry_point_ids_are_unique() {
        let entries = load_entry_points(&agents_root());
        assert!(entries.len() > 50, "каталог прочитан ({} записей)", entries.len());
        let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for e in &entries {
            let key = e.id.to_ascii_lowercase();
            if let Some(prev) = seen.insert(key, e.source_path.clone()) {
                panic!(
                    "дубль ID '{}': '{}' и '{}' — в списке агентов это один пункт, а запустится \
                     непредсказуемый из двух",
                    e.id, prev, e.source_path
                );
            }
        }
    }
}
