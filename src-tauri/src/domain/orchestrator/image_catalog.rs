use crate::domain::agent_manager::AgentProfile;
use crate::infra::{ChatAttachment, ChatMessage};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// Агент потребляет каталог изображений (нужен список `hist_*` / `req_*` id
/// вложений сессии). SSOT для ранее размножившегося условия, где у апскейлера
/// каталог не появлялся вовсе (`tool == "edit_image"` ×6: orchestrator/mod.rs
/// ×4, api/chat.rs ×2 — см. tasks/21.10.26 …md §1.1).
pub fn agent_consumes_image_catalog(agent: &AgentProfile) -> bool {
    agent
        .tools
        .iter()
        .any(|tool| tool == "edit_image" || tool == "upscale_image")
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
    id: String,
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
        let mut candidates = Vec::new();
        let mut stored_request_attachments = Vec::new();
        let mut attachment_refs = Vec::new();

        for (index, attachment) in request_attachments.iter().enumerate() {
            attachment_refs.push(serde_json::json!({
                "file_name": attachment.file_name,
                "mime_type": attachment.mime_type,
                "file_path": attachment.file_path,
                "is_dir": attachment.is_dir.unwrap_or(false),
                "origin": "current",
            }));
            if !attachment.mime_type.starts_with("image/") {
                continue;
            }
            let candidate = ImageCandidate {
                id: format!("req_{index}"),
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
            if has_current_images && is_current_user_message {
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
                if !attachment.mime_type.starts_with("image/") {
                    continue;
                }
                candidates.push(ImageCandidate {
                    id: history_image_id(&message_id, attachment_index, attachment),
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

    pub fn candidate_ids(&self) -> Vec<&str> {
        self.candidates
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect()
    }

    pub fn candidate_prompt(&self) -> String {
        if self.candidates.is_empty() {
            return String::new();
        }
        let mut out = String::from("[ДОСТУПНЫЕ ИЗОБРАЖЕНИЯ]\n");
        for candidate in &self.candidates {
            let origin = match candidate.origin {
                ImageOrigin::CurrentRequest => "текущий запрос",
                ImageOrigin::History => "история чата",
            };
            out.push_str(&format!(
                "- id=\"{}\"; origin={}; message_id={}; author={}; file=\"{}\"; mime={}; context=\"{}\"\n",
                candidate.id,
                origin,
                candidate.message_id,
                candidate.author,
                safe_file_name(&candidate.file_name),
                candidate.mime_type,
                candidate.context
            ));
        }
        out.push_str(
            "Выбирай только существующие id. Порядок source_image_ids = роли картинок: \
             первый id — холст (<image1>, его композиция/поза/фон сохраняются), дальше — \
             источники материала (<image2>, ...). В промпте адресуй картинки тегами \
             <image1>, <image2>.",
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
            .map(|candidate| format!("\"{}\"", candidate.id))
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
        let mut seen = HashSet::new();
        let mut resolved = Vec::with_capacity(source_image_ids.len());
        for source_id in source_image_ids {
            if !seen.insert(source_id.clone()) {
                continue;
            }
            // Терпимый ресолв (image_ids): модель регулярно искажает id в
            // свободном тексте (лог 21.10.2026 00:51–00:57 — три «Неизвестный
            // source_image_id» подряд при верном контексте). Канонизация
            // (скобки/кавычки/подчёркивания) + восстановление по ≤2 простым
            // правкам. Неоднозначность и «далёкие» — честный отказ, не выбор.
            let candidate_ids: Vec<&str> = self
                .candidates
                .iter()
                .map(|candidate| candidate.id.as_str())
                .collect();
            let lookup = super::image_ids::lookup_id(source_id, &candidate_ids);
            let matched_id = super::image_ids::finalize_lookup(
                lookup,
                source_id,
                &self.valid_ids(),
            )?;
            let candidate = self
                .candidates
                .iter()
                .find(|candidate| candidate.id == matched_id)
                .ok_or_else(|| {
                    format!(
                        "Изображение '{}' исчезло из каталога между стадиями ресолва",
                        matched_id
                    )
                })?;
            resolved.push(self.resolve_candidate(candidate, messages)?);
        }
        Ok(resolved)
    }

    fn resolve_candidate(
        &self,
        candidate: &ImageCandidate,
        messages: &[ChatMessage],
    ) -> Result<ChatAttachment, String> {
        match &candidate.locator {
            ImageLocator::Request(index) => self
                .request_attachments
                .get(*index)
                .cloned()
                .ok_or_else(|| format!("Изображение '{}' отсутствует в текущем запросе", candidate.id)),
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
            .map(|candidate| candidate.id.as_str())
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
        let after_id = after.candidate_ids().into_iter().next().unwrap();
        assert_eq!(after_id, before_id);
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
}
