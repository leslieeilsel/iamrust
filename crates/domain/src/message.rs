use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{AttachmentId, ConversationId, DomainError, MessageId, UserId, validate_message_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    File,
    Audio,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: AttachmentId,
    pub kind: AttachmentKind,
    pub file_name: String,
    pub mime_type: String,
    pub byte_size: u64,
    pub sha256: Option<String>,
    pub storage_key: String,
    pub thumbnail_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardedMessage {
    pub sender_id: UserId,
    pub sender_name: String,
    pub content: MessageContent,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum MessageContent {
    Text {
        text: String,
    },
    Image {
        attachment: Attachment,
    },
    File {
        attachment: Attachment,
    },
    Audio {
        attachment: Attachment,
        duration_ms: u32,
    },
    Sticker {
        attachment: Attachment,
        name: String,
    },
    ForwardBundle {
        title: String,
        messages: Vec<ForwardedMessage>,
    },
    System {
        text: String,
    },
}

impl MessageContent {
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Text { text } => validate_message_text(text),
            Self::System { text } if text.trim().is_empty() => Err(DomainError::EmptyMessage),
            Self::ForwardBundle { title, messages }
                if title.trim().is_empty()
                    || title.chars().count() > 80
                    || !(2..=100).contains(&messages.len())
                    || messages.iter().any(|message| {
                        message.sender_name.trim().is_empty()
                            || message.sender_name.chars().count() > 80
                            || matches!(
                                &message.content,
                                Self::System { .. } | Self::ForwardBundle { .. }
                            )
                    }) =>
            {
                Err(DomainError::Validation {
                    field: "forward_bundle",
                    reason: "invalid_bundle",
                })
            }
            Self::Image { attachment }
            | Self::File { attachment }
            | Self::Audio { attachment, .. }
            | Self::Sticker { attachment, .. }
                if attachment.byte_size == 0 =>
            {
                Err(DomainError::Validation {
                    field: "attachment",
                    reason: "empty_file",
                })
            }
            Self::Sticker { name, .. } if name.trim().is_empty() || name.chars().count() > 48 => {
                Err(DomainError::Validation {
                    field: "sticker_name",
                    reason: "invalid_length",
                })
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Pending,
    Sent,
    Delivered,
    Read,
    Failed,
    Recalled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub client_message_id: MessageId,
    pub conversation_id: ConversationId,
    pub sender_id: UserId,
    pub sequence: Option<u64>,
    pub content: MessageContent,
    pub status: MessageStatus,
    pub reply_to: Option<MessageId>,
    #[serde(default)]
    pub mentions: Vec<UserId>,
    #[serde(default)]
    pub mention_all: bool,
    pub created_at: DateTime<Utc>,
    pub server_created_at: Option<DateTime<Utc>>,
    pub edited_at: Option<DateTime<Utc>>,
}

impl Message {
    pub fn pending(
        client_message_id: MessageId,
        conversation_id: ConversationId,
        sender_id: UserId,
        content: MessageContent,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        content.validate()?;
        Ok(Self {
            id: MessageId::new(),
            client_message_id,
            conversation_id,
            sender_id,
            sequence: None,
            content,
            status: MessageStatus::Pending,
            reply_to: None,
            mentions: Vec::new(),
            mention_all: false,
            created_at: now,
            server_created_at: None,
            edited_at: None,
        })
    }

    pub fn mark_sent(
        &mut self,
        sequence: u64,
        server_time: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if !matches!(self.status, MessageStatus::Pending | MessageStatus::Failed) {
            return Err(DomainError::InvalidTransition {
                from: "final",
                to: "sent",
            });
        }
        self.sequence = Some(sequence);
        self.server_created_at = Some(server_time);
        self.status = MessageStatus::Sent;
        Ok(())
    }

    pub fn mark_failed(&mut self) -> Result<(), DomainError> {
        if self.status != MessageStatus::Pending {
            return Err(DomainError::InvalidTransition {
                from: "final",
                to: "failed",
            });
        }
        self.status = MessageStatus::Failed;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn pending_message_transitions_once() {
        let mut message = Message::pending(
            MessageId::new(),
            ConversationId::new(),
            UserId::new(),
            MessageContent::Text {
                text: "hello".to_owned(),
            },
            Utc::now(),
        )
        .unwrap();
        message.mark_sent(1, Utc::now()).unwrap();
        assert_eq!(message.status, MessageStatus::Sent);
        assert!(message.mark_sent(2, Utc::now()).is_err());
    }

    proptest! {
        #[test]
        fn every_pending_or_failed_message_can_be_acknowledged_once(sequence in 1_u64..) {
            let mut message = Message::pending(
                MessageId::new(),
                ConversationId::new(),
                UserId::new(),
                MessageContent::Text { text: "property".to_owned() },
                Utc::now(),
            ).unwrap();
            if sequence % 2 == 0 {
                message.mark_failed().unwrap();
            }
            message.mark_sent(sequence, Utc::now()).unwrap();
            prop_assert_eq!(message.sequence, Some(sequence));
            prop_assert_eq!(message.status, MessageStatus::Sent);
            prop_assert!(message.mark_failed().is_err());
            prop_assert!(message.mark_sent(sequence.saturating_add(1), Utc::now()).is_err());
        }
    }
}
