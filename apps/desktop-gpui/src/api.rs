use std::{
    fmt,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use iamrust_domain::{
    Attachment, Conversation, ConversationId, DeviceId, FriendRequest, FriendRequestId, Message,
    UserId, UserProfile,
};
use iamrust_protocol::{
    AddGroupMembersRequest, BootstrapResponse, ChangePasswordRequest, ClientFrame,
    CompleteUploadRequest, CompleteUploadResponse, CreateDirectConversationRequest,
    CreateGroupAnnouncementRequest, CreateGroupPollRequest, CreateGroupRequest,
    DecideGroupJoinRequest, DeviceInfo, DisableSecondFactorRequest, DownloadAuthorizationResponse,
    FavoriteMessageRequest, ForwardMessagesRequest, ForwardMode, FriendRequestCreate,
    FriendRequestDecision, FriendRequestDecisionBody, GroupAnnouncement, GroupFileItem,
    GroupJoinRequest, GroupMuteRequest, GroupPoll, GroupSettingsResponse, LoginRequest,
    MarkReadRequest, MessageAck, MessageDetails, MessageReaction, MessageReactionRequest, Page,
    PasswordResetConfirmRequest, PasswordResetRequest, QrLoginPollResponse, QrLoginSecretRequest,
    QrLoginStartRequest, QrLoginStartResponse, QrLoginStatus, RecoveryCodesResponse,
    RefreshRequest, RegenerateRecoveryCodesRequest, RegisterRequest, ReportUserRequest,
    SecondFactorCodeRequest, SecondFactorSetupResponse, SecondFactorStatus, SendMessageRequest,
    SessionResponse, SyncResponse, TransferGroupOwnershipRequest,
    UpdateConversationSettingsRequest, UpdateGroupMemberRequest, UpdateGroupRequest,
    UpdateProfileRequest, UploadAuthorizationRequest, UploadAuthorizationResponse,
    VoteGroupPollRequest, WS_PROTOCOL_VERSION, WebSocketTicketResponse,
};
use reqwest::{StatusCode, blocking::Client, header};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use url::{Host, Url};

const CREDENTIAL_SERVICE: &str = "app.iamrust.desktop";
const CREDENTIAL_ACCOUNT: &str = "refresh-token";
const MAX_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_UPLOAD_BYTES: u64 = 100 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Default)]
struct SessionTokens {
    access: Option<String>,
    refresh: Option<String>,
}

trait CredentialStore: fmt::Debug + Send + Sync {
    fn save(&self, token: &str) -> Result<(), ClientError>;
    fn load(&self) -> Result<Option<String>, ClientError>;
    fn clear(&self) -> Result<(), ClientError>;
}

#[derive(Debug)]
struct SystemCredentialStore;

pub struct ApiClient {
    base_url: Url,
    client: Client,
    session: Mutex<SessionTokens>,
    refresh_gate: Mutex<()>,
    credentials: Arc<dyn CredentialStore>,
}

#[derive(Debug)]
pub(crate) struct WebSocketSession {
    pub url: Url,
    pub hello: ClientFrame,
}

impl fmt::Debug for ApiClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiClient")
            .field("base_url", &self.base_url)
            .field("credentials", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("客户端 API 地址配置无效")]
    InvalidConfiguration,
    #[error("无法连接服务器")]
    Offline,
    #[error("系统凭据库不可用")]
    CredentialStore,
    #[error("服务器响应无效")]
    InvalidResponse,
    #[error("{0}")]
    LocalFile(&'static str),
    #[error("请求失败：{message_key}")]
    Request {
        status: u16,
        code: String,
        message_key: String,
        retryable: bool,
    },
}

impl ClientError {
    pub fn user_message(&self) -> String {
        match self {
            Self::InvalidConfiguration => "服务器地址配置无效".to_owned(),
            Self::Offline => "无法连接服务器，请检查网络或服务是否启动".to_owned(),
            Self::CredentialStore => "系统凭据库不可用，无法安全保存登录状态".to_owned(),
            Self::InvalidResponse => "服务器返回了无法识别的数据".to_owned(),
            Self::LocalFile(message) => (*message).to_owned(),
            Self::Request {
                status,
                code,
                message_key,
                retryable,
            } => {
                let suffix = if *retryable {
                    "，可以稍后重试"
                } else {
                    ""
                };
                format!("请求失败（{status}/{code}/{message_key}）{suffix}")
            }
        }
    }
}

impl ApiClient {
    pub fn from_environment() -> Result<Self, ClientError> {
        let configured =
            std::env::var("IAMRUST_API_URL").unwrap_or_else(|_| "http://127.0.0.1:3780".to_owned());
        Self::new(&configured)
    }

    pub fn new(configured: &str) -> Result<Self, ClientError> {
        Self::with_credentials(configured, Arc::new(SystemCredentialStore))
    }

    fn with_credentials(
        configured: &str,
        credentials: Arc<dyn CredentialStore>,
    ) -> Result<Self, ClientError> {
        let mut base_url = Url::parse(configured).map_err(|_| ClientError::InvalidConfiguration)?;
        let loopback = has_loopback_host(&base_url);
        if !matches!(base_url.scheme(), "http" | "https")
            || (base_url.scheme() != "https" && !loopback)
        {
            return Err(ClientError::InvalidConfiguration);
        }
        base_url.set_query(None);
        base_url.set_fragment(None);
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("I-Am-Rust-GPUI/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ClientError::InvalidConfiguration)?;
        Ok(Self {
            base_url,
            client,
            session: Mutex::new(SessionTokens::default()),
            refresh_gate: Mutex::new(()),
            credentials,
        })
    }

    pub fn login(
        &self,
        login: &str,
        password: &str,
        second_factor_code: Option<&str>,
    ) -> Result<SessionResponse, ClientError> {
        let request = LoginRequest {
            login: login.trim().to_owned(),
            password: password.to_owned(),
            second_factor_code: second_factor_code
                .map(str::trim)
                .filter(|code| !code.is_empty())
                .map(str::to_owned),
            device_name: device_name(),
            platform: Some(std::env::consts::OS.to_owned()),
            app_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        };
        self.authenticate("/api/v1/auth/login", &request)
    }

    pub fn register(
        &self,
        email: &str,
        username: &str,
        password: &str,
        nickname: &str,
    ) -> Result<SessionResponse, ClientError> {
        let request = RegisterRequest {
            email: email.trim().to_owned(),
            username: username.trim().to_owned(),
            password: password.to_owned(),
            nickname: nickname.trim().to_owned(),
            device_name: device_name(),
            platform: Some(std::env::consts::OS.to_owned()),
            app_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        };
        self.authenticate("/api/v1/auth/register", &request)
    }

    pub fn request_password_reset(&self, email: &str) -> Result<(), ClientError> {
        self.send_json::<Value, _>(
            reqwest::Method::POST,
            "/api/v1/auth/password-reset/request",
            Some(&PasswordResetRequest {
                email: email.trim().to_owned(),
            }),
            None,
        )?;
        Ok(())
    }

    pub fn confirm_password_reset(
        &self,
        reset_token: &str,
        new_password: &str,
    ) -> Result<(), ClientError> {
        self.send_json::<Value, _>(
            reqwest::Method::POST,
            "/api/v1/auth/password-reset/confirm",
            Some(&PasswordResetConfirmRequest {
                reset_token: reset_token.trim().to_owned(),
                new_password: new_password.to_owned(),
            }),
            None,
        )?;
        Ok(())
    }

    pub fn change_password(
        &self,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), ClientError> {
        let body = serde_json::to_value(ChangePasswordRequest {
            current_password: current_password.to_owned(),
            new_password: new_password.to_owned(),
        })
        .map_err(|_| ClientError::InvalidResponse)?;
        self.authorized_json::<Value>(
            reqwest::Method::POST,
            "/api/v1/auth/change-password",
            Some(&body),
        )?;
        Ok(())
    }

    pub fn devices(&self) -> Result<Vec<DeviceInfo>, ClientError> {
        self.authorized_json(reqwest::Method::GET, "/api/v1/devices", None)
    }

    pub fn revoke_device(&self, device_id: DeviceId) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::DELETE,
            &format!("/api/v1/devices/{device_id}"),
            None,
        )?;
        Ok(())
    }

    pub fn second_factor_status(&self) -> Result<SecondFactorStatus, ClientError> {
        self.authorized_json(reqwest::Method::GET, "/api/v1/me/second-factor", None)
    }

    pub fn begin_second_factor_setup(&self) -> Result<SecondFactorSetupResponse, ClientError> {
        self.authorized_json(reqwest::Method::POST, "/api/v1/me/second-factor", None)
    }

    pub fn enable_second_factor(&self, code: &str) -> Result<RecoveryCodesResponse, ClientError> {
        let body = serde_json::to_value(SecondFactorCodeRequest {
            code: code.trim().to_owned(),
        })
        .map_err(|_| ClientError::InvalidResponse)?;
        self.authorized_json(
            reqwest::Method::POST,
            "/api/v1/me/second-factor/enable",
            Some(&body),
        )
    }

    pub fn disable_second_factor(
        &self,
        current_password: &str,
        code: &str,
    ) -> Result<(), ClientError> {
        let body = serde_json::to_value(DisableSecondFactorRequest {
            current_password: current_password.to_owned(),
            code: code.trim().to_owned(),
        })
        .map_err(|_| ClientError::InvalidResponse)?;
        self.authorized_json::<Value>(
            reqwest::Method::DELETE,
            "/api/v1/me/second-factor",
            Some(&body),
        )?;
        Ok(())
    }

    pub fn regenerate_recovery_codes(
        &self,
        current_password: &str,
        code: &str,
    ) -> Result<RecoveryCodesResponse, ClientError> {
        let body = serde_json::to_value(RegenerateRecoveryCodesRequest {
            current_password: current_password.to_owned(),
            code: code.trim().to_owned(),
        })
        .map_err(|_| ClientError::InvalidResponse)?;
        self.authorized_json(
            reqwest::Method::POST,
            "/api/v1/me/second-factor/recovery-codes",
            Some(&body),
        )
    }

    pub fn approve_qr_payload(&self, payload: &str) -> Result<(), ClientError> {
        let (challenge_id, secret) = parse_qr_payload(payload)?;
        let body = serde_json::to_value(QrLoginSecretRequest { secret })
            .map_err(|_| ClientError::InvalidResponse)?;
        self.authorized_json::<Value>(
            reqwest::Method::POST,
            &format!("/api/v1/auth/qr-login/{challenge_id}/approve"),
            Some(&body),
        )?;
        Ok(())
    }

    pub fn upload_file(
        &self,
        path: &Path,
        image_only: bool,
    ) -> Result<CompleteUploadResponse, ClientError> {
        let metadata =
            std::fs::metadata(path).map_err(|_| ClientError::LocalFile("无法读取所选文件"))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_UPLOAD_BYTES {
            return Err(ClientError::LocalFile("文件必须介于 1 B 与 100 MiB 之间"));
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.chars().count() <= 255)
            .ok_or(ClientError::LocalFile("文件名无效"))?
            .to_owned();
        let bytes = std::fs::read(path).map_err(|_| ClientError::LocalFile("无法读取所选文件"))?;
        let detected_image = image_mime_type(&bytes);
        if image_only && detected_image.is_none() {
            return Err(ClientError::LocalFile("请选择 PNG、JPEG、GIF 或 WebP 图片"));
        }
        if image_only && bytes.len() > 25 * 1024 * 1024 {
            return Err(ClientError::LocalFile("图片不能超过 25 MiB"));
        }
        let mime_type = detected_image
            .unwrap_or_else(|| mime_type_from_path(path))
            .to_owned();
        let authorization: UploadAuthorizationResponse = self.authorized_json(
            reqwest::Method::POST,
            "/api/v1/uploads/authorize",
            Some(&json!(UploadAuthorizationRequest {
                file_name,
                mime_type,
                byte_size: metadata.len(),
                sha256: Some(hex_digest(&Sha256::digest(&bytes))),
            })),
        )?;
        let upload_url =
            Url::parse(&authorization.upload_url).map_err(|_| ClientError::InvalidResponse)?;
        if !is_safe_transfer_url(&upload_url) {
            return Err(ClientError::InvalidResponse);
        }
        let mut upload = self
            .client
            .put(upload_url)
            .timeout(Duration::from_secs(600))
            .body(bytes);
        for (name, value) in authorization.required_headers {
            let name =
                header::HeaderName::try_from(name).map_err(|_| ClientError::InvalidResponse)?;
            let value =
                header::HeaderValue::try_from(value).map_err(|_| ClientError::InvalidResponse)?;
            upload = upload.header(name, value);
        }
        let response = upload.send().map_err(|_| ClientError::Offline)?;
        if !response.status().is_success() {
            return Err(ClientError::Request {
                status: response.status().as_u16(),
                code: "upload_failed".to_owned(),
                message_key: "upload_failed".to_owned(),
                retryable: response.status().is_server_error(),
            });
        }
        let completed: CompleteUploadResponse = self.authorized_json(
            reqwest::Method::POST,
            "/api/v1/uploads/complete",
            Some(&json!(CompleteUploadRequest {
                attachment_id: authorization.attachment_id,
            })),
        )?;
        if completed.attachment.id != authorization.attachment_id
            || completed.attachment.byte_size != metadata.len()
        {
            return Err(ClientError::InvalidResponse);
        }
        Ok(completed)
    }

    pub fn download_attachment(
        &self,
        attachment: &Attachment,
        destination: &Path,
    ) -> Result<PathBuf, ClientError> {
        if attachment.byte_size == 0 || attachment.byte_size > MAX_DOWNLOAD_BYTES {
            return Err(ClientError::InvalidResponse);
        }
        let file_name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .ok_or(ClientError::LocalFile("保存文件名无效"))?;
        let parent = destination
            .parent()
            .ok_or(ClientError::LocalFile("保存目录无效"))?;
        let parent =
            std::fs::canonicalize(parent).map_err(|_| ClientError::LocalFile("保存目录不可用"))?;
        let destination = parent.join(file_name);
        if destination.exists() {
            return Err(ClientError::LocalFile("目标文件已存在，请选择其他名称"));
        }
        let authorization: DownloadAuthorizationResponse = self.authorized_json(
            reqwest::Method::GET,
            &format!("/api/v1/attachments/{}/download", attachment.id),
            None,
        )?;
        if authorization.attachment.id != attachment.id
            || authorization.attachment.byte_size != attachment.byte_size
        {
            return Err(ClientError::InvalidResponse);
        }
        let download_url =
            Url::parse(&authorization.download_url).map_err(|_| ClientError::InvalidResponse)?;
        if !is_safe_transfer_url(&download_url) {
            return Err(ClientError::InvalidResponse);
        }
        let mut response = self
            .client
            .get(download_url)
            .timeout(Duration::from_secs(600))
            .send()
            .map_err(|_| ClientError::Offline)?;
        if !response.status().is_success() {
            return Err(ClientError::Request {
                status: response.status().as_u16(),
                code: "download_failed".to_owned(),
                message_key: "download_failed".to_owned(),
                retryable: response.status().is_server_error(),
            });
        }
        if response
            .content_length()
            .is_some_and(|size| size != attachment.byte_size)
        {
            return Err(ClientError::InvalidResponse);
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&parent)
            .map_err(|_| ClientError::LocalFile("无法创建临时下载文件"))?;
        let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
        let mut received = 0_u64;
        let mut hasher = Sha256::new();
        loop {
            let read = response
                .read(&mut buffer)
                .map_err(|_| ClientError::Offline)?;
            if read == 0 {
                break;
            }
            received = received.saturating_add(read as u64);
            if received > attachment.byte_size || received > MAX_DOWNLOAD_BYTES {
                return Err(ClientError::InvalidResponse);
            }
            temporary
                .write_all(&buffer[..read])
                .map_err(|_| ClientError::LocalFile("写入下载文件失败"))?;
            hasher.update(&buffer[..read]);
        }
        let digest = hex_digest(&hasher.finalize());
        if received != attachment.byte_size
            || authorization
                .attachment
                .sha256
                .as_deref()
                .is_some_and(|expected| !digest.eq_ignore_ascii_case(expected))
        {
            return Err(ClientError::InvalidResponse);
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| ClientError::LocalFile("写入下载文件失败"))?;
        temporary
            .persist_noclobber(&destination)
            .map_err(|_| ClientError::LocalFile("无法保存文件，目标可能已存在"))?;
        Ok(destination)
    }

    pub fn begin_qr_login(&self) -> Result<QrLoginStartResponse, ClientError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/v1/auth/qr-login",
            Some(&QrLoginStartRequest {
                device_name: device_name(),
                platform: Some(std::env::consts::OS.to_owned()),
                app_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
            None,
        )
    }

    pub fn poll_qr_login(
        &self,
        challenge: &QrLoginStartResponse,
    ) -> Result<Option<SessionResponse>, ClientError> {
        let response: QrLoginPollResponse = self.send_json(
            reqwest::Method::POST,
            &format!("/api/v1/auth/qr-login/{}/poll", challenge.challenge_id),
            Some(&QrLoginSecretRequest {
                secret: challenge.secret.clone(),
            }),
            None,
        )?;
        if response.status == QrLoginStatus::Pending {
            return Ok(None);
        }
        let session = response.session.ok_or(ClientError::InvalidResponse)?;
        self.accept_session(&session)?;
        Ok(Some(session))
    }

    pub fn restore(&self) -> Result<Option<SessionResponse>, ClientError> {
        let Some(refresh) = self.credentials.load()? else {
            return Ok(None);
        };
        self.lock_session().refresh = Some(refresh);
        let _refresh_guard = self.lock_refresh_gate();
        match self.refresh_session() {
            Ok(session) => Ok(Some(session)),
            Err(ClientError::Request { status: 401, .. }) => {
                self.clear_session();
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub fn bootstrap(&self) -> Result<BootstrapResponse, ClientError> {
        self.authorized_json(reqwest::Method::GET, "/api/v1/bootstrap", None)
    }

    pub fn search_user(&self, username: &str) -> Result<Vec<UserProfile>, ClientError> {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("username", username.trim())
            .finish();
        self.authorized_json(
            reqwest::Method::GET,
            &format!("/api/v1/users/search?{query}"),
            None,
        )
    }

    pub fn send_friend_request(
        &self,
        username: String,
        message: String,
    ) -> Result<FriendRequest, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            "/api/v1/friend-requests",
            Some(&json!(FriendRequestCreate { username, message })),
        )
    }

    pub fn decide_friend_request(
        &self,
        request_id: FriendRequestId,
        decision: FriendRequestDecision,
    ) -> Result<FriendRequest, ClientError> {
        self.authorized_json(
            reqwest::Method::PATCH,
            &format!("/api/v1/friend-requests/{request_id}"),
            Some(&json!(FriendRequestDecisionBody { decision })),
        )
    }

    pub fn create_direct(&self, peer_user_id: UserId) -> Result<Conversation, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            "/api/v1/conversations/direct",
            Some(&json!(CreateDirectConversationRequest { peer_user_id })),
        )
    }

    pub fn create_group(
        &self,
        name: String,
        member_ids: Vec<UserId>,
    ) -> Result<Conversation, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            "/api/v1/conversations/group",
            Some(&json!(CreateGroupRequest { name, member_ids })),
        )
    }

    pub fn update_conversation_settings(
        &self,
        conversation_id: ConversationId,
        request: UpdateConversationSettingsRequest,
    ) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::PATCH,
            &format!("/api/v1/conversations/{conversation_id}/settings"),
            Some(&json!(request)),
        )?;
        Ok(())
    }

    pub fn mark_all_read(&self) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::POST,
            "/api/v1/conversations/read-all",
            None,
        )?;
        Ok(())
    }

    pub fn group_settings(
        &self,
        conversation_id: ConversationId,
    ) -> Result<GroupSettingsResponse, ClientError> {
        self.authorized_json(
            reqwest::Method::GET,
            &format!("/api/v1/groups/{conversation_id}"),
            None,
        )
    }

    pub fn update_group(
        &self,
        conversation_id: ConversationId,
        name: Option<String>,
    ) -> Result<Conversation, ClientError> {
        self.authorized_json(
            reqwest::Method::PATCH,
            &format!("/api/v1/groups/{conversation_id}"),
            Some(&json!(UpdateGroupRequest {
                name,
                avatar_url: None,
                avatar_attachment_id: None,
            })),
        )
    }

    pub fn add_group_members(
        &self,
        conversation_id: ConversationId,
        member_ids: Vec<UserId>,
    ) -> Result<Conversation, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            &format!("/api/v1/groups/{conversation_id}/members"),
            Some(&json!(AddGroupMembersRequest { member_ids })),
        )
    }

    pub fn update_group_member_role(
        &self,
        conversation_id: ConversationId,
        member_id: UserId,
        role: iamrust_domain::MemberRole,
    ) -> Result<Conversation, ClientError> {
        self.authorized_json(
            reqwest::Method::PATCH,
            &format!("/api/v1/groups/{conversation_id}/members/{member_id}"),
            Some(&json!(UpdateGroupMemberRequest {
                nickname: None,
                role: Some(role),
                muted_until: None,
            })),
        )
    }

    pub fn update_group_member_mute(
        &self,
        conversation_id: ConversationId,
        member_id: UserId,
        muted_until: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Conversation, ClientError> {
        self.authorized_json(
            reqwest::Method::PATCH,
            &format!("/api/v1/groups/{conversation_id}/members/{member_id}"),
            Some(&json!(UpdateGroupMemberRequest {
                nickname: None,
                role: None,
                muted_until: Some(muted_until),
            })),
        )
    }

    pub fn remove_group_member(
        &self,
        conversation_id: ConversationId,
        member_id: UserId,
    ) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::DELETE,
            &format!("/api/v1/groups/{conversation_id}/members/{member_id}"),
            None,
        )?;
        Ok(())
    }

    pub fn leave_group(&self, conversation_id: ConversationId) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::POST,
            &format!("/api/v1/groups/{conversation_id}/leave"),
            None,
        )?;
        Ok(())
    }

    pub fn disband_group(&self, conversation_id: ConversationId) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::DELETE,
            &format!("/api/v1/groups/{conversation_id}"),
            None,
        )?;
        Ok(())
    }

    pub fn transfer_group(
        &self,
        conversation_id: ConversationId,
        user_id: UserId,
    ) -> Result<Conversation, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            &format!("/api/v1/groups/{conversation_id}/transfer"),
            Some(&json!(TransferGroupOwnershipRequest { user_id })),
        )
    }

    pub fn set_group_mute(
        &self,
        conversation_id: ConversationId,
        muted: bool,
    ) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::POST,
            &format!("/api/v1/groups/{conversation_id}/mute"),
            Some(&json!(GroupMuteRequest { muted })),
        )?;
        Ok(())
    }

    pub fn group_announcements(
        &self,
        conversation_id: ConversationId,
    ) -> Result<Vec<GroupAnnouncement>, ClientError> {
        self.authorized_json(
            reqwest::Method::GET,
            &format!("/api/v1/groups/{conversation_id}/announcements"),
            None,
        )
    }

    pub fn create_group_announcement(
        &self,
        conversation_id: ConversationId,
        content: String,
    ) -> Result<GroupAnnouncement, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            &format!("/api/v1/groups/{conversation_id}/announcements"),
            Some(&json!(CreateGroupAnnouncementRequest { content })),
        )
    }

    pub fn read_group_announcement(&self, announcement_id: uuid::Uuid) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::POST,
            &format!("/api/v1/group-announcements/{announcement_id}/read"),
            None,
        )?;
        Ok(())
    }

    pub fn group_files(
        &self,
        conversation_id: ConversationId,
    ) -> Result<Vec<GroupFileItem>, ClientError> {
        self.authorized_json(
            reqwest::Method::GET,
            &format!("/api/v1/groups/{conversation_id}/files"),
            None,
        )
    }

    pub fn group_join_requests(
        &self,
        conversation_id: ConversationId,
    ) -> Result<Vec<GroupJoinRequest>, ClientError> {
        self.authorized_json(
            reqwest::Method::GET,
            &format!("/api/v1/groups/{conversation_id}/join-requests"),
            None,
        )
    }

    pub fn decide_group_join_request(
        &self,
        request_id: uuid::Uuid,
        accept: bool,
    ) -> Result<GroupJoinRequest, ClientError> {
        self.authorized_json(
            reqwest::Method::PATCH,
            &format!("/api/v1/group-join-requests/{request_id}"),
            Some(&json!(DecideGroupJoinRequest { accept })),
        )
    }

    pub fn group_polls(
        &self,
        conversation_id: ConversationId,
    ) -> Result<Vec<GroupPoll>, ClientError> {
        self.authorized_json(
            reqwest::Method::GET,
            &format!("/api/v1/groups/{conversation_id}/polls"),
            None,
        )
    }

    pub fn create_group_poll(
        &self,
        conversation_id: ConversationId,
        request: CreateGroupPollRequest,
    ) -> Result<GroupPoll, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            &format!("/api/v1/groups/{conversation_id}/polls"),
            Some(&json!(request)),
        )
    }

    pub fn vote_group_poll(
        &self,
        poll_id: uuid::Uuid,
        option_ids: Vec<uuid::Uuid>,
    ) -> Result<GroupPoll, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            &format!("/api/v1/polls/{poll_id}/vote"),
            Some(&json!(VoteGroupPollRequest { option_ids })),
        )
    }

    pub fn delete_friend(&self, friend_id: UserId) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::DELETE,
            &format!("/api/v1/friends/{friend_id}"),
            None,
        )?;
        Ok(())
    }

    pub fn block_user(&self, user_id: UserId, blocked: bool) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            if blocked {
                reqwest::Method::POST
            } else {
                reqwest::Method::DELETE
            },
            &format!("/api/v1/blocks/{user_id}"),
            None,
        )?;
        Ok(())
    }

    pub fn report_user(
        &self,
        user_id: UserId,
        reason: String,
        details: Option<String>,
    ) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::POST,
            &format!("/api/v1/reports/{user_id}"),
            Some(&json!(ReportUserRequest { reason, details })),
        )?;
        Ok(())
    }

    pub fn update_profile(
        &self,
        request: UpdateProfileRequest,
    ) -> Result<UserProfile, ClientError> {
        self.authorized_json(reqwest::Method::PATCH, "/api/v1/me", Some(&json!(request)))
    }

    pub fn messages(
        &self,
        conversation_id: ConversationId,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Page<Message>, ClientError> {
        let limit = limit.clamp(1, 100);
        let before = before.map_or_else(String::new, |value| format!("&before={value}"));
        self.authorized_json(
            reqwest::Method::GET,
            &format!("/api/v1/conversations/{conversation_id}/messages?limit={limit}{before}"),
            None,
        )
    }

    pub fn send_message(
        &self,
        conversation_id: ConversationId,
        request: &SendMessageRequest,
    ) -> Result<MessageAck, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            &format!("/api/v1/conversations/{conversation_id}/messages"),
            Some(&json!(request)),
        )
    }

    pub fn recall_message(
        &self,
        message_id: iamrust_domain::MessageId,
    ) -> Result<Message, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            &format!("/api/v1/messages/{message_id}/recall"),
            None,
        )
    }

    pub fn forward_message(
        &self,
        message_id: iamrust_domain::MessageId,
        target_conversation_id: ConversationId,
    ) -> Result<Vec<Message>, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            "/api/v1/messages/forward",
            Some(&json!(ForwardMessagesRequest {
                message_ids: vec![message_id],
                target_conversation_id,
                mode: ForwardMode::Individually,
            })),
        )
    }

    pub fn message_details(
        &self,
        message_id: iamrust_domain::MessageId,
    ) -> Result<MessageDetails, ClientError> {
        self.authorized_json(
            reqwest::Method::GET,
            &format!("/api/v1/messages/{message_id}"),
            None,
        )
    }

    pub fn react_to_message(
        &self,
        message_id: iamrust_domain::MessageId,
        emoji: &str,
        active: bool,
    ) -> Result<Vec<MessageReaction>, ClientError> {
        self.authorized_json(
            reqwest::Method::POST,
            &format!("/api/v1/messages/{message_id}/reaction"),
            Some(&json!(MessageReactionRequest {
                emoji: emoji.to_owned(),
                active,
            })),
        )
    }

    pub fn favorite_message(
        &self,
        message_id: iamrust_domain::MessageId,
        favorite: bool,
    ) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::POST,
            &format!("/api/v1/messages/{message_id}/favorite"),
            Some(&json!(FavoriteMessageRequest { favorite })),
        )?;
        Ok(())
    }

    pub fn mark_read(
        &self,
        conversation_id: ConversationId,
        through_sequence: u64,
    ) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::POST,
            &format!("/api/v1/conversations/{conversation_id}/read"),
            Some(&json!(MarkReadRequest { through_sequence })),
        )?;
        Ok(())
    }

    pub fn save_draft(
        &self,
        conversation_id: ConversationId,
        draft: String,
    ) -> Result<(), ClientError> {
        self.authorized_json::<Value>(
            reqwest::Method::PATCH,
            &format!("/api/v1/conversations/{conversation_id}/settings"),
            Some(&json!({ "draft": draft })),
        )?;
        Ok(())
    }

    pub fn sync(&self, after: u64, limit: usize) -> Result<SyncResponse, ClientError> {
        self.authorized_json(
            reqwest::Method::GET,
            &format!("/api/v1/sync?after={after}&limit={}", limit.clamp(1, 500)),
            None,
        )
    }

    pub(crate) fn websocket_session(
        &self,
        last_cursor: u64,
    ) -> Result<WebSocketSession, ClientError> {
        let ticket: WebSocketTicketResponse =
            self.authorized_json(reqwest::Method::POST, "/api/v1/ws-ticket", None)?;
        let mut url = self.safe_url("/api/v1/ws")?;
        let websocket_scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(websocket_scheme)
            .map_err(|()| ClientError::InvalidConfiguration)?;
        url.query_pairs_mut().append_pair("ticket", &ticket.ticket);
        Ok(WebSocketSession {
            url,
            hello: ClientFrame::Hello {
                protocol_version: WS_PROTOCOL_VERSION,
                client_version: env!("CARGO_PKG_VERSION").to_owned(),
                access_token: String::new(),
                last_cursor,
            },
        })
    }

    pub fn logout(&self) {
        let refresh = self.lock_session().refresh.clone();
        if let Some(refresh_token) = refresh {
            let _ = self.send_json::<Value, _>(
                reqwest::Method::POST,
                "/api/v1/auth/logout",
                Some(&json!({ "refresh_token": refresh_token })),
                None,
            );
        }
        self.clear_session();
    }

    fn authenticate<T: Serialize>(
        &self,
        path: &str,
        request: &T,
    ) -> Result<SessionResponse, ClientError> {
        let session = self.send_json(reqwest::Method::POST, path, Some(request), None)?;
        self.accept_session(&session)?;
        Ok(session)
    }

    fn authorized_json<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T, ClientError> {
        let access = self
            .lock_session()
            .access
            .clone()
            .ok_or_else(authentication_required)?;
        match self.send_json(method.clone(), path, body, Some(&access)) {
            Err(ClientError::Request { status: 401, .. }) => {
                let _refresh_guard = self.lock_refresh_gate();
                let latest_access = self.lock_session().access.clone();
                if latest_access.as_deref() == Some(access.as_str())
                    && let Err(error) = self.refresh_session()
                {
                    if matches!(error, ClientError::Request { status: 401, .. }) {
                        self.clear_session();
                    }
                    return Err(error);
                }
                let latest_access = self
                    .lock_session()
                    .access
                    .clone()
                    .ok_or_else(authentication_required)?;
                self.send_json(method, path, body, Some(&latest_access))
            }
            result => result,
        }
    }

    fn refresh_session(&self) -> Result<SessionResponse, ClientError> {
        let refresh_token = self
            .lock_session()
            .refresh
            .clone()
            .ok_or_else(authentication_required)?;
        let session = self.send_json(
            reqwest::Method::POST,
            "/api/v1/auth/refresh",
            Some(&RefreshRequest { refresh_token }),
            None,
        )?;
        self.accept_session(&session)?;
        Ok(session)
    }

    fn accept_session(&self, session: &SessionResponse) -> Result<(), ClientError> {
        self.credentials.save(&session.refresh_token)?;
        *self.lock_session() = SessionTokens {
            access: Some(session.access_token.clone()),
            refresh: Some(session.refresh_token.clone()),
        };
        Ok(())
    }

    fn clear_session(&self) {
        *self.lock_session() = SessionTokens::default();
        let _ = self.credentials.clear();
    }

    fn lock_session(&self) -> MutexGuard<'_, SessionTokens> {
        self.session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_refresh_gate(&self) -> MutexGuard<'_, ()> {
        self.refresh_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn send_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&B>,
        access_token: Option<&str>,
    ) -> Result<T, ClientError> {
        let url = self.safe_url(path)?;
        let mut request = self
            .client
            .request(method, url)
            .header(header::ACCEPT, "application/json");
        if let Some(access_token) = access_token {
            request = request.bearer_auth(access_token);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().map_err(|_| ClientError::Offline)?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE_BYTES)
        {
            return Err(ClientError::InvalidResponse);
        }
        let bytes = response.bytes().map_err(|_| ClientError::Offline)?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(ClientError::InvalidResponse);
        }
        if !status.is_success() {
            return Err(parse_error(status, &bytes));
        }
        if status == StatusCode::NO_CONTENT {
            return serde_json::from_value(Value::Null).map_err(|_| ClientError::InvalidResponse);
        }
        serde_json::from_slice(&bytes).map_err(|_| ClientError::InvalidResponse)
    }

    fn safe_url(&self, path: &str) -> Result<Url, ClientError> {
        if !path.starts_with("/api/v1/")
            || path.contains("..")
            || path.contains('\\')
            || path.chars().any(char::is_control)
        {
            return Err(ClientError::InvalidConfiguration);
        }
        let url = self
            .base_url
            .join(path.trim_start_matches('/'))
            .map_err(|_| ClientError::InvalidConfiguration)?;
        if url.origin() != self.base_url.origin() {
            return Err(ClientError::InvalidConfiguration);
        }
        Ok(url)
    }
}

fn image_mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn mime_type_from_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("txt" | "log" | "md") => "text/plain",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("m4a") => "audio/mp4",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

fn is_safe_transfer_url(url: &Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return false;
    }
    if url.scheme() == "https" {
        return url.host_str().is_some();
    }
    url.scheme() == "http" && has_loopback_host(url)
}

fn has_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        },
    )
}

fn parse_qr_payload(payload: &str) -> Result<(uuid::Uuid, String), ClientError> {
    let payload = payload.trim();
    if payload.len() > 2_048 {
        return Err(ClientError::InvalidResponse);
    }
    let url = Url::parse(payload).map_err(|_| ClientError::InvalidResponse)?;
    if url.scheme() != "iamrust"
        || url.host_str() != Some("auth")
        || url.path() != "/qr-login"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return Err(ClientError::InvalidResponse);
    }
    let mut challenge_id = None;
    let mut secret = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "challenge_id" if challenge_id.is_none() => challenge_id = Some(value.into_owned()),
            "secret" if secret.is_none() => secret = Some(value.into_owned()),
            _ => return Err(ClientError::InvalidResponse),
        }
    }
    let challenge_id = challenge_id
        .and_then(|value| uuid::Uuid::parse_str(&value).ok())
        .ok_or(ClientError::InvalidResponse)?;
    let secret = secret.ok_or(ClientError::InvalidResponse)?;
    if secret.len() != 64 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ClientError::InvalidResponse);
    }
    Ok((challenge_id, secret))
}

fn parse_error(status: StatusCode, bytes: &[u8]) -> ClientError {
    let body = serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null);
    ClientError::Request {
        status: status.as_u16(),
        code: body
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        message_key: body
            .get("message_key")
            .and_then(Value::as_str)
            .unwrap_or("request_failed")
            .to_owned(),
        retryable: body
            .get("retryable")
            .and_then(Value::as_bool)
            .unwrap_or(status.is_server_error()),
    }
}

fn authentication_required() -> ClientError {
    ClientError::Request {
        status: 401,
        code: "authentication_required".to_owned(),
        message_key: "authentication_required".to_owned(),
        retryable: false,
    }
}

fn device_name() -> String {
    format!("I Am Rust on {}", std::env::consts::OS)
}

impl CredentialStore for SystemCredentialStore {
    fn save(&self, token: &str) -> Result<(), ClientError> {
        keyring::Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT)
            .map_err(|_| ClientError::CredentialStore)?
            .set_password(token)
            .map_err(|_| ClientError::CredentialStore)
    }

    fn load(&self) -> Result<Option<String>, ClientError> {
        let entry = keyring::Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT)
            .map_err(|_| ClientError::CredentialStore)?;
        match entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(ClientError::CredentialStore),
        }
    }

    fn clear(&self) -> Result<(), ClientError> {
        let entry = keyring::Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT)
            .map_err(|_| ClientError::CredentialStore)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(ClientError::CredentialStore),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        sync::{
            Barrier,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
    };

    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode as AxumStatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use serde_json::json;
    use tokio::sync::oneshot;

    use super::*;

    #[derive(Debug, Default)]
    struct MemoryCredentialStore(Mutex<Option<String>>);

    impl CredentialStore for MemoryCredentialStore {
        fn save(&self, token: &str) -> Result<(), ClientError> {
            *self.0.lock().unwrap() = Some(token.to_owned());
            Ok(())
        }

        fn load(&self) -> Result<Option<String>, ClientError> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn clear(&self) -> Result<(), ClientError> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[test]
    fn allows_https_and_loopback_http_only() {
        assert!(ApiClient::new("http://127.0.0.1:3780").is_ok());
        assert!(ApiClient::new("http://localhost:3780").is_ok());
        assert!(ApiClient::new("https://chat.example.com").is_ok());
        assert!(ApiClient::new("http://chat.example.com").is_err());
        assert!(ApiClient::new("file:///tmp/socket").is_err());
    }

    #[test]
    fn parses_structured_server_errors_without_exposing_body() {
        let error = parse_error(
            StatusCode::TOO_MANY_REQUESTS,
            br#"{"code":"rate_limited","message_key":"try_later","retryable":true,"secret":"not shown"}"#,
        );
        assert!(matches!(
            error,
            ClientError::Request {
                status: 429,
                retryable: true,
                ..
            }
        ));
        assert!(!error.to_string().contains("not shown"));
    }

    #[test]
    fn detects_only_supported_image_magic_headers() {
        assert_eq!(image_mime_type(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(
            image_mime_type(&[0xff, 0xd8, 0xff, 0xe0]),
            Some("image/jpeg")
        );
        assert_eq!(image_mime_type(b"GIF87arest"), Some("image/gif"));
        assert_eq!(image_mime_type(b"GIF89arest"), Some("image/gif"));
        assert_eq!(
            image_mime_type(b"RIFF\x04\x00\x00\x00WEBPrest"),
            Some("image/webp")
        );
        assert_eq!(image_mime_type(b"not really an image.png"), None);
        assert_eq!(image_mime_type(b"RIFFtoo-short"), None);
    }

    #[test]
    fn transfer_urls_require_https_or_loopback_http_without_credentials() {
        for allowed in [
            "https://cdn.example.com/file?signature=abc",
            "http://127.0.0.1:3000/file",
            "http://[::1]:3000/file",
            "http://localhost:3000/file",
        ] {
            assert!(is_safe_transfer_url(&Url::parse(allowed).unwrap()));
        }
        for denied in [
            "http://cdn.example.com/file",
            "ftp://cdn.example.com/file",
            "https://user:password@cdn.example.com/file",
            "https://cdn.example.com/file#fragment",
            "file:///tmp/file",
        ] {
            assert!(!is_safe_transfer_url(&Url::parse(denied).unwrap()));
        }
    }

    #[test]
    fn qr_approval_payload_accepts_only_the_expected_deep_link() {
        let id = "018f47b2-a456-7def-8123-456789abcdef";
        let secret = "a".repeat(64);
        let payload = format!("iamrust://auth/qr-login?challenge_id={id}&secret={secret}");
        assert_eq!(parse_qr_payload(&payload).unwrap().1, secret);
        assert!(parse_qr_payload("https://example.com/qr-login").is_err());
        assert!(
            parse_qr_payload(&format!(
                "iamrust://auth/qr-login?challenge_id={id}&secret={}&extra=1",
                "a".repeat(64)
            ))
            .is_err()
        );
    }

    #[test]
    fn concurrent_unauthorized_requests_rotate_refresh_token_once() {
        let refresh_count = Arc::new(AtomicUsize::new(0));
        let (address, shutdown) = spawn_test_server(refresh_count.clone());
        let credentials = Arc::new(MemoryCredentialStore::default());
        let client = Arc::new(
            ApiClient::with_credentials(&format!("http://{address}"), credentials).unwrap(),
        );
        *client.lock_session() = SessionTokens {
            access: Some("expired-access".to_owned()),
            refresh: Some("refresh-0".to_owned()),
        };

        let threads = 8;
        let barrier = Arc::new(Barrier::new(threads));
        let handles = (0..threads)
            .map(|_| {
                let client = client.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    client
                        .authorized_json::<Value>(reqwest::Method::GET, "/api/v1/bootstrap", None)
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            assert_eq!(handle.join().unwrap()["ok"], true);
        }
        assert_eq!(refresh_count.load(Ordering::SeqCst), 1);
        let _ = shutdown.send(());
    }

    fn spawn_test_server(refresh_count: Arc<AtomicUsize>) -> (SocketAddr, oneshot::Sender<()>) {
        let (address_tx, address_rx) = std::sync::mpsc::sync_channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let app = Router::new()
                    .route(
                        "/api/v1/bootstrap",
                        get(|headers: HeaderMap| async move {
                            if headers
                                .get(axum::http::header::AUTHORIZATION)
                                .and_then(|value| value.to_str().ok())
                                == Some("Bearer fresh-access")
                            {
                                Json(json!({ "ok": true })).into_response()
                            } else {
                                (
                                    AxumStatusCode::UNAUTHORIZED,
                                    Json(json!({
                                        "code": "authentication_required",
                                        "message_key": "authentication_required",
                                        "retryable": false
                                    })),
                                )
                                    .into_response()
                            }
                        }),
                    )
                    .route(
                        "/api/v1/auth/refresh",
                        post(|State(refresh_count): State<Arc<AtomicUsize>>| async move {
                            refresh_count.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            Json(json!({
                                "access_token": "fresh-access",
                                "refresh_token": "refresh-1",
                                "access_expires_at": "2030-01-01T00:00:00Z",
                                "refresh_expires_at": "2030-02-01T00:00:00Z",
                                "profile": {
                                    "id": "00000000-0000-0000-0000-000000000001",
                                    "username": "tester",
                                    "nickname": "Tester",
                                    "avatar_url": null,
                                    "avatar_attachment_id": null,
                                    "signature": "",
                                    "gender": null,
                                    "birthday": null,
                                    "region": null,
                                    "presence": "online",
                                    "last_seen_at": null
                                },
                                "device_id": "00000000-0000-0000-0000-000000000002"
                            }))
                        }),
                    )
                    .with_state(refresh_count);
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                address_tx.send(listener.local_addr().unwrap()).unwrap();
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .unwrap();
            });
        });
        (address_rx.recv().unwrap(), shutdown_tx)
    }
}
