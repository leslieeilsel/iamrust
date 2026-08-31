//! Framework-independent domain model for I Am Rust.

mod conversation;
mod error;
mod friend;
mod id;
mod message;
mod sync;
mod user;
mod validation;

pub use conversation::{Conversation, ConversationKind, ConversationMember, MemberRole};
pub use error::DomainError;
pub use friend::{FriendRequest, FriendRequestStatus, Friendship};
pub use id::{AttachmentId, ConversationId, DeviceId, FriendRequestId, GroupId, MessageId, UserId};
pub use message::{
    Attachment, AttachmentKind, ForwardedMessage, Message, MessageContent, MessageStatus,
};
pub use sync::{EventKind, SyncEvent, SyncState, UnreadCounters};
pub use user::{
    Presence, ProfilePrivacySettings, ProfileVisibility, UserProfile, UserProfileUpdate,
};
pub use validation::{
    MAX_MESSAGE_CHARS, validate_email, validate_message_text, validate_password, validate_username,
};
