use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Nonce},
};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use iamrust_domain::{
    Attachment, AttachmentId, AttachmentKind, Conversation, ConversationId, ConversationKind,
    ConversationMember, DeviceId, DomainError, EventKind, ForwardedMessage, FriendRequest,
    FriendRequestId, FriendRequestStatus, Friendship, MemberRole, Message, MessageContent,
    MessageId, Presence, ProfilePrivacySettings, ProfileVisibility, SyncEvent, UserId, UserProfile,
    UserProfileUpdate, validate_email, validate_password, validate_username,
};
use iamrust_protocol::{
    AddGroupMembersRequest, AdminAuditEntry, BootstrapResponse, CallSignal, ConversationSettings,
    ConversationState, CreateGroupAnnouncementRequest, CreateGroupJoinRequest,
    CreateGroupPollRequest, CreateStickerRequest, DecideGroupJoinRequest, DeviceInfo, ForwardMode,
    FriendRequestDecision, FriendSettings, GroupAnnouncement, GroupFileItem, GroupJoinRequest,
    GroupJoinRequestStatus, GroupMuteRequest, GroupPoll, GroupPollOption, MessageAck,
    MessageDetails, MessageReaction, PersonalDataExport, ReportUserRequest, ScheduleMessageRequest,
    ScheduledMessageInfo, ScheduledMessageResponse, SecondFactorSetupResponse, SecondFactorStatus,
    SendMessageRequest, Sticker, SyncResponse, TransferGroupOwnershipRequest,
    UpdateConversationSettingsRequest, UpdateFriendSettingsRequest, UpdateGroupMemberRequest,
    UpdateGroupRequest, VoteGroupPollRequest,
};
use rand::Rng as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock, broadcast};
use url::Url;
use uuid::Uuid;

const ACCESS_TOKEN_MINUTES: i64 = 15;
const REFRESH_TOKEN_DAYS: i64 = 30;
const MAX_SYNC_PAGE: usize = 500;
const MAX_MESSAGE_PAGE: usize = 200;
const CALL_ROOM_TTL_MINUTES: i64 = 180;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterInput {
    pub email: String,
    pub username: String,
    pub password: String,
    pub nickname: String,
    pub device_name: String,
    pub platform: String,
    pub app_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginInput {
    pub login: String,
    pub password: String,
    pub device_name: String,
    pub platform: String,
    pub app_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateProfileInput {
    pub nickname: String,
    pub signature: String,
    pub avatar_url: Option<Url>,
    pub avatar_attachment_id: Option<AttachmentId>,
    pub gender: Option<String>,
    pub birthday: Option<NaiveDate>,
    pub region: Option<String>,
    pub presence: Option<Presence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedSession {
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at: DateTime<Utc>,
    pub refresh_expires_at: DateTime<Utc>,
    pub profile: UserProfile,
    pub device_id: DeviceId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordResetDelivery {
    pub email: String,
    pub reset_token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentAuthorization {
    pub attachment: Attachment,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypingSignal {
    pub conversation_id: ConversationId,
    pub user_id: UserId,
    pub active: bool,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSignalDelivery {
    pub conversation_id: ConversationId,
    pub call_id: Uuid,
    pub from_user_id: UserId,
    pub signal: CallSignal,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("the requested account data is already in use")]
    AccountConflict,
    #[error("credentials are invalid")]
    InvalidCredentials,
    #[error("a second authentication factor is required")]
    SecondFactorRequired,
    #[error("the second authentication factor is invalid")]
    InvalidSecondFactor,
    #[error("authentication is required")]
    Unauthorized,
    #[error("the session has expired")]
    SessionExpired,
    #[error("refresh token reuse was detected")]
    RefreshTokenReuse,
    #[error("resource not found")]
    NotFound,
    #[error("resource conflict")]
    Conflict,
    #[error("persistent storage is unavailable")]
    Storage,
}

#[derive(Clone)]
pub struct ChatService {
    store: Arc<RwLock<Store>>,
    events: broadcast::Sender<(Vec<UserId>, SyncEvent)>,
    typing: broadcast::Sender<(Vec<UserId>, TypingSignal)>,
    calls: broadcast::Sender<(Vec<UserId>, CallSignalDelivery)>,
    connections: Arc<Mutex<HashMap<UserId, usize>>>,
    qr_logins: Arc<Mutex<HashMap<Uuid, QrLoginChallenge>>>,
    call_rooms: Arc<Mutex<HashMap<(ConversationId, Uuid), CallRoom>>>,
    data_encryption_key: [u8; 32],
    database: Option<PgPool>,
}

#[derive(Debug, Clone)]
struct CallRoom {
    participants: HashSet<UserId>,
    expires_at: DateTime<Utc>,
}

impl fmt::Debug for ChatService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChatService")
            .field("data_encryption_key", &"[REDACTED]")
            .field("database_enabled", &self.database.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Account {
    password_hash: String,
    profile: UserProfile,
    #[serde(default)]
    deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    suspended: bool,
    #[serde(default)]
    second_factor: Option<SecondFactorState>,
    #[serde(default)]
    pending_second_factor: Option<PendingSecondFactor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SecondFactorState {
    encrypted_secret: Vec<u8>,
    recovery_code_hashes: Vec<String>,
    enabled_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingSecondFactor {
    encrypted_secret: Vec<u8>,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct QrLoginChallenge {
    secret_hash: String,
    approved_user_id: Option<UserId>,
    device_name: String,
    platform: String,
    app_version: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrLoginChallengeInfo {
    pub challenge_id: Uuid,
    pub secret: String,
    pub qr_payload: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TokenRecord {
    user_id: UserId,
    device_id: DeviceId,
    family_id: Uuid,
    expires_at: DateTime<Utc>,
    revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceRecord {
    id: DeviceId,
    user_id: UserId,
    name: String,
    platform: String,
    app_version: String,
    last_seen_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PasswordResetRecord {
    user_id: UserId,
    expires_at: DateTime<Utc>,
    attempts: u8,
    consumed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserReport {
    id: Uuid,
    reporter_id: UserId,
    reported_id: UserId,
    reason: String,
    details: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserEvent {
    recipients: HashSet<UserId>,
    event: SyncEvent,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ReceiptState {
    delivered_to: HashSet<UserId>,
    read_by: HashSet<UserId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScheduledMessage {
    id: Uuid,
    actor: UserId,
    conversation_id: ConversationId,
    client_message_id: MessageId,
    content: MessageContent,
    reply_to: Option<MessageId>,
    #[serde(default)]
    mentions: Vec<UserId>,
    #[serde(default)]
    mention_all: bool,
    scheduled_for: DateTime<Utc>,
    expires_in_seconds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingAttachment {
    attachment: Attachment,
    owner_id: UserId,
    expires_at: DateTime<Utc>,
    available: bool,
    #[serde(default)]
    quarantined: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    accounts: HashMap<UserId, Account>,
    profile_privacy: HashMap<UserId, ProfilePrivacySettings>,
    preferred_presence: HashMap<UserId, Presence>,
    by_email: HashMap<String, UserId>,
    by_username: HashMap<String, UserId>,
    access_tokens: HashMap<String, TokenRecord>,
    refresh_tokens: HashMap<String, TokenRecord>,
    consumed_refresh_tokens: HashMap<String, Uuid>,
    devices: HashMap<DeviceId, DeviceRecord>,
    password_resets: HashMap<String, PasswordResetRecord>,
    friend_requests: HashMap<FriendRequestId, FriendRequest>,
    friendships: Vec<Friendship>,
    friend_settings: HashMap<(UserId, UserId), FriendSettings>,
    blocks: HashSet<(UserId, UserId)>,
    reports: Vec<UserReport>,
    audit_events: Vec<AdminAuditEntry>,
    conversations: HashMap<ConversationId, Conversation>,
    conversation_settings: HashMap<(UserId, ConversationId), ConversationSettings>,
    group_mute_all: HashSet<ConversationId>,
    group_announcements: HashMap<Uuid, GroupAnnouncement>,
    group_join_requests: HashMap<Uuid, GroupJoinRequest>,
    group_polls: HashMap<Uuid, GroupPoll>,
    messages: HashMap<ConversationId, Vec<Message>>,
    message_dedup: HashMap<(UserId, MessageId), (MessageId, u64)>,
    message_reactions: HashMap<MessageId, HashMap<String, HashSet<UserId>>>,
    message_receipts: HashMap<MessageId, ReceiptState>,
    favorite_messages: HashSet<(UserId, MessageId)>,
    message_expirations: HashMap<MessageId, DateTime<Utc>>,
    scheduled_messages: Vec<ScheduledMessage>,
    attachments: HashMap<AttachmentId, PendingAttachment>,
    stickers: HashMap<Uuid, Sticker>,
    read_positions: HashMap<(UserId, ConversationId), u64>,
    events: Vec<UserEvent>,
    cursor: u64,
}

impl Default for ChatService {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatService {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(512);
        let (typing, _) = broadcast::channel(256);
        let (calls, _) = broadcast::channel(512);
        Self {
            store: Arc::new(RwLock::new(Store::default())),
            events,
            typing,
            calls,
            connections: Arc::new(Mutex::new(HashMap::new())),
            qr_logins: Arc::new(Mutex::new(HashMap::new())),
            call_rooms: Arc::new(Mutex::new(HashMap::new())),
            data_encryption_key: derive_encryption_key("iamrust-development-only-encryption-key"),
            database: None,
        }
    }

    pub async fn postgres(database: PgPool) -> Result<Self, ApplicationError> {
        let payload = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT payload FROM application_state_snapshots WHERE singleton = true",
        )
        .fetch_optional(&database)
        .await
        .map_err(|_| ApplicationError::Storage)?;
        let store = payload.map_or_else(
            || Ok(Store::default()),
            |payload| rmp_serde::from_slice(&payload).map_err(|_| ApplicationError::Storage),
        )?;
        let (events, _) = broadcast::channel(512);
        let (typing, _) = broadcast::channel(256);
        let (calls, _) = broadcast::channel(512);
        Ok(Self {
            store: Arc::new(RwLock::new(store)),
            events,
            typing,
            calls,
            connections: Arc::new(Mutex::new(HashMap::new())),
            qr_logins: Arc::new(Mutex::new(HashMap::new())),
            call_rooms: Arc::new(Mutex::new(HashMap::new())),
            data_encryption_key: derive_encryption_key("iamrust-development-only-encryption-key"),
            database: Some(database),
        })
    }

    #[must_use]
    pub fn with_data_encryption_key(mut self, secret: &str) -> Self {
        self.data_encryption_key = derive_encryption_key(secret);
        self
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(Vec<UserId>, SyncEvent)> {
        self.events.subscribe()
    }

    pub fn subscribe_typing(&self) -> broadcast::Receiver<(Vec<UserId>, TypingSignal)> {
        self.typing.subscribe()
    }

    pub fn subscribe_calls(&self) -> broadcast::Receiver<(Vec<UserId>, CallSignalDelivery)> {
        self.calls.subscribe()
    }

    pub async fn publish_call_signal(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        call_id: Uuid,
        target_user_id: Option<UserId>,
        signal: CallSignal,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        let oversized = match &signal {
            CallSignal::Offer { sdp } | CallSignal::Answer { sdp } => sdp.len() > 262_144,
            CallSignal::IceCandidate { candidate, .. } => candidate.len() > 8_192,
            _ => false,
        };
        if oversized || call_id.is_nil() {
            return Err(DomainError::Validation {
                field: "call_signal",
                reason: "invalid_payload",
            }
            .into());
        }
        if matches!(signal, CallSignal::Participants { .. }) {
            return Err(DomainError::Forbidden.into());
        }
        let store = self.store.read().await;
        let conversation = store
            .conversations
            .get(&conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        if !conversation.can_send(actor, now) {
            return Err(DomainError::Forbidden.into());
        }
        let recipients = if let Some(target) = target_user_id {
            if target == actor || !conversation.members.contains_key(&target) {
                return Err(DomainError::Forbidden.into());
            }
            vec![target]
        } else {
            conversation
                .members
                .keys()
                .copied()
                .filter(|user_id| *user_id != actor)
                .collect::<Vec<_>>()
        };
        drop(store);
        let participant_roster = self
            .update_call_room(
                actor,
                conversation_id,
                call_id,
                target_user_id,
                &signal,
                now,
            )
            .await?;
        let _ = self.calls.send((
            recipients,
            CallSignalDelivery {
                conversation_id,
                call_id,
                from_user_id: actor,
                signal,
            },
        ));
        if let Some(user_ids) = participant_roster {
            let _ = self.calls.send((
                user_ids.clone(),
                CallSignalDelivery {
                    conversation_id,
                    call_id,
                    from_user_id: actor,
                    signal: CallSignal::Participants { user_ids },
                },
            ));
        }
        Ok(())
    }

    async fn update_call_room(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        call_id: Uuid,
        target_user_id: Option<UserId>,
        signal: &CallSignal,
        now: DateTime<Utc>,
    ) -> Result<Option<Vec<UserId>>, ApplicationError> {
        let mut rooms = self.call_rooms.lock().await;
        rooms.retain(|_, room| room.expires_at > now);
        let expires_at = now + Duration::minutes(CALL_ROOM_TTL_MINUTES);
        match signal {
            CallSignal::Invite { .. } => {
                if rooms.contains_key(&(conversation_id, call_id)) {
                    return Err(ApplicationError::Conflict);
                }
                rooms.insert(
                    (conversation_id, call_id),
                    CallRoom {
                        participants: HashSet::from([actor]),
                        expires_at,
                    },
                );
                Ok(None)
            }
            CallSignal::Accept => {
                let room = rooms
                    .get_mut(&(conversation_id, call_id))
                    .ok_or(ApplicationError::NotFound)?;
                room.participants.insert(actor);
                room.expires_at = expires_at;
                let mut participants = room.participants.iter().copied().collect::<Vec<_>>();
                participants.sort_unstable();
                Ok(Some(participants))
            }
            CallSignal::Offer { .. }
            | CallSignal::Answer { .. }
            | CallSignal::IceCandidate { .. } => {
                let room = rooms
                    .get_mut(&(conversation_id, call_id))
                    .ok_or(ApplicationError::NotFound)?;
                if !room.participants.contains(&actor)
                    || target_user_id.is_some_and(|target| !room.participants.contains(&target))
                {
                    return Err(DomainError::Forbidden.into());
                }
                room.expires_at = expires_at;
                Ok(None)
            }
            CallSignal::Hangup => {
                if let Some(room) = rooms.get_mut(&(conversation_id, call_id)) {
                    room.participants.remove(&actor);
                    if room.participants.is_empty() {
                        rooms.remove(&(conversation_id, call_id));
                    } else {
                        room.expires_at = expires_at;
                    }
                }
                Ok(None)
            }
            CallSignal::Reject | CallSignal::Busy | CallSignal::Participants { .. } => Ok(None),
        }
    }

    pub async fn publish_typing(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        active: bool,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        let store = self.store.read().await;
        let conversation = store
            .conversations
            .get(&conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        if !conversation.can_read(actor) {
            return Err(DomainError::Forbidden.into());
        }
        let recipients = conversation
            .members
            .keys()
            .copied()
            .filter(|user_id| *user_id != actor)
            .collect::<Vec<_>>();
        drop(store);
        let _ = self.typing.send((
            recipients,
            TypingSignal {
                conversation_id,
                user_id: actor,
                active,
                expires_at: now + Duration::seconds(6),
            },
        ));
        Ok(())
    }

    pub async fn connection_opened(
        &self,
        actor: UserId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        let first = {
            let mut connections = self.connections.lock().await;
            let count = connections.entry(actor).or_default();
            *count = count.saturating_add(1);
            *count == 1
        };
        if !first {
            return Ok(());
        }
        if let Err(error) = self.set_connection_presence(actor, true, now).await {
            self.connections.lock().await.remove(&actor);
            return Err(error);
        }
        Ok(())
    }

    pub async fn connection_closed(
        &self,
        actor: UserId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        let last = {
            let mut connections = self.connections.lock().await;
            let Some(count) = connections.get_mut(&actor) else {
                return Ok(());
            };
            *count = count.saturating_sub(1);
            if *count == 0 {
                connections.remove(&actor);
                true
            } else {
                false
            }
        };
        if last {
            self.set_connection_presence(actor, false, now).await?;
        }
        Ok(())
    }

    async fn set_connection_presence(
        &self,
        actor: UserId,
        connected: bool,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let preferred = store
                .preferred_presence
                .get(&actor)
                .copied()
                .unwrap_or(Presence::Online);
            let profile = {
                let account = store
                    .accounts
                    .get_mut(&actor)
                    .filter(|account| account.deleted_at.is_none())
                    .ok_or(ApplicationError::NotFound)?;
                account.profile.presence = if connected {
                    preferred
                } else {
                    Presence::Offline
                };
                account.profile.last_seen_at = Some(now);
                account.profile.clone()
            };
            append_profile_events(store, actor, &profile, now)?;
            Ok(())
        })
        .await
    }

    async fn mutate<R>(
        &self,
        operation: impl FnOnce(&mut Store) -> Result<R, ApplicationError>,
    ) -> Result<R, ApplicationError> {
        let mut store = self.store.write().await;
        let previous = store.clone();
        let first_new_event = store.events.len();
        let result = match operation(&mut store) {
            Ok(result) => result,
            Err(error) => {
                *store = previous;
                return Err(error);
            }
        };

        if let Some(database) = &self.database {
            let Ok(payload) = rmp_serde::to_vec_named(&*store) else {
                *store = previous;
                return Err(ApplicationError::Storage);
            };
            if sqlx::query(
                "INSERT INTO application_state_snapshots (singleton, format_version, payload, updated_at) \
                 VALUES (true, 1, $1, now()) \
                 ON CONFLICT (singleton) DO UPDATE SET \
                 format_version = EXCLUDED.format_version, payload = EXCLUDED.payload, updated_at = now()",
            )
            .bind(payload)
            .execute(database)
            .await
            .is_err()
            {
                *store = previous;
                return Err(ApplicationError::Storage);
            }
        }

        let published: Vec<_> = store.events[first_new_event..]
            .iter()
            .map(|entry| {
                (
                    entry.recipients.iter().copied().collect::<Vec<_>>(),
                    entry.event.clone(),
                )
            })
            .collect();
        drop(store);
        for event in published {
            let _ = self.events.send(event);
        }
        Ok(result)
    }

    pub async fn register(
        &self,
        input: RegisterInput,
        now: DateTime<Utc>,
    ) -> Result<AuthenticatedSession, ApplicationError> {
        validate_email(&input.email)?;
        validate_username(&input.username)?;
        validate_password(&input.password)?;
        validate_device_name(&input.device_name)?;
        validate_device_metadata(&input.platform, &input.app_version)?;
        validate_nickname(&input.nickname)?;

        let email = input.email.trim().to_ascii_lowercase();
        let username_key = input.username.to_ascii_lowercase();
        let password_hash = hash_password(&input.password)?;
        let user_id = UserId::new();
        let device_id = DeviceId::new();
        let device_name = input.device_name;
        let platform = input.platform;
        let app_version = input.app_version;
        let profile = UserProfile {
            id: user_id,
            username: input.username,
            nickname: input.nickname.trim().to_owned(),
            avatar_url: None,
            avatar_attachment_id: None,
            signature: String::new(),
            gender: None,
            birthday: None,
            region: None,
            presence: Presence::Online,
            last_seen_at: Some(now),
        };

        self.mutate(move |store| {
            if store.by_email.contains_key(&email) || store.by_username.contains_key(&username_key)
            {
                return Err(ApplicationError::AccountConflict);
            }
            store.by_email.insert(email, user_id);
            store.by_username.insert(username_key, user_id);
            store.accounts.insert(
                user_id,
                Account {
                    password_hash,
                    profile: profile.clone(),
                    deleted_at: None,
                    suspended: false,
                    second_factor: None,
                    pending_second_factor: None,
                },
            );
            store
                .profile_privacy
                .insert(user_id, ProfilePrivacySettings::default());
            store.preferred_presence.insert(user_id, Presence::Online);
            store.devices.insert(
                device_id,
                DeviceRecord {
                    id: device_id,
                    user_id,
                    name: device_name,
                    platform,
                    app_version,
                    last_seen_at: now,
                    revoked_at: None,
                },
            );
            Ok(issue_session(store, user_id, device_id, profile, now))
        })
        .await
    }

    pub async fn login(
        &self,
        input: LoginInput,
        now: DateTime<Utc>,
    ) -> Result<AuthenticatedSession, ApplicationError> {
        self.login_with_second_factor(input, None, now).await
    }

    pub async fn login_with_second_factor(
        &self,
        input: LoginInput,
        second_factor_code: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<AuthenticatedSession, ApplicationError> {
        validate_device_name(&input.device_name)?;
        validate_device_metadata(&input.platform, &input.app_version)?;
        let lookup = input.login.trim().to_ascii_lowercase();
        let device_name = input.device_name;
        let platform = input.platform;
        let app_version = input.app_version;
        let encryption_key = self.data_encryption_key;
        self.mutate(move |store| {
            let user_id = store
                .by_email
                .get(&lookup)
                .or_else(|| store.by_username.get(&lookup))
                .copied();

            let verified = user_id
                .and_then(|id| store.accounts.get(&id))
                .filter(|account| account.deleted_at.is_none() && !account.suspended)
                .is_some_and(|account| verify_password(&input.password, &account.password_hash));
            if !verified {
                return Err(ApplicationError::InvalidCredentials);
            }

            let user_id = user_id.expect("verified account has an id");
            let profile = {
                let account = store.accounts.get_mut(&user_id).expect("account exists");
                verify_second_factor_for_account(
                    account,
                    second_factor_code.as_deref(),
                    now,
                    &encryption_key,
                )?;
                account.profile.presence = Presence::Online;
                account.profile.last_seen_at = Some(now);
                account.profile.clone()
            };
            let device_id = DeviceId::new();
            store.devices.insert(
                device_id,
                DeviceRecord {
                    id: device_id,
                    user_id,
                    name: device_name,
                    platform,
                    app_version,
                    last_seen_at: now,
                    revoked_at: None,
                },
            );
            Ok(issue_session(store, user_id, device_id, profile, now))
        })
        .await
    }

    pub async fn second_factor_status(
        &self,
        actor: UserId,
    ) -> Result<SecondFactorStatus, ApplicationError> {
        let store = self.store.read().await;
        let account = store
            .accounts
            .get(&actor)
            .filter(|account| account.deleted_at.is_none())
            .ok_or(ApplicationError::NotFound)?;
        Ok(SecondFactorStatus {
            enabled: account.second_factor.is_some(),
            recovery_codes_remaining: account
                .second_factor
                .as_ref()
                .map_or(0, |state| state.recovery_code_hashes.len()),
        })
    }

    pub async fn begin_second_factor_setup(
        &self,
        actor: UserId,
        now: DateTime<Utc>,
    ) -> Result<SecondFactorSetupResponse, ApplicationError> {
        let mut secret = vec![0_u8; 20];
        rand::rng().fill_bytes(&mut secret);
        let encoded = BASE32_NOPAD.encode(&secret);
        let encrypted_secret = encrypt_secret(&self.data_encryption_key, &secret)?;
        let expires_at = now + Duration::minutes(10);
        let encoded_for_response = encoded.clone();
        self.mutate(move |store| {
            let account = store
                .accounts
                .get_mut(&actor)
                .filter(|account| account.deleted_at.is_none())
                .ok_or(ApplicationError::NotFound)?;
            if account.second_factor.is_some() {
                return Err(ApplicationError::Conflict);
            }
            let username = account.profile.username.clone();
            account.pending_second_factor = Some(PendingSecondFactor {
                encrypted_secret,
                expires_at,
            });
            Ok(SecondFactorSetupResponse {
                secret: encoded_for_response.clone(),
                otpauth_uri: format!(
                    "otpauth://totp/I%20Am%20Rust:{username}?secret={encoded_for_response}&issuer=I%20Am%20Rust&algorithm=SHA1&digits=6&period=30"
                ),
                expires_at,
            })
        })
        .await
    }

    pub async fn enable_second_factor(
        &self,
        actor: UserId,
        code: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, ApplicationError> {
        let code = code.trim().to_owned();
        let encryption_key = self.data_encryption_key;
        self.mutate(move |store| {
            let account = store
                .accounts
                .get_mut(&actor)
                .filter(|account| account.deleted_at.is_none())
                .ok_or(ApplicationError::NotFound)?;
            let pending = account
                .pending_second_factor
                .as_ref()
                .filter(|pending| pending.expires_at > now)
                .cloned()
                .ok_or(ApplicationError::Conflict)?;
            let secret = decrypt_secret(&encryption_key, &pending.encrypted_secret)?;
            if !verify_totp(&secret, &code, now) {
                return Err(ApplicationError::InvalidSecondFactor);
            }
            let (recovery_codes, recovery_code_hashes) = generate_recovery_codes();
            account.second_factor = Some(SecondFactorState {
                encrypted_secret: pending.encrypted_secret,
                recovery_code_hashes,
                enabled_at: now,
            });
            account.pending_second_factor = None;
            Ok(recovery_codes)
        })
        .await
    }

    pub async fn disable_second_factor(
        &self,
        actor: UserId,
        current_password: &str,
        code: &str,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        let current_password = current_password.to_owned();
        let code = code.trim().to_owned();
        let encryption_key = self.data_encryption_key;
        self.mutate(move |store| {
            let account = store
                .accounts
                .get_mut(&actor)
                .filter(|account| account.deleted_at.is_none())
                .ok_or(ApplicationError::NotFound)?;
            if !verify_password(&current_password, &account.password_hash) {
                return Err(ApplicationError::InvalidCredentials);
            }
            verify_second_factor_for_account(account, Some(&code), now, &encryption_key)?;
            account.second_factor = None;
            account.pending_second_factor = None;
            Ok(())
        })
        .await
    }

    pub async fn regenerate_recovery_codes(
        &self,
        actor: UserId,
        current_password: &str,
        code: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, ApplicationError> {
        let current_password = current_password.to_owned();
        let code = code.trim().to_owned();
        let encryption_key = self.data_encryption_key;
        self.mutate(move |store| {
            let account = store
                .accounts
                .get_mut(&actor)
                .filter(|account| account.deleted_at.is_none())
                .ok_or(ApplicationError::NotFound)?;
            if !verify_password(&current_password, &account.password_hash) {
                return Err(ApplicationError::InvalidCredentials);
            }
            verify_second_factor_for_account(account, Some(&code), now, &encryption_key)?;
            let (recovery_codes, recovery_code_hashes) = generate_recovery_codes();
            let state = account
                .second_factor
                .as_mut()
                .ok_or(ApplicationError::Conflict)?;
            state.recovery_code_hashes = recovery_code_hashes;
            Ok(recovery_codes)
        })
        .await
    }

    pub async fn begin_qr_login(
        &self,
        device_name: String,
        platform: String,
        app_version: String,
        now: DateTime<Utc>,
    ) -> Result<QrLoginChallengeInfo, ApplicationError> {
        validate_device_name(&device_name)?;
        validate_device_metadata(&platform, &app_version)?;
        let challenge_id = Uuid::now_v7();
        let secret = random_token();
        let expires_at = now + Duration::minutes(5);
        self.qr_logins.lock().await.insert(
            challenge_id,
            QrLoginChallenge {
                secret_hash: hash_token(&secret),
                approved_user_id: None,
                device_name,
                platform,
                app_version,
                expires_at,
            },
        );
        Ok(QrLoginChallengeInfo {
            challenge_id,
            qr_payload: format!(
                "iamrust://auth/qr-login?challenge_id={challenge_id}&secret={secret}"
            ),
            secret,
            expires_at,
        })
    }

    pub async fn approve_qr_login(
        &self,
        actor: UserId,
        challenge_id: Uuid,
        secret: &str,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        let mut challenges = self.qr_logins.lock().await;
        challenges.retain(|_, challenge| challenge.expires_at > now);
        let challenge = challenges
            .get_mut(&challenge_id)
            .ok_or(ApplicationError::NotFound)?;
        if !secure_eq(&challenge.secret_hash, &hash_token(secret)) {
            return Err(ApplicationError::NotFound);
        }
        if challenge.approved_user_id.is_some() {
            return Err(ApplicationError::Conflict);
        }
        challenge.approved_user_id = Some(actor);
        Ok(())
    }

    pub async fn poll_qr_login(
        &self,
        challenge_id: Uuid,
        secret: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<AuthenticatedSession>, ApplicationError> {
        let approved = {
            let mut challenges = self.qr_logins.lock().await;
            challenges.retain(|_, challenge| challenge.expires_at > now);
            let challenge = challenges
                .get(&challenge_id)
                .ok_or(ApplicationError::NotFound)?;
            if !secure_eq(&challenge.secret_hash, &hash_token(secret)) {
                return Err(ApplicationError::NotFound);
            }
            let Some(user_id) = challenge.approved_user_id else {
                return Ok(None);
            };
            let challenge = challenges
                .remove(&challenge_id)
                .expect("verified QR login challenge exists");
            Some((user_id, challenge))
        };
        let Some((user_id, challenge)) = approved else {
            return Ok(None);
        };
        self.mutate(move |store| {
            let profile = store
                .accounts
                .get(&user_id)
                .filter(|account| account.deleted_at.is_none() && !account.suspended)
                .map(|account| account.profile.clone())
                .ok_or(ApplicationError::Unauthorized)?;
            let device_id = DeviceId::new();
            store.devices.insert(
                device_id,
                DeviceRecord {
                    id: device_id,
                    user_id,
                    name: challenge.device_name,
                    platform: challenge.platform,
                    app_version: challenge.app_version,
                    last_seen_at: now,
                    revoked_at: None,
                },
            );
            Ok(Some(issue_session(store, user_id, device_id, profile, now)))
        })
        .await
    }

    pub async fn authenticate_access(
        &self,
        access_token: &str,
        now: DateTime<Utc>,
    ) -> Result<UserId, ApplicationError> {
        self.authenticate_identity(access_token, now)
            .await
            .map(|(user_id, _)| user_id)
    }

    pub async fn authenticate_identity(
        &self,
        access_token: &str,
        now: DateTime<Utc>,
    ) -> Result<(UserId, DeviceId), ApplicationError> {
        let token_hash = hash_token(access_token);
        let store = self.store.read().await;
        let record = store
            .access_tokens
            .get(&token_hash)
            .ok_or(ApplicationError::Unauthorized)?;
        if record.revoked || record.expires_at <= now {
            return Err(ApplicationError::SessionExpired);
        }
        if store
            .devices
            .get(&record.device_id)
            .is_none_or(|device| device.revoked_at.is_some())
        {
            return Err(ApplicationError::SessionExpired);
        }
        if store
            .accounts
            .get(&record.user_id)
            .is_none_or(|account| account.deleted_at.is_some() || account.suspended)
        {
            return Err(ApplicationError::SessionExpired);
        }
        Ok((record.user_id, record.device_id))
    }

    pub async fn refresh(
        &self,
        refresh_token: &str,
        now: DateTime<Utc>,
    ) -> Result<AuthenticatedSession, ApplicationError> {
        let token_hash = hash_token(refresh_token);
        self.mutate(move |store| {
            if let Some(family_id) = store.consumed_refresh_tokens.get(&token_hash).copied() {
                revoke_family(store, family_id);
                return Ok(Err(ApplicationError::RefreshTokenReuse));
            }
            let record = store
                .refresh_tokens
                .remove(&token_hash)
                .ok_or(ApplicationError::Unauthorized)?;
            if record.revoked || record.expires_at <= now {
                return Err(ApplicationError::SessionExpired);
            }
            store
                .consumed_refresh_tokens
                .insert(token_hash, record.family_id);
            let profile = store
                .accounts
                .get(&record.user_id)
                .filter(|account| account.deleted_at.is_none())
                .map(|account| account.profile.clone())
                .ok_or(ApplicationError::Unauthorized)?;
            Ok(Ok(issue_session_in_family(
                store,
                record.user_id,
                record.device_id,
                profile,
                now,
                record.family_id,
            )))
        })
        .await?
    }

    pub async fn logout(&self, refresh_token: &str) -> Result<(), ApplicationError> {
        let token_hash = hash_token(refresh_token);
        self.mutate(move |store| {
            if let Some(record) = store.refresh_tokens.remove(&token_hash) {
                revoke_family(store, record.family_id);
                store
                    .consumed_refresh_tokens
                    .insert(token_hash, record.family_id);
            }
            Ok(())
        })
        .await
    }

    pub async fn devices(&self, user_id: UserId, current_device_id: DeviceId) -> Vec<DeviceInfo> {
        let mut devices: Vec<_> = self
            .store
            .read()
            .await
            .devices
            .values()
            .filter(|device| device.user_id == user_id && device.revoked_at.is_none())
            .map(|device| DeviceInfo {
                id: device.id,
                name: device.name.clone(),
                platform: device.platform.clone(),
                app_version: device.app_version.clone(),
                last_seen_at: device.last_seen_at,
                current: device.id == current_device_id,
            })
            .collect();
        devices.sort_by_key(|device| std::cmp::Reverse(device.last_seen_at));
        devices
    }

    pub async fn revoke_device(
        &self,
        actor: UserId,
        device_id: DeviceId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let device = store
                .devices
                .get_mut(&device_id)
                .filter(|device| device.user_id == actor)
                .ok_or(ApplicationError::NotFound)?;
            device.revoked_at = Some(now);
            for token in store
                .access_tokens
                .values_mut()
                .chain(store.refresh_tokens.values_mut())
            {
                if token.device_id == device_id {
                    token.revoked = true;
                }
            }
            Ok(())
        })
        .await
    }

    pub async fn change_password(
        &self,
        actor: UserId,
        current_password: String,
        new_password: String,
    ) -> Result<(), ApplicationError> {
        validate_password(&new_password)?;
        let new_hash = hash_password(&new_password)?;
        self.mutate(move |store| {
            let account = store
                .accounts
                .get_mut(&actor)
                .ok_or(ApplicationError::NotFound)?;
            if !verify_password(&current_password, &account.password_hash) {
                return Err(ApplicationError::InvalidCredentials);
            }
            account.password_hash = new_hash;
            revoke_user_sessions(store, actor, None);
            Ok(())
        })
        .await
    }

    pub async fn request_password_reset(
        &self,
        email: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<PasswordResetDelivery>, ApplicationError> {
        validate_email(email)?;
        let email = email.trim().to_ascii_lowercase();
        let reset_token = random_token();
        let token_hash = hash_token(&reset_token);
        let expires_at = now + Duration::minutes(15);
        self.mutate(move |store| {
            let Some(user_id) = store.by_email.get(&email).copied() else {
                return Ok(None);
            };
            store.password_resets.retain(|_, reset| {
                reset.user_id != user_id || reset.consumed || reset.expires_at <= now
            });
            store.password_resets.insert(
                token_hash,
                PasswordResetRecord {
                    user_id,
                    expires_at,
                    attempts: 0,
                    consumed: false,
                },
            );
            Ok(Some(PasswordResetDelivery {
                email,
                reset_token,
                expires_at,
            }))
        })
        .await
    }

    pub async fn reset_password(
        &self,
        reset_token: &str,
        new_password: String,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        validate_password(&new_password)?;
        let token_hash = hash_token(reset_token);
        let new_hash = hash_password(&new_password)?;
        self.mutate(move |store| {
            let user_id = {
                let reset = store
                    .password_resets
                    .get_mut(&token_hash)
                    .ok_or(ApplicationError::InvalidCredentials)?;
                reset.attempts = reset.attempts.saturating_add(1);
                if reset.consumed || reset.expires_at <= now || reset.attempts > 5 {
                    return Err(ApplicationError::InvalidCredentials);
                }
                reset.consumed = true;
                reset.user_id
            };
            store
                .accounts
                .get_mut(&user_id)
                .ok_or(ApplicationError::NotFound)?
                .password_hash = new_hash;
            revoke_user_sessions(store, user_id, Some(now));
            Ok(())
        })
        .await
    }

    pub async fn profile(&self, user_id: UserId) -> Result<UserProfile, ApplicationError> {
        self.store
            .read()
            .await
            .accounts
            .get(&user_id)
            .filter(|account| account.deleted_at.is_none())
            .map(|account| account.profile.clone())
            .ok_or(ApplicationError::NotFound)
    }

    pub async fn account_email(&self, user_id: UserId) -> Result<String, ApplicationError> {
        self.store
            .read()
            .await
            .by_email
            .iter()
            .find_map(|(email, id)| (*id == user_id).then(|| email.clone()))
            .ok_or(ApplicationError::NotFound)
    }

    pub async fn update_profile(
        &self,
        user_id: UserId,
        input: UpdateProfileInput,
        now: DateTime<Utc>,
    ) -> Result<UserProfile, ApplicationError> {
        self.mutate(move |store| {
            if let Some(attachment_id) = input.avatar_attachment_id {
                let attachment = store
                    .attachments
                    .get(&attachment_id)
                    .filter(|attachment| attachment.owner_id == user_id && attachment.available)
                    .ok_or(ApplicationError::NotFound)?;
                if attachment.attachment.kind != AttachmentKind::Image {
                    return Err(DomainError::Validation {
                        field: "avatar_attachment_id",
                        reason: "image_required",
                    }
                    .into());
                }
            }
            let current_presence = store
                .accounts
                .get(&user_id)
                .ok_or(ApplicationError::NotFound)?
                .profile
                .presence;
            let presence = input.presence.unwrap_or_else(|| {
                store
                    .preferred_presence
                    .get(&user_id)
                    .copied()
                    .unwrap_or(current_presence)
            });
            let profile = {
                let account = store
                    .accounts
                    .get_mut(&user_id)
                    .ok_or(ApplicationError::NotFound)?;
                account.profile.update_public_fields(UserProfileUpdate {
                    nickname: input.nickname,
                    signature: input.signature,
                    avatar_url: input.avatar_url,
                    avatar_attachment_id: input.avatar_attachment_id,
                    gender: input.gender,
                    birthday: input.birthday,
                    region: input.region,
                    presence,
                })?;
                account.profile.clone()
            };
            if let Some(presence) = input.presence {
                store.preferred_presence.insert(user_id, presence);
            }
            append_profile_events(store, user_id, &profile, now)?;
            Ok(profile)
        })
        .await
    }

    pub async fn profile_privacy(&self, actor: UserId) -> ProfilePrivacySettings {
        self.store
            .read()
            .await
            .profile_privacy
            .get(&actor)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn update_profile_privacy(
        &self,
        actor: UserId,
        settings: ProfilePrivacySettings,
        now: DateTime<Utc>,
    ) -> Result<ProfilePrivacySettings, ApplicationError> {
        self.mutate(move |store| {
            if store
                .accounts
                .get(&actor)
                .is_none_or(|account| account.deleted_at.is_some())
            {
                return Err(ApplicationError::NotFound);
            }
            store.profile_privacy.insert(actor, settings.clone());
            let recipients = store
                .friendships
                .iter()
                .filter(|friendship| friendship.contains(actor))
                .map(|friendship| {
                    if friendship.lower_user_id == actor {
                        friendship.upper_user_id
                    } else {
                        friendship.lower_user_id
                    }
                })
                .collect::<Vec<_>>();
            for recipient in recipients {
                let visible =
                    visible_profile(store, recipient, actor).ok_or(ApplicationError::NotFound)?;
                append_event(
                    store,
                    [recipient],
                    EventKind::PresenceUpdated,
                    json!({ "profile": visible }),
                    now,
                );
            }
            append_event(
                store,
                [actor],
                EventKind::PresenceUpdated,
                json!({ "privacy": settings }),
                now,
            );
            Ok(settings)
        })
        .await
    }

    pub async fn export_personal_data(
        &self,
        actor: UserId,
        now: DateTime<Utc>,
    ) -> Result<PersonalDataExport, ApplicationError> {
        let store = self.store.read().await;
        let account = store
            .accounts
            .get(&actor)
            .filter(|account| account.deleted_at.is_none())
            .ok_or(ApplicationError::NotFound)?;
        let email = store
            .by_email
            .iter()
            .find_map(|(email, id)| (*id == actor).then(|| email.clone()))
            .ok_or(ApplicationError::NotFound)?;
        let mut friend_ids = store
            .friendships
            .iter()
            .filter(|friendship| friendship.contains(actor))
            .map(|friendship| {
                if friendship.lower_user_id == actor {
                    friendship.upper_user_id
                } else {
                    friendship.lower_user_id
                }
            })
            .collect::<Vec<_>>();
        friend_ids.sort_unstable();
        let mut friend_requests = store
            .friend_requests
            .values()
            .filter(|request| request.sender_id == actor || request.recipient_id == actor)
            .cloned()
            .collect::<Vec<_>>();
        friend_requests.sort_by_key(|request| request.created_at);
        let mut conversations = store
            .conversations
            .values()
            .filter(|conversation| conversation.can_read(actor))
            .cloned()
            .collect::<Vec<_>>();
        conversations.sort_by_key(|conversation| conversation.created_at);
        let conversation_ids = conversations
            .iter()
            .map(|conversation| conversation.id)
            .collect::<HashSet<_>>();
        let mut messages = store
            .messages
            .iter()
            .filter(|(conversation_id, _)| conversation_ids.contains(conversation_id))
            .flat_map(|(_, messages)| messages.iter().cloned())
            .collect::<Vec<_>>();
        messages.sort_by_key(|message| (message.created_at, message.id));
        Ok(PersonalDataExport {
            generated_at: now,
            email,
            profile: account.profile.clone(),
            privacy: store
                .profile_privacy
                .get(&actor)
                .cloned()
                .unwrap_or_default(),
            friend_ids,
            friend_requests,
            conversations,
            messages,
        })
    }

    pub async fn delete_account(
        &self,
        actor: UserId,
        current_password: String,
        confirmation: String,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        if confirmation != "DELETE" {
            return Err(DomainError::Validation {
                field: "confirmation",
                reason: "confirmation_required",
            }
            .into());
        }
        self.mutate(move |store| delete_account_in_store(store, actor, &current_password, now))
            .await
    }

    pub async fn authorize_attachment(
        &self,
        actor: UserId,
        file_name: String,
        mime_type: String,
        byte_size: u64,
        sha256: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<AttachmentAuthorization, ApplicationError> {
        validate_attachment_metadata(&file_name, &mime_type, byte_size, sha256.as_deref())?;
        let id = AttachmentId::new();
        let kind = attachment_kind_for_mime(&mime_type);
        let storage_key = format!("uploads/{actor}/{id}");
        let attachment = Attachment {
            id,
            kind,
            file_name: file_name.trim().to_owned(),
            mime_type: mime_type.trim().to_ascii_lowercase(),
            byte_size,
            sha256: sha256.map(|value| value.to_ascii_lowercase()),
            storage_key,
            thumbnail_key: None,
        };
        let expires_at = now + Duration::minutes(10);
        self.mutate(move |store| {
            store.attachments.insert(
                id,
                PendingAttachment {
                    attachment: attachment.clone(),
                    owner_id: actor,
                    expires_at,
                    available: false,
                    quarantined: false,
                },
            );
            Ok(AttachmentAuthorization {
                attachment,
                expires_at,
            })
        })
        .await
    }

    pub async fn complete_attachment(
        &self,
        actor: UserId,
        attachment_id: AttachmentId,
        now: DateTime<Utc>,
    ) -> Result<Attachment, ApplicationError> {
        self.mutate(move |store| {
            let pending = store
                .attachments
                .get_mut(&attachment_id)
                .filter(|pending| pending.owner_id == actor)
                .ok_or(ApplicationError::NotFound)?;
            if pending.expires_at <= now {
                return Err(ApplicationError::Conflict);
            }
            if pending.quarantined {
                return Err(DomainError::Forbidden.into());
            }
            pending.available = true;
            Ok(pending.attachment.clone())
        })
        .await
    }

    pub async fn quarantine_attachment(
        &self,
        actor: UserId,
        attachment_id: AttachmentId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let pending = store
                .attachments
                .get_mut(&attachment_id)
                .filter(|pending| pending.owner_id == actor && !pending.available)
                .ok_or(ApplicationError::NotFound)?;
            pending.quarantined = true;
            pending.expires_at = now + Duration::days(7);
            Ok(())
        })
        .await
    }

    pub async fn pending_attachment(
        &self,
        actor: UserId,
        attachment_id: AttachmentId,
        now: DateTime<Utc>,
    ) -> Result<Attachment, ApplicationError> {
        self.store
            .read()
            .await
            .attachments
            .get(&attachment_id)
            .filter(|pending| {
                pending.owner_id == actor && pending.expires_at > now && !pending.quarantined
            })
            .map(|pending| pending.attachment.clone())
            .ok_or(ApplicationError::NotFound)
    }

    pub async fn stickers(&self, actor: UserId) -> Vec<Sticker> {
        let mut stickers = self
            .store
            .read()
            .await
            .stickers
            .values()
            .filter(|sticker| sticker.owner_id == actor)
            .cloned()
            .collect::<Vec<_>>();
        stickers.sort_by_key(|sticker| sticker.created_at);
        stickers
    }

    pub async fn create_sticker(
        &self,
        actor: UserId,
        request: CreateStickerRequest,
        now: DateTime<Utc>,
    ) -> Result<Sticker, ApplicationError> {
        let name = request.name.trim().to_owned();
        let shortcut = normalize_optional(request.shortcut);
        if name.is_empty()
            || name.chars().count() > 48
            || shortcut.as_deref().is_some_and(|value| {
                value.chars().count() > 32 || value.contains(char::is_whitespace)
            })
        {
            return Err(DomainError::Validation {
                field: "sticker",
                reason: "invalid_metadata",
            }
            .into());
        }
        self.mutate(move |store| {
            if store
                .stickers
                .values()
                .filter(|sticker| sticker.owner_id == actor)
                .count()
                >= 100
            {
                return Err(ApplicationError::Conflict);
            }
            let attachment = store
                .attachments
                .get(&request.attachment_id)
                .filter(|pending| {
                    pending.owner_id == actor
                        && pending.available
                        && pending.attachment.kind == AttachmentKind::Image
                        && pending.attachment.byte_size <= 10 * 1024 * 1024
                })
                .map(|pending| pending.attachment.clone())
                .ok_or(ApplicationError::NotFound)?;
            let sticker = Sticker {
                id: Uuid::now_v7(),
                owner_id: actor,
                attachment,
                name,
                shortcut,
                created_at: now,
            };
            store.stickers.insert(sticker.id, sticker.clone());
            Ok(sticker)
        })
        .await
    }

    pub async fn delete_sticker(
        &self,
        actor: UserId,
        sticker_id: Uuid,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            if store
                .stickers
                .get(&sticker_id)
                .is_none_or(|sticker| sticker.owner_id != actor)
            {
                return Err(ApplicationError::NotFound);
            }
            store.stickers.remove(&sticker_id);
            Ok(())
        })
        .await
    }

    pub async fn attachment_for_download(
        &self,
        actor: UserId,
        attachment_id: AttachmentId,
    ) -> Result<Attachment, ApplicationError> {
        let store = self.store.read().await;
        let pending = store
            .attachments
            .get(&attachment_id)
            .filter(|pending| pending.available)
            .ok_or(ApplicationError::NotFound)?;
        if pending.owner_id == actor {
            return Ok(pending.attachment.clone());
        }
        if store
            .accounts
            .values()
            .any(|account| account.profile.avatar_attachment_id == Some(attachment_id))
        {
            return Ok(pending.attachment.clone());
        }
        if store.conversations.values().any(|conversation| {
            conversation.avatar_attachment_id == Some(attachment_id) && conversation.can_read(actor)
        }) {
            return Ok(pending.attachment.clone());
        }
        let can_access = store.messages.values().flatten().any(|message| {
            message_contains_attachment(&message.content, attachment_id)
                && store
                    .conversations
                    .get(&message.conversation_id)
                    .is_some_and(|conversation| conversation.can_read(actor))
        });
        if !can_access {
            return Err(DomainError::Forbidden.into());
        }
        Ok(pending.attachment.clone())
    }

    pub async fn cleanup_expired_attachments(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, ApplicationError> {
        self.mutate(move |store| {
            let expired: Vec<_> = store
                .attachments
                .values()
                .filter(|pending| !pending.available && pending.expires_at <= now)
                .map(|pending| pending.attachment.storage_key.clone())
                .collect();
            store
                .attachments
                .retain(|_, pending| pending.available || pending.expires_at > now);
            Ok(expired)
        })
        .await
    }

    pub async fn search_user_exact(
        &self,
        actor: UserId,
        username: &str,
    ) -> Result<Option<UserProfile>, ApplicationError> {
        let store = self.store.read().await;
        let result = store
            .by_username
            .get(&username.trim().to_ascii_lowercase())
            .copied()
            .filter(|user_id| {
                *user_id != actor
                    && !store.blocks.contains(&(actor, *user_id))
                    && !store.blocks.contains(&(*user_id, actor))
            })
            .and_then(|user_id| visible_profile(&store, actor, user_id));
        Ok(result)
    }

    pub async fn send_friend_request(
        &self,
        sender_id: UserId,
        username: &str,
        message: String,
        now: DateTime<Utc>,
    ) -> Result<FriendRequest, ApplicationError> {
        let username = username.trim().to_ascii_lowercase();
        self.mutate(move |store| {
            let recipient_id = store
                .by_username
                .get(&username)
                .copied()
                .ok_or(ApplicationError::NotFound)?;
            if store.blocks.contains(&(sender_id, recipient_id))
                || store.blocks.contains(&(recipient_id, sender_id))
            {
                return Err(DomainError::Forbidden.into());
            }
            if are_friends(store, sender_id, recipient_id)
                || store.friend_requests.values().any(|request| {
                    request.status == FriendRequestStatus::Pending
                        && request.sender_id == sender_id
                        && request.recipient_id == recipient_id
                })
            {
                return Err(ApplicationError::Conflict);
            }
            let request = FriendRequest::new(sender_id, recipient_id, message, now)?;
            store.friend_requests.insert(request.id, request.clone());
            append_event(
                store,
                [sender_id, recipient_id],
                EventKind::FriendshipUpdated,
                json!({ "friend_request": request }),
                now,
            );
            Ok(request)
        })
        .await
    }

    pub async fn friend_requests(&self, user_id: UserId) -> Vec<FriendRequest> {
        let mut requests: Vec<_> = self
            .store
            .read()
            .await
            .friend_requests
            .values()
            .filter(|request| request.sender_id == user_id || request.recipient_id == user_id)
            .cloned()
            .collect();
        requests.sort_by_key(|request| std::cmp::Reverse(request.updated_at));
        requests
    }

    pub async fn friend_request_profiles(&self, actor: UserId) -> Vec<UserProfile> {
        let store = self.store.read().await;
        let ids: HashSet<_> = store
            .friend_requests
            .values()
            .filter_map(|request| {
                if request.sender_id == actor {
                    Some(request.recipient_id)
                } else if request.recipient_id == actor {
                    Some(request.sender_id)
                } else {
                    None
                }
            })
            .collect();
        let mut profiles: Vec<_> = ids
            .into_iter()
            .filter_map(|user_id| visible_profile(&store, actor, user_id))
            .collect();
        profiles.sort_by_key(|profile| profile.nickname.to_lowercase());
        profiles
    }

    pub async fn decide_friend_request(
        &self,
        actor: UserId,
        request_id: FriendRequestId,
        decision: FriendRequestDecision,
        now: DateTime<Utc>,
    ) -> Result<FriendRequest, ApplicationError> {
        self.mutate(move |store| {
            let request = {
                let request = store
                    .friend_requests
                    .get_mut(&request_id)
                    .ok_or(ApplicationError::NotFound)?;
                match decision {
                    FriendRequestDecision::Accept => request.accept(actor, now)?,
                    FriendRequestDecision::Reject => request.reject(actor, now)?,
                }
                request.clone()
            };
            if decision == FriendRequestDecision::Accept {
                let friendship = Friendship::new(request.sender_id, request.recipient_id, now)?;
                store.friendships.push(friendship);
                for (actor, friend) in [
                    (request.sender_id, request.recipient_id),
                    (request.recipient_id, request.sender_id),
                ] {
                    store.friend_settings.insert(
                        (actor, friend),
                        FriendSettings {
                            user_id: friend,
                            remark: None,
                            group: None,
                            share_presence: true,
                            allow_files: true,
                        },
                    );
                }
                let direct = Conversation::direct(request.sender_id, request.recipient_id, now)?;
                store_conversation(store, direct);
            }
            append_event(
                store,
                [request.sender_id, request.recipient_id],
                EventKind::FriendshipUpdated,
                json!({ "friend_request": request }),
                now,
            );
            Ok(request)
        })
        .await
    }

    pub async fn friends(&self, user_id: UserId) -> Vec<UserProfile> {
        let store = self.store.read().await;
        let ids: HashSet<_> = store
            .friendships
            .iter()
            .filter_map(|friendship| {
                if friendship.lower_user_id == user_id {
                    Some(friendship.upper_user_id)
                } else if friendship.upper_user_id == user_id {
                    Some(friendship.lower_user_id)
                } else {
                    None
                }
            })
            .collect();
        let mut friends: Vec<_> = ids
            .iter()
            .filter_map(|id| visible_profile(&store, user_id, *id))
            .collect();
        friends.sort_by_key(|profile| profile.nickname.to_lowercase());
        friends
    }

    pub async fn friend_settings(&self, actor: UserId) -> Vec<FriendSettings> {
        let mut settings: Vec<_> = self
            .store
            .read()
            .await
            .friend_settings
            .iter()
            .filter(|((owner, _), _)| *owner == actor)
            .map(|(_, settings)| settings.clone())
            .collect();
        settings.sort_by_key(|settings| settings.user_id);
        settings
    }

    pub async fn update_friend_settings(
        &self,
        actor: UserId,
        friend_id: UserId,
        request: UpdateFriendSettingsRequest,
        now: DateTime<Utc>,
    ) -> Result<FriendSettings, ApplicationError> {
        validate_optional_label(request.remark.as_deref(), "remark", 48)?;
        validate_optional_label(request.group.as_deref(), "friend_group", 48)?;
        self.mutate(move |store| {
            if !are_friends(store, actor, friend_id) {
                return Err(ApplicationError::NotFound);
            }
            let settings = FriendSettings {
                user_id: friend_id,
                remark: normalize_optional(request.remark),
                group: normalize_optional(request.group),
                share_presence: request.share_presence,
                allow_files: request.allow_files,
            };
            store
                .friend_settings
                .insert((actor, friend_id), settings.clone());
            append_event(
                store,
                [actor],
                EventKind::FriendshipUpdated,
                json!({ "friend_settings": settings }),
                now,
            );
            Ok(settings)
        })
        .await
    }

    pub async fn delete_friend(
        &self,
        actor: UserId,
        friend_id: UserId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let before = store.friendships.len();
            store.friendships.retain(|friendship| {
                !(friendship.contains(actor) && friendship.contains(friend_id))
            });
            if before == store.friendships.len() {
                return Err(ApplicationError::NotFound);
            }
            store.friend_settings.remove(&(actor, friend_id));
            store.friend_settings.remove(&(friend_id, actor));
            append_event(
                store,
                [actor, friend_id],
                EventKind::FriendshipUpdated,
                json!({ "removed_friend_ids": [actor, friend_id] }),
                now,
            );
            Ok(())
        })
        .await
    }

    pub async fn block_user(
        &self,
        actor: UserId,
        target: UserId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        if actor == target {
            return Err(DomainError::SelfTarget.into());
        }
        self.mutate(move |store| {
            if !store.accounts.contains_key(&target) {
                return Err(ApplicationError::NotFound);
            }
            store.blocks.insert((actor, target));
            store
                .friendships
                .retain(|friendship| !(friendship.contains(actor) && friendship.contains(target)));
            store.friend_settings.remove(&(actor, target));
            store.friend_settings.remove(&(target, actor));
            for request in store.friend_requests.values_mut() {
                if request.status == FriendRequestStatus::Pending
                    && ((request.sender_id == actor && request.recipient_id == target)
                        || (request.sender_id == target && request.recipient_id == actor))
                {
                    request.status = FriendRequestStatus::Cancelled;
                    request.updated_at = now;
                }
            }
            append_event(
                store,
                [actor, target],
                EventKind::FriendshipUpdated,
                json!({ "blocked_by": actor, "target_id": target }),
                now,
            );
            Ok(())
        })
        .await
    }

    pub async fn unblock_user(
        &self,
        actor: UserId,
        target: UserId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            if !store.blocks.remove(&(actor, target)) {
                return Err(ApplicationError::NotFound);
            }
            append_event(
                store,
                [actor],
                EventKind::FriendshipUpdated,
                json!({ "unblocked_user_id": target }),
                now,
            );
            Ok(())
        })
        .await
    }

    pub async fn report_user(
        &self,
        actor: UserId,
        target: UserId,
        request: ReportUserRequest,
        now: DateTime<Utc>,
    ) -> Result<Uuid, ApplicationError> {
        if actor == target {
            return Err(DomainError::SelfTarget.into());
        }
        let reason = request.reason.trim().to_owned();
        let allowed = [
            "spam",
            "harassment",
            "impersonation",
            "unsafe_file",
            "other",
        ];
        if !allowed.contains(&reason.as_str())
            || request
                .details
                .as_ref()
                .is_some_and(|value| value.chars().count() > 1000)
        {
            return Err(DomainError::Validation {
                field: "report",
                reason: "invalid_value",
            }
            .into());
        }
        self.mutate(move |store| {
            if !store.accounts.contains_key(&target) {
                return Err(ApplicationError::NotFound);
            }
            let id = Uuid::now_v7();
            store.reports.push(UserReport {
                id,
                reporter_id: actor,
                reported_id: target,
                reason,
                details: request.details,
                created_at: now,
            });
            Ok(id)
        })
        .await
    }

    pub async fn create_direct(
        &self,
        actor: UserId,
        peer: UserId,
        now: DateTime<Utc>,
    ) -> Result<Conversation, ApplicationError> {
        self.mutate(move |store| {
            if !are_friends(store, actor, peer) {
                return Err(DomainError::Forbidden.into());
            }
            if let Some(existing) = store.conversations.values().find(|conversation| {
                conversation.members.len() == 2
                    && conversation.members.contains_key(&actor)
                    && conversation.members.contains_key(&peer)
                    && matches!(
                        conversation.kind,
                        iamrust_domain::ConversationKind::Direct { .. }
                    )
            }) {
                return Ok(conversation_for_user(existing.clone(), actor));
            }
            let conversation = Conversation::direct(actor, peer, now)?;
            store_conversation(store, conversation.clone());
            for recipient in [actor, peer] {
                append_event(
                    store,
                    [recipient],
                    EventKind::ConversationUpdated,
                    json!({
                        "conversation": conversation_for_user(conversation.clone(), recipient)
                    }),
                    now,
                );
            }
            Ok(conversation_for_user(conversation, actor))
        })
        .await
    }

    pub async fn create_group(
        &self,
        owner: UserId,
        member_ids: Vec<UserId>,
        name: String,
        now: DateTime<Utc>,
    ) -> Result<Conversation, ApplicationError> {
        self.mutate(move |store| {
            if member_ids
                .iter()
                .any(|member| *member != owner && !are_friends(store, owner, *member))
            {
                return Err(DomainError::Forbidden.into());
            }
            let conversation = Conversation::group(owner, member_ids, name, now)?;
            let recipients: Vec<_> = conversation.members.keys().copied().collect();
            store_conversation(store, conversation.clone());
            append_event(
                store,
                recipients,
                EventKind::ConversationUpdated,
                json!({ "conversation": conversation }),
                now,
            );
            Ok(conversation)
        })
        .await
    }

    pub async fn update_group(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        request: UpdateGroupRequest,
        now: DateTime<Utc>,
    ) -> Result<Conversation, ApplicationError> {
        if let Some(name) = &request.name {
            validate_group_name(name)?;
        }
        if let Some(Some(avatar_url)) = &request.avatar_url {
            validate_public_url(avatar_url, "avatar_url")?;
        }
        self.mutate(move |store| {
            if let Some(Some(attachment_id)) = request.avatar_attachment_id {
                let pending = store
                    .attachments
                    .get(&attachment_id)
                    .filter(|pending| pending.owner_id == actor && pending.available)
                    .ok_or(ApplicationError::NotFound)?;
                if pending.attachment.kind != AttachmentKind::Image {
                    return Err(DomainError::Validation {
                        field: "avatar_attachment_id",
                        reason: "image_required",
                    }
                    .into());
                }
            }
            let (conversation, recipients, description) = {
                let conversation = store
                    .conversations
                    .get_mut(&conversation_id)
                    .ok_or(ApplicationError::NotFound)?;
                ensure_group(conversation)?;
                require_role_at_least(conversation, actor, MemberRole::Administrator)?;
                let mut changes = Vec::new();
                if let Some(name) = request.name {
                    conversation.name = name.trim().to_owned();
                    changes.push("群名称已更新");
                }
                if let Some(avatar_url) = request.avatar_url {
                    conversation.avatar_url = avatar_url;
                    conversation.avatar_attachment_id = None;
                    changes.push("群头像已更新");
                }
                if let Some(avatar_attachment_id) = request.avatar_attachment_id {
                    conversation.avatar_attachment_id = avatar_attachment_id;
                    if avatar_attachment_id.is_some() {
                        conversation.avatar_url = None;
                    }
                    changes.push("群头像已更新");
                }
                conversation.updated_at = now;
                (
                    conversation.clone(),
                    conversation.members.keys().copied().collect::<Vec<_>>(),
                    changes.join("，"),
                )
            };
            if !description.is_empty() {
                append_system_message(store, conversation_id, actor, description, now)?;
            }
            append_event(
                store,
                recipients,
                EventKind::ConversationUpdated,
                json!({ "conversation": conversation }),
                now,
            );
            Ok(conversation)
        })
        .await
    }

    pub async fn add_group_members(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        request: AddGroupMembersRequest,
        now: DateTime<Utc>,
    ) -> Result<Conversation, ApplicationError> {
        if request.member_ids.is_empty() || request.member_ids.len() > 100 {
            return Err(DomainError::Validation {
                field: "member_ids",
                reason: "invalid_count",
            }
            .into());
        }
        self.mutate(move |store| {
            let existing = store
                .conversations
                .get(&conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            ensure_group(existing)?;
            require_member(existing, actor)?;
            for member_id in &request.member_ids {
                if *member_id == actor || !are_friends(store, actor, *member_id) {
                    return Err(DomainError::Forbidden.into());
                }
                if !store.accounts.contains_key(member_id) {
                    return Err(ApplicationError::NotFound);
                }
            }

            let added: Vec<_> = {
                let conversation = store
                    .conversations
                    .get_mut(&conversation_id)
                    .expect("conversation checked");
                let mut added = Vec::new();
                for member_id in request.member_ids {
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        conversation.members.entry(member_id)
                    {
                        entry.insert(ConversationMember {
                            user_id: member_id,
                            role: MemberRole::Member,
                            nickname: None,
                            muted_until: None,
                            joined_at: now,
                        });
                        added.push(member_id);
                    }
                }
                if conversation.members.len() > 500 {
                    return Err(DomainError::Validation {
                        field: "member_ids",
                        reason: "group_member_limit",
                    }
                    .into());
                }
                conversation.updated_at = now;
                added
            };
            for member_id in &added {
                store.conversation_settings.insert(
                    (*member_id, conversation_id),
                    default_conversation_settings(conversation_id),
                );
            }
            if !added.is_empty() {
                append_system_message(
                    store,
                    conversation_id,
                    actor,
                    format!("已邀请 {} 位成员加入群聊", added.len()),
                    now,
                )?;
            }
            let conversation = store
                .conversations
                .get(&conversation_id)
                .expect("conversation exists")
                .clone();
            let recipients = conversation.members.keys().copied().collect::<Vec<_>>();
            append_event(
                store,
                recipients,
                EventKind::GroupMembershipUpdated,
                json!({ "conversation": conversation, "added_member_ids": added }),
                now,
            );
            Ok(conversation)
        })
        .await
    }

    pub async fn leave_group(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let recipients = {
                let conversation = store
                    .conversations
                    .get_mut(&conversation_id)
                    .ok_or(ApplicationError::NotFound)?;
                ensure_group(conversation)?;
                let member = conversation
                    .members
                    .get(&actor)
                    .ok_or(DomainError::Forbidden)?;
                if member.role == MemberRole::Owner {
                    return Err(ApplicationError::Conflict);
                }
                conversation.members.remove(&actor);
                conversation.updated_at = now;
                conversation.members.keys().copied().collect::<Vec<_>>()
            };
            store
                .conversation_settings
                .remove(&(actor, conversation_id));
            append_system_message(
                store,
                conversation_id,
                actor,
                "一位成员已退出群聊".to_owned(),
                now,
            )?;
            append_event(
                store,
                recipients,
                EventKind::GroupMembershipUpdated,
                json!({ "conversation_id": conversation_id, "left_user_id": actor }),
                now,
            );
            Ok(())
        })
        .await
    }

    pub async fn disband_group(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let conversation = store
                .conversations
                .get(&conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            ensure_group(conversation)?;
            require_role_at_least(conversation, actor, MemberRole::Owner)?;
            let recipients = conversation.members.keys().copied().collect::<Vec<_>>();
            store.conversations.remove(&conversation_id);
            store
                .conversation_settings
                .retain(|(_, id), _| *id != conversation_id);
            store.group_mute_all.remove(&conversation_id);
            append_event(
                store,
                recipients,
                EventKind::ConversationUpdated,
                json!({ "conversation_id": conversation_id, "disbanded": true }),
                now,
            );
            Ok(())
        })
        .await
    }

    pub async fn update_group_member(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        target: UserId,
        request: UpdateGroupMemberRequest,
        now: DateTime<Utc>,
    ) -> Result<Conversation, ApplicationError> {
        if request
            .nickname
            .as_ref()
            .and_then(Option::as_deref)
            .is_some_and(|value| value.trim().chars().count() > 48)
        {
            return Err(DomainError::Validation {
                field: "group_nickname",
                reason: "invalid_length",
            }
            .into());
        }
        self.mutate(move |store| {
            let (conversation, recipients) = {
                let conversation = store
                    .conversations
                    .get_mut(&conversation_id)
                    .ok_or(ApplicationError::NotFound)?;
                ensure_group(conversation)?;
                let actor_role = require_member(conversation, actor)?.role;
                let target_role = require_member(conversation, target)?.role;
                if request.nickname.is_some()
                    && actor != target
                    && actor_role < MemberRole::Administrator
                {
                    return Err(DomainError::Forbidden.into());
                }
                if let Some(role) = request.role
                    && (actor_role != MemberRole::Owner
                        || role == MemberRole::Owner
                        || target_role == MemberRole::Owner)
                {
                    return Err(DomainError::Forbidden.into());
                }
                if request.muted_until.is_some()
                    && (actor_role < MemberRole::Administrator || actor_role <= target_role)
                {
                    return Err(DomainError::Forbidden.into());
                }
                let member = conversation
                    .members
                    .get_mut(&target)
                    .expect("membership checked");
                if let Some(nickname) = request.nickname {
                    member.nickname = normalize_optional(nickname);
                }
                if let Some(role) = request.role {
                    member.role = role;
                }
                if let Some(muted_until) = request.muted_until {
                    member.muted_until = muted_until;
                }
                conversation.updated_at = now;
                (
                    conversation.clone(),
                    conversation.members.keys().copied().collect::<Vec<_>>(),
                )
            };
            append_system_message(
                store,
                conversation_id,
                actor,
                "群成员设置已更新".to_owned(),
                now,
            )?;
            append_event(
                store,
                recipients,
                EventKind::GroupMembershipUpdated,
                json!({ "conversation": conversation, "updated_member_id": target }),
                now,
            );
            Ok(conversation)
        })
        .await
    }

    pub async fn remove_group_member(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        target: UserId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let recipients = {
                let conversation = store
                    .conversations
                    .get_mut(&conversation_id)
                    .ok_or(ApplicationError::NotFound)?;
                ensure_group(conversation)?;
                conversation.remove_member(actor, target, now)?;
                conversation.members.keys().copied().collect::<Vec<_>>()
            };
            store
                .conversation_settings
                .remove(&(target, conversation_id));
            append_system_message(
                store,
                conversation_id,
                actor,
                "一位成员已被移出群聊".to_owned(),
                now,
            )?;
            append_event(
                store,
                recipients,
                EventKind::GroupMembershipUpdated,
                json!({ "conversation_id": conversation_id, "removed_member_id": target }),
                now,
            );
            Ok(())
        })
        .await
    }

    pub async fn transfer_group_ownership(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        request: TransferGroupOwnershipRequest,
        now: DateTime<Utc>,
    ) -> Result<Conversation, ApplicationError> {
        self.mutate(move |store| {
            let (conversation, recipients) = {
                let conversation = store
                    .conversations
                    .get_mut(&conversation_id)
                    .ok_or(ApplicationError::NotFound)?;
                ensure_group(conversation)?;
                conversation.transfer_ownership(actor, request.user_id, now)?;
                (
                    conversation.clone(),
                    conversation.members.keys().copied().collect::<Vec<_>>(),
                )
            };
            append_system_message(store, conversation_id, actor, "群主已转让".to_owned(), now)?;
            append_event(
                store,
                recipients,
                EventKind::GroupMembershipUpdated,
                json!({ "conversation": conversation }),
                now,
            );
            Ok(conversation)
        })
        .await
    }

    pub async fn set_group_mute(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        request: GroupMuteRequest,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let recipients = {
                let conversation = store
                    .conversations
                    .get(&conversation_id)
                    .ok_or(ApplicationError::NotFound)?;
                ensure_group(conversation)?;
                require_role_at_least(conversation, actor, MemberRole::Administrator)?;
                conversation.members.keys().copied().collect::<Vec<_>>()
            };
            if request.muted {
                store.group_mute_all.insert(conversation_id);
            } else {
                store.group_mute_all.remove(&conversation_id);
            }
            append_system_message(
                store,
                conversation_id,
                actor,
                if request.muted {
                    "群聊已开启全员禁言".to_owned()
                } else {
                    "群聊已解除全员禁言".to_owned()
                },
                now,
            )?;
            append_event(
                store,
                recipients,
                EventKind::GroupMembershipUpdated,
                json!({ "conversation_id": conversation_id, "muted": request.muted }),
                now,
            );
            Ok(())
        })
        .await
    }

    pub async fn group_announcements(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
    ) -> Result<Vec<GroupAnnouncement>, ApplicationError> {
        let store = self.store.read().await;
        let conversation = store
            .conversations
            .get(&conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        ensure_group(conversation)?;
        require_member(conversation, actor)?;
        let mut announcements: Vec<_> = store
            .group_announcements
            .values()
            .filter(|announcement| announcement.conversation_id == conversation_id)
            .cloned()
            .collect();
        announcements.sort_by_key(|announcement| std::cmp::Reverse(announcement.updated_at));
        Ok(announcements)
    }

    pub async fn group_files(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
    ) -> Result<Vec<GroupFileItem>, ApplicationError> {
        let store = self.store.read().await;
        let conversation = store
            .conversations
            .get(&conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        ensure_group(conversation)?;
        require_member(conversation, actor)?;
        Ok(store
            .messages
            .get(&conversation_id)
            .into_iter()
            .flatten()
            .rev()
            .filter_map(|message| {
                let MessageContent::File { attachment } = &message.content else {
                    return None;
                };
                Some(GroupFileItem {
                    message_id: message.id,
                    sender_id: message.sender_id,
                    attachment: attachment.clone(),
                    created_at: message.server_created_at.unwrap_or(message.created_at),
                })
            })
            .take(500)
            .collect())
    }

    pub async fn group_mute_status(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
    ) -> Result<bool, ApplicationError> {
        let store = self.store.read().await;
        let conversation = store
            .conversations
            .get(&conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        if !matches!(conversation.kind, ConversationKind::Group { .. })
            || !conversation.can_read(actor)
        {
            return Err(DomainError::Forbidden.into());
        }
        Ok(store.group_mute_all.contains(&conversation_id))
    }

    pub async fn create_group_announcement(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        request: CreateGroupAnnouncementRequest,
        now: DateTime<Utc>,
    ) -> Result<GroupAnnouncement, ApplicationError> {
        let content = request.content.trim().to_owned();
        if content.is_empty() || content.chars().count() > 4000 {
            return Err(DomainError::Validation {
                field: "announcement",
                reason: "invalid_length",
            }
            .into());
        }
        self.mutate(move |store| {
            let recipients = {
                let conversation = store
                    .conversations
                    .get(&conversation_id)
                    .ok_or(ApplicationError::NotFound)?;
                ensure_group(conversation)?;
                require_role_at_least(conversation, actor, MemberRole::Administrator)?;
                conversation.members.keys().copied().collect::<Vec<_>>()
            };
            let announcement = GroupAnnouncement {
                id: Uuid::now_v7(),
                conversation_id,
                author_id: actor,
                content,
                read_by: vec![actor],
                created_at: now,
                updated_at: now,
            };
            store
                .group_announcements
                .insert(announcement.id, announcement.clone());
            append_system_message(
                store,
                conversation_id,
                actor,
                "发布了新的群公告".to_owned(),
                now,
            )?;
            append_event(
                store,
                recipients,
                EventKind::ConversationUpdated,
                json!({ "group_announcement": announcement }),
                now,
            );
            Ok(announcement)
        })
        .await
    }

    pub async fn read_group_announcement(
        &self,
        actor: UserId,
        announcement_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let conversation_id = store
                .group_announcements
                .get(&announcement_id)
                .map(|announcement| announcement.conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            let conversation = store
                .conversations
                .get(&conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            require_member(conversation, actor)?;
            let announcement = store
                .group_announcements
                .get_mut(&announcement_id)
                .expect("announcement checked");
            if !announcement.read_by.contains(&actor) {
                announcement.read_by.push(actor);
                announcement.updated_at = now;
            }
            Ok(())
        })
        .await
    }

    pub async fn request_to_join_group(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        request: CreateGroupJoinRequest,
        now: DateTime<Utc>,
    ) -> Result<GroupJoinRequest, ApplicationError> {
        if request.message.chars().count() > 120 {
            return Err(DomainError::Validation {
                field: "message",
                reason: "invalid_length",
            }
            .into());
        }
        self.mutate(move |store| {
            let recipients = {
                let conversation = store
                    .conversations
                    .get(&conversation_id)
                    .ok_or(ApplicationError::NotFound)?;
                ensure_group(conversation)?;
                if conversation.can_read(actor) {
                    return Err(ApplicationError::Conflict);
                }
                conversation
                    .members
                    .values()
                    .filter(|member| member.role >= MemberRole::Administrator)
                    .map(|member| member.user_id)
                    .collect::<Vec<_>>()
            };
            if store.group_join_requests.values().any(|join_request| {
                join_request.conversation_id == conversation_id
                    && join_request.applicant_id == actor
                    && join_request.status == GroupJoinRequestStatus::Pending
            }) {
                return Err(ApplicationError::Conflict);
            }
            let join_request = GroupJoinRequest {
                id: Uuid::now_v7(),
                conversation_id,
                applicant_id: actor,
                message: request.message,
                status: GroupJoinRequestStatus::Pending,
                reviewed_by: None,
                created_at: now,
                updated_at: now,
            };
            store
                .group_join_requests
                .insert(join_request.id, join_request.clone());
            append_event(
                store,
                recipients,
                EventKind::GroupMembershipUpdated,
                json!({ "group_join_request": join_request }),
                now,
            );
            Ok(join_request)
        })
        .await
    }

    pub async fn group_join_requests(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
    ) -> Result<Vec<GroupJoinRequest>, ApplicationError> {
        let store = self.store.read().await;
        let conversation = store
            .conversations
            .get(&conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        ensure_group(conversation)?;
        require_role_at_least(conversation, actor, MemberRole::Administrator)?;
        let mut requests: Vec<_> = store
            .group_join_requests
            .values()
            .filter(|request| request.conversation_id == conversation_id)
            .cloned()
            .collect();
        requests.sort_by_key(|request| std::cmp::Reverse(request.updated_at));
        Ok(requests)
    }

    pub async fn decide_group_join_request(
        &self,
        actor: UserId,
        request_id: Uuid,
        request: DecideGroupJoinRequest,
        now: DateTime<Utc>,
    ) -> Result<GroupJoinRequest, ApplicationError> {
        self.mutate(move |store| {
            let (conversation_id, applicant_id) = {
                let join_request = store
                    .group_join_requests
                    .get(&request_id)
                    .ok_or(ApplicationError::NotFound)?;
                if join_request.status != GroupJoinRequestStatus::Pending {
                    return Err(ApplicationError::Conflict);
                }
                (join_request.conversation_id, join_request.applicant_id)
            };
            let recipients = {
                let conversation = store
                    .conversations
                    .get(&conversation_id)
                    .ok_or(ApplicationError::NotFound)?;
                require_role_at_least(conversation, actor, MemberRole::Administrator)?;
                let mut recipients = conversation.members.keys().copied().collect::<Vec<_>>();
                recipients.push(applicant_id);
                recipients
            };
            if request.accept {
                let conversation = store
                    .conversations
                    .get_mut(&conversation_id)
                    .expect("conversation checked");
                conversation.members.insert(
                    applicant_id,
                    ConversationMember {
                        user_id: applicant_id,
                        role: MemberRole::Member,
                        nickname: None,
                        muted_until: None,
                        joined_at: now,
                    },
                );
                conversation.updated_at = now;
                store.conversation_settings.insert(
                    (applicant_id, conversation_id),
                    default_conversation_settings(conversation_id),
                );
            }
            let join_request = store
                .group_join_requests
                .get_mut(&request_id)
                .expect("join request checked");
            join_request.status = if request.accept {
                GroupJoinRequestStatus::Accepted
            } else {
                GroupJoinRequestStatus::Rejected
            };
            join_request.reviewed_by = Some(actor);
            join_request.updated_at = now;
            let join_request = join_request.clone();
            if request.accept {
                append_system_message(
                    store,
                    conversation_id,
                    actor,
                    "一位新成员已加入群聊".to_owned(),
                    now,
                )?;
            }
            append_event(
                store,
                recipients,
                EventKind::GroupMembershipUpdated,
                json!({ "group_join_request": join_request }),
                now,
            );
            Ok(join_request)
        })
        .await
    }

    pub async fn create_group_poll(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        request: CreateGroupPollRequest,
        now: DateTime<Utc>,
    ) -> Result<GroupPoll, ApplicationError> {
        let question = request.question.trim().to_owned();
        let options: Vec<_> = request
            .options
            .into_iter()
            .map(|label| label.trim().to_owned())
            .collect();
        if question.is_empty()
            || question.chars().count() > 240
            || !(2..=10).contains(&options.len())
            || options
                .iter()
                .any(|option| option.is_empty() || option.chars().count() > 160)
            || request.closes_at.is_some_and(|closes_at| closes_at <= now)
        {
            return Err(DomainError::Validation {
                field: "poll",
                reason: "invalid_value",
            }
            .into());
        }
        self.mutate(move |store| {
            let recipients = {
                let conversation = store
                    .conversations
                    .get(&conversation_id)
                    .ok_or(ApplicationError::NotFound)?;
                ensure_group(conversation)?;
                require_member(conversation, actor)?;
                conversation.members.keys().copied().collect::<Vec<_>>()
            };
            let poll = GroupPoll {
                id: Uuid::now_v7(),
                conversation_id,
                creator_id: actor,
                question,
                multiple_choice: request.multiple_choice,
                options: options
                    .into_iter()
                    .map(|label| GroupPollOption {
                        id: Uuid::now_v7(),
                        label,
                        voter_ids: Vec::new(),
                    })
                    .collect(),
                closes_at: request.closes_at,
                created_at: now,
            };
            store.group_polls.insert(poll.id, poll.clone());
            append_system_message(
                store,
                conversation_id,
                actor,
                "发起了群投票".to_owned(),
                now,
            )?;
            append_event(
                store,
                recipients,
                EventKind::ConversationUpdated,
                json!({ "group_poll": poll }),
                now,
            );
            Ok(poll)
        })
        .await
    }

    pub async fn group_polls(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
    ) -> Result<Vec<GroupPoll>, ApplicationError> {
        let store = self.store.read().await;
        let conversation = store
            .conversations
            .get(&conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        ensure_group(conversation)?;
        require_member(conversation, actor)?;
        let mut polls: Vec<_> = store
            .group_polls
            .values()
            .filter(|poll| poll.conversation_id == conversation_id)
            .cloned()
            .collect();
        polls.sort_by_key(|poll| std::cmp::Reverse(poll.created_at));
        Ok(polls)
    }

    pub async fn vote_group_poll(
        &self,
        actor: UserId,
        poll_id: Uuid,
        request: VoteGroupPollRequest,
        now: DateTime<Utc>,
    ) -> Result<GroupPoll, ApplicationError> {
        self.mutate(move |store| {
            let conversation_id = store
                .group_polls
                .get(&poll_id)
                .map(|poll| poll.conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            let recipients = {
                let conversation = store
                    .conversations
                    .get(&conversation_id)
                    .ok_or(ApplicationError::NotFound)?;
                require_member(conversation, actor)?;
                conversation.members.keys().copied().collect::<Vec<_>>()
            };
            let poll = store.group_polls.get_mut(&poll_id).expect("poll checked");
            if poll.closes_at.is_some_and(|closes_at| closes_at <= now)
                || request.option_ids.is_empty()
                || (!poll.multiple_choice && request.option_ids.len() != 1)
                || request.option_ids.len() > poll.options.len()
                || request
                    .option_ids
                    .iter()
                    .any(|id| !poll.options.iter().any(|option| option.id == *id))
            {
                return Err(DomainError::Validation {
                    field: "option_ids",
                    reason: "invalid_value",
                }
                .into());
            }
            for option in &mut poll.options {
                option.voter_ids.retain(|voter| *voter != actor);
                if request.option_ids.contains(&option.id) {
                    option.voter_ids.push(actor);
                }
            }
            let poll = poll.clone();
            append_event(
                store,
                recipients,
                EventKind::ConversationUpdated,
                json!({ "group_poll": poll }),
                now,
            );
            Ok(poll)
        })
        .await
    }

    pub async fn conversations(&self, user_id: UserId) -> Vec<Conversation> {
        let store = self.store.read().await;
        let mut conversations: Vec<_> = store
            .conversations
            .values()
            .filter(|conversation| conversation.can_read(user_id))
            .filter_map(|conversation| {
                let settings = store.conversation_settings.get(&(user_id, conversation.id));
                if settings.is_some_and(|settings| settings.hidden) {
                    return None;
                }
                let mut conversation = conversation.clone();
                if let Some(settings) = settings {
                    conversation.pinned = settings.pinned;
                    conversation.muted = settings.muted;
                }
                Some(conversation_for_user(conversation, user_id))
            })
            .collect();
        conversations.sort_by_key(|conversation| {
            (
                std::cmp::Reverse(conversation.pinned),
                std::cmp::Reverse(conversation.updated_at),
            )
        });
        conversations
    }

    pub async fn conversation_settings(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
    ) -> Result<ConversationSettings, ApplicationError> {
        let store = self.store.read().await;
        let conversation = store
            .conversations
            .get(&conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        if !conversation.can_read(actor) {
            return Err(DomainError::Forbidden.into());
        }
        Ok(store
            .conversation_settings
            .get(&(actor, conversation_id))
            .cloned()
            .unwrap_or_else(|| default_conversation_settings(conversation_id)))
    }

    pub async fn conversation_states(&self, actor: UserId) -> Vec<ConversationState> {
        let store = self.store.read().await;
        store
            .conversations
            .values()
            .filter(|conversation| conversation.can_read(actor))
            .map(|conversation| {
                let settings = store
                    .conversation_settings
                    .get(&(actor, conversation.id))
                    .cloned()
                    .unwrap_or_else(|| default_conversation_settings(conversation.id));
                let unread_count = store.messages.get(&conversation.id).map_or(0, |messages| {
                    messages
                        .iter()
                        .filter(|message| {
                            message.sender_id != actor
                                && message
                                    .sequence
                                    .is_some_and(|sequence| sequence > settings.last_read_sequence)
                        })
                        .count() as u64
                });
                ConversationState {
                    conversation_id: conversation.id,
                    pinned: settings.pinned,
                    muted: settings.muted,
                    hidden: settings.hidden,
                    manually_unread: settings.manually_unread,
                    last_read_sequence: settings.last_read_sequence,
                    unread_count,
                    draft: settings.draft,
                    label: settings.label,
                }
            })
            .collect()
    }

    pub async fn update_conversation_settings(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        request: UpdateConversationSettingsRequest,
        now: DateTime<Utc>,
    ) -> Result<ConversationSettings, ApplicationError> {
        if request
            .draft
            .as_ref()
            .is_some_and(|draft| draft.chars().count() > 8000)
        {
            return Err(DomainError::Validation {
                field: "draft",
                reason: "invalid_length",
            }
            .into());
        }
        if request
            .label
            .as_ref()
            .and_then(Option::as_deref)
            .is_some_and(|label| label.trim().chars().count() > 48)
        {
            return Err(DomainError::Validation {
                field: "label",
                reason: "invalid_length",
            }
            .into());
        }
        self.mutate(move |store| {
            let conversation = store
                .conversations
                .get(&conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            if !conversation.can_read(actor) {
                return Err(DomainError::Forbidden.into());
            }
            let settings = store
                .conversation_settings
                .entry((actor, conversation_id))
                .or_insert_with(|| default_conversation_settings(conversation_id));
            if let Some(value) = request.pinned {
                settings.pinned = value;
            }
            if let Some(value) = request.muted {
                settings.muted = value;
            }
            if let Some(value) = request.hidden {
                settings.hidden = value;
            }
            if let Some(value) = request.manually_unread {
                settings.manually_unread = value;
            }
            if let Some(value) = request.draft {
                settings.draft = value;
            }
            if let Some(value) = request.label {
                settings.label = normalize_optional(value);
            }
            let settings = settings.clone();
            append_event(
                store,
                [actor],
                EventKind::ConversationUpdated,
                json!({ "conversation_settings": settings }),
                now,
            );
            Ok(settings)
        })
        .await
    }

    pub async fn mark_all_read(
        &self,
        actor: UserId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let conversation_ids: Vec<_> = store
                .conversations
                .values()
                .filter(|conversation| conversation.can_read(actor))
                .map(|conversation| conversation.id)
                .collect();
            for conversation_id in conversation_ids {
                let latest = store
                    .messages
                    .get(&conversation_id)
                    .and_then(|messages| messages.last())
                    .and_then(|message| message.sequence)
                    .unwrap_or_default();
                store
                    .read_positions
                    .insert((actor, conversation_id), latest);
                let settings = store
                    .conversation_settings
                    .entry((actor, conversation_id))
                    .or_insert_with(|| default_conversation_settings(conversation_id));
                settings.last_read_sequence = latest;
                settings.manually_unread = false;
            }
            append_event(
                store,
                [actor],
                EventKind::ReadPositionUpdated,
                json!({ "all_read": true }),
                now,
            );
            Ok(())
        })
        .await
    }

    pub async fn schedule_message(
        &self,
        actor: UserId,
        request: ScheduleMessageRequest,
        now: DateTime<Utc>,
    ) -> Result<ScheduledMessageResponse, ApplicationError> {
        let ScheduleMessageRequest {
            conversation_id,
            client_message_id,
            content,
            reply_to,
            mut mentions,
            mention_all,
            scheduled_for,
            expires_in_seconds,
        } = request;
        content.validate()?;
        mentions.sort_unstable();
        mentions.dedup();
        if scheduled_for < now + Duration::seconds(10)
            || scheduled_for > now + Duration::days(30)
            || expires_in_seconds.is_some_and(|seconds| !(5..=604_800).contains(&seconds))
        {
            return Err(DomainError::Validation {
                field: "scheduled_for",
                reason: "invalid_range",
            }
            .into());
        }
        self.mutate(move |store| {
            validate_message_mentions(
                store,
                actor,
                conversation_id,
                &content,
                &mentions,
                mention_all,
            )?;
            let conversation = store
                .conversations
                .get(&conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            if !conversation.can_send(actor, now) {
                return Err(DomainError::Forbidden.into());
            }
            if store.scheduled_messages.iter().any(|message| {
                message.actor == actor && message.client_message_id == client_message_id
            }) || store
                .message_dedup
                .contains_key(&(actor, client_message_id))
            {
                return Err(ApplicationError::Conflict);
            }
            let id = Uuid::now_v7();
            store.scheduled_messages.push(ScheduledMessage {
                id,
                actor,
                conversation_id,
                client_message_id,
                content,
                reply_to,
                mentions,
                mention_all,
                scheduled_for,
                expires_in_seconds,
            });
            Ok(ScheduledMessageResponse {
                schedule_id: id,
                scheduled_for,
            })
        })
        .await
    }

    pub async fn scheduled_messages(&self, actor: UserId) -> Vec<ScheduledMessageInfo> {
        let mut messages: Vec<_> = self
            .store
            .read()
            .await
            .scheduled_messages
            .iter()
            .filter(|message| message.actor == actor)
            .map(|message| ScheduledMessageInfo {
                schedule_id: message.id,
                conversation_id: message.conversation_id,
                content: message.content.clone(),
                reply_to: message.reply_to,
                mentions: message.mentions.clone(),
                mention_all: message.mention_all,
                scheduled_for: message.scheduled_for,
                expires_in_seconds: message.expires_in_seconds,
            })
            .collect();
        messages.sort_by_key(|message| message.scheduled_for);
        messages
    }

    pub async fn cancel_scheduled_message(
        &self,
        actor: UserId,
        schedule_id: Uuid,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let before = store.scheduled_messages.len();
            store
                .scheduled_messages
                .retain(|message| !(message.id == schedule_id && message.actor == actor));
            if before == store.scheduled_messages.len() {
                return Err(ApplicationError::NotFound);
            }
            Ok(())
        })
        .await
    }

    pub async fn deliver_due_messages(&self, now: DateTime<Utc>) -> usize {
        let due: Vec<_> = self
            .store
            .read()
            .await
            .scheduled_messages
            .iter()
            .filter(|message| message.scheduled_for <= now)
            .cloned()
            .collect();
        let mut delivered = 0;
        for scheduled in due {
            if self
                .send_message_request(
                    scheduled.actor,
                    scheduled.conversation_id,
                    SendMessageRequest {
                        client_message_id: scheduled.client_message_id,
                        content: scheduled.content,
                        reply_to: scheduled.reply_to,
                        mentions: scheduled.mentions,
                        mention_all: scheduled.mention_all,
                        expires_in_seconds: scheduled.expires_in_seconds,
                    },
                    now,
                )
                .await
                .is_ok()
                && self
                    .mutate(|store| {
                        store
                            .scheduled_messages
                            .retain(|message| message.id != scheduled.id);
                        Ok(())
                    })
                    .await
                    .is_ok()
            {
                delivered += 1;
            }
        }
        delivered
    }

    pub async fn send_message(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        client_message_id: MessageId,
        content: MessageContent,
        reply_to: Option<MessageId>,
        now: DateTime<Utc>,
    ) -> Result<(Message, MessageAck), ApplicationError> {
        self.send_message_request(
            actor,
            conversation_id,
            SendMessageRequest {
                client_message_id,
                content,
                reply_to,
                mentions: Vec::new(),
                mention_all: false,
                expires_in_seconds: None,
            },
            now,
        )
        .await
    }

    pub async fn send_message_request(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        request: SendMessageRequest,
        now: DateTime<Utc>,
    ) -> Result<(Message, MessageAck), ApplicationError> {
        if request
            .expires_in_seconds
            .is_some_and(|seconds| !(5..=604_800).contains(&seconds))
        {
            return Err(DomainError::Validation {
                field: "expires_in_seconds",
                reason: "invalid_range",
            }
            .into());
        }
        self.mutate(move |store| send_message_in_store(store, actor, conversation_id, request, now))
            .await
    }

    pub async fn messages(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        before_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<Message>, ApplicationError> {
        let store = self.store.read().await;
        let conversation = store
            .conversations
            .get(&conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        if !conversation.can_read(actor) {
            return Err(DomainError::Forbidden.into());
        }
        let mut messages: Vec<_> = store
            .messages
            .get(&conversation_id)
            .into_iter()
            .flatten()
            .filter(|message| {
                store
                    .message_expirations
                    .get(&message.id)
                    .is_none_or(|expires_at| *expires_at > Utc::now())
            })
            .filter(|message| {
                before_sequence
                    .is_none_or(|cursor| message.sequence.is_some_and(|seq| seq < cursor))
            })
            .rev()
            .take(limit.clamp(1, MAX_MESSAGE_PAGE))
            .cloned()
            .collect();
        messages.reverse();
        for message in &mut messages {
            if message.sender_id != actor {
                continue;
            }
            if let Some(receipts) = store.message_receipts.get(&message.id) {
                if receipts.read_by.iter().any(|user_id| {
                    *user_id != actor
                        && store
                            .profile_privacy
                            .get(user_id)
                            .cloned()
                            .unwrap_or_default()
                            .read_receipts_enabled
                }) {
                    message.status = iamrust_domain::MessageStatus::Read;
                } else if receipts
                    .delivered_to
                    .iter()
                    .any(|user_id| *user_id != actor)
                {
                    message.status = iamrust_domain::MessageStatus::Delivered;
                }
            }
        }
        Ok(messages)
    }

    pub async fn message_details(
        &self,
        actor: UserId,
        message_id: MessageId,
    ) -> Result<MessageDetails, ApplicationError> {
        let store = self.store.read().await;
        let message = find_message(&store, message_id).ok_or(ApplicationError::NotFound)?;
        let conversation = store
            .conversations
            .get(&message.conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        if !conversation.can_read(actor) {
            return Err(DomainError::Forbidden.into());
        }
        let mut reactions: Vec<_> = store
            .message_reactions
            .get(&message_id)
            .into_iter()
            .flat_map(|reactions| reactions.iter())
            .map(|(emoji, users)| {
                let mut user_ids = users.iter().copied().collect::<Vec<_>>();
                user_ids.sort_unstable();
                MessageReaction {
                    emoji: emoji.clone(),
                    user_ids,
                }
            })
            .collect();
        reactions.sort_by(|left, right| left.emoji.cmp(&right.emoji));
        let receipts = store
            .message_receipts
            .get(&message_id)
            .cloned()
            .unwrap_or_default();
        let mut delivered_to = receipts.delivered_to.into_iter().collect::<Vec<_>>();
        let mut read_by = receipts
            .read_by
            .into_iter()
            .filter(|user_id| {
                *user_id == actor
                    || store
                        .profile_privacy
                        .get(user_id)
                        .cloned()
                        .unwrap_or_default()
                        .read_receipts_enabled
            })
            .collect::<Vec<_>>();
        delivered_to.sort_unstable();
        read_by.sort_unstable();
        Ok(MessageDetails {
            message: message.clone(),
            reactions,
            delivered_to,
            read_by,
            favorited: store.favorite_messages.contains(&(actor, message_id)),
            expires_at: store.message_expirations.get(&message_id).copied(),
        })
    }

    pub async fn message_text_for_translation(
        &self,
        actor: UserId,
        message_id: MessageId,
    ) -> Result<String, ApplicationError> {
        let store = self.store.read().await;
        let message = find_message(&store, message_id).ok_or(ApplicationError::NotFound)?;
        let conversation = store
            .conversations
            .get(&message.conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        if !conversation.can_read(actor) {
            return Err(DomainError::Forbidden.into());
        }
        let MessageContent::Text { text } = &message.content else {
            return Err(DomainError::Validation {
                field: "message_id",
                reason: "text_message_required",
            }
            .into());
        };
        Ok(text.clone())
    }

    pub async fn message_audio_for_transcription(
        &self,
        actor: UserId,
        message_id: MessageId,
    ) -> Result<Attachment, ApplicationError> {
        let store = self.store.read().await;
        let message = find_message(&store, message_id).ok_or(ApplicationError::NotFound)?;
        let conversation = store
            .conversations
            .get(&message.conversation_id)
            .ok_or(ApplicationError::NotFound)?;
        if !conversation.can_read(actor) {
            return Err(DomainError::Forbidden.into());
        }
        let MessageContent::Audio { attachment, .. } = &message.content else {
            return Err(DomainError::Validation {
                field: "message_id",
                reason: "audio_message_required",
            }
            .into());
        };
        Ok(attachment.clone())
    }

    pub async fn acknowledge_delivery(
        &self,
        actor: UserId,
        message_id: MessageId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let message = find_message(store, message_id)
                .cloned()
                .ok_or(ApplicationError::NotFound)?;
            let conversation = store
                .conversations
                .get(&message.conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            if !conversation.can_read(actor) || message.sender_id == actor {
                return Err(DomainError::Forbidden.into());
            }
            store
                .message_receipts
                .entry(message_id)
                .or_default()
                .delivered_to
                .insert(actor);
            append_event(
                store,
                [message.sender_id],
                EventKind::MessageUpdated,
                json!({ "message_id": message_id, "delivered_to": actor }),
                now,
            );
            Ok(())
        })
        .await
    }

    pub async fn recall_message(
        &self,
        actor: UserId,
        message_id: MessageId,
        now: DateTime<Utc>,
    ) -> Result<Message, ApplicationError> {
        self.mutate(move |store| {
            let original = find_message(store, message_id)
                .cloned()
                .ok_or(ApplicationError::NotFound)?;
            let conversation = store
                .conversations
                .get(&original.conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            let can_moderate = matches!(conversation.kind, ConversationKind::Group { .. })
                && require_member(conversation, actor)?.role >= MemberRole::Administrator;
            if original.sender_id != actor && !can_moderate {
                return Err(DomainError::Forbidden.into());
            }
            if original.status == iamrust_domain::MessageStatus::Recalled
                || (original.sender_id == actor
                    && now.signed_duration_since(original.created_at) > Duration::minutes(2))
            {
                return Err(ApplicationError::Conflict);
            }
            let recipients = conversation.members.keys().copied().collect::<Vec<_>>();
            let messages = store
                .messages
                .get_mut(&original.conversation_id)
                .expect("message conversation exists");
            let message = messages
                .iter_mut()
                .find(|message| message.id == message_id)
                .expect("message checked");
            message.status = iamrust_domain::MessageStatus::Recalled;
            message.content = MessageContent::System {
                text: "消息已撤回".to_owned(),
            };
            message.edited_at = Some(now);
            let message = message.clone();
            store.message_expirations.remove(&message_id);
            append_event(
                store,
                recipients,
                EventKind::MessageUpdated,
                json!({ "message": message }),
                now,
            );
            Ok(message)
        })
        .await
    }

    pub async fn react_to_message(
        &self,
        actor: UserId,
        message_id: MessageId,
        emoji: String,
        active: bool,
        now: DateTime<Utc>,
    ) -> Result<Vec<MessageReaction>, ApplicationError> {
        let emoji = emoji.trim().to_owned();
        if emoji.is_empty() || emoji.chars().count() > 8 {
            return Err(DomainError::Validation {
                field: "emoji",
                reason: "invalid_length",
            }
            .into());
        }
        self.mutate(move |store| {
            let message = find_message(store, message_id).ok_or(ApplicationError::NotFound)?;
            let conversation = store
                .conversations
                .get(&message.conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            if !conversation.can_read(actor) {
                return Err(DomainError::Forbidden.into());
            }
            let recipients = conversation.members.keys().copied().collect::<Vec<_>>();
            let reactions = store.message_reactions.entry(message_id).or_default();
            let users = reactions.entry(emoji.clone()).or_default();
            if active {
                users.insert(actor);
            } else {
                users.remove(&actor);
            }
            reactions.retain(|_, users| !users.is_empty());
            let mut output = reactions
                .iter()
                .map(|(emoji, users)| {
                    let mut user_ids = users.iter().copied().collect::<Vec<_>>();
                    user_ids.sort_unstable();
                    MessageReaction {
                        emoji: emoji.clone(),
                        user_ids,
                    }
                })
                .collect::<Vec<_>>();
            output.sort_by(|left, right| left.emoji.cmp(&right.emoji));
            append_event(
                store,
                recipients,
                EventKind::MessageUpdated,
                json!({ "message_id": message_id, "reactions": output }),
                now,
            );
            Ok(output)
        })
        .await
    }

    pub async fn set_message_favorite(
        &self,
        actor: UserId,
        message_id: MessageId,
        favorite: bool,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let message = find_message(store, message_id).ok_or(ApplicationError::NotFound)?;
            let conversation = store
                .conversations
                .get(&message.conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            if !conversation.can_read(actor) {
                return Err(DomainError::Forbidden.into());
            }
            if favorite {
                store.favorite_messages.insert((actor, message_id));
            } else {
                store.favorite_messages.remove(&(actor, message_id));
            }
            Ok(())
        })
        .await
    }

    pub async fn favorite_messages(&self, actor: UserId) -> Vec<Message> {
        let store = self.store.read().await;
        let mut messages: Vec<_> = store
            .favorite_messages
            .iter()
            .filter(|(user_id, _)| *user_id == actor)
            .filter_map(|(_, message_id)| find_message(&store, *message_id))
            .cloned()
            .collect();
        messages.sort_by_key(|message| std::cmp::Reverse(message.server_created_at));
        messages
    }

    pub async fn forward_messages(
        &self,
        actor: UserId,
        message_ids: Vec<MessageId>,
        target_conversation_id: ConversationId,
        mode: ForwardMode,
        now: DateTime<Utc>,
    ) -> Result<Vec<Message>, ApplicationError> {
        let unique: HashSet<_> = message_ids.iter().copied().collect();
        if unique.is_empty() || unique.len() != message_ids.len() || unique.len() > 100 {
            return Err(DomainError::Validation {
                field: "message_ids",
                reason: "invalid_count",
            }
            .into());
        }
        if mode == ForwardMode::Merged && unique.len() < 2 {
            return Err(DomainError::Validation {
                field: "message_ids",
                reason: "bundle_requires_multiple_messages",
            }
            .into());
        }
        self.mutate(move |store| {
            forward_messages_in_store(store, actor, message_ids, target_conversation_id, mode, now)
        })
        .await
    }

    pub async fn mark_read(
        &self,
        actor: UserId,
        conversation_id: ConversationId,
        through_sequence: u64,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let conversation = store
                .conversations
                .get(&conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            if !conversation.can_read(actor) {
                return Err(DomainError::Forbidden.into());
            }
            let recipients: Vec<_> = conversation.members.keys().copied().collect();
            let latest = store
                .messages
                .get(&conversation_id)
                .and_then(|messages| messages.last())
                .and_then(|message| message.sequence)
                .unwrap_or_default();
            let next = through_sequence.min(latest);
            let read_message_ids: Vec<_> = store
                .messages
                .get(&conversation_id)
                .into_iter()
                .flatten()
                .filter(|message| {
                    message.sender_id != actor
                        && message.sequence.is_some_and(|sequence| sequence <= next)
                })
                .map(|message| message.id)
                .collect();
            for message_id in read_message_ids {
                let receipts = store.message_receipts.entry(message_id).or_default();
                receipts.delivered_to.insert(actor);
                receipts.read_by.insert(actor);
            }
            let position = store
                .read_positions
                .entry((actor, conversation_id))
                .or_default();
            *position = (*position).max(next);
            let settings = store
                .conversation_settings
                .entry((actor, conversation_id))
                .or_insert_with(|| default_conversation_settings(conversation_id));
            settings.last_read_sequence = settings.last_read_sequence.max(next);
            settings.manually_unread = false;
            append_event(
                store,
                recipients,
                EventKind::ReadPositionUpdated,
                json!({
                    "user_id": actor,
                    "conversation_id": conversation_id,
                    "through_sequence": next
                }),
                now,
            );
            Ok(())
        })
        .await
    }

    pub async fn sync(&self, actor: UserId, after: u64, limit: usize) -> SyncResponse {
        let store = self.store.read().await;
        let limit = limit.clamp(1, MAX_SYNC_PAGE);
        let events: Vec<_> = store
            .events
            .iter()
            .filter(|entry| entry.event.cursor > after && entry.recipients.contains(&actor))
            .take(limit + 1)
            .map(|entry| entry.event.clone())
            .collect();
        let has_more = events.len() > limit;
        let mut events = events;
        events.truncate(limit);
        let next_cursor = events.last().map_or(after, |event| event.cursor);
        SyncResponse {
            events,
            next_cursor,
            has_more,
        }
    }

    pub async fn bootstrap(&self, actor: UserId) -> Result<BootstrapResponse, ApplicationError> {
        let profile = self.profile(actor).await?;
        let profile_privacy = self.profile_privacy(actor).await;
        let friends = self.friends(actor).await;
        let friend_settings = self.friend_settings(actor).await;
        let friend_requests = self.friend_requests(actor).await;
        let friend_request_profiles = self.friend_request_profiles(actor).await;
        let conversations = self.conversations(actor).await;
        let conversation_states = self.conversation_states(actor).await;
        let cursor = self.store.read().await.cursor;
        Ok(BootstrapResponse {
            profile,
            profile_privacy,
            friends,
            friend_settings,
            friend_requests,
            friend_request_profiles,
            conversations,
            conversation_states,
            cursor,
            server_features: json!({
                "protocol_version": iamrust_protocol::WS_PROTOCOL_VERSION,
                "attachments": true,
                "voice_messages": true,
                "calls": true,
                "custom_stickers": true,
                "translation": true
            }),
        })
    }

    pub async fn latest_cursor(&self) -> u64 {
        self.store.read().await.cursor
    }

    pub async fn admin_set_user_suspended(
        &self,
        target: UserId,
        suspended: bool,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            let account = store
                .accounts
                .get_mut(&target)
                .filter(|account| account.deleted_at.is_none())
                .ok_or(ApplicationError::NotFound)?;
            account.suspended = suspended;
            if suspended {
                revoke_user_sessions(store, target, Some(now));
            }
            store.audit_events.push(AdminAuditEntry {
                id: Uuid::now_v7(),
                actor_id: None,
                action: if suspended {
                    "user.suspend".to_owned()
                } else {
                    "user.restore".to_owned()
                },
                target_user_id: Some(target),
                outcome: "success".to_owned(),
                created_at: now,
            });
            Ok(())
        })
        .await
    }

    pub async fn admin_revoke_user_sessions(
        &self,
        target: UserId,
        now: DateTime<Utc>,
    ) -> Result<(), ApplicationError> {
        self.mutate(move |store| {
            if store
                .accounts
                .get(&target)
                .is_none_or(|account| account.deleted_at.is_some())
            {
                return Err(ApplicationError::NotFound);
            }
            revoke_user_sessions(store, target, Some(now));
            store.audit_events.push(AdminAuditEntry {
                id: Uuid::now_v7(),
                actor_id: None,
                action: "sessions.revoke".to_owned(),
                target_user_id: Some(target),
                outcome: "success".to_owned(),
                created_at: now,
            });
            Ok(())
        })
        .await
    }

    pub async fn admin_audit(&self, limit: usize) -> Vec<AdminAuditEntry> {
        let store = self.store.read().await;
        let mut entries = store.audit_events.clone();
        entries.extend(store.reports.iter().map(|report| AdminAuditEntry {
            id: report.id,
            actor_id: Some(report.reporter_id),
            action: format!("report.{}", report.reason),
            target_user_id: Some(report.reported_id),
            outcome: "recorded".to_owned(),
            created_at: report.created_at,
        }));
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
        entries.truncate(limit.clamp(1, 500));
        entries
    }
}

fn delete_account_in_store(
    store: &mut Store,
    actor: UserId,
    current_password: &str,
    now: DateTime<Utc>,
) -> Result<(), ApplicationError> {
    let account = store
        .accounts
        .get(&actor)
        .filter(|account| account.deleted_at.is_none())
        .ok_or(ApplicationError::NotFound)?;
    if !verify_password(current_password, &account.password_hash) {
        return Err(ApplicationError::InvalidCredentials);
    }
    let recipients = account_relationship_recipients(store, actor);
    detach_account_from_conversations(store, actor, now);
    let profile = anonymize_account(store, actor, now)?;
    purge_account_relationships(store, actor, now);
    append_event(
        store,
        recipients,
        EventKind::PresenceUpdated,
        json!({ "profile": profile, "deleted": true }),
        now,
    );
    Ok(())
}

fn account_relationship_recipients(store: &Store, actor: UserId) -> HashSet<UserId> {
    let mut recipients = store
        .friendships
        .iter()
        .filter(|friendship| friendship.contains(actor))
        .flat_map(|friendship| [friendship.lower_user_id, friendship.upper_user_id])
        .filter(|user_id| *user_id != actor)
        .collect::<HashSet<_>>();
    for conversation in store.conversations.values() {
        if conversation.can_read(actor) {
            recipients.extend(
                conversation
                    .members
                    .keys()
                    .copied()
                    .filter(|user_id| *user_id != actor),
            );
        }
    }
    recipients
}

fn detach_account_from_conversations(store: &mut Store, actor: UserId, now: DateTime<Utc>) {
    for conversation in store.conversations.values_mut() {
        if !conversation.can_read(actor) {
            continue;
        }
        let successor = matches!(conversation.kind, ConversationKind::Group { .. })
            .then(|| conversation.members.get(&actor).map(|member| member.role))
            .flatten()
            .filter(|role| *role == MemberRole::Owner)
            .and_then(|_| {
                conversation
                    .members
                    .values()
                    .filter(|member| member.user_id != actor)
                    .max_by_key(|member| (member.role, std::cmp::Reverse(member.joined_at)))
                    .map(|member| member.user_id)
            });
        if let Some(member) = successor.and_then(|user_id| conversation.members.get_mut(&user_id)) {
            member.role = MemberRole::Owner;
        }
        conversation.members.remove(&actor);
        conversation.updated_at = now;
    }
}

fn anonymize_account(
    store: &mut Store,
    actor: UserId,
    now: DateTime<Utc>,
) -> Result<UserProfile, ApplicationError> {
    let account = store
        .accounts
        .get_mut(&actor)
        .ok_or(ApplicationError::NotFound)?;
    account.deleted_at = Some(now);
    account.profile.username = format!("deleted-{}", actor.to_string().replace('-', ""));
    account.profile.nickname = "已注销用户".to_owned();
    account.profile.avatar_url = None;
    account.profile.avatar_attachment_id = None;
    account.profile.signature.clear();
    account.profile.gender = None;
    account.profile.birthday = None;
    account.profile.region = None;
    account.profile.presence = Presence::Offline;
    account.profile.last_seen_at = None;
    Ok(account.profile.clone())
}

fn purge_account_relationships(store: &mut Store, actor: UserId, now: DateTime<Utc>) {
    store.by_email.retain(|_, user_id| *user_id != actor);
    store.by_username.retain(|_, user_id| *user_id != actor);
    revoke_user_sessions(store, actor, Some(now));
    store
        .password_resets
        .retain(|_, reset| reset.user_id != actor);
    store
        .friend_requests
        .retain(|_, request| request.sender_id != actor && request.recipient_id != actor);
    store
        .friendships
        .retain(|friendship| !friendship.contains(actor));
    store
        .friend_settings
        .retain(|(owner, friend), _| *owner != actor && *friend != actor);
    store
        .conversation_settings
        .retain(|(user_id, _), _| *user_id != actor);
    store
        .read_positions
        .retain(|(user_id, _), _| *user_id != actor);
    store
        .blocks
        .retain(|(owner, target)| *owner != actor && *target != actor);
    store.reports.retain(|report| report.reporter_id != actor);
    store
        .favorite_messages
        .retain(|(user_id, _)| *user_id != actor);
    store
        .scheduled_messages
        .retain(|message| message.actor != actor);
    store
        .stickers
        .retain(|_, sticker| sticker.owner_id != actor);
    store.profile_privacy.remove(&actor);
    store.preferred_presence.remove(&actor);
}

fn validate_device_name(device_name: &str) -> Result<(), DomainError> {
    if !(1..=80).contains(&device_name.trim().chars().count()) {
        return Err(DomainError::Validation {
            field: "device_name",
            reason: "invalid_length",
        });
    }
    Ok(())
}

fn validate_device_metadata(platform: &str, app_version: &str) -> Result<(), DomainError> {
    if platform.trim().is_empty()
        || platform.trim().chars().count() > 32
        || app_version.trim().is_empty()
        || app_version.trim().chars().count() > 32
    {
        return Err(DomainError::Validation {
            field: "device_metadata",
            reason: "invalid_length",
        });
    }
    Ok(())
}

fn validate_nickname(nickname: &str) -> Result<(), DomainError> {
    if !(1..=48).contains(&nickname.trim().chars().count()) {
        return Err(DomainError::Validation {
            field: "nickname",
            reason: "invalid_length",
        });
    }
    Ok(())
}

fn validate_optional_label(
    value: Option<&str>,
    field: &'static str,
    maximum: usize,
) -> Result<(), DomainError> {
    if value.is_some_and(|value| value.trim().chars().count() > maximum) {
        return Err(DomainError::Validation {
            field,
            reason: "invalid_length",
        });
    }
    Ok(())
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_owned();
        (!value.is_empty()).then_some(value)
    })
}

fn validate_group_name(name: &str) -> Result<(), DomainError> {
    if !(1..=80).contains(&name.trim().chars().count()) {
        return Err(DomainError::Validation {
            field: "group_name",
            reason: "invalid_length",
        });
    }
    Ok(())
}

fn validate_public_url(value: &str, field: &'static str) -> Result<(), DomainError> {
    let valid = Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
    if !valid {
        return Err(DomainError::Validation {
            field,
            reason: "invalid_url",
        });
    }
    Ok(())
}

fn validate_attachment_metadata(
    file_name: &str,
    mime_type: &str,
    byte_size: u64,
    sha256: Option<&str>,
) -> Result<(), DomainError> {
    let file_name = file_name.trim();
    let mime_type = mime_type.trim().to_ascii_lowercase();
    if file_name.is_empty()
        || file_name.chars().count() > 255
        || file_name
            .chars()
            .any(|character| matches!(character, '/' | '\\' | '\0'))
        || matches!(file_name, "." | "..")
    {
        return Err(DomainError::Validation {
            field: "file_name",
            reason: "unsafe_name",
        });
    }
    let maximum = if mime_type.starts_with("image/") || mime_type.starts_with("audio/") {
        25 * 1024 * 1024
    } else {
        100 * 1024 * 1024
    };
    if byte_size == 0 || byte_size > maximum {
        return Err(DomainError::Validation {
            field: "byte_size",
            reason: "invalid_range",
        });
    }
    if mime_type.len() > 127
        || !mime_type.contains('/')
        || mime_type.chars().any(char::is_whitespace)
    {
        return Err(DomainError::Validation {
            field: "mime_type",
            reason: "invalid_value",
        });
    }
    if sha256
        .is_some_and(|hash| hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(DomainError::Validation {
            field: "sha256",
            reason: "invalid_value",
        });
    }
    Ok(())
}

fn attachment_kind_for_mime(mime_type: &str) -> AttachmentKind {
    if mime_type.starts_with("image/") {
        AttachmentKind::Image
    } else if mime_type.starts_with("audio/") {
        AttachmentKind::Audio
    } else if mime_type.starts_with("video/") {
        AttachmentKind::Video
    } else {
        AttachmentKind::File
    }
}

fn message_contains_attachment(content: &MessageContent, attachment_id: AttachmentId) -> bool {
    match content {
        MessageContent::Image { attachment }
        | MessageContent::File { attachment }
        | MessageContent::Audio { attachment, .. }
        | MessageContent::Sticker { attachment, .. } => attachment.id == attachment_id,
        MessageContent::ForwardBundle { messages, .. } => messages
            .iter()
            .any(|message| message_contains_attachment(&message.content, attachment_id)),
        MessageContent::Text { .. } | MessageContent::System { .. } => false,
    }
}

fn validate_message_attachment(
    store: &Store,
    actor: UserId,
    content: &MessageContent,
) -> Result<(), ApplicationError> {
    let attachment = match content {
        MessageContent::Image { attachment }
        | MessageContent::File { attachment }
        | MessageContent::Audio { attachment, .. }
        | MessageContent::Sticker { attachment, .. } => attachment,
        MessageContent::ForwardBundle { .. } => return Err(DomainError::Forbidden.into()),
        MessageContent::Text { .. } | MessageContent::System { .. } => return Ok(()),
    };
    let pending = store
        .attachments
        .get(&attachment.id)
        .filter(|pending| pending.owner_id == actor && pending.available)
        .ok_or(DomainError::Forbidden)?;
    if pending.attachment != *attachment {
        return Err(DomainError::Validation {
            field: "attachment",
            reason: "metadata_mismatch",
        }
        .into());
    }
    Ok(())
}

fn send_message_in_store(
    store: &mut Store,
    actor: UserId,
    conversation_id: ConversationId,
    request: SendMessageRequest,
    now: DateTime<Utc>,
) -> Result<(Message, MessageAck), ApplicationError> {
    let SendMessageRequest {
        client_message_id,
        content,
        reply_to,
        mut mentions,
        mention_all,
        expires_in_seconds,
    } = request;
    if let Some((message_id, sequence)) = store
        .message_dedup
        .get(&(actor, client_message_id))
        .copied()
    {
        let existing = find_message(store, message_id)
            .cloned()
            .ok_or(ApplicationError::NotFound)?;
        return Ok((
            existing,
            MessageAck {
                client_message_id,
                message_id,
                sequence,
                server_time: now,
            },
        ));
    }
    mentions.sort_unstable();
    mentions.dedup();
    validate_message_mentions(
        store,
        actor,
        conversation_id,
        &content,
        &mentions,
        mention_all,
    )?;
    let recipients = message_recipients_for_send(store, actor, conversation_id, &content, now)?;
    validate_message_attachment(store, actor, &content)?;
    let sequence = store
        .messages
        .get(&conversation_id)
        .and_then(|messages| messages.last())
        .and_then(|message| message.sequence)
        .unwrap_or_default()
        + 1;
    let mut message = Message::pending(client_message_id, conversation_id, actor, content, now)?;
    message.reply_to = reply_to;
    message.mentions = mentions;
    message.mention_all = mention_all;
    message.mark_sent(sequence, now)?;
    store
        .messages
        .entry(conversation_id)
        .or_default()
        .push(message.clone());
    store
        .message_dedup
        .insert((actor, client_message_id), (message.id, sequence));
    store
        .message_receipts
        .entry(message.id)
        .or_default()
        .delivered_to
        .insert(actor);
    if let Some(seconds) = expires_in_seconds {
        store
            .message_expirations
            .insert(message.id, now + Duration::seconds(i64::from(seconds)));
    }
    if let Some(conversation) = store.conversations.get_mut(&conversation_id) {
        conversation.updated_at = now;
    }
    append_event(
        store,
        recipients,
        EventKind::MessageCreated,
        json!({ "message": message }),
        now,
    );
    let ack = MessageAck {
        client_message_id,
        message_id: message.id,
        sequence,
        server_time: now,
    };
    Ok((message, ack))
}

fn validate_message_mentions(
    store: &Store,
    actor: UserId,
    conversation_id: ConversationId,
    content: &MessageContent,
    mentions: &[UserId],
    mention_all: bool,
) -> Result<(), ApplicationError> {
    if mentions.is_empty() && !mention_all {
        return Ok(());
    }
    if !matches!(content, MessageContent::Text { .. }) || mentions.len() > 50 {
        return Err(DomainError::Validation {
            field: "mentions",
            reason: "invalid_mentions",
        }
        .into());
    }
    let conversation = store
        .conversations
        .get(&conversation_id)
        .ok_or(ApplicationError::NotFound)?;
    ensure_group(conversation)?;
    let actor_member = require_member(conversation, actor)?;
    if mentions
        .iter()
        .any(|user_id| *user_id == actor || !conversation.members.contains_key(user_id))
    {
        return Err(DomainError::Validation {
            field: "mentions",
            reason: "member_required",
        }
        .into());
    }
    if mention_all && actor_member.role == MemberRole::Member {
        return Err(DomainError::Forbidden.into());
    }
    Ok(())
}

fn message_recipients_for_send(
    store: &Store,
    actor: UserId,
    conversation_id: ConversationId,
    content: &MessageContent,
    now: DateTime<Utc>,
) -> Result<Vec<UserId>, ApplicationError> {
    let conversation = store
        .conversations
        .get(&conversation_id)
        .ok_or(ApplicationError::NotFound)?;
    if !conversation.can_send(actor, now) {
        return Err(DomainError::Forbidden.into());
    }
    if matches!(conversation.kind, ConversationKind::Group { .. })
        && store.group_mute_all.contains(&conversation_id)
        && require_member(conversation, actor)?.role == MemberRole::Member
    {
        return Err(DomainError::Forbidden.into());
    }
    if matches!(conversation.kind, ConversationKind::Direct { .. }) {
        let peer = conversation
            .members
            .keys()
            .copied()
            .find(|member| *member != actor)
            .ok_or(ApplicationError::NotFound)?;
        if !are_friends(store, actor, peer)
            || store.blocks.contains(&(actor, peer))
            || store.blocks.contains(&(peer, actor))
        {
            return Err(DomainError::Forbidden.into());
        }
        if matches!(
            content,
            MessageContent::Image { .. }
                | MessageContent::File { .. }
                | MessageContent::Audio { .. }
                | MessageContent::Sticker { .. }
        ) && store
            .friend_settings
            .get(&(peer, actor))
            .is_some_and(|settings| !settings.allow_files)
        {
            return Err(DomainError::Forbidden.into());
        }
    }
    Ok(conversation.members.keys().copied().collect())
}

fn ensure_group(conversation: &Conversation) -> Result<(), ApplicationError> {
    if !matches!(conversation.kind, ConversationKind::Group { .. }) {
        return Err(ApplicationError::NotFound);
    }
    Ok(())
}

fn require_member(
    conversation: &Conversation,
    user_id: UserId,
) -> Result<&ConversationMember, ApplicationError> {
    conversation
        .members
        .get(&user_id)
        .ok_or_else(|| DomainError::Forbidden.into())
}

fn require_role_at_least(
    conversation: &Conversation,
    user_id: UserId,
    minimum: MemberRole,
) -> Result<(), ApplicationError> {
    if require_member(conversation, user_id)?.role < minimum {
        return Err(DomainError::Forbidden.into());
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, ApplicationError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|_| ApplicationError::InvalidCredentials)
}

fn verify_password(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

fn verify_second_factor_for_account(
    account: &mut Account,
    code: Option<&str>,
    now: DateTime<Utc>,
    encryption_key: &[u8; 32],
) -> Result<(), ApplicationError> {
    let Some(state) = account.second_factor.as_mut() else {
        return Ok(());
    };
    let code = code
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .ok_or(ApplicationError::SecondFactorRequired)?;
    let secret = decrypt_secret(encryption_key, &state.encrypted_secret)?;
    if verify_totp(&secret, code, now) {
        return Ok(());
    }
    let normalized = normalize_recovery_code(code);
    let hashed = hash_token(&normalized);
    if let Some(index) = state
        .recovery_code_hashes
        .iter()
        .position(|candidate| secure_eq(candidate, &hashed))
    {
        state.recovery_code_hashes.swap_remove(index);
        return Ok(());
    }
    Err(ApplicationError::InvalidSecondFactor)
}

fn derive_encryption_key(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

fn encrypt_secret(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, ApplicationError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| ApplicationError::Storage)?;
    let mut nonce_bytes = [0_u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce: &Nonce<Aes256Gcm> = (&nonce_bytes).into();
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| ApplicationError::Storage)?;
    let mut encrypted = Vec::with_capacity(nonce_bytes.len() + ciphertext.len());
    encrypted.extend_from_slice(&nonce_bytes);
    encrypted.extend_from_slice(&ciphertext);
    Ok(encrypted)
}

fn decrypt_secret(key: &[u8; 32], encrypted: &[u8]) -> Result<Vec<u8>, ApplicationError> {
    if encrypted.len() <= 12 {
        return Err(ApplicationError::Storage);
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| ApplicationError::Storage)?;
    let nonce_bytes: &[u8; 12] = encrypted[..12]
        .try_into()
        .map_err(|_| ApplicationError::Storage)?;
    let nonce: &Nonce<Aes256Gcm> = nonce_bytes.into();
    cipher
        .decrypt(nonce, &encrypted[12..])
        .map_err(|_| ApplicationError::Storage)
}

fn verify_totp(secret: &[u8], code: &str, now: DateTime<Utc>) -> bool {
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let counter = now.timestamp().max(0).cast_unsigned() / 30;
    [
        counter.saturating_sub(1),
        counter,
        counter.saturating_add(1),
    ]
    .into_iter()
    .any(|candidate| secure_eq(&totp_code(secret, candidate), code))
}

fn totp_code(secret: &[u8], counter: u64) -> String {
    let mut mac = <Hmac<Sha1> as hmac::KeyInit>::new_from_slice(secret)
        .expect("HMAC accepts secrets of any size");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let binary = (u32::from(digest[offset] & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    format!("{:06}", binary % 1_000_000)
}

fn generate_recovery_codes() -> (Vec<String>, Vec<String>) {
    let codes: Vec<_> = (0..10)
        .map(|_| {
            let mut bytes = [0_u8; 8];
            rand::rng().fill_bytes(&mut bytes);
            let raw = to_hex(&bytes);
            format!(
                "{}-{}-{}-{}",
                &raw[0..4],
                &raw[4..8],
                &raw[8..12],
                &raw[12..16]
            )
        })
        .collect();
    let hashes = codes
        .iter()
        .map(|code| hash_token(&normalize_recovery_code(code)))
        .collect();
    (codes, hashes)
}

fn normalize_recovery_code(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_hexdigit)
        .flat_map(char::to_lowercase)
        .collect()
}

fn secure_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn issue_session(
    store: &mut Store,
    user_id: UserId,
    device_id: DeviceId,
    profile: UserProfile,
    now: DateTime<Utc>,
) -> AuthenticatedSession {
    issue_session_in_family(store, user_id, device_id, profile, now, Uuid::now_v7())
}

fn issue_session_in_family(
    store: &mut Store,
    user_id: UserId,
    device_id: DeviceId,
    profile: UserProfile,
    now: DateTime<Utc>,
    family_id: Uuid,
) -> AuthenticatedSession {
    let access_token = random_token();
    let refresh_token = random_token();
    let access_expires_at = now + Duration::minutes(ACCESS_TOKEN_MINUTES);
    let refresh_expires_at = now + Duration::days(REFRESH_TOKEN_DAYS);
    store.access_tokens.insert(
        hash_token(&access_token),
        TokenRecord {
            user_id,
            device_id,
            family_id,
            expires_at: access_expires_at,
            revoked: false,
        },
    );
    store.refresh_tokens.insert(
        hash_token(&refresh_token),
        TokenRecord {
            user_id,
            device_id,
            family_id,
            expires_at: refresh_expires_at,
            revoked: false,
        },
    );
    AuthenticatedSession {
        access_token,
        refresh_token,
        access_expires_at,
        refresh_expires_at,
        profile,
        device_id,
    }
}

fn revoke_family(store: &mut Store, family_id: Uuid) {
    for record in store
        .access_tokens
        .values_mut()
        .chain(store.refresh_tokens.values_mut())
    {
        if family_id == record.family_id {
            record.revoked = true;
        }
    }
}

fn revoke_user_sessions(store: &mut Store, user_id: UserId, revoked_at: Option<DateTime<Utc>>) {
    for record in store
        .access_tokens
        .values_mut()
        .chain(store.refresh_tokens.values_mut())
    {
        if record.user_id == user_id {
            record.revoked = true;
        }
    }
    if let Some(revoked_at) = revoked_at {
        for device in store.devices.values_mut() {
            if device.user_id == user_id {
                device.revoked_at = Some(revoked_at);
            }
        }
    }
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    to_hex(&bytes)
}

fn hash_token(token: &str) -> String {
    to_hex(&Sha256::digest(token.as_bytes()))
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn are_friends(store: &Store, a: UserId, b: UserId) -> bool {
    store
        .friendships
        .iter()
        .any(|friendship| friendship.contains(a) && friendship.contains(b))
}

fn visible_profile(store: &Store, viewer: UserId, target: UserId) -> Option<UserProfile> {
    let account = store
        .accounts
        .get(&target)
        .filter(|account| account.deleted_at.is_none())?;
    let mut profile = account.profile.clone();
    if viewer == target {
        return Some(profile);
    }
    let privacy = store
        .profile_privacy
        .get(&target)
        .cloned()
        .unwrap_or_default();
    let can_view = |visibility: ProfileVisibility| match visibility {
        ProfileVisibility::Everyone => true,
        ProfileVisibility::Friends => are_friends(store, viewer, target),
        ProfileVisibility::Nobody => false,
    };
    if !can_view(privacy.gender_visibility) {
        profile.gender = None;
    }
    if !can_view(privacy.birthday_visibility) {
        profile.birthday = None;
    }
    if !can_view(privacy.region_visibility) {
        profile.region = None;
    }
    let friend_setting_hides_presence = store
        .friend_settings
        .get(&(target, viewer))
        .is_some_and(|settings| !settings.share_presence);
    if profile.presence == Presence::Invisible
        || !can_view(privacy.presence_visibility)
        || friend_setting_hides_presence
    {
        profile.presence = Presence::Offline;
        profile.last_seen_at = None;
    }
    Some(profile)
}

fn append_profile_events(
    store: &mut Store,
    actor: UserId,
    profile: &UserProfile,
    now: DateTime<Utc>,
) -> Result<(), ApplicationError> {
    let recipients = store
        .friendships
        .iter()
        .filter(|friendship| friendship.contains(actor))
        .map(|friendship| {
            if friendship.lower_user_id == actor {
                friendship.upper_user_id
            } else {
                friendship.lower_user_id
            }
        })
        .collect::<Vec<_>>();
    append_event(
        store,
        [actor],
        EventKind::PresenceUpdated,
        json!({ "profile": profile }),
        now,
    );
    for recipient in recipients {
        let visible = visible_profile(store, recipient, actor).ok_or(ApplicationError::NotFound)?;
        append_event(
            store,
            [recipient],
            EventKind::PresenceUpdated,
            json!({ "profile": visible }),
            now,
        );
    }
    Ok(())
}

fn default_conversation_settings(conversation_id: ConversationId) -> ConversationSettings {
    ConversationSettings {
        conversation_id,
        pinned: false,
        muted: false,
        hidden: false,
        manually_unread: false,
        last_read_sequence: 0,
        draft: String::new(),
        label: None,
    }
}

fn conversation_for_user(mut conversation: Conversation, user_id: UserId) -> Conversation {
    if matches!(conversation.kind, ConversationKind::Direct { .. })
        && let Some(peer_user_id) = conversation
            .members
            .keys()
            .copied()
            .find(|member_id| *member_id != user_id)
    {
        conversation.kind = ConversationKind::Direct { peer_user_id };
    }
    conversation
}

fn store_conversation(store: &mut Store, conversation: Conversation) {
    for user_id in conversation.members.keys().copied() {
        store
            .conversation_settings
            .entry((user_id, conversation.id))
            .or_insert_with(|| default_conversation_settings(conversation.id));
    }
    store.conversations.insert(conversation.id, conversation);
}

fn find_message(store: &Store, message_id: MessageId) -> Option<&Message> {
    store
        .messages
        .values()
        .flat_map(|messages| messages.iter())
        .find(|message| message.id == message_id)
}

fn forward_messages_in_store(
    store: &mut Store,
    actor: UserId,
    message_ids: Vec<MessageId>,
    target_conversation_id: ConversationId,
    mode: ForwardMode,
    now: DateTime<Utc>,
) -> Result<Vec<Message>, ApplicationError> {
    let source_messages = collect_forward_sources(store, actor, message_ids, mode)?;
    let target = store
        .conversations
        .get(&target_conversation_id)
        .ok_or(ApplicationError::NotFound)?;
    if !target.can_send(actor, now) {
        return Err(DomainError::Forbidden.into());
    }
    let recipients = target.members.keys().copied().collect::<Vec<_>>();
    let contents = match mode {
        ForwardMode::Individually => source_messages
            .into_iter()
            .map(|message| message.content)
            .collect(),
        ForwardMode::Merged => vec![MessageContent::ForwardBundle {
            title: "聊天记录".to_owned(),
            messages: source_messages,
        }],
    };
    let mut sequence = store
        .messages
        .get(&target_conversation_id)
        .and_then(|messages| messages.last())
        .and_then(|message| message.sequence)
        .unwrap_or_default();
    let mut forwarded = Vec::with_capacity(contents.len());
    for content in contents {
        sequence = sequence.saturating_add(1);
        forwarded.push(append_forwarded_message(
            store,
            actor,
            target_conversation_id,
            content,
            sequence,
            &recipients,
            now,
        )?);
    }
    if let Some(conversation) = store.conversations.get_mut(&target_conversation_id) {
        conversation.updated_at = now;
    }
    Ok(forwarded)
}

fn collect_forward_sources(
    store: &Store,
    actor: UserId,
    message_ids: Vec<MessageId>,
    mode: ForwardMode,
) -> Result<Vec<ForwardedMessage>, ApplicationError> {
    message_ids
        .into_iter()
        .map(|message_id| {
            let message = find_message(store, message_id)
                .cloned()
                .ok_or(ApplicationError::NotFound)?;
            let source = store
                .conversations
                .get(&message.conversation_id)
                .ok_or(ApplicationError::NotFound)?;
            if !source.can_read(actor) || matches!(&message.content, MessageContent::System { .. })
            {
                return Err(DomainError::Forbidden.into());
            }
            if mode == ForwardMode::Merged
                && matches!(message.content, MessageContent::ForwardBundle { .. })
            {
                return Err(DomainError::Validation {
                    field: "message_ids",
                    reason: "nested_bundle",
                }
                .into());
            }
            let sender_name = store.accounts.get(&message.sender_id).map_or_else(
                || "已注销用户".to_owned(),
                |account| account.profile.nickname.clone(),
            );
            Ok(ForwardedMessage {
                sender_id: message.sender_id,
                sender_name,
                content: message.content,
                created_at: message.server_created_at.unwrap_or(message.created_at),
            })
        })
        .collect()
}

fn append_forwarded_message(
    store: &mut Store,
    actor: UserId,
    conversation_id: ConversationId,
    content: MessageContent,
    sequence: u64,
    recipients: &[UserId],
    now: DateTime<Utc>,
) -> Result<Message, ApplicationError> {
    let client_message_id = MessageId::new();
    let mut message = Message::pending(client_message_id, conversation_id, actor, content, now)?;
    message.mark_sent(sequence, now)?;
    store
        .messages
        .entry(conversation_id)
        .or_default()
        .push(message.clone());
    store
        .message_dedup
        .insert((actor, client_message_id), (message.id, sequence));
    store
        .message_receipts
        .entry(message.id)
        .or_default()
        .delivered_to
        .insert(actor);
    append_event(
        store,
        recipients.iter().copied(),
        EventKind::MessageCreated,
        json!({ "message": message, "forwarded": true }),
        now,
    );
    Ok(message)
}

fn append_system_message(
    store: &mut Store,
    conversation_id: ConversationId,
    actor: UserId,
    text: String,
    now: DateTime<Utc>,
) -> Result<Message, ApplicationError> {
    let recipients = store
        .conversations
        .get(&conversation_id)
        .ok_or(ApplicationError::NotFound)?
        .members
        .keys()
        .copied()
        .collect::<Vec<_>>();
    let sequence = store
        .messages
        .get(&conversation_id)
        .and_then(|messages| messages.last())
        .and_then(|message| message.sequence)
        .unwrap_or_default()
        + 1;
    let client_message_id = MessageId::new();
    let mut message = Message::pending(
        client_message_id,
        conversation_id,
        actor,
        MessageContent::System { text },
        now,
    )?;
    message.mark_sent(sequence, now)?;
    store
        .messages
        .entry(conversation_id)
        .or_default()
        .push(message.clone());
    if let Some(conversation) = store.conversations.get_mut(&conversation_id) {
        conversation.updated_at = now;
    }
    append_event(
        store,
        recipients,
        EventKind::MessageCreated,
        json!({ "message": message }),
        now,
    );
    Ok(message)
}

fn append_event(
    store: &mut Store,
    recipients: impl IntoIterator<Item = UserId>,
    kind: EventKind,
    payload: serde_json::Value,
    now: DateTime<Utc>,
) {
    store.cursor = store.cursor.saturating_add(1);
    let recipients: HashSet<_> = recipients.into_iter().collect();
    let event = SyncEvent {
        id: Uuid::now_v7(),
        cursor: store.cursor,
        kind,
        payload,
        created_at: now,
    };
    store.events.push(UserEvent { recipients, event });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register_input(username: &str) -> RegisterInput {
        RegisterInput {
            email: format!("{username}@example.com"),
            username: username.to_owned(),
            password: "StrongPass1".to_owned(),
            nickname: username.to_owned(),
            device_name: "test".to_owned(),
            platform: "test".to_owned(),
            app_version: "0.1.0".to_owned(),
        }
    }

    fn login_input(username: &str) -> LoginInput {
        LoginInput {
            login: username.to_owned(),
            password: "StrongPass1".to_owned(),
            device_name: "test".to_owned(),
            platform: "test".to_owned(),
            app_version: "0.1.0".to_owned(),
        }
    }

    #[test]
    fn argon2_0_5_password_hashes_remain_valid() {
        let legacy_hash = "$argon2id$v=19$m=65536,t=2,p=1$c29tZXNhbHQ$CTFhFdXPJO1aFaMaO6Mm5c8y7cJHAph8ArZWb2GRPPc";
        assert!(verify_password("password", legacy_hash));
        assert!(!verify_password("wrong-password", legacy_hash));
    }

    #[tokio::test]
    async fn registration_and_login_do_not_reveal_duplicates_as_credentials() {
        let service = ChatService::new();
        let now = Utc::now();
        let first = service
            .register(register_input("alice"), now)
            .await
            .unwrap();
        assert_eq!(first.profile.username, "alice");
        assert_eq!(
            service.register(register_input("alice"), now).await,
            Err(ApplicationError::AccountConflict)
        );
        assert_eq!(
            service
                .login(
                    LoginInput {
                        login: "alice".to_owned(),
                        password: "wrong-password".to_owned(),
                        device_name: "test".to_owned(),
                        platform: "test".to_owned(),
                        app_version: "0.1.0".to_owned(),
                    },
                    now,
                )
                .await,
            Err(ApplicationError::InvalidCredentials)
        );
    }

    #[tokio::test]
    async fn administrators_can_suspend_restore_and_audit_accounts() {
        let service = ChatService::new();
        let now = Utc::now();
        let session = service
            .register(register_input("moderated_user"), now)
            .await
            .unwrap();
        service
            .admin_set_user_suspended(session.profile.id, true, now)
            .await
            .unwrap();
        assert_eq!(
            service
                .authenticate_access(&session.access_token, now)
                .await,
            Err(ApplicationError::SessionExpired)
        );
        assert!(
            service
                .login(
                    LoginInput {
                        login: "moderated_user".to_owned(),
                        password: "StrongPass1".to_owned(),
                        device_name: "test".to_owned(),
                        platform: "test".to_owned(),
                        app_version: "0.1.0".to_owned(),
                    },
                    now,
                )
                .await
                .is_err()
        );
        service
            .admin_set_user_suspended(session.profile.id, false, now)
            .await
            .unwrap();
        assert_eq!(service.admin_audit(10).await.len(), 2);
    }

    #[tokio::test]
    async fn service_state_snapshot_round_trips_non_string_indexes() {
        let service = ChatService::new();
        let now = Utc::now();
        let session = service
            .register(register_input("snapshot_user"), now)
            .await
            .unwrap();
        let store = service.store.read().await.clone();
        let encoded = rmp_serde::to_vec_named(&store).unwrap();
        let restored: Store = rmp_serde::from_slice(&encoded).unwrap();
        assert_eq!(
            restored
                .accounts
                .get(&session.profile.id)
                .map(|account| &account.profile),
            Some(&session.profile)
        );
        assert!(!restored.refresh_tokens.is_empty());
    }

    #[tokio::test]
    async fn password_reset_revokes_sessions_and_device_revoke_is_scoped() {
        let service = ChatService::new();
        let now = Utc::now();
        let first = service
            .register(register_input("security_user"), now)
            .await
            .unwrap();
        let second = service
            .login(
                LoginInput {
                    login: "security_user".to_owned(),
                    password: "StrongPass1".to_owned(),
                    device_name: "second device".to_owned(),
                    platform: "linux".to_owned(),
                    app_version: "0.1.0".to_owned(),
                },
                now,
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .devices(first.profile.id, second.device_id)
                .await
                .len(),
            2
        );
        service
            .revoke_device(first.profile.id, first.device_id, now)
            .await
            .unwrap();
        assert_eq!(
            service.authenticate_access(&first.access_token, now).await,
            Err(ApplicationError::SessionExpired)
        );
        assert_eq!(
            service.authenticate_access(&second.access_token, now).await,
            Ok(first.profile.id)
        );

        let delivery = service
            .request_password_reset("security_user@example.com", now)
            .await
            .unwrap()
            .unwrap();
        service
            .reset_password(&delivery.reset_token, "AnotherPass2".to_owned(), now)
            .await
            .unwrap();
        assert_eq!(
            service.authenticate_access(&second.access_token, now).await,
            Err(ApplicationError::SessionExpired)
        );
        assert!(
            service
                .login(
                    LoginInput {
                        login: "security_user".to_owned(),
                        password: "AnotherPass2".to_owned(),
                        device_name: "restored".to_owned(),
                        platform: "macos".to_owned(),
                        app_version: "0.1.0".to_owned(),
                    },
                    now,
                )
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn group_roles_mute_announcements_and_polls_enforce_membership() {
        let service = ChatService::new();
        let now = Utc::now();
        let alice = service
            .register(register_input("group_alice"), now)
            .await
            .unwrap();
        let bob = service
            .register(register_input("group_bob"), now)
            .await
            .unwrap();
        let charlie = service
            .register(register_input("group_charlie"), now)
            .await
            .unwrap();
        for username in ["group_bob", "group_charlie"] {
            let request = service
                .send_friend_request(alice.profile.id, username, String::new(), now)
                .await
                .unwrap();
            let recipient = if username == "group_bob" {
                bob.profile.id
            } else {
                charlie.profile.id
            };
            service
                .decide_friend_request(recipient, request.id, FriendRequestDecision::Accept, now)
                .await
                .unwrap();
        }
        let group = service
            .create_group(
                alice.profile.id,
                vec![bob.profile.id, charlie.profile.id],
                "Core team".to_owned(),
                now,
            )
            .await
            .unwrap();
        let avatar = service
            .authorize_attachment(
                alice.profile.id,
                "group-avatar.webp".to_owned(),
                "image/webp".to_owned(),
                128,
                None,
                now,
            )
            .await
            .unwrap();
        service
            .complete_attachment(alice.profile.id, avatar.attachment.id, now)
            .await
            .unwrap();
        let group = service
            .update_group(
                alice.profile.id,
                group.id,
                UpdateGroupRequest {
                    avatar_attachment_id: Some(Some(avatar.attachment.id)),
                    ..UpdateGroupRequest::default()
                },
                now,
            )
            .await
            .unwrap();
        assert_eq!(group.avatar_attachment_id, Some(avatar.attachment.id));
        assert_eq!(
            service
                .attachment_for_download(charlie.profile.id, avatar.attachment.id)
                .await
                .unwrap(),
            avatar.attachment
        );
        let mut calls = service.subscribe_calls();
        let call_id = Uuid::now_v7();
        service
            .publish_call_signal(
                alice.profile.id,
                group.id,
                call_id,
                None,
                CallSignal::Invite { video: false },
                now,
            )
            .await
            .unwrap();
        let (call_recipients, call) = calls.recv().await.unwrap();
        assert!(call_recipients.contains(&bob.profile.id));
        assert!(call_recipients.contains(&charlie.profile.id));
        assert_eq!(call.call_id, call_id);
        service
            .publish_call_signal(
                bob.profile.id,
                group.id,
                call_id,
                None,
                CallSignal::Accept,
                now,
            )
            .await
            .unwrap();
        let (_, accepted) = calls.recv().await.unwrap();
        assert_eq!(accepted.signal, CallSignal::Accept);
        let (roster_recipients, roster) = calls.recv().await.unwrap();
        assert!(roster_recipients.contains(&alice.profile.id));
        assert!(roster_recipients.contains(&bob.profile.id));
        assert!(!roster_recipients.contains(&charlie.profile.id));
        assert!(matches!(
            roster.signal,
            CallSignal::Participants { ref user_ids }
                if user_ids.contains(&alice.profile.id) && user_ids.contains(&bob.profile.id)
        ));
        assert_eq!(
            service
                .publish_call_signal(
                    charlie.profile.id,
                    group.id,
                    call_id,
                    Some(bob.profile.id),
                    CallSignal::Offer {
                        sdp: "not-yet-a-participant".to_owned(),
                    },
                    now,
                )
                .await,
            Err(ApplicationError::Domain(DomainError::Forbidden))
        );
        service
            .update_group_member(
                alice.profile.id,
                group.id,
                bob.profile.id,
                UpdateGroupMemberRequest {
                    role: Some(MemberRole::Administrator),
                    ..UpdateGroupMemberRequest::default()
                },
                now,
            )
            .await
            .unwrap();
        service
            .set_group_mute(
                bob.profile.id,
                group.id,
                GroupMuteRequest { muted: true },
                now,
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .send_message(
                    charlie.profile.id,
                    group.id,
                    MessageId::new(),
                    MessageContent::Text {
                        text: "blocked by mute".to_owned(),
                    },
                    None,
                    now,
                )
                .await,
            Err(ApplicationError::Domain(DomainError::Forbidden))
        );
        let announcement = service
            .create_group_announcement(
                bob.profile.id,
                group.id,
                CreateGroupAnnouncementRequest {
                    content: "Ship safely".to_owned(),
                },
                now,
            )
            .await
            .unwrap();
        service
            .read_group_announcement(charlie.profile.id, announcement.id, now)
            .await
            .unwrap();
        let poll = service
            .create_group_poll(
                alice.profile.id,
                group.id,
                CreateGroupPollRequest {
                    question: "Release today?".to_owned(),
                    options: vec!["Yes".to_owned(), "No".to_owned()],
                    multiple_choice: false,
                    closes_at: None,
                },
                now,
            )
            .await
            .unwrap();
        let voted = service
            .vote_group_poll(
                charlie.profile.id,
                poll.id,
                VoteGroupPollRequest {
                    option_ids: vec![poll.options[0].id],
                },
                now,
            )
            .await
            .unwrap();
        assert_eq!(voted.options[0].voter_ids, vec![charlie.profile.id]);

        let (message, _) = service
            .send_message_request(
                bob.profile.id,
                group.id,
                SendMessageRequest {
                    client_message_id: MessageId::new(),
                    content: MessageContent::Text {
                        text: "@group_charlie @所有人 admin update".to_owned(),
                    },
                    reply_to: None,
                    mentions: vec![charlie.profile.id, charlie.profile.id],
                    mention_all: true,
                    expires_in_seconds: None,
                },
                now,
            )
            .await
            .unwrap();
        assert_eq!(message.mentions, vec![charlie.profile.id]);
        assert!(message.mention_all);
        service
            .acknowledge_delivery(charlie.profile.id, message.id, now)
            .await
            .unwrap();
        service
            .react_to_message(charlie.profile.id, message.id, "👍".to_owned(), true, now)
            .await
            .unwrap();
        service
            .set_message_favorite(charlie.profile.id, message.id, true)
            .await
            .unwrap();
        let details = service
            .message_details(charlie.profile.id, message.id)
            .await
            .unwrap();
        assert_eq!(details.reactions[0].user_ids, vec![charlie.profile.id]);
        assert!(details.favorited);
        let (second_message, _) = service
            .send_message(
                bob.profile.id,
                group.id,
                MessageId::new(),
                MessageContent::Text {
                    text: "second update".to_owned(),
                },
                None,
                now,
            )
            .await
            .unwrap();
        let file = service
            .authorize_attachment(
                bob.profile.id,
                "release-notes.pdf".to_owned(),
                "application/pdf".to_owned(),
                256,
                None,
                now,
            )
            .await
            .unwrap();
        service
            .complete_attachment(bob.profile.id, file.attachment.id, now)
            .await
            .unwrap();
        service
            .send_message(
                bob.profile.id,
                group.id,
                MessageId::new(),
                MessageContent::File {
                    attachment: file.attachment.clone(),
                },
                None,
                now,
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .group_files(charlie.profile.id, group.id)
                .await
                .unwrap()[0]
                .attachment,
            file.attachment
        );
        let bundled = service
            .forward_messages(
                bob.profile.id,
                vec![message.id, second_message.id],
                group.id,
                ForwardMode::Merged,
                now,
            )
            .await
            .unwrap();
        assert!(matches!(
            &bundled[0].content,
            MessageContent::ForwardBundle { messages, .. } if messages.len() == 2
        ));
        service
            .recall_message(bob.profile.id, message.id, now + Duration::seconds(30))
            .await
            .unwrap();

        let scheduled = service
            .schedule_message(
                alice.profile.id,
                ScheduleMessageRequest {
                    conversation_id: group.id,
                    client_message_id: MessageId::new(),
                    content: MessageContent::Text {
                        text: "later".to_owned(),
                    },
                    reply_to: None,
                    mentions: Vec::new(),
                    mention_all: false,
                    scheduled_for: now + Duration::seconds(10),
                    expires_in_seconds: None,
                },
                now,
            )
            .await
            .unwrap();
        assert_eq!(service.scheduled_messages(alice.profile.id).await.len(), 1);
        assert_eq!(
            service
                .deliver_due_messages(scheduled.scheduled_for + Duration::seconds(1))
                .await,
            1
        );
        assert!(
            service
                .scheduled_messages(alice.profile.id)
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn friend_flow_and_idempotent_message_send_form_a_closed_loop() {
        let service = ChatService::new();
        let now = Utc::now();
        let alice = service
            .register(register_input("alice"), now)
            .await
            .unwrap();
        let bob = service.register(register_input("bob"), now).await.unwrap();
        let request = service
            .send_friend_request(alice.profile.id, "bob", "hi".to_owned(), now)
            .await
            .unwrap();
        service
            .decide_friend_request(
                bob.profile.id,
                request.id,
                FriendRequestDecision::Accept,
                now,
            )
            .await
            .unwrap();
        let conversation = service
            .create_direct(alice.profile.id, bob.profile.id, now)
            .await
            .unwrap();
        let alice_conversation = service.conversations(alice.profile.id).await.remove(0);
        let bob_conversation = service.conversations(bob.profile.id).await.remove(0);
        assert_eq!(
            alice_conversation.kind,
            ConversationKind::Direct {
                peer_user_id: bob.profile.id
            }
        );
        assert_eq!(
            bob_conversation.kind,
            ConversationKind::Direct {
                peer_user_id: alice.profile.id
            }
        );
        let client_message_id = MessageId::new();
        let first = service
            .send_message(
                alice.profile.id,
                conversation.id,
                client_message_id,
                MessageContent::Text {
                    text: "hello".to_owned(),
                },
                None,
                now,
            )
            .await
            .unwrap();
        let duplicate = service
            .send_message(
                alice.profile.id,
                conversation.id,
                client_message_id,
                MessageContent::Text {
                    text: "hello".to_owned(),
                },
                None,
                now,
            )
            .await
            .unwrap();
        assert_eq!(first.0.id, duplicate.0.id);
        assert_eq!(
            service
                .messages(bob.profile.id, conversation.id, None, 50)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn custom_sticker_can_be_saved_sent_and_removed() {
        let service = ChatService::new();
        let now = Utc::now();
        let alice = service
            .register(register_input("sticker_alice"), now)
            .await
            .unwrap();
        let bob = service
            .register(register_input("sticker_bob"), now)
            .await
            .unwrap();
        let request = service
            .send_friend_request(alice.profile.id, "sticker_bob", String::new(), now)
            .await
            .unwrap();
        service
            .decide_friend_request(
                bob.profile.id,
                request.id,
                FriendRequestDecision::Accept,
                now,
            )
            .await
            .unwrap();
        let conversation = service
            .create_direct(alice.profile.id, bob.profile.id, now)
            .await
            .unwrap();
        let upload = service
            .authorize_attachment(
                alice.profile.id,
                "crab.webp".to_owned(),
                "image/webp".to_owned(),
                512,
                None,
                now,
            )
            .await
            .unwrap();
        service
            .complete_attachment(alice.profile.id, upload.attachment.id, now)
            .await
            .unwrap();
        let sticker = service
            .create_sticker(
                alice.profile.id,
                CreateStickerRequest {
                    attachment_id: upload.attachment.id,
                    name: "Ferris".to_owned(),
                    shortcut: Some(":ferris:".to_owned()),
                },
                now,
            )
            .await
            .unwrap();
        assert_eq!(
            service.stickers(alice.profile.id).await,
            vec![sticker.clone()]
        );
        let (message, _) = service
            .send_message(
                alice.profile.id,
                conversation.id,
                MessageId::new(),
                MessageContent::Sticker {
                    attachment: sticker.attachment.clone(),
                    name: sticker.name.clone(),
                },
                None,
                now,
            )
            .await
            .unwrap();
        assert!(matches!(message.content, MessageContent::Sticker { .. }));
        assert_eq!(
            service
                .attachment_for_download(bob.profile.id, sticker.attachment.id)
                .await
                .unwrap(),
            sticker.attachment
        );
        service
            .delete_sticker(alice.profile.id, sticker.id)
            .await
            .unwrap();
        assert!(service.stickers(alice.profile.id).await.is_empty());
    }

    #[tokio::test]
    async fn refresh_tokens_rotate_and_reuse_is_rejected() {
        let service = ChatService::new();
        let now = Utc::now();
        let session = service
            .register(register_input("alice"), now)
            .await
            .unwrap();
        let bob = service.register(register_input("bob"), now).await.unwrap();
        service.refresh(&session.refresh_token, now).await.unwrap();
        assert_eq!(
            service.refresh(&session.refresh_token, now).await,
            Err(ApplicationError::RefreshTokenReuse)
        );
        assert_eq!(
            service.authenticate_access(&bob.access_token, now).await,
            Ok(bob.profile.id)
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn privacy_export_and_account_deletion_form_a_closed_loop() {
        let service = ChatService::new();
        let now = Utc::now();
        let alice = service
            .register(register_input("privacy_alice"), now)
            .await
            .unwrap();
        let bob = service
            .register(register_input("privacy_bob"), now)
            .await
            .unwrap();
        service
            .update_profile(
                bob.profile.id,
                UpdateProfileInput {
                    nickname: "Bob".to_owned(),
                    signature: "private profile".to_owned(),
                    avatar_url: None,
                    avatar_attachment_id: None,
                    gender: Some("unspecified".to_owned()),
                    birthday: NaiveDate::from_ymd_opt(2000, 1, 2),
                    region: Some("Shanghai".to_owned()),
                    presence: Some(Presence::Busy),
                },
                now,
            )
            .await
            .unwrap();
        service
            .update_profile_privacy(
                bob.profile.id,
                ProfilePrivacySettings {
                    gender_visibility: ProfileVisibility::Nobody,
                    birthday_visibility: ProfileVisibility::Friends,
                    region_visibility: ProfileVisibility::Everyone,
                    presence_visibility: ProfileVisibility::Friends,
                    read_receipts_enabled: false,
                },
                now,
            )
            .await
            .unwrap();

        let public_bob = service
            .search_user_exact(alice.profile.id, "privacy_bob")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(public_bob.gender, None);
        assert_eq!(public_bob.birthday, None);
        assert_eq!(public_bob.region.as_deref(), Some("Shanghai"));
        assert_eq!(public_bob.presence, Presence::Offline);

        let request = service
            .send_friend_request(alice.profile.id, "privacy_bob", String::new(), now)
            .await
            .unwrap();
        service
            .decide_friend_request(
                bob.profile.id,
                request.id,
                FriendRequestDecision::Accept,
                now,
            )
            .await
            .unwrap();
        let conversation = service
            .create_direct(alice.profile.id, bob.profile.id, now)
            .await
            .unwrap();
        let (message, _) = service
            .send_message(
                alice.profile.id,
                conversation.id,
                MessageId::new(),
                MessageContent::Text {
                    text: "privacy check".to_owned(),
                },
                None,
                now,
            )
            .await
            .unwrap();
        service
            .mark_read(bob.profile.id, conversation.id, 1, now)
            .await
            .unwrap();
        assert!(
            service
                .message_details(alice.profile.id, message.id)
                .await
                .unwrap()
                .read_by
                .is_empty()
        );

        let export = service
            .export_personal_data(alice.profile.id, now)
            .await
            .unwrap();
        assert_eq!(export.email, "privacy_alice@example.com");
        assert_eq!(export.messages.len(), 1);
        assert_eq!(
            service
                .delete_account(
                    alice.profile.id,
                    "StrongPass1".to_owned(),
                    "wrong".to_owned(),
                    now,
                )
                .await,
            Err(ApplicationError::Domain(DomainError::Validation {
                field: "confirmation",
                reason: "confirmation_required",
            }))
        );
        service
            .delete_account(
                alice.profile.id,
                "StrongPass1".to_owned(),
                "DELETE".to_owned(),
                now,
            )
            .await
            .unwrap();
        assert_eq!(
            service.authenticate_access(&alice.access_token, now).await,
            Err(ApplicationError::SessionExpired)
        );
        assert!(service.friends(bob.profile.id).await.is_empty());
        assert_eq!(
            service
                .login(
                    LoginInput {
                        login: "privacy_alice".to_owned(),
                        password: "StrongPass1".to_owned(),
                        device_name: "test".to_owned(),
                        platform: "test".to_owned(),
                        app_version: "0.1.0".to_owned(),
                    },
                    now,
                )
                .await,
            Err(ApplicationError::InvalidCredentials)
        );
    }

    #[tokio::test]
    async fn totp_and_one_use_recovery_codes_protect_password_login() {
        let service = ChatService::new()
            .with_data_encryption_key("test-encryption-key-with-at-least-32-bytes");
        let now = Utc::now();
        let session = service
            .register(register_input("second_factor_user"), now)
            .await
            .unwrap();
        let setup = service
            .begin_second_factor_setup(session.profile.id, now)
            .await
            .unwrap();
        let secret = BASE32_NOPAD.decode(setup.secret.as_bytes()).unwrap();
        let code = totp_code(&secret, now.timestamp().cast_unsigned() / 30);
        let recovery_codes = service
            .enable_second_factor(session.profile.id, &code, now)
            .await
            .unwrap();
        assert_eq!(recovery_codes.len(), 10);
        assert_eq!(
            service.login(login_input("second_factor_user"), now).await,
            Err(ApplicationError::SecondFactorRequired)
        );
        assert_eq!(
            service
                .login_with_second_factor(
                    login_input("second_factor_user"),
                    Some("000000".to_owned()),
                    now,
                )
                .await,
            Err(ApplicationError::InvalidSecondFactor)
        );
        assert!(
            service
                .login_with_second_factor(login_input("second_factor_user"), Some(code), now,)
                .await
                .is_ok()
        );
        assert!(
            service
                .login_with_second_factor(
                    login_input("second_factor_user"),
                    Some(recovery_codes[0].clone()),
                    now,
                )
                .await
                .is_ok()
        );
        assert_eq!(
            service
                .login_with_second_factor(
                    login_input("second_factor_user"),
                    Some(recovery_codes[0].clone()),
                    now,
                )
                .await,
            Err(ApplicationError::InvalidSecondFactor)
        );
    }

    #[tokio::test]
    async fn qr_login_requires_authenticated_approval_and_is_one_use() {
        let service = ChatService::new();
        let now = Utc::now();
        let approver = service
            .register(register_input("qr_user"), now)
            .await
            .unwrap();
        let challenge = service
            .begin_qr_login(
                "new desktop".to_owned(),
                "linux".to_owned(),
                "0.1.0".to_owned(),
                now,
            )
            .await
            .unwrap();
        assert!(
            service
                .poll_qr_login(challenge.challenge_id, &challenge.secret, now)
                .await
                .unwrap()
                .is_none()
        );
        service
            .approve_qr_login(
                approver.profile.id,
                challenge.challenge_id,
                &challenge.secret,
                now,
            )
            .await
            .unwrap();
        let session = service
            .poll_qr_login(challenge.challenge_id, &challenge.secret, now)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.profile.id, approver.profile.id);
        assert_eq!(
            service
                .poll_qr_login(challenge.challenge_id, &challenge.secret, now)
                .await,
            Err(ApplicationError::NotFound)
        );
    }
}
