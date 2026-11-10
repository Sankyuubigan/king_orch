use crate::domain::agent_manager::AgentProfile;
use crate::infra::{ChatAttachment, ChatMessage};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// Агент потребляет каталог изображений (нужен список `hist_*` / `req_*` id
/// вложений сессии). SSOT для ранее размножившегося условия, где у апскейлера
/// каталог не появлялся вовсе (`tool == "edit_image"` ×6: orchestrator/mod.rs
/// ×4, api/chat.rs ×2 — см. tasks/21.10.26 …md §1.1). `generate_video` тоже:
/// без каталога видео-агент подбирал ID вслепую по имени файла (сессия
/// 1793324737511 — попытка 2 из 3 ушла на «Неизвестный source_image_id»).
pub fn agent_consumes_image_catalog(agent: &AgentProfile) -> bool {
    agent.tools.iter().any(|tool| {
        tool == "edit_image" || tool == "upscale_image" || tool == "generate_video"
    })
}

/// Агент получает маркер активного бандла (`[АКТИВНЫЙ БОНДЛ]`) — гейт
/// image-тулов, БЕЗ `generate_video`. FLUX (01.11.26): генератор
/// (`generate_image`) каталог изображений не читает, но с третьей базой
/// (flux) выбор LoRA-стека обязан опираться на ФАКТ активного бандла, а не
/// на угадывание по тексту запроса. Каталог-агентам (`edit_image` /
/// `upscale_image`) маркер идёт как раньше — вместе с каталогом;
/// generate-only агенту — БЕЗ блока каталога (`media_prompt_block("",
/// marker)` — тест `media_prompt_block_marker_alone_without_catalog`).
pub fn agent_receives_bundle_marker(agent: &AgentProfile) -> bool {
    agent
        .tools
        .iter()
        .any(|tool| tool == "generate_image" || tool == "edit_image" || tool == "upscale_image")
}

// ─────────────── F5 (29.10.26): маркер активного бандла ───────────────
// Корень инцидентов F4: агент НЕ ЗНАЕТ, какой бандл активен — знания о базе
// жили только в хвосте правил, а чек-лист говорил «теги обязательны»
// универсально. PRP F5 §3.2: runtime-контекст = данные, правила = текст —
// агент получает record-факт активного бандла тем же механизмом, что и
// блок `[ДОСТУПНЫЕ ИЗОБРАЖЕНИЯ]` (F1). SSOT один: `image_models_catalog.json`
// + `app_config.json` (та же цепочка, что у тулов: `media.rs::
// resolve_bundle_for_tools` / `media_lora.rs::active_lora_base`).

/// Факты активного бандла: marker-текст (инжектится в системный промпт) +
/// отдельные поля для лога.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveBundleFacts {
    /// Текст маркера — ровно то, что получит агент.
    pub marker: String,
    /// Имя бандла в каталоге (ключ `selected_bundle`).
    pub name: String,
    /// База (`lora_base` каталога): qwen / krea2 / - (не задана).
    pub base: String,
    /// ref-edit-режим каталога: native / edit_lora / нет.
    pub ref_edit: String,
}

impl ActiveBundleFacts {
    /// Record-факт активного бандла из записи каталога (НЕ инструкция:
    /// ветвление правил живёт в `image_editor.md`, по маркеру).
    fn from_entry(entry: &crate::infra::ImageBundleEntry) -> Self {
        let base = entry
            .lora_base
            .as_deref()
            .map(str::trim)
            .filter(|base| !base.is_empty());
        let base = base.unwrap_or("-");
        let base_line = if base == "-" {
            "base=- (в каталоге не указана)".to_string()
        } else {
            format!("base={}", base)
        };
        let ref_edit = match entry.ref_edit_mode {
            Some(crate::infra::RefEditMode::Native) => "native",
            Some(crate::infra::RefEditMode::EditLora) => "edit_lora",
            None => "нет",
        };
        let ref_edit_line = match entry.ref_edit_mode {
            Some(crate::infra::RefEditMode::Native) => {
                "ref_edit=native — референс-conditioning вшит в стек".to_string()
            }
            Some(crate::infra::RefEditMode::EditLora) => {
                let lora = entry.ref_edit_lora.as_deref().unwrap_or("");
                let preset = entry.ref_edit_preset.as_deref().unwrap_or("");
                format!(
                    "ref_edit=edit_lora — identity-лора {} подключается движком сама \
                     (preset={}, сила {:.2}; сила живёт в каталоге, не в сетке агента)",
                    if lora.is_empty() { "(стем не указан)" } else { lora },
                    if preset.is_empty() { "(не задан)" } else { preset },
                    entry.ref_edit_lora_strength
                )
            }
            None => "ref_edit=нет — редактирование по референсам не поддержано".to_string(),
        };
        let bundle_line = if entry.label.trim().is_empty() {
            format!("bundle={}", entry.name)
        } else {
            format!("bundle={} — «{}»", entry.name, entry.label)
        };
        let marker = format!(
            "[АКТИВНЫЙ БОНДЛ]\n{}\n{}\n{}",
            bundle_line, base_line, ref_edit_line
        );
        Self {
            marker,
            name: entry.name.clone(),
            base: base.to_string(),
            ref_edit: ref_edit.to_string(),
        }
    }
}

/// Активный бандл SSOT-цепочки по готовности файлов. Выбор юзера (
/// `selected_bundle` из конфига) → дефолт каталога; файлы бандла не
/// докачаны / бандл отсутствует в каталоге → `None`.
///
/// Fail-soft по PRP F5 §3.3: маркер — добавление к промпту, не условие
/// работоспособности; при деградации агент живёт как раньше, а причина
/// фиксируется в логе (правило честности §2.2) — боевая сессия не падает.
/// Полнота файлов проверяется как у `resolve_bundle_for_tools`:
/// маркер описывает работоспособный бандл, а не ожидание закачки.
fn marker_facts_from_selection(
    selected: Option<&str>,
    bundle_dir: &std::path::Path,
) -> Option<ActiveBundleFacts> {
    let selected = selected.map(str::trim).filter(|selected| !selected.is_empty());
    let entry = match selected {
        Some(name) => match crate::infra::bundle_by_name(name) {
            Some(entry) => entry,
            None => {
                log::warn!(
                    "[IMG] [АКТИВНЫЙ БОНДЛ] не собран: бандл «{}» (активный) в каталоге отсутствует",
                    name
                );
                return None;
            }
        },
        None => match crate::infra::default_bundle_entry() {
            Some(entry) => entry,
            None => {
                log::warn!(
                    "[IMG] [АКТИВНЫЙ БОНДЛ] не собран: в каталоге нет бандла по умолчанию"
                );
                return None;
            }
        },
    };
    let missing: Vec<&str> = entry
        .files
        .iter()
        .filter(|file| !bundle_dir.join(&file.filename).exists())
        .map(|file| file.filename.as_str())
        .collect();
    if !missing.is_empty() {
        log::warn!(
            "[IMG] [АКТИВНЫЙ БОНДЛ] не собран: у «{}» не скачаны файлы {}",
            entry.name,
            missing.join(", ")
        );
        return None;
    }
    Some(ActiveBundleFacts::from_entry(&entry))
}

/// Факты активного бандла в момент вызова: текущий конфиг хоста (`app_config.
/// json`) + папка бандлов. `None` — маркера в промпте не будет (fail-soft).
pub fn active_image_bundle_facts() -> Option<ActiveBundleFacts> {
    let cfg = crate::infra::load_image_config_early();
    marker_facts_from_selection(
        cfg.selected_bundle.as_deref(),
        &crate::infra::bundle_dir_early(),
    )
}

/// Маркер активного бандла (F5) — текст `[АКТИВНЫЙ БОНДЛ]` для системного
/// промпта меди-агента. Точка вызова: сборка системного промпта меди-агента (
/// `orchestrator::run_agent_node`) и превью промпта (`api/chat.rs`).
pub fn active_image_bundle_marker() -> Option<String> {
    active_image_bundle_facts().map(|facts| facts.marker)
}

/// Сборка меди-блока системного промпта (та же точка, где строился каталог
/// изображений): каталог `[ДОСТУПНЫЕ ИЗОБРАЖЕНИЯ]` + маркер `[АКТИВНЫЙ БОНДЛ]`
/// СРАЗУ ПОСЛЕ каталога. Порядок частей зафиксирован тестами. Пустые части
/// не порождают пустых разделов.
pub(crate) fn media_prompt_block(catalog: &str, marker: Option<&str>) -> String {
    let mut block = String::new();
    if !catalog.is_empty() {
        block.push_str(catalog);
    }
    if let Some(marker) = marker {
        if !block.is_empty() {
            block.push_str("\n\n");
        }
        block.push_str(marker);
    }
    block
}

/// Относительный путь документации баз от корня agents/ (решение юзера
/// 01.11.26: документация по каждой модели — отдельный файл).
const BRANCH_DOCS_REL: &str = "image-gen/backend/branches";

/// Документация активной базы (`[ВЕТКА БАЗЫ]`) — правила адресации
/// референсов и LoRA-стемы КОНКРЕТНОГО стека, `agents/<BRANCH_DOCS_REL>/
/// <base>.md`. Инжектится хостом только для активной базы (§00.2): выбор
/// ветки делает код, а не модель — чужие базы в контексте агента физически
/// отсутствуют, смешение веток структурно невозможно. Fail-soft: файла нет —
/// блока нет (агент живёт по общим правилам), причина — в логе.
pub fn branch_doc_block(base: &str) -> Option<String> {
    branch_doc_block_from(&crate::infra::find_agents_dir_early(), base)
}

fn branch_doc_block_from(agents_root: &std::path::Path, base: &str) -> Option<String> {
    let base = base.trim();
    // base уходит в построение файлового пути; имена баз из каталога —
    // только [a-z0-9_-] (защита от traversal/мусора до join).
    let valid = !base.is_empty()
        && base
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !valid {
        log::warn!("[IMG] [ВЕТКА БАЗЫ] пропущена: некорректное имя базы «{}»", base);
        return None;
    }
    let path = agents_root.join(BRANCH_DOCS_REL).join(format!("{}.md", base));
    match std::fs::read_to_string(&path) {
        Ok(doc) if !doc.trim().is_empty() => {
            Some(format!("[ВЕТКА БАЗЫ: {}]\n{}", base, doc.trim()))
        }
        Ok(_) => {
            log::warn!("[IMG] [ВЕТКА БАЗЫ] файл пустой: {}", path.display());
            None
        }
        Err(err) => {
            log::warn!("[IMG] [ВЕТКА БАЗЫ] файл не прочитан ({}): {}", path.display(), err);
            None
        }
    }
}

/// ЕДИНЫЙ сборщик меди-блока системного промпта — SSOT для оркестратора
/// (`run_agent_node`) и превью (`api/chat.rs`): превью показывает ровно тот
/// промпт, который получит модель. Порядок частей: каталог `[ДОСТУПНЫЕ
/// ИЗОБРАЖЕНИЯ]` → маркер `[АКТИВНЫЙ БОНДЛ]` → ветка `[ВЕТКА БАЗЫ]`.
/// Пустые части не порождают пустых разделов.
pub fn full_media_prompt_block(catalog: &str, facts: Option<&ActiveBundleFacts>) -> String {
    let mut block = media_prompt_block(catalog, facts.map(|item| item.marker.as_str()));
    if let Some(facts) = facts {
        if let Some(branch) = branch_doc_block(&facts.base) {
            if !block.is_empty() {
                block.push_str("\n\n");
            }
            block.push_str(&branch);
        }
    }
    block
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ImageLocator {
    Request(usize),
    Message {
        message_id: String,
        message_index: usize,
        attachment_index: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ImageOrigin {
    CurrentRequest,
    History,
}

#[derive(Clone, Debug)]
struct ImageCandidate {
    /// Реальный id (легаси-схема: hex-хвост). Алиасы не стабильны между
    /// прогонами (нумерация от старых к новым), поэтому при сравнении
    /// вне одного build'а («id выживает удаление соседа») — надёжнее hex.
    id: String,
    /// Короткий алиас для промпта модели (F1, 29.10.26): модель получает
    /// ТОЛЬКО алиасы (`req_0`, `hist_01`…), сырой hex покидает хост только
    /// как легаси-путь резолва. Модель перестаёт пересказывать 16
    /// случайных hex-символов — источник зацикливания image_editor
    /// (сессия 1793282510012) устранён конструктивно.
    alias: String,
    locator: ImageLocator,
    origin: ImageOrigin,
    message_id: String,
    author: String,
    file_name: String,
    mime_type: String,
    context: String,
}

#[derive(Clone, Debug, Default)]
pub struct RequestMedia {
    request_attachments: Vec<ChatAttachment>,
    candidates: Vec<ImageCandidate>,
    /// Аудио-вложения текущего запроса (backing store для `aud_req_NN`).
    audio_request_attachments: Vec<ChatAttachment>,
    /// Аудио-кандидаты (mime audio/*): сэмплы голоса для voice-фн refmod-тулов
    /// (Этап 3). Алиасы `aud_req_NN` / `aud_hist_NN` — не пересекаются с
    /// image-кандидатами (`req_NN` / `hist_NN`).
    audio_candidates: Vec<ImageCandidate>,
    /// Все вложения (любого формата) текущего запроса + истории — структурные
    /// ссылки `{file_name, mime_type, file_path, is_dir, origin}` для графа
    /// (`user_message.output.attachments`) и allow-read тулов. Политика
    /// видимости 28.10.2026: не-image вложения — не «мусор», а полноправные
    /// данные с путями.
    attachment_refs: Vec<serde_json::Value>,
}

impl RequestMedia {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn build(request_attachments: &[ChatAttachment], messages: &[ChatMessage]) -> Self {
        let current_user = messages
            .iter()
            .rev()
            .find(|m| m.author.as_deref() == Some("user"));
        let current_user_id = current_user
            .and_then(|m| m.id.as_deref())
            .unwrap_or("")
            .to_string();
        let current_user_context = current_user
            .map(|m| compact_context(&m.content))
            .unwrap_or_default();
        let has_current_images = request_attachments
            .iter()
            .any(|attachment| attachment.mime_type.starts_with("image/"));
        let has_current_audios = request_attachments
            .iter()
            .any(|attachment| attachment.mime_type.starts_with("audio/"));
        let mut candidates = Vec::new();
        let mut stored_request_attachments = Vec::new();
        let mut audio_candidates = Vec::new();
        let mut stored_audio_attachments = Vec::new();
        let mut attachment_refs = Vec::new();
        // Алиасы истории от старых к новым (порядок обхода messages). Две
        // цифры ровно: `hist_01`…`hist_25` — канонические виды алиасов не
        // содержатся друг в друге («hist01» ⊄ «hist02»), так что стадия
        // embedded-подстроки в lookup_id не порождает ложных матчей между
        // алиасами (у одноцифровой схемы `hist_1`/`hist_10` было бы).
        let mut history_sequence = 0usize;
        let mut audio_history_sequence = 0usize;

        for (index, attachment) in request_attachments.iter().enumerate() {
            attachment_refs.push(serde_json::json!({
                "file_name": attachment.file_name,
                "mime_type": attachment.mime_type,
                "file_path": attachment.file_path,
                "is_dir": attachment.is_dir.unwrap_or(false),
                "origin": "current",
            }));
            if attachment.mime_type.starts_with("audio/") {
                // Сэмпл голоса (Этап 3): алиас `aud_req_NN`, позиция в
                // audio_request_attachments = locator Request(index).
                audio_candidates.push(ImageCandidate {
                    id: format!("aud_req_{index}"),
                    alias: format!("aud_req_{index}"),
                    locator: ImageLocator::Request(stored_audio_attachments.len()),
                    origin: ImageOrigin::CurrentRequest,
                    message_id: current_user_id.clone(),
                    author: "user".to_string(),
                    file_name: attachment.file_name.clone(),
                    mime_type: attachment.mime_type.clone(),
                    context: current_user_context.clone(),
                });
                stored_audio_attachments.push(attachment.clone());
                continue;
            }
            if !attachment.mime_type.starts_with("image/") {
                continue;
            }
            let candidate = ImageCandidate {
                id: format!("req_{index}"),
                alias: format!("req_{index}"),
                locator: ImageLocator::Request(stored_request_attachments.len()),
                origin: ImageOrigin::CurrentRequest,
                message_id: current_user_id.clone(),
                author: "user".to_string(),
                file_name: attachment.file_name.clone(),
                mime_type: attachment.mime_type.clone(),
                context: current_user_context.clone(),
            };
            stored_request_attachments.push(attachment.clone());
            candidates.push(candidate);
        }

        for (message_index, message) in messages.iter().enumerate() {
            if message.msg_type == "thought" || message.msg_type == "signal" {
                continue;
            }
            let is_current_user_message =
                message.id.as_deref() == Some(current_user_id.as_str());
            if (has_current_images || has_current_audios) && is_current_user_message {
                continue;
            }
            let Some(attachments) = message.attachments.as_ref() else {
                continue;
            };
            let message_id = message
                .id
                .clone()
                .unwrap_or_else(|| format!("message_{message_index}"));
            for (attachment_index, attachment) in attachments.iter().enumerate() {
                // Ссылки на вложения истории — любого формата (текущий ход уже
                // учтён в request_attachments с origin "current").
                if !is_current_user_message {
                    attachment_refs.push(serde_json::json!({
                        "file_name": attachment.file_name,
                        "mime_type": attachment.mime_type,
                        "file_path": attachment.file_path,
                        "is_dir": attachment.is_dir.unwrap_or(false),
                        "origin": "history",
                    }));
                }
                if attachment.mime_type.starts_with("audio/") {
                    audio_history_sequence += 1;
                    audio_candidates.push(ImageCandidate {
                        id: history_image_id(&message_id, attachment_index, attachment),
                        alias: format!("aud_hist_{:02}", audio_history_sequence),
                        locator: ImageLocator::Message {
                            message_id: message_id.clone(),
                            message_index,
                            attachment_index,
                        },
                        origin: ImageOrigin::History,
                        message_id: message_id.clone(),
                        author: message.author.clone().unwrap_or_default(),
                        file_name: attachment.file_name.clone(),
                        mime_type: attachment.mime_type.clone(),
                        context: compact_context(&message.content),
                    });
                    continue;
                }
                if !attachment.mime_type.starts_with("image/") {
                    continue;
                }
                history_sequence += 1;
                candidates.push(ImageCandidate {
                    id: history_image_id(&message_id, attachment_index, attachment),
                    alias: format!("hist_{:02}", history_sequence),
                    locator: ImageLocator::Message {
                        message_id: message_id.clone(),
                        message_index,
                        attachment_index,
                    },
                    origin: ImageOrigin::History,
                    message_id: message_id.clone(),
                    author: message.author.clone().unwrap_or_default(),
                    file_name: attachment.file_name.clone(),
                    mime_type: attachment.mime_type.clone(),
                    context: compact_context(&message.content),
                });
            }
        }

        Self {
            request_attachments: stored_request_attachments,
            candidates,
            audio_request_attachments: stored_audio_attachments,
            audio_candidates,
            attachment_refs,
        }
    }

    pub fn current_attachments(&self) -> &[ChatAttachment] {
        &self.request_attachments
    }

    /// Все вложения (любого формата) текущего запроса и истории — структурные
    /// ссылки для графа и allow-read тулов.
    pub fn attachment_refs(&self) -> &[serde_json::Value] {
        &self.attachment_refs
    }

    /// Пути прикреплённых файлов (текущий запрос + история сессии) — список
    /// allow-read для Deno-тулов графа: юзер явно отдал эти файлы в чат
    /// (повторный вопрос без повторного прикрепления берёт .jhd из истории —
    /// он должен так же читаться тулом).
    pub fn current_attachment_paths(&self) -> Vec<String> {
        self.attachment_refs
            .iter()
            .filter_map(|r| {
                r.get("file_path")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect()
    }

    /// Строки `[Вложение: имя → путь]` текущего запроса — добавляются к тексту
    /// текущего хода ({{ user_message }} и user_message.output.text).
    pub fn current_attachment_lines(&self) -> String {
        let mut out = String::new();
        for r in &self.attachment_refs {
            if r.get("origin").and_then(|v| v.as_str()) != Some("current") {
                continue;
            }
            let name = r.get("file_name").and_then(|v| v.as_str()).unwrap_or("");
            match r.get("file_path").and_then(|v| v.as_str()) {
                Some(path) => out.push_str(&format!("\n[Вложение: {} → {}]", name, path)),
                None => out.push_str(&format!("\n[Вложение: {}]", name)),
            }
        }
        out
    }

    /// Алиасы кандидатов — ЕДИНСТВЕННАЯ форма id, которую видит модель
    /// (промпт, suffix, списки допустимых). Сырой hex наружу не идёт;
    /// `resolve` принимает и его как легаси-путь.
    pub fn candidate_ids(&self) -> Vec<&str> {
        self.candidates
            .iter()
            .map(|candidate| candidate.alias.as_str())
            .collect()
    }

    /// Строки перечисления кандидатов — общие для image-, audio- и video-каталога.
    fn candidate_rows(&self) -> String {
        Self::rows_of(&self.candidates)
    }

    fn rows_of(candidates: &[ImageCandidate]) -> String {
        let mut out = String::new();
        for candidate in candidates {
            let origin = match candidate.origin {
                ImageOrigin::CurrentRequest => "текущий запрос",
                ImageOrigin::History => "история чата",
            };
            out.push_str(&format!(
                "- id=\"{}\"; origin={}; message_id={}; author={}; file=\"{}\"; mime={}; context=\"{}\"\n",
                candidate.alias,
                origin,
                candidate.message_id,
                candidate.author,
                safe_file_name(&candidate.file_name),
                candidate.mime_type,
                candidate.context
            ));
        }
        out
    }

    pub fn candidate_prompt(&self) -> String {
        if self.candidates.is_empty() {
            return String::new();
        }
        let mut out = String::from("[ДОСТУПНЫЕ ИЗОБРАЖЕНИЯ]\n");
        out.push_str(&self.candidate_rows());
        out.push_str(
            "Выбирай только существующие id. Порядок source_image_ids = роли картинок: \
             первый id — холст (<image1>, его композиция/поза/фон сохраняются), дальше — \
             источники материала (<image2>, ...). В промпте адресуй картинки тегами \
             <image1>, <image2>.",
        );
        out
    }

    /// Каталог для видео-агента (`{{ video_image_candidates }}`): порядок
    /// «холст → материалы» из image-редактора тут не действует — у видео явная
    /// роль картинок (первый кадр vs референсы 1–3, см. тул-схему
    /// generate_video / video_maker.md).
    pub fn video_candidate_prompt(&self) -> String {
        if self.candidates.is_empty() {
            return String::new();
        }
        let mut out = String::from("[ДОСТУПНЫЕ ИЗОБРАЖЕНИЯ]\n");
        out.push_str(&self.candidate_rows());
        out.push_str(
            "Выбирай только существующие id. РОВНО ОДИН id в `source_image_ids` = картинка-ПЕРВЫЙ КАДР (аргумент image_usage:\"first_frame\"). 1–3 id = РЕФЕРЕНСЫ сцены/персонажа — движок возьмёт их как образцы (image_usage:\"reference\"). Правила выбора роли — в правилах твоего агента.",
        );
        out
    }

    /// Каталог для refmod-агента (`{{ refmod_image_candidates }}`): те же строки
    /// кандидатов, футер — мягкая рекомендация расстановки кадров (первым —
    /// крупный план лица): порядок source_image_ids = порядок кадров карточки.
    /// Чужой футер (холст/первый кадр) здесь эпистемически неверен — модель
    /// расставила бы кадры не так.
    pub fn refmod_candidate_prompt(&self) -> String {
        if self.candidates.is_empty() {
            return String::new();
        }
        let mut out = String::from("[ДОСТУПНЫЕ ИЗОБРАЖЕНИЯ]\n");
        out.push_str(&self.candidate_rows());
        out.push_str(
            "Выбирай только существующие id. Порядок source_image_ids = порядок кадров карточки \
             (1-й id станет Picture 1 для движка видео). Рекомендуется первым передавать самый \
             чёткий крупный план лица без перекрытий, дальше — другие ракурсы (до 8). Все фото — \
             ОДИН И ТОТ ЖЕ человек. Габариты фото любые — нормализация при сохранении \
             автоматическая.",
        );
        out
    }

    /// Каталог таблиц для Excel-агента (`{{ spreadsheet_files }}`): вложения
    /// .xlsx/.csv/.tsv (+ честная пометка для не поддерживаемых .xls/.xlsm —
    /// молча терять файл юзера запрещено). Источник путей — attachment_refs
    /// (политика видимости 28.10.2026: пути видны модели; excel-тулы гейтят
    /// их allow-read списком `current_attachment_paths()`).
    pub fn spreadsheet_files_prompt(&self) -> String {
        let table_exts = ["xlsx", "csv", "tsv"];
        let rejected_exts = ["xls", "xlsm", "ods"];
        let mut rows = String::new();
        let mut rejected = String::new();
        for r in &self.attachment_refs {
            let name = r.get("file_name").and_then(|v| v.as_str()).unwrap_or("");
            let ext = name
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let origin = match r.get("origin").and_then(|v| v.as_str()) {
                Some("current") => "текущий запрос",
                _ => "история чата",
            };
            let path = r.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            if table_exts.contains(&ext.as_str()) {
                rows.push_str(&format!("- file=\"{}\"; path={}; origin={}\n", name, path, origin));
            } else if rejected_exts.contains(&ext.as_str()) {
                rejected.push_str(&format!("- file=\"{}\" (origin={})\n", name, origin));
            }
        }
        if rows.is_empty() && rejected.is_empty() {
            return String::new();
        }
        let mut out = String::from("[ТАБЛИЦЫ ЗАПРОСА]\n");
        if !rows.is_empty() {
            out.push_str(&rows);
            out.push_str(
                "Передавай path из этого списка в аргумент `path` excel-тулов. Файлы истории \
                 тоже доступны: юзер уже отдавал их в чат.\n",
            );
        }
        if !rejected.is_empty() {
            out.push_str(&format!(
                "[НЕ ПОДДЕРЖИВАЮТСЯ] старые форматы (честно скажи пользователю, что нужен .xlsx):\n{}",
                rejected
            ));
        }
        out
    }

    /// Каталог видео для ffmpeg-агента (`{{ video_files }}`): вложения-
    /// контейнеры текущего запроса и истории с абсолютными путями —
    /// источники ffmpeg_probe/ffmpeg_run (гейт allow-read тот же, что у
    /// excel-тулов: `current_attachment_paths()`).
    pub fn video_files_prompt(&self) -> String {
        let video_exts = [
            "mp4", "mkv", "mov", "m4v", "ts", "flv", "wmv", "avi", "webm", "mpg", "mpeg", "3gp",
            "ogv",
        ];
        let mut rows = String::new();
        for r in &self.attachment_refs {
            let name = r.get("file_name").and_then(|v| v.as_str()).unwrap_or("");
            let ext = name
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if !video_exts.contains(&ext.as_str()) {
                continue;
            }
            let origin = match r.get("origin").and_then(|v| v.as_str()) {
                Some("current") => "текущий запрос",
                _ => "история чата",
            };
            let path = r.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            rows.push_str(&format!("- file=\"{}\"; path={}; origin={}\n", name, path, origin));
        }
        if rows.is_empty() {
            return String::new();
        }
        let mut out = String::from("[ВИДЕО ЗАПРОСА]\n");
        out.push_str(&rows);
        out.push_str(
            "Передавай path из этого списка в аргумент `path` ffmpeg_probe/ffmpeg_run. \
             Файлы истории тоже доступны: юзер уже отдавал их в чат. Сначала вызови \
             ffmpeg_probe, чтобы знать длительность и дорожки.\n",
        );
        out
    }

    /// Каталог аудио для voice-фн refmod-тулов (`{{ audio_candidates }}`,
    /// Этап 3): сэмплы голоса, прикладываемые к refmod_create/refmod_edit.
    pub fn audio_candidates_prompt(&self) -> String {
        if self.audio_candidates.is_empty() {
            return String::new();
        }
        let mut out = String::from("[ДОСТУПНЫЕ АУДИО-САМПЛЫ]\n");
        out.push_str(&Self::rows_of(&self.audio_candidates));
        out.push_str(
            "Выбирай только существующие id. id уходят в `source_audio_ids` refmod-тулов \
             (сэмпл голоса персонажа карточки). mp3/wav/m4a/ogg-vorbis/flac поддержаны; \
             записи webm/opus движок не декодирует — честно попроси mp3 или wav.",
        );
        out
    }

    pub fn message_suffix(&self, message_id: Option<&str>) -> String {
        let Some(message_id) = message_id else {
            return String::new();
        };
        let ids = self
            .candidates
            .iter()
            .filter(|candidate| candidate.message_id == message_id)
            .map(|candidate| format!("\"{}\"", candidate.alias))
            .collect::<Vec<_>>();
        if ids.is_empty() {
            String::new()
        } else {
            format!("\n[Изображения этого сообщения: {}]", ids.join(", "))
        }
    }

    pub fn resolve_arguments(
        &self,
        arguments: &serde_json::Value,
        messages: &[ChatMessage],
    ) -> Result<Vec<ChatAttachment>, String> {
        let source_image_ids = arguments
            .get("source_image_ids")
            .ok_or_else(|| "source_image_ids обязателен".to_string())?
            .as_array()
            .ok_or_else(|| "source_image_ids должен быть массивом строк".to_string())?;
        let source_image_ids = source_image_ids
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "source_image_ids должен содержать только строки".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.resolve(&source_image_ids, messages)
    }

    /// Резолв аудио-вложений для voice-фн (Этап 3). `source_audio_ids`
    /// ОПЦИОНАЛЕН: нет / [] → Ok(vec![]) — обязательность пары решает тул
    /// по своей семантике (create — опционально; edit с voice_desc без
    /// голоса — честный Usage на своей стороне).
    pub fn resolve_audio_arguments(
        &self,
        arguments: &serde_json::Value,
        messages: &[ChatMessage],
    ) -> Result<Vec<ChatAttachment>, String> {
        let Some(source_audio_ids) = arguments.get("source_audio_ids") else {
            return Ok(Vec::new());
        };
        let source_audio_ids = source_audio_ids
            .as_array()
            .ok_or_else(|| "source_audio_ids должен быть массивом строк".to_string())?;
        let source_audio_ids = source_audio_ids
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "source_audio_ids должен содержать только строки".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.resolve_audio(&source_audio_ids, messages)
    }

    pub fn resolve(
        &self,
        source_image_ids: &[String],
        messages: &[ChatMessage],
    ) -> Result<Vec<ChatAttachment>, String> {
        if source_image_ids.is_empty() {
            return Err(format!(
                "source_image_ids пуст. Выбери ID из списка доступных изображений: {}",
                self.valid_ids()
            ));
        }
        self.resolve_from(&self.candidates, &self.request_attachments, source_image_ids, messages)
    }

    /// Аудио-версия `resolve` (backing store — audio_request_attachments,
    /// каталог — audio_candidates; допустимость пустого списка решает
    /// resolve_audio_arguments).
    pub fn resolve_audio(
        &self,
        source_audio_ids: &[String],
        messages: &[ChatMessage],
    ) -> Result<Vec<ChatAttachment>, String> {
        if source_audio_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.resolve_from(
            &self.audio_candidates,
            &self.audio_request_attachments,
            source_audio_ids,
            messages,
        )
    }

    /// Общий ресолв id → вложения (медиа-агностичен). Двухфазный поиск
    /// (F1, 29.10.26): сцена 1 — алиасы (модель видит только их), сцена 2 —
    /// легаси hex-id из старой схемы чата.
    fn resolve_from(
        &self,
        candidates: &[ImageCandidate],
        backing: &[ChatAttachment],
        source_ids: &[String],
        messages: &[ChatMessage],
    ) -> Result<Vec<ChatAttachment>, String> {
        let valid = if candidates.is_empty() {
            "(нет доступных)".to_string()
        } else {
            candidates
                .iter()
                .map(|candidate| candidate.alias.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut seen = HashSet::new();
        let mut resolved = Vec::with_capacity(source_ids.len());
        for source_id in source_ids {
            if !seen.insert(source_id.clone()) {
                continue;
            }
            // Фазы раздельные НАРОЧНО: алиасы короткие («hist01»), а
            // fuzz-восстановление по hex-списку не должно иметь шанса
            // сматчить короткую форму (insertion-дистанция до 16-символьного
            // hex — заведомо > 2). Ошибки показывают только алиасы.
            let alias_ids: Vec<&str> = candidates
                .iter()
                .map(|candidate| candidate.alias.as_str())
                .collect();
            let real_ids: Vec<&str> = candidates
                .iter()
                .map(|candidate| candidate.id.as_str())
                .collect();
            let lookup = super::image_ids::lookup_id(source_id, &alias_ids);
            let lookup = if matches!(lookup, super::image_ids::IdLookup::NotFound { .. }) {
                super::image_ids::lookup_id(source_id, &real_ids)
            } else {
                lookup
            };
            let matched_id = super::image_ids::finalize_lookup(lookup, source_id, &valid)?;
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.alias == matched_id || candidate.id == matched_id)
                .ok_or_else(|| {
                    format!(
                        "Вложение '{}' исчезло из каталога между стадиями ресолва",
                        matched_id
                    )
                })?;
            resolved.push(Self::resolve_candidate_from(backing, candidate, messages)?);
        }
        Ok(resolved)
    }

    fn resolve_candidate_from(
        backing: &[ChatAttachment],
        candidate: &ImageCandidate,
        messages: &[ChatMessage],
    ) -> Result<ChatAttachment, String> {
        match &candidate.locator {
            ImageLocator::Request(index) => backing
                .get(*index)
                .cloned()
                .ok_or_else(|| format!("Вложение '{}' отсутствует в текущем запросе", candidate.id)),
            ImageLocator::Message {
                message_id,
                message_index,
                attachment_index,
            } => {
                let message = messages
                    .get(*message_index)
                    .filter(|m| m.id.as_deref() == Some(message_id.as_str()))
                    .or_else(|| {
                        messages
                            .iter()
                            .find(|m| m.id.as_deref() == Some(message_id.as_str()))
                    })
                    .ok_or_else(|| {
                        format!("Сообщение '{}' для изображения '{}' отсутствует в сессии", message_id, candidate.id)
                    })?;
                message
                    .attachments
                    .as_ref()
                    .and_then(|attachments| attachments.get(*attachment_index))
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "Вложение '{}' сообщения '{}' отсутствует",
                            candidate.id, message_id
                        )
                    })
            }
        }
    }

    fn valid_ids(&self) -> String {
        if self.candidates.is_empty() {
            return "(нет доступных изображений)".to_string();
        }
        self.candidates
            .iter()
            .map(|candidate| candidate.alias.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn safe_file_name(file_name: &str) -> &str {
    file_name
        .rsplit(|character| character == '/' || character == '\\')
        .next()
        .unwrap_or(file_name)
}

fn history_image_id(
    message_id: &str,
    attachment_index: usize,
    attachment: &ChatAttachment,
) -> String {
    let mut hasher = Sha256::new();
    for part in [
        message_id,
        &attachment_index.to_string(),
        &attachment.file_name,
        &attachment.mime_type,
        attachment.file_path.as_deref().unwrap_or(&attachment.data_base64),
    ] {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    format!("hist_{}", &digest[..16])
}

fn compact_context(content: &str) -> String {
    // Пути в контексте видимы (политика 28.10.2026) — только усечение длины.
    content.chars().take(240).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(name: &str, data: &str) -> ChatAttachment {
        ChatAttachment {
            file_name: name.to_string(),
            mime_type: "image/png".to_string(),
            data_base64: data.to_string(),
            file_path: None,
            is_dir: Some(false),
        }
    }

    fn message(id: &str, author: &str, content: &str, images: Vec<ChatAttachment>) -> ChatMessage {
        ChatMessage {
            id: Some(id.to_string()),
            msg_type: "message".to_string(),
            content: content.to_string(),
            sub_calls: None,
            author: Some(author.to_string()),
            model: None,
            time_sec: None,
            attachments: Some(images),
            phase: Some(2),
            author_name: None,
        }
    }

    #[test]
    fn catalog_keeps_all_history_images_and_deduplicates_current_request() {
        let current = attachment("current.png", "current");
        let messages = vec![
            message("msg_1", "image_generator", "first result", vec![attachment("one.png", "one")]),
            message("msg_2", "user", "edit this", vec![current.clone()]),
            message("msg_3", "image_generator", "second result", vec![attachment("two.png", "two")]),
        ];
        let media = RequestMedia::build(&[current], &messages);
        let prompt = media.candidate_prompt();
        let ids = prompt
            .lines()
            .filter_map(|line| line.split("id=\"").nth(1))
            .filter_map(|value| value.split('"').next())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], "req_0");
        assert!(ids[1..].iter().all(|id| id.starts_with("hist_")));
    }

    #[test]
    fn resolve_preserves_model_selected_order() {
        let messages = vec![
            message("msg_1", "user", "sources", vec![attachment("one.png", "one")]),
            message("msg_2", "user", "sources", vec![attachment("two.png", "two")]),
        ];
        let media = RequestMedia::empty();
        let candidates = RequestMedia::build(&[], &messages);
        let two_id = candidates
            .candidates
            .iter()
            .find(|candidate| candidate.file_name == "two.png")
            .map(|candidate| candidate.id.clone())
            .unwrap();
        let one_id = candidates
            .candidates
            .iter()
            .find(|candidate| candidate.file_name == "one.png")
            .map(|candidate| candidate.id.clone())
            .unwrap();
        let resolved = candidates.resolve(&[two_id, one_id], &messages).unwrap();
        assert_eq!(resolved[0].file_name, "two.png");
        assert_eq!(resolved[1].file_name, "one.png");
        assert!(media.current_attachments().is_empty());
    }

    /// Политика видимости (28.10.2026): пути в контексте каталога ВИДНЫ.
    #[test]
    fn candidate_prompt_shows_filesystem_paths() {
        let messages = vec![message(
            "msg_1",
            "image_generator",
            "saved at D:\\private\\output\\image.png; /home/user/image.png; output/image.png; \\\\server\\share\\image.png",
            vec![attachment("image.png", "data")],
        )];
        let prompt = RequestMedia::build(&[], &messages).candidate_prompt();
        assert!(prompt.contains("D:\\private\\output\\image.png"), "{}", prompt);
        assert!(prompt.contains("/home/user/image.png"), "{}", prompt);
        assert!(
            !prompt.contains("Файловые пути не используй"),
            "запрет путей удалён из каталога"
        );
    }

    /// Не-image вложения попадают в attachment_refs (баг «видны только картинки»).
    #[test]
    fn attachment_refs_include_non_images_and_history() {
        let jhd = ChatAttachment {
            file_name: "карта.jhd".to_string(),
            mime_type: "application/octet-stream".to_string(),
            data_base64: String::new(),
            file_path: Some("C:\\Users\\user\\Desktop\\карта.jhd".to_string()),
            is_dir: Some(false),
        };
        let current = message("msg_1", "user", "текущий ход", vec![jhd.clone()]);
        let old_jhd = ChatAttachment {
            file_name: "старая.jhd".to_string(),
            mime_type: "application/octet-stream".to_string(),
            data_base64: String::new(),
            file_path: Some("C:\\Downloads\\старая.jhd".to_string()),
            is_dir: Some(false),
        };
        let old = message("msg_0", "user", "прошлый ход", vec![old_jhd.clone()]);

        let media = RequestMedia::build(std::slice::from_ref(&jhd), &[old, current]);

        let refs = media.attachment_refs().to_vec();
        assert_eq!(refs.len(), 2, "current + history: {:?}", refs);
        assert_eq!(refs[0].get("origin").and_then(|v| v.as_str()), Some("current"));
        assert_eq!(
            refs[0].get("file_path").and_then(|v| v.as_str()),
            Some("C:\\Users\\user\\Desktop\\карта.jhd")
        );
        assert_eq!(refs[1].get("origin").and_then(|v| v.as_str()), Some("history"));

        // allow-read: пути ОБЕИХ вложений (текущее + история) — юзер явно отдал
        // оба файла в чат; повторный вопрос без прикрепления берёт из истории.
        let paths = media.current_attachment_paths();
        assert_eq!(
            paths,
            vec![
                "C:\\Users\\user\\Desktop\\карта.jhd".to_string(),
                "C:\\Downloads\\старая.jhd".to_string(),
            ]
        );

        // Строки видимости — только текущего запроса (история уже видна
        // в `{{ messages }}` через content_with_attachments).
        let lines = media.current_attachment_lines();
        assert!(
            lines.contains("[Вложение: карта.jhd → C:\\Users\\user\\Desktop\\карта.jhd]"),
            "{}",
            lines
        );
        assert!(!lines.contains("старая.jhd"), "строки видимости — только current");
    }

    #[test]
    fn history_ids_survive_previous_message_deletion() {
        // Реальный id (hex) стабилен по содержанию сообщения; алиас — нет
        // (нумерация от старых к новым сдвигается при удалении соседа).
        // Сравнивая `id`, проверяем инвариант легаси-формы.
        let first = message("msg_1", "user", "one", vec![attachment("one.png", "one")]);
        let kept = message("msg_2", "user", "two", vec![attachment("two.png", "two")]);
        let before = RequestMedia::build(&[], &[first.clone(), kept.clone()]);
        let after = RequestMedia::build(&[], &[kept]);
        let before_id = before
            .candidates
            .iter()
            .find(|candidate| candidate.message_id == "msg_2")
            .map(|candidate| candidate.id.clone())
            .unwrap();
        let after_id = after
            .candidates
            .iter()
            .next()
            .map(|candidate| candidate.id.clone())
            .unwrap();
        assert_eq!(after_id, before_id);
    }

    /// F1 (29.10.26): алиасы короткие и уникальные, нумерация истории —
    /// от старых к новым. Никаких 16-символьных hex в форме для модели.
    #[test]
    fn catalog_aliases_are_short_sequential_and_unique() {
        let messages = vec![
            message("msg_1", "image_generator", "first", vec![attachment("one.png", "one")]),
            message("msg_2", "user", "second", vec![attachment("two.png", "two")]),
            message("msg_3", "image_generator", "third", vec![attachment("three.png", "three")]),
        ];
        let media = RequestMedia::build(&[], &messages);
        let aliases = media.candidate_ids();
        assert_eq!(aliases, vec!["hist_01", "hist_02", "hist_03"], "{:?}", aliases);
        // Уникальность (инвариант на любом размере каталога).
        let unique: std::collections::HashSet<_> = aliases.iter().collect();
        assert_eq!(unique.len(), aliases.len());
        // Короткость: каждый алиас короче любого hex-идентификатора
        // (> 16+5 символов в легаси-форме) — вся суть F1.
        for alias in &aliases {
            assert!(alias.len() <= 10, "алиас {} слишком длинный", alias);
        }
    }

    /// F1: промпт модели показывает ТОЛЬКО алиасы — сырой hex покидает хост.
    #[test]
    fn candidate_prompt_shows_aliases_not_hex() {
        let messages = vec![message(
            "msg_1",
            "image_generator",
            "result",
            vec![attachment("image.png", "data")],
        )];
        let media = RequestMedia::build(&[], &messages);
        let prompt = media.candidate_prompt();
        let real_id = media
            .candidates
            .iter()
            .next()
            .map(|candidate| candidate.id.clone())
            .unwrap();
        assert!(real_id.starts_with("hist_") && real_id.len() > 10, "{}", real_id);
        assert!(!prompt.contains(&real_id), "hex утёк в промпт: {}", prompt);
        assert!(prompt.contains("id=\"hist_01\""), "{}", prompt);
        // Список допустимых в ошибках — тоже алиасы.
        let error = media.resolve(&["нет-такого-id".to_string()], &messages).unwrap_err();
        assert!(error.contains("hist_01"), "{}", error);
        assert!(!error.contains(&real_id), "hex утёк в ошибку: {}", error);
    }

    /// F1: алиас принимает канонизацию/кавычки/обрамление и полный
    /// fuzzy-класс (≤2 правок) — как раньше делал hex; легаси hex тоже
    /// принимается.
    #[test]
    fn resolve_accepts_alias_forms_and_legacy_hex() {
        let messages = vec![
            message("msg_1", "user", "sources", vec![attachment("one.png", "one")]),
            message("msg_2", "user", "sources", vec![attachment("two.png", "two")]),
        ];
        let media = RequestMedia::build(&[], &messages);
        // Точный алиас + обрамление кавычками/скобками/лишним подчёркиванием.
        for form in ["hist_01", "\"hist_01\"", "(hist_01)", "hist__01"] {
            let resolved = media
                .resolve(&[form.to_string()], &messages)
                .unwrap_or_else(|error| panic!("форма '{}' должна резолвиться: {}", form, error));
            assert_eq!(resolved[0].file_name, "one.png");
        }
        // Легаси: точный hex-id из прошлых прогонов продолжает работать.
        let real_id = media
            .candidates
            .iter()
            .next()
            .map(|candidate| candidate.id.clone())
            .unwrap();
        let resolved = media.resolve(&[real_id], &messages).unwrap();
        assert_eq!(resolved[0].file_name, "one.png");
    }

    #[test]
    fn catalog_has_no_history_limit() {
        let messages = (0..25)
            .map(|index| {
                message(
                    &format!("msg_{index}"),
                    "image_generator",
                    "result",
                    vec![attachment(&format!("{index}.png"), &format!("data-{index}"))],
                )
            })
            .collect::<Vec<_>>();
        let media = RequestMedia::build(&[], &messages);
        assert_eq!(media.candidate_ids().len(), 25);
    }

    #[test]
    fn candidate_prompt_explains_reference_roles() {
        let messages = vec![message(
            "msg_1",
            "user",
            "source",
            vec![attachment("one.png", "one"), attachment("two.png", "two")],
        )];
        let prompt = RequestMedia::build(&[], &messages).candidate_prompt();
        assert!(prompt.contains("<image1>"), "роль холста: {}", prompt);
        assert!(prompt.contains("<image2>"), "роль источника: {}", prompt);
    }

    #[test]
    fn unknown_id_returns_valid_candidates() {
        let messages = vec![message("msg_1", "user", "source", vec![attachment("one.png", "one")])];
        let media = RequestMedia::build(&[], &messages);
        let valid_id = media.candidate_ids().into_iter().next().unwrap().to_string();
        let error = media.resolve(&["C:\\image.png".to_string()], &messages).unwrap_err();
        assert!(error.contains(&valid_id));
        assert!(error.contains("Неизвестный source_image_id"));
    }

    // ─────────────── F5 (29.10.26): маркер активного бандла ───────────────

    /// Каталог таблиц: xlsx/csv/tsv — с путями и origin; xls/xlsm — честная
    /// пометка «не поддерживается»; без таблиц — пустой блок.
    #[test]
    fn spreadsheet_files_prompt_lists_tables_and_rejects_old_formats() {
        let xlsx = ChatAttachment {
            file_name: "отчёт.xlsx".to_string(),
            mime_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(),
            data_base64: String::new(),
            file_path: Some("C:\\Users\\user\\Desktop\\отчёт.xlsx".to_string()),
            is_dir: Some(false),
        };
        let csv = ChatAttachment {
            file_name: "данные.csv".to_string(),
            mime_type: "text/csv".to_string(),
            data_base64: String::new(),
            file_path: Some("C:\\Downloads\\данные.csv".to_string()),
            is_dir: Some(false),
        };
        let old = ChatAttachment {
            file_name: "старый.xls".to_string(),
            mime_type: "application/vnd.ms-excel".to_string(),
            data_base64: String::new(),
            file_path: Some("C:\\Downloads\\старый.xls".to_string()),
            is_dir: Some(false),
        };
        let png = attachment("картинка.png", "img");
        let all = vec![xlsx, csv, old, png];
        let current = message("msg_1", "user", "ход", all.clone());

        let media = RequestMedia::build(&all, &[current]);
        let prompt = media.spreadsheet_files_prompt();
        assert!(prompt.contains("file=\"отчёт.xlsx\""), "{prompt}");
        assert!(prompt.contains("path=C:\\Users\\user\\Desktop\\отчёт.xlsx"), "{prompt}");
        assert!(prompt.contains("origin=текущий запрос"), "{prompt}");
        assert!(prompt.contains("данные.csv"), "{prompt}");
        assert!(prompt.contains("НЕ ПОДДЕРЖИВАЮТСЯ"), "{prompt}");
        assert!(prompt.contains("старый.xls"), "{prompt}");
        assert!(!prompt.contains("картинка.png"), "не-таблицы не попадают в каталог: {prompt}");

        // Пустой набор — пустой блок (без мусорных заголовков).
        assert_eq!(RequestMedia::empty().spreadsheet_files_prompt(), "");
    }

    /// Уникальная temp-подпапка теста (паттерн media_lora_tests: pid + метка
    /// + наносекунды — параллельные тесты и повторные прогоны не пересекаются).
    fn marker_sandbox(tag: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "image_catalog_marker_{}_{tag}_{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp-папка создана");
        dir
    }

    fn stub_bundle_files(dir: &std::path::Path, entry: &crate::infra::ImageBundleEntry) {
        for file in &entry.files {
            std::fs::write(dir.join(&file.filename), b"stub").expect("файл-заглушка записан");
        }
    }

    /// M1: маркер содержит имя бандла и base (record-факт, а не инструкция).
    #[test]
    fn marker_facts_contain_bundle_name_and_base() {
        let entry: crate::infra::ImageBundleEntry = serde_json::from_str(
            r#"{"name":"krea2-test-q4km","label":"Krea 2 Test","lora_base":"krea2",
                "ref_edit_mode":"edit_lora","ref_edit_lora":"krea2_identity_edit_v2_r128",
                "ref_edit_preset":"krea2_edit","ref_edit_lora_strength":1.0,
                "files":[{"role":"diffusion","filename":"krea2-test.gguf","download_url":"https://example.invalid/krea2.gguf"}]}"#,
        )
        .expect("синтетический бандл разобран");
        let facts = ActiveBundleFacts::from_entry(&entry);
        assert!(facts.marker.contains("bundle=krea2-test-q4km"), "{}", facts.marker);
        assert!(facts.marker.contains("base=krea2"), "{}", facts.marker);
        assert!(facts.marker.contains("ref_edit=edit_lora"), "{}", facts.marker);
        // SSOT: описание identity-лоры берётся из каталога, без дублей в коде.
        assert!(
            facts.marker.contains("krea2_identity_edit_v2_r128"),
            "{}",
            facts.marker
        );
    }

    /// M1: native-режим (Qwen) — факт без ссылок на identity-лору.
    #[test]
    fn marker_facts_native_mode_recorded() {
        let entry: crate::infra::ImageBundleEntry = serde_json::from_str(
            r#"{"name":"qwen-test-turbo","lora_base":"qwen","ref_edit_mode":"native",
                "files":[{"role":"diffusion","filename":"qwen-test.gguf","download_url":"https://example.invalid/qwen.gguf"}]}"#,
        )
        .expect("синтетический бандл разобран");
        let facts = ActiveBundleFacts::from_entry(&entry);
        assert!(facts.marker.contains("bundle=qwen-test-turbo"), "{}", facts.marker);
        assert!(facts.marker.contains("base=qwen"), "{}", facts.marker);
        assert!(facts.marker.contains("ref_edit=native"), "{}", facts.marker);
        assert_eq!(facts.base, "qwen");
        assert_eq!(facts.ref_edit, "native");
    }

    /// PRP F5 §6: кастомный бандл — маркер показывает известные поля,
    /// факты не выдумываются (base=-, ref_edit=нет).
    #[test]
    fn marker_facts_unknown_base_is_recorded_not_invented() {
        let entry: crate::infra::ImageBundleEntry = serde_json::from_str(
            r#"{"name":"custom-bundle","files":[{"role":"diffusion","filename":"custom.gguf","download_url":"https://example.invalid/custom.gguf"}]}"#,
        )
        .expect("синтетический бандл разобран");
        let facts = ActiveBundleFacts::from_entry(&entry);
        assert!(facts.marker.contains("base=-"), "{}", facts.marker);
        assert!(facts.marker.contains("ref_edit=нет"), "{}", facts.marker);
        assert_eq!(facts.base, "-");
        assert_eq!(facts.ref_edit, "нет");
    }

    /// M1: пустой конфиг (выбор не задан) на папке без скачанных файлов —
    /// `None`, без паники; после докачки файлов дефолта — маркер собирается.
    #[test]
    fn marker_resolves_default_bundle_and_requires_files() {
        let dir = marker_sandbox("default");
        // Ни один файл не скачан → None (fail-soft).
        assert!(
            marker_facts_from_selection(None, &dir).is_none(),
            "пустой бандл-каталог не должен давать маркер"
        );
        let default = crate::infra::default_bundle_entry().expect("каталог содержит дефолт");
        stub_bundle_files(&dir, &default);
        let facts = marker_facts_from_selection(None, &dir).expect("файлы на месте");
        assert_eq!(facts.name, default.name);
        let expected_base = default.lora_base.clone().unwrap_or_else(|| "-".to_string());
        assert!(
            facts.marker.contains(&format!("base={}", expected_base)),
            "{}",
            facts.marker
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M1: несуществующий бандл в `selected_bundle` — None (не паника).
    #[test]
    fn marker_unknown_selected_bundle_is_none() {
        let dir = marker_sandbox("unknown_selected");
        assert!(marker_facts_from_selection(Some("net-takogo-bundle"), &dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M1: файлы готовятся по выбранному бандлу — у krea2 свои файлы; если
    /// скачан только дефолтный qwen-набор, маркер krea2 не собирается.
    #[test]
    fn marker_requires_files_of_the_selected_bundle() {
        let dir = marker_sandbox("selected_krea2");
        let default = crate::infra::default_bundle_entry().expect("каталог содержит дефолт");
        stub_bundle_files(&dir, &default);
        let krea = crate::infra::bundle_by_name("krea2-turbo-q4km").expect("krea2 в каталоге");
        stub_bundle_files(&dir, &krea);
        let facts = marker_facts_from_selection(Some("krea2-turbo-q4km"), &dir)
            .expect("krea2-файлы на месте");
        assert_eq!(facts.name, "krea2-turbo-q4km");
        assert_eq!(facts.base, "krea2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M2: сборка меди-блока — маркер идёт СРАЗУ ПОСЛЕ каталога изображений
    /// (PRP F5 M1: «маркер идёт СРАЗУ ПОСЛЕ каталога»).
    #[test]
    fn media_prompt_block_appends_marker_after_catalog() {
        let facts: crate::infra::ImageBundleEntry = serde_json::from_str(
            r#"{"name":"qwen-test","lora_base":"qwen","ref_edit_mode":"native",
                "files":[{"role":"diffusion","filename":"qwen-test.gguf","download_url":"https://example.invalid/qwen.gguf"}]}"#,
        )
        .expect("синтетический бандл разобран");
        let facts = ActiveBundleFacts::from_entry(&facts);
        let block = media_prompt_block(
            "[ДОСТУПНЫЕ ИЗОБРАЖЕНИЯ]\n- id=\"hist_01\"",
            Some(&facts.marker),
        );
        let catalog_pos = block.find("[ДОСТУПНЫЕ ИЗОБРАЖЕНИЯ]").expect("каталог на месте");
        let marker_pos = block.find("[АКТИВНЫЙ БОНДЛ]").expect("маркер на месте");
        assert!(marker_pos > catalog_pos, "маркер после каталога: {}", block);
        assert!(
            block.contains("base=qwen"),
            "маркер просочился с базой: {}",
            block
        );
    }

    /// M2: маркер НЕ гейтится каталогом — бандл-факт полезен и без вложений
    /// в текущем ходе (история чата может содержать картинки).
    #[test]
    fn media_prompt_block_marker_alone_without_catalog() {
        let entry: crate::infra::ImageBundleEntry = serde_json::from_str(
            r#"{"name":"solo","lora_base":"krea2","ref_edit_mode":"edit_lora",
                "ref_edit_lora":"lora_x","ref_edit_preset":"preset_x",
                "files":[{"role":"diffusion","filename":"solo.gguf","download_url":"https://example.invalid/solo.gguf"}]}"#,
        )
        .expect("синтетический бандл разобран");
        let facts = ActiveBundleFacts::from_entry(&entry);
        let block = media_prompt_block("", Some(&facts.marker));
        assert_eq!(block, facts.marker, "без каталога маркер без пустых разделов");
        // И наоборот: ни каталога, ни маркера — пустой блок (не «\n\n» мусор).
        assert_eq!(media_prompt_block("", None), "");
        assert_eq!(media_prompt_block("CAT", None), "CAT");
    }

    /// Гейт маркера (FLUX 01.11.26): generate_image получает маркер, генератор
    /// без media-тулов — нет; edit/upscale входят в оба гейта (каталог+маркер),
    /// generate_video — только в каталог-гейт (маркер видео-агенту не менялся).
    #[test]
    fn agent_receives_bundle_marker_gates_by_image_tools() {
        fn profile(tools: &[&str]) -> AgentProfile {
            AgentProfile {
                id: "a".into(),
                name: "A".into(),
                description: String::new(),
                system_prompt: String::new(),
                mode: "worker".into(),
                mcp_servers: Vec::new(),
                subagents: Vec::new(),
                folder: None,
                replace_report: false,
                tools: tools.iter().map(|s| s.to_string()).collect(),
                current_date: false,
                temperature: None,
                vision: false,
                think_budget_chars: None,
                disable_reasoning: false,
                require_tool_call: false,
                visible: true,
            }
        }
        let generator = profile(&["generate_image"]);
        let editor = profile(&["edit_image"]);
        let video = profile(&["generate_video"]);
        let plain = profile(&[]);
        assert!(agent_receives_bundle_marker(&generator));
        assert!(!agent_consumes_image_catalog(&generator));
        assert!(agent_receives_bundle_marker(&editor));
        assert!(agent_consumes_image_catalog(&editor));
        assert!(!agent_receives_bundle_marker(&video));
        assert!(agent_consumes_image_catalog(&video));
        assert!(!agent_receives_bundle_marker(&plain));
        assert!(!agent_consumes_image_catalog(&plain));
    }

    /// G7 (решение юзера 01.11.26): ветка активной базы инжектится файлом,
    /// некорректная base отсекается до join (traversal), отсутствующий файл —
    /// fail-soft (None + никакой части блока).
    #[test]
    fn branch_doc_block_reads_active_base_file_only() {
        let root = marker_sandbox("branch-docs");
        let dir = root.join("image-gen").join("backend").join("branches");
        std::fs::create_dir_all(&dir).expect("ветки-каталог создан");
        std::fs::write(dir.join("flux.md"), "# База FLUX\nАдресация словами.\n")
            .expect("ветка flux записана");

        let block = branch_doc_block_from(&root, "flux").expect("ветка flux найдена");
        assert!(block.starts_with("[ВЕТКА БАЗЫ: flux]\n"), "заголовок: {}", block);
        assert!(block.contains("Адресация словами"), "тело: {}", block);

        // Чужая/несуществующая база — блока нет (не содержимое другой базы).
        assert!(branch_doc_block_from(&root, "qwen").is_none());
        // Traversal и мусор отсекаются ДО join.
        assert!(branch_doc_block_from(&root, "../evil").is_none());
        assert!(branch_doc_block_from(&root, "").is_none());
        assert!(branch_doc_block_from(&root, "FLUX").is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    /// G7: единый сборщик — порядок каталог → маркер → ветка; без фактов
    /// (None) и без каталога — пусто/только ветка по наличию частей.
    #[test]
    fn full_media_block_orders_catalog_marker_branch() {
        let entry: crate::infra::ImageBundleEntry = serde_json::from_str(
            r#"{"name":"flux-test","lora_base":"flux","ref_edit_mode":"native",
                "files":[{"role":"diffusion","filename":"flux-test.gguf","download_url":"https://example.invalid/flux.gguf"}]}"#,
        )
        .expect("синтетический бандл разобран");
        let facts = ActiveBundleFacts::from_entry(&entry);
        // base=flux взят из бандла, файл ветки в раннем резолвере искать нельзя
        // (зависимость от машины) — порядок проверяем на части с маркером.
        let block = full_media_prompt_block("[ДОСТУПНЫЕ ИЗОБРАЖЕНИЯ]\n- id=\"hist_01\"", Some(&facts));
        let catalog_pos = block.find("[ДОСТУПНЫЕ ИЗОБРАЖЕНИЯ]").expect("каталог на месте");
        let marker_pos = block.find("[АКТИВНЫЙ БОНДЛ]").expect("маркер на месте");
        assert!(marker_pos > catalog_pos, "маркер после каталога: {}", block);
        // Ветка flux реально лежит в репо — в dev-чекоуте early-резолвер её
        // находит; если не находит (нет agents/) — блок просто без ветки.
        if let Some(branch_pos) = block.find("[ВЕТКА БАЗЫ: flux]") {
            assert!(branch_pos > marker_pos, "ветка после маркера: {}", block);
        }
        assert_eq!(full_media_prompt_block("", None), "", "без фактов — пусто");
    }

    // ── Аудио-кандидаты (Этап 3: голос RefMod-карточек) ──

    fn audio_attachment(name: &str, data: &str) -> ChatAttachment {
        ChatAttachment {
            file_name: name.to_string(),
            mime_type: "audio/wav".to_string(),
            data_base64: data.to_string(),
            file_path: None,
            is_dir: Some(false),
        }
    }

    /// Аудио-вложения попадают в СВОЙ каталог (aud_req/aud_hist), не смешиваясь
    /// с картинками: счётчики и алиасы независимы.
    #[test]
    fn audio_candidates_get_own_aliases_and_prompt() {
        let messages = vec![message(
            "msg_1",
            "user",
            "вот голос",
            vec![
                audio_attachment("voice_req.wav", "cur"),
                attachment("current.png", "img"),
            ],
        )];
        // Текущий запрос — аудио-сэмпл (первый аргумент build), как из чата.
        let media = RequestMedia::build(&[audio_attachment("voice_req.wav", "cur")], &messages);
        let audio_ids = media
            .audio_candidates
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(audio_ids.len(), 1, "одно аудио-вложение: {}", audio_ids.len());
        let audio_prompt = media.audio_candidates_prompt();
        assert!(audio_prompt.contains("aud_req_0"), "{}", audio_prompt);
        assert!(audio_prompt.contains("voice_req.wav"), "{}", audio_prompt);
        assert!(
            audio_prompt.contains("source_audio_ids"),
            "футер про usage тулов: {}",
            audio_prompt
        );
        // Картинный каталог при этом аудио не содержит.
        let image_prompt = media.candidate_prompt();
        assert!(!image_prompt.contains("voice_req.wav"), "{}", image_prompt);
    }

    /// История: аудио из прошлых сообщений — aud_hist_NN (отдельный счётчик),
    /// skip-current работает и для аудио (текущий сэмпл не дублируется hist).
    #[test]
    fn audio_history_and_skip_current() {
        let messages = vec![
            message("msg_1", "user", "старый голос", vec![audio_attachment("old.wav", "old")]),
            message(
                "msg_2",
                "user",
                "новый голос",
                vec![audio_attachment("new.wav", "new")],
            ),
        ];
        // Текущий запрос — аудио из последнего юзер-сообщения.
        let media = RequestMedia::build(&[audio_attachment("new.wav", "new")], &messages);
        let prompt = media.audio_candidates_prompt();
        assert!(prompt.contains("aud_req_0") && prompt.contains("new.wav"), "{}", prompt);
        assert!(prompt.contains("aud_hist_") && prompt.contains("old.wav"), "{}", prompt);
        // Дедупликация: текущий сэмпл не живёт и как hist.
        let new_hist = prompt
            .lines()
            .filter(|line| line.contains("new.wav") && line.contains("aud_hist"))
            .count();
        assert_eq!(new_hist, 0, "текущий сэмпл не дублируется в истории: {}", prompt);
    }

    /// resolve_audio_arguments: ids ОПЦИОНАЛЬНЫ (нет → Ok(vec![])), валидные
    /// резолвятся, неизвестные — честная ошибка.
    #[test]
    fn resolve_audio_arguments_optional_and_strict() {
        let messages = vec![message(
            "msg_1",
            "user",
            "голос",
            vec![audio_attachment("voice.wav", "cur")],
        )];
        let media = RequestMedia::build(&[], &messages);
        let audio_id = media.audio_candidates[0].id.clone();
        // Нет поля → пусто (тул сам решает обязательность).
        assert!(
            media
                .resolve_audio_arguments(&serde_json::json!({}), &messages)
                .unwrap()
                .is_empty()
        );
        // Валидный id → резолв в аттачмент.
        let resolved = media
            .resolve_audio_arguments(
                &serde_json::json!({"source_audio_ids": [audio_id]}),
                &messages,
            )
            .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].file_name, "voice.wav");
        // Неизвестный id → ошибка.
        assert!(
            media
                .resolve_audio_arguments(
                    &serde_json::json!({"source_audio_ids": ["aud_nonexistent"]}),
                    &messages,
                )
                .is_err()
        );
    }
}
