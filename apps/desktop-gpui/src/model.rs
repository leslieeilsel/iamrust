#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Navigation {
    Chats,
    Contacts,
    Search,
    Settings,
}

impl Navigation {
    pub const ALL: [Self; 4] = [Self::Chats, Self::Contacts, Self::Search, Self::Settings];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Chats => "会话",
            Self::Contacts => "联系人",
            Self::Search => "搜索",
            Self::Settings => "设置",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPreview {
    pub id: Option<ConversationId>,
    pub name: String,
    pub summary: String,
    pub timestamp: String,
    pub unread: usize,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub message_id: Option<MessageId>,
    pub client_message_id: Option<MessageId>,
    pub reply_to: Option<MessageId>,
    pub author: String,
    pub body: String,
    pub outgoing: bool,
    pub timestamp: String,
    pub status: String,
    pub attachment: Option<Attachment>,
}

pub fn demo_conversations() -> Vec<ConversationPreview> {
    vec![
        ConversationPreview {
            id: None,
            name: "Rustacean 小组".to_owned(),
            summary: "Ferris：GPUI 原生界面已经启动".to_owned(),
            timestamp: "刚刚".to_owned(),
            unread: 3,
            muted: false,
        },
        ConversationPreview {
            id: None,
            name: "Alice".to_owned(),
            summary: "周末一起完善离线同步吧".to_owned(),
            timestamp: "09:42".to_owned(),
            unread: 0,
            muted: false,
        },
        ConversationPreview {
            id: None,
            name: "发布值班".to_owned(),
            summary: "三平台构建矩阵已就绪".to_owned(),
            timestamp: "昨天".to_owned(),
            unread: 8,
            muted: true,
        },
    ]
}

pub fn demo_messages() -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            message_id: None,
            client_message_id: None,
            reply_to: None,
            author: "Ferris".to_owned(),
            body: "欢迎来到 I Am Rust。界面正在迁移到 Rust 原生 GPUI。".to_owned(),
            outgoing: false,
            timestamp: "09:40".to_owned(),
            status: "已发送".to_owned(),
            attachment: None,
        },
        ChatMessage {
            message_id: None,
            client_message_id: None,
            reply_to: None,
            author: "我".to_owned(),
            body: "领域模型和服务端会保留，桌面交互逐页迁移。".to_owned(),
            outgoing: true,
            timestamp: "09:41".to_owned(),
            status: "已发送".to_owned(),
            attachment: None,
        },
        ChatMessage {
            message_id: None,
            client_message_id: None,
            reply_to: None,
            author: "Ferris".to_owned(),
            body: "很好，先把认证、三栏布局和消息编辑器跑通。".to_owned(),
            outgoing: false,
            timestamp: "09:42".to_owned(),
            status: "已发送".to_owned(),
            attachment: None,
        },
    ]
}

pub fn conversations_from_bootstrap(bootstrap: &BootstrapResponse) -> Vec<ConversationPreview> {
    let mut conversations = bootstrap
        .conversations
        .iter()
        .map(|conversation| {
            let state = bootstrap
                .conversation_states
                .iter()
                .find(|state| state.conversation_id == conversation.id);
            let name = match &conversation.kind {
                ConversationKind::Direct { peer_user_id } => bootstrap
                    .friends
                    .iter()
                    .find(|profile| profile.id == *peer_user_id)
                    .map(|profile| profile.nickname.clone())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "未知联系人".to_owned()),
                ConversationKind::Group { .. } => {
                    if conversation.name.trim().is_empty() {
                        "未命名群聊".to_owned()
                    } else {
                        conversation.name.clone()
                    }
                }
            };
            ConversationPreview {
                id: Some(conversation.id),
                name,
                summary: state
                    .filter(|state| !state.draft.trim().is_empty())
                    .map_or_else(
                        || "还没有消息".to_owned(),
                        |state| format!("草稿：{}", state.draft),
                    ),
                timestamp: conversation.updated_at.format("%m-%d %H:%M").to_string(),
                unread: state
                    .and_then(|state| usize::try_from(state.unread_count).ok())
                    .unwrap_or(usize::MAX),
                muted: state.map_or(conversation.muted, |state| state.muted),
            }
        })
        .collect::<Vec<_>>();
    conversations.sort_by_key(|preview| {
        let pinned = preview.id.is_some_and(|id| {
            bootstrap
                .conversation_states
                .iter()
                .find(|state| state.conversation_id == id)
                .is_some_and(|state| state.pinned)
        });
        !pinned
    });
    conversations
}

pub fn messages_from_domain(
    messages: &[Message],
    bootstrap: &BootstrapResponse,
) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|message| {
            let outgoing = message.sender_id == bootstrap.profile.id;
            let attachment = match &message.content {
                MessageContent::Image { attachment }
                | MessageContent::File { attachment }
                | MessageContent::Audio { attachment, .. }
                | MessageContent::Sticker { attachment, .. } => Some(attachment.clone()),
                _ => None,
            };
            let author = if outgoing {
                "我".to_owned()
            } else {
                bootstrap
                    .friends
                    .iter()
                    .find(|profile| profile.id == message.sender_id)
                    .map_or_else(|| "群成员".to_owned(), display_name)
            };
            ChatMessage {
                message_id: Some(message.id),
                client_message_id: Some(message.client_message_id),
                reply_to: message.reply_to,
                author,
                body: message_body(&message.content),
                outgoing,
                timestamp: message
                    .server_created_at
                    .unwrap_or(message.created_at)
                    .format("%m-%d %H:%M")
                    .to_string(),
                status: message_status(message.status),
                attachment,
            }
        })
        .collect()
}

fn display_name(profile: &UserProfile) -> String {
    if profile.nickname.trim().is_empty() {
        profile.username.clone()
    } else {
        profile.nickname.clone()
    }
}

fn message_body(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { text } | MessageContent::System { text } => text.clone(),
        MessageContent::Image { .. } => "[图片]".to_owned(),
        MessageContent::File { attachment } => format!("[文件] {}", attachment.file_name),
        MessageContent::Audio { duration_ms, .. } => format!("[语音] {} 秒", duration_ms / 1_000),
        MessageContent::Sticker { name, .. } => format!("[表情] {name}"),
        MessageContent::ForwardBundle { title, .. } => format!("[聊天记录] {title}"),
    }
}

fn message_status(status: MessageStatus) -> String {
    match status {
        MessageStatus::Pending => "发送中".to_owned(),
        MessageStatus::Sent => "已发送".to_owned(),
        MessageStatus::Delivered => "已送达".to_owned(),
        MessageStatus::Read => "已读".to_owned(),
        MessageStatus::Failed => "发送失败".to_owned(),
        MessageStatus::Recalled => "已撤回".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_labels_are_stable_and_complete() {
        assert_eq!(Navigation::ALL.len(), 4);
        assert_eq!(
            Navigation::ALL.map(Navigation::label),
            ["会话", "联系人", "搜索", "设置"]
        );
    }

    #[test]
    fn demo_state_contains_unread_and_bidirectional_messages() {
        assert!(demo_conversations().iter().any(|item| item.unread > 0));
        let messages = demo_messages();
        assert!(messages.iter().any(|message| message.outgoing));
        assert!(messages.iter().any(|message| !message.outgoing));
    }
}
use iamrust_domain::{
    Attachment, ConversationId, ConversationKind, Message, MessageContent, MessageId,
    MessageStatus, UserProfile,
};
use iamrust_protocol::BootstrapResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Login,
    Register,
    PasswordReset,
    PasswordResetConfirm,
    QrLogin,
}
