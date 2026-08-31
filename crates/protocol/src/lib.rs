//! Versioned REST and WebSocket protocol shared by client and server.

use chrono::{DateTime, NaiveDate, Utc};
use iamrust_domain::{
    AttachmentId, Conversation, ConversationId, DeviceId, FriendRequest, MemberRole, Message,
    MessageContent, MessageId, Presence, ProfilePrivacySettings, SyncEvent, UserId, UserProfile,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const REST_API_VERSION: &str = "v1";
pub const WS_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Validation,
    AuthenticationRequired,
    AuthenticationFailed,
    Forbidden,
    NotFound,
    Conflict,
    RateLimited,
    PayloadTooLarge,
    UnsupportedMediaType,
    ProtocolMismatch,
    SyncRequired,
    Internal,
    ServiceUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message_key: String,
    pub field: Option<String>,
    pub correlation_id: Uuid,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub username: String,
    pub password: String,
    pub nickname: String,
    pub device_name: String,
    pub platform: Option<String>,
    pub app_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub login: String,
    pub password: String,
    #[serde(default)]
    pub second_factor_code: Option<String>,
    pub device_name: String,
    pub platform: Option<String>,
    pub app_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at: DateTime<Utc>,
    pub refresh_expires_at: DateTime<Utc>,
    pub profile: UserProfile,
    pub device_id: DeviceId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordResetRequest {
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordResetConfirmRequest {
    pub reset_token: String,
    pub new_password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondFactorStatus {
    pub enabled: bool,
    pub recovery_codes_remaining: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondFactorSetupResponse {
    pub secret: String,
    pub otpauth_uri: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondFactorCodeRequest {
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisableSecondFactorRequest {
    pub current_password: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegenerateRecoveryCodesRequest {
    pub current_password: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryCodesResponse {
    pub recovery_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrLoginStartRequest {
    pub device_name: String,
    pub platform: Option<String>,
    pub app_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrLoginStartResponse {
    pub challenge_id: Uuid,
    pub secret: String,
    pub qr_payload: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrLoginSecretRequest {
    pub secret: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QrLoginStatus {
    Pending,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrLoginPollResponse {
    pub status: QrLoginStatus,
    pub session: Option<SessionResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub platform: String,
    pub app_version: String,
    pub last_seen_at: DateTime<Utc>,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminSuspendUserRequest {
    pub suspended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminAuditEntry {
    pub id: Uuid,
    pub actor_id: Option<UserId>,
    pub action: String,
    pub target_user_id: Option<UserId>,
    pub outcome: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateProfileRequest {
    pub nickname: String,
    pub signature: String,
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub avatar_attachment_id: Option<AttachmentId>,
    #[serde(default)]
    pub gender: Option<String>,
    #[serde(default)]
    pub birthday: Option<NaiveDate>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub presence: Option<Presence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteAccountRequest {
    pub current_password: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonalDataExport {
    pub generated_at: DateTime<Utc>,
    pub email: String,
    pub profile: UserProfile,
    pub privacy: ProfilePrivacySettings,
    pub friend_ids: Vec<UserId>,
    pub friend_requests: Vec<FriendRequest>,
    pub conversations: Vec<Conversation>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRequestCreate {
    pub username: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendRequestDecision {
    Accept,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRequestDecisionBody {
    pub decision: FriendRequestDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendSettings {
    pub user_id: UserId,
    pub remark: Option<String>,
    pub group: Option<String>,
    pub share_presence: bool,
    pub allow_files: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateFriendSettingsRequest {
    pub remark: Option<String>,
    pub group: Option<String>,
    pub share_presence: bool,
    pub allow_files: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportUserRequest {
    pub reason: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateDirectConversationRequest {
    pub peer_user_id: UserId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    pub member_ids: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct ConversationSettings {
    pub conversation_id: ConversationId,
    pub pinned: bool,
    pub muted: bool,
    pub hidden: bool,
    pub manually_unread: bool,
    pub last_read_sequence: u64,
    pub draft: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct ConversationState {
    pub conversation_id: ConversationId,
    pub pinned: bool,
    pub muted: bool,
    pub hidden: bool,
    pub manually_unread: bool,
    pub last_read_sequence: u64,
    pub unread_count: u64,
    pub draft: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UpdateConversationSettingsRequest {
    pub pinned: Option<bool>,
    pub muted: Option<bool>,
    pub hidden: Option<bool>,
    pub manually_unread: Option<bool>,
    pub draft: Option<String>,
    pub label: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub avatar_url: Option<Option<String>>,
    #[serde(default)]
    pub avatar_attachment_id: Option<Option<AttachmentId>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddGroupMembersRequest {
    pub member_ids: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UpdateGroupMemberRequest {
    pub nickname: Option<Option<String>>,
    pub role: Option<MemberRole>,
    pub muted_until: Option<Option<DateTime<Utc>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferGroupOwnershipRequest {
    pub user_id: UserId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMuteRequest {
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSettingsResponse {
    pub mute_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupAnnouncement {
    pub id: Uuid,
    pub conversation_id: ConversationId,
    pub author_id: UserId,
    pub content: String,
    pub read_by: Vec<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateGroupAnnouncementRequest {
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupFileItem {
    pub message_id: MessageId,
    pub sender_id: UserId,
    pub attachment: iamrust_domain::Attachment,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupJoinRequestStatus {
    Pending,
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupJoinRequest {
    pub id: Uuid,
    pub conversation_id: ConversationId,
    pub applicant_id: UserId,
    pub message: String,
    pub status: GroupJoinRequestStatus,
    pub reviewed_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateGroupJoinRequest {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecideGroupJoinRequest {
    pub accept: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupPollOption {
    pub id: Uuid,
    pub label: String,
    pub voter_ids: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupPoll {
    pub id: Uuid,
    pub conversation_id: ConversationId,
    pub creator_id: UserId,
    pub question: String,
    pub multiple_choice: bool,
    pub options: Vec<GroupPollOption>,
    pub closes_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateGroupPollRequest {
    pub question: String,
    pub options: Vec<String>,
    pub multiple_choice: bool,
    pub closes_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteGroupPollRequest {
    pub option_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendMessageRequest {
    pub client_message_id: MessageId,
    pub content: MessageContent,
    pub reply_to: Option<MessageId>,
    #[serde(default)]
    pub mentions: Vec<UserId>,
    #[serde(default)]
    pub mention_all: bool,
    pub expires_in_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleMessageRequest {
    pub conversation_id: ConversationId,
    pub client_message_id: MessageId,
    pub content: MessageContent,
    pub reply_to: Option<MessageId>,
    #[serde(default)]
    pub mentions: Vec<UserId>,
    #[serde(default)]
    pub mention_all: bool,
    pub scheduled_for: DateTime<Utc>,
    pub expires_in_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledMessageResponse {
    pub schedule_id: Uuid,
    pub scheduled_for: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledMessageInfo {
    pub schedule_id: Uuid,
    pub conversation_id: ConversationId,
    pub content: MessageContent,
    pub reply_to: Option<MessageId>,
    pub mentions: Vec<UserId>,
    pub mention_all: bool,
    pub scheduled_for: DateTime<Utc>,
    pub expires_in_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardMessagesRequest {
    pub message_ids: Vec<MessageId>,
    pub target_conversation_id: ConversationId,
    #[serde(default)]
    pub mode: ForwardMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardMode {
    #[default]
    Individually,
    Merged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReactionRequest {
    pub emoji: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslateMessageRequest {
    pub target_language: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslateMessageResponse {
    pub source_language: Option<String>,
    pub target_language: String,
    pub translated_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscribeMessageResponse {
    pub text: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sticker {
    pub id: Uuid,
    pub owner_id: UserId,
    pub attachment: iamrust_domain::Attachment,
    pub name: String,
    pub shortcut: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateStickerRequest {
    pub attachment_id: AttachmentId,
    pub name: String,
    pub shortcut: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FavoriteMessageRequest {
    pub favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReaction {
    pub emoji: String,
    pub user_ids: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageDetails {
    pub message: Message,
    pub reactions: Vec<MessageReaction>,
    pub delivered_to: Vec<UserId>,
    pub read_by: Vec<UserId>,
    pub favorited: bool,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageAck {
    pub client_message_id: MessageId,
    pub message_id: MessageId,
    pub sequence: u64,
    pub server_time: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkReadRequest {
    pub through_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadAuthorizationRequest {
    pub file_name: String,
    pub mime_type: String,
    pub byte_size: u64,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadAuthorizationResponse {
    pub attachment_id: iamrust_domain::AttachmentId,
    pub storage_key: String,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
    pub required_headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteUploadRequest {
    pub attachment_id: iamrust_domain::AttachmentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteUploadResponse {
    pub attachment: iamrust_domain::Attachment,
    pub download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadAuthorizationResponse {
    pub download_url: String,
    pub expires_at: DateTime<Utc>,
    pub attachment: iamrust_domain::Attachment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSocketTicketResponse {
    pub ticket: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallSignal {
    Invite {
        video: bool,
    },
    Accept,
    Offer {
        sdp: String,
    },
    Answer {
        sdp: String,
    },
    IceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
    Participants {
        user_ids: Vec<UserId>,
    },
    Reject,
    Busy,
    Hangup,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncResponse {
    pub events: Vec<SyncEvent>,
    pub next_cursor: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Hello {
        protocol_version: u16,
        client_version: String,
        access_token: String,
        last_cursor: u64,
    },
    Ping {
        nonce: Uuid,
    },
    SendMessage {
        request_id: Uuid,
        conversation_id: ConversationId,
        message: Box<SendMessageRequest>,
    },
    MarkRead {
        request_id: Uuid,
        conversation_id: ConversationId,
        through_sequence: u64,
    },
    Typing {
        conversation_id: ConversationId,
        active: bool,
    },
    CallSignal {
        conversation_id: ConversationId,
        call_id: Uuid,
        #[serde(default)]
        target_user_id: Option<UserId>,
        signal: CallSignal,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Welcome {
        protocol_version: u16,
        user_id: UserId,
        server_time: DateTime<Utc>,
        latest_cursor: u64,
    },
    Pong {
        nonce: Uuid,
    },
    Ack {
        request_id: Uuid,
        ack: MessageAck,
    },
    Event {
        event: SyncEvent,
    },
    Typing {
        conversation_id: ConversationId,
        user_id: UserId,
        active: bool,
        expires_at: DateTime<Utc>,
    },
    CallSignal {
        conversation_id: ConversationId,
        call_id: Uuid,
        from_user_id: UserId,
        signal: CallSignal,
    },
    Error {
        request_id: Option<Uuid>,
        error: ApiError,
    },
    Close {
        code: u16,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "entity", content = "value", rename_all = "snake_case")]
pub enum SearchResult {
    User(UserProfile),
    Conversation(Conversation),
    Message(Message),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BootstrapResponse {
    pub profile: UserProfile,
    #[serde(default)]
    pub profile_privacy: ProfilePrivacySettings,
    pub friends: Vec<UserProfile>,
    #[serde(default)]
    pub friend_settings: Vec<FriendSettings>,
    pub friend_requests: Vec<FriendRequest>,
    #[serde(default)]
    pub friend_request_profiles: Vec<UserProfile>,
    pub conversations: Vec<Conversation>,
    #[serde(default)]
    pub conversation_states: Vec<ConversationState>,
    pub cursor: u64,
    pub server_features: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_frame_is_tagged_and_versioned() {
        let frame = ClientFrame::Ping { nonce: Uuid::nil() };
        let json = serde_json::to_value(frame).unwrap();
        assert_eq!(json["type"], "ping");
        assert_eq!(WS_PROTOCOL_VERSION, 1);
    }
}
