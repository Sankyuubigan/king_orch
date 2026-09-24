use crate::infra::{ChatAttachment, ChatMessage};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

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

        for (index, attachment) in request_attachments.iter().enumerate() {
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
            if has_current_images && message.id.as_deref() == Some(current_user_id.as_str()) {
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
        }
    }

    pub fn current_attachments(&self) -> &[ChatAttachment] {
        &self.request_attachments
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
        out.push_str("Выбирай только существующие id. Файловые пути не используй.");
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
            let candidate = self
                .candidates
                .iter()
                .find(|candidate| candidate.id == *source_id)
                .ok_or_else(|| {
                    format!(
                        "Неизвестный source_image_id. Допустимые ID: {}",
                        self.valid_ids()
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
        &attachment.data_base64,
    ] {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    format!("hist_{}", &digest[..16])
}

fn compact_context(content: &str) -> String {
    super::prompt::sanitize_model_visible_text(content)
        .chars()
        .take(240)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(name: &str, data: &str) -> ChatAttachment {
        ChatAttachment {
            file_name: name.to_string(),
            mime_type: "image/png".to_string(),
            data_base64: data.to_string(),
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

    #[test]
    fn candidate_prompt_hides_filesystem_paths() {
        let messages = vec![message(
            "msg_1",
            "image_generator",
            "saved at D:\\private\\output\\image.png; /home/user/image.png; output/image.png; \\\\server\\share\\image.png",
            vec![attachment("image.png", "data")],
        )];
        let prompt = RequestMedia::build(&[], &messages).candidate_prompt();
        assert!(!prompt.contains("D:\\private"));
        assert!(!prompt.contains("/home/user"));
        assert!(!prompt.contains("output/image.png"));
        assert!(!prompt.contains("\\\\server\\share"));
        assert_eq!(prompt.matches("[filesystem path omitted]").count(), 4);
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
    fn unknown_id_returns_valid_candidates() {
        let messages = vec![message("msg_1", "user", "source", vec![attachment("one.png", "one")])];
        let media = RequestMedia::build(&[], &messages);
        let valid_id = media.candidate_ids().into_iter().next().unwrap().to_string();
        let error = media.resolve(&["C:\\image.png".to_string()], &messages).unwrap_err();
        assert!(error.contains(&valid_id));
        assert!(error.contains("Неизвестный source_image_id"));
    }
}
