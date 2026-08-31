use std::{
    collections::HashMap,
    fmt,
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, FromRef, FromRequestParts, Path, Query, State, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header, request::Parts},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{delete, get, patch, post},
};
use chrono::Utc;
use iamrust_application::{
    AuthenticatedSession, ChatService, LoginInput, RegisterInput, UpdateProfileInput,
};
use iamrust_domain::{
    AttachmentId, ConversationId, DeviceId, FriendRequestId, ProfilePrivacySettings, UserId,
};
use iamrust_protocol::{
    AddGroupMembersRequest, AdminAuditEntry, AdminSuspendUserRequest, BootstrapResponse,
    ChangePasswordRequest, CompleteUploadRequest, CompleteUploadResponse,
    CreateDirectConversationRequest, CreateGroupAnnouncementRequest, CreateGroupJoinRequest,
    CreateGroupPollRequest, CreateGroupRequest, CreateStickerRequest, DecideGroupJoinRequest,
    DeleteAccountRequest, DisableSecondFactorRequest, DownloadAuthorizationResponse,
    FavoriteMessageRequest, ForwardMessagesRequest, FriendRequestCreate, FriendRequestDecisionBody,
    GroupFileItem, GroupMuteRequest, GroupSettingsResponse, LoginRequest, LogoutRequest,
    MarkReadRequest, MessageReactionRequest, Page, PasswordResetConfirmRequest,
    PasswordResetRequest, PersonalDataExport, QrLoginPollResponse, QrLoginSecretRequest,
    QrLoginStartRequest, QrLoginStartResponse, QrLoginStatus, RecoveryCodesResponse,
    RefreshRequest, RegenerateRecoveryCodesRequest, RegisterRequest, ReportUserRequest,
    ScheduleMessageRequest, SecondFactorCodeRequest, SecondFactorSetupResponse, SecondFactorStatus,
    SendMessageRequest, SessionResponse, TranscribeMessageResponse, TransferGroupOwnershipRequest,
    TranslateMessageRequest, TranslateMessageResponse, UpdateConversationSettingsRequest,
    UpdateFriendSettingsRequest, UpdateGroupMemberRequest, UpdateGroupRequest,
    UpdateProfileRequest, UploadAuthorizationRequest, UploadAuthorizationResponse,
    VoteGroupPollRequest, WebSocketTicketResponse,
};
use opentelemetry::{global, propagation::Extractor};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::sync::Mutex;
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use url::Url;

use crate::{
    error::AppError,
    mailer::Mailer,
    malware::{MalwareScanner, ScanVerdict},
    object_store::ObjectStore,
    transcription::Transcriber,
    translation::Translator,
    websocket,
};

#[derive(Debug, Clone)]
pub struct AppState {
    pub service: ChatService,
    rate_limiter: RateLimiter,
    metrics: Arc<Metrics>,
    websocket_tickets: Arc<Mutex<HashMap<String, WebSocketTicket>>>,
    mailer: Option<Mailer>,
    object_store: Option<ObjectStore>,
    translator: Option<Translator>,
    transcriber: Option<Transcriber>,
    malware_scanner: Option<MalwareScanner>,
    admin_token: Option<AdminToken>,
    database: Option<PgPool>,
}

#[derive(Clone)]
struct AdminToken(Arc<str>);

impl fmt::Debug for AdminToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl AppState {
    pub fn new(service: ChatService) -> Self {
        Self {
            service,
            rate_limiter: RateLimiter::default(),
            metrics: Arc::new(Metrics::default()),
            websocket_tickets: Arc::new(Mutex::new(HashMap::new())),
            mailer: None,
            object_store: None,
            translator: None,
            transcriber: None,
            malware_scanner: None,
            admin_token: None,
            database: None,
        }
    }

    pub fn with_database(mut self, database: PgPool) -> Self {
        self.database = Some(database);
        self
    }

    pub fn with_object_store(mut self, object_store: ObjectStore) -> Self {
        self.object_store = Some(object_store);
        self
    }

    pub fn with_mailer(mut self, mailer: Mailer) -> Self {
        self.mailer = Some(mailer);
        self
    }

    pub fn with_translator(mut self, translator: Option<Translator>) -> Self {
        self.translator = translator;
        self
    }

    pub fn with_transcriber(mut self, transcriber: Option<Transcriber>) -> Self {
        self.transcriber = transcriber;
        self
    }

    pub fn with_malware_scanner(mut self, scanner: Option<MalwareScanner>) -> Self {
        self.malware_scanner = scanner;
        self
    }

    pub fn with_admin_token(mut self, token: Option<String>) -> Self {
        self.admin_token = token.map(|token| AdminToken(Arc::from(token)));
        self
    }
}

#[derive(Debug, Default)]
struct Metrics {
    requests: AtomicU64,
    client_errors: AtomicU64,
    server_errors: AtomicU64,
    request_latency_micros: AtomicU64,
    request_latency_samples: AtomicU64,
    auth_failures: AtomicU64,
    messages_sent: AtomicU64,
    message_failures: AtomicU64,
    message_ack_latency_micros: AtomicU64,
    message_ack_latency_samples: AtomicU64,
    sync_backlog_events: AtomicU64,
    websocket_connections_total: AtomicU64,
    websocket_connections_active: AtomicU64,
    quarantined_files: AtomicU64,
}

#[derive(Debug, Clone, Default)]
struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, Bucket>>>,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    started_at: Instant,
    count: u32,
}

#[derive(Debug, Clone, Copy)]
struct WebSocketTicket {
    user_id: UserId,
    expires_at: Instant,
}

struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(axum::http::HeaderName::as_str).collect()
    }
}

impl RateLimiter {
    async fn check(&self, key: String, max: u32, window: Duration) -> Result<(), AppError> {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().await;
        buckets.retain(|_, bucket| now.duration_since(bucket.started_at) <= window);
        let bucket = buckets.entry(key).or_insert(Bucket {
            started_at: now,
            count: 0,
        });
        if now.duration_since(bucket.started_at) > window {
            *bucket = Bucket {
                started_at: now,
                count: 0,
            };
        }
        if bucket.count >= max {
            return Err(AppError::rate_limited());
        }
        bucket.count += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CurrentUser(pub UserId, pub DeviceId);

#[derive(Debug, Clone, Copy)]
struct AdminAccess;

impl<S> FromRequestParts<S> for CurrentUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let value = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|header| header.to_str().ok())
            .and_then(|header| header.strip_prefix("Bearer "))
            .ok_or_else(AppError::unauthorized)?;
        state
            .service
            .authenticate_identity(value, Utc::now())
            .await
            .map(|(user_id, device_id)| Self(user_id, device_id))
            .map_err(Into::into)
    }
}

impl<S> FromRequestParts<S> for AdminAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let state = AppState::from_ref(state);
        let result = parts
            .headers
            .get("x-admin-token")
            .and_then(|value| value.to_str().ok())
            .zip(state.admin_token.as_ref())
            .filter(|(supplied, expected)| constant_time_secret_eq(supplied, &expected.0))
            .map(|_| Self)
            .ok_or_else(AppError::unauthorized);
        std::future::ready(result)
    }
}

fn constant_time_secret_eq(left: &str, right: &str) -> bool {
    let left = Sha256::digest(left.as_bytes());
    let right = Sha256::digest(right.as_bytes());
    left.iter()
        .zip(right.iter())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[allow(clippy::too_many_lines)]
pub fn router(state: AppState) -> Router {
    let observer_metrics = state.metrics.clone();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list([
            HeaderValue::from_static("http://127.0.0.1:1420"),
            HeaderValue::from_static("http://127.0.0.1:1421"),
            HeaderValue::from_static("http://tauri.localhost"),
            HeaderValue::from_static("tauri://localhost"),
        ]))
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::HeaderName::from_static("x-request-id"),
            header::HeaderName::from_static("traceparent"),
            header::HeaderName::from_static("tracestate"),
        ])
        .expose_headers([header::HeaderName::from_static("x-request-id")]);
    let trace = TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<Body>| {
        let request_id = request
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("missing");
        let span = tracing::info_span!(
            "http_request",
            request_id,
            method = %request.method(),
            path = %request.uri().path()
        );
        let parent = global::get_text_map_propagator(|propagator| {
            propagator.extract(&HeaderExtractor(request.headers()))
        });
        let _parent_result = span.set_parent(parent);
        span
    });

    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/metrics", get(metrics))
        .route("/admin", get(admin_ui))
        .route("/api/v1/openapi.json", get(openapi))
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/qr-login", post(begin_qr_login))
        .route(
            "/api/v1/auth/qr-login/{challenge_id}/poll",
            post(poll_qr_login),
        )
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .route(
            "/api/v1/auth/password-reset/request",
            post(request_password_reset),
        )
        .route(
            "/api/v1/auth/password-reset/confirm",
            post(confirm_password_reset),
        )
        .route("/api/v1/auth/change-password", post(change_password))
        .route("/api/v1/devices", get(devices))
        .route("/api/v1/devices/{device_id}", delete(revoke_device))
        .route("/api/v1/me", get(me).patch(update_me).delete(delete_me))
        .route(
            "/api/v1/me/second-factor",
            get(second_factor_status)
                .post(begin_second_factor_setup)
                .delete(disable_second_factor),
        )
        .route(
            "/api/v1/me/second-factor/enable",
            post(enable_second_factor),
        )
        .route(
            "/api/v1/me/second-factor/recovery-codes",
            post(regenerate_recovery_codes),
        )
        .route(
            "/api/v1/auth/qr-login/{challenge_id}/approve",
            post(approve_qr_login),
        )
        .route(
            "/api/v1/me/privacy",
            get(profile_privacy).patch(update_profile_privacy),
        )
        .route("/api/v1/me/export", get(export_me))
        .route("/api/v1/me/stickers", get(stickers).post(create_sticker))
        .route("/api/v1/me/stickers/{sticker_id}", delete(delete_sticker))
        .route("/api/v1/bootstrap", get(bootstrap))
        .route("/api/v1/users/search", get(search_users))
        .route("/api/v1/friends", get(friends))
        .route("/api/v1/friends/settings", get(friend_settings))
        .route(
            "/api/v1/friends/{friend_id}",
            patch(update_friend_settings).delete(delete_friend),
        )
        .route(
            "/api/v1/blocks/{user_id}",
            post(block_user).delete(unblock_user),
        )
        .route("/api/v1/reports/{user_id}", post(report_user))
        .route(
            "/api/v1/friend-requests",
            get(friend_requests).post(create_friend_request),
        )
        .route(
            "/api/v1/friend-requests/{request_id}",
            patch(decide_friend_request),
        )
        .route("/api/v1/conversations", get(conversations))
        .route("/api/v1/conversations/read-all", post(mark_all_read))
        .route("/api/v1/conversations/direct", post(create_direct))
        .route("/api/v1/conversations/group", post(create_group))
        .route(
            "/api/v1/groups/{conversation_id}",
            get(group_settings)
                .patch(update_group)
                .delete(disband_group),
        )
        .route(
            "/api/v1/groups/{conversation_id}/members",
            post(add_group_members),
        )
        .route(
            "/api/v1/groups/{conversation_id}/members/{member_id}",
            patch(update_group_member).delete(remove_group_member),
        )
        .route("/api/v1/groups/{conversation_id}/leave", post(leave_group))
        .route(
            "/api/v1/groups/{conversation_id}/transfer",
            post(transfer_group_ownership),
        )
        .route(
            "/api/v1/groups/{conversation_id}/mute",
            post(set_group_mute),
        )
        .route(
            "/api/v1/groups/{conversation_id}/announcements",
            get(group_announcements).post(create_group_announcement),
        )
        .route(
            "/api/v1/group-announcements/{announcement_id}/read",
            post(read_group_announcement),
        )
        .route(
            "/api/v1/groups/{conversation_id}/join-requests",
            get(group_join_requests).post(request_to_join_group),
        )
        .route(
            "/api/v1/group-join-requests/{request_id}",
            patch(decide_group_join_request),
        )
        .route(
            "/api/v1/groups/{conversation_id}/polls",
            get(group_polls).post(create_group_poll),
        )
        .route("/api/v1/groups/{conversation_id}/files", get(group_files))
        .route("/api/v1/polls/{poll_id}/vote", post(vote_group_poll))
        .route(
            "/api/v1/conversations/{conversation_id}/settings",
            get(conversation_settings).patch(update_conversation_settings),
        )
        .route(
            "/api/v1/conversations/{conversation_id}/messages",
            get(messages).post(send_message),
        )
        .route("/api/v1/messages/forward", post(forward_messages))
        .route("/api/v1/messages/favorites", get(favorite_messages))
        .route(
            "/api/v1/messages/{message_id}/translate",
            post(translate_message),
        )
        .route(
            "/api/v1/messages/{message_id}/transcribe",
            post(transcribe_message),
        )
        .route("/api/v1/messages/{message_id}", get(message_details))
        .route(
            "/api/v1/messages/{message_id}/delivery",
            post(acknowledge_delivery),
        )
        .route("/api/v1/messages/{message_id}/recall", post(recall_message))
        .route(
            "/api/v1/messages/{message_id}/reaction",
            post(react_to_message),
        )
        .route(
            "/api/v1/messages/{message_id}/favorite",
            post(set_message_favorite),
        )
        .route(
            "/api/v1/scheduled-messages",
            get(scheduled_messages).post(schedule_message),
        )
        .route(
            "/api/v1/scheduled-messages/{schedule_id}",
            delete(cancel_scheduled_message),
        )
        .route(
            "/api/v1/conversations/{conversation_id}/read",
            post(mark_read),
        )
        .route("/api/v1/sync", get(sync))
        .route("/api/v1/uploads/authorize", post(authorize_upload))
        .route("/api/v1/uploads/complete", post(complete_upload))
        .route(
            "/api/v1/attachments/{attachment_id}/download",
            get(authorize_download),
        )
        .route("/api/v1/ws-ticket", post(websocket_ticket))
        .route("/api/v1/ws", get(websocket_upgrade))
        .route(
            "/api/v1/admin/users/{user_id}/suspension",
            post(admin_set_user_suspension),
        )
        .route(
            "/api/v1/admin/users/{user_id}/sessions/revoke",
            post(admin_revoke_user_sessions),
        )
        .route("/api/v1/admin/audit", get(admin_audit))
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            observer_metrics,
            observe_http_request,
        ))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(trace)
        .layer(SetSensitiveRequestHeadersLayer::new(std::iter::once(
            header::AUTHORIZATION,
        )))
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(cors)
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}

async fn observe_http_request(
    State(metrics): State<Arc<Metrics>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let started = Instant::now();
    let response = next.run(request).await;
    metrics.requests.fetch_add(1, Ordering::Relaxed);
    if response.status().is_client_error() {
        metrics.client_errors.fetch_add(1, Ordering::Relaxed);
    }
    if response.status().is_server_error() {
        metrics.server_errors.fetch_add(1, Ordering::Relaxed);
    }
    metrics
        .request_latency_micros
        .fetch_add(micros_u64(started.elapsed()), Ordering::Relaxed);
    metrics
        .request_latency_samples
        .fetch_add(1, Ordering::Relaxed);
    response
}

fn micros_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn duration_seconds(micros: u64) -> String {
    format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000)
}

async fn live() -> Json<Value> {
    Json(json!({ "status": "alive" }))
}

async fn admin_ui() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; form-action 'none'; base-uri 'none'; frame-ancestors 'none'",
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        Html(include_str!("../admin.html")),
    )
}

async fn ready(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let database = if let Some(pool) = &state.database {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(pool)
            .await
            .map_err(|_| {
                AppError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    iamrust_protocol::ErrorCode::ServiceUnavailable,
                    "error.database_unavailable",
                    None,
                    true,
                )
            })?;
        "ok"
    } else {
        "disabled"
    };
    Ok(Json(json!({
        "status": "ready",
        "dependencies": { "application": "ok", "database": database }
    })))
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let request_latency_seconds =
        duration_seconds(state.metrics.request_latency_micros.load(Ordering::Relaxed));
    let ack_latency_seconds = duration_seconds(
        state
            .metrics
            .message_ack_latency_micros
            .load(Ordering::Relaxed),
    );
    let metrics = format!(
        concat!(
            "# TYPE iamrust_http_requests_total counter\n",
            "iamrust_http_requests_total {}\n",
            "# TYPE iamrust_http_client_errors_total counter\n",
            "iamrust_http_client_errors_total {}\n",
            "# TYPE iamrust_http_server_errors_total counter\n",
            "iamrust_http_server_errors_total {}\n",
            "# TYPE iamrust_http_request_duration_seconds summary\n",
            "iamrust_http_request_duration_seconds_sum {}\n",
            "iamrust_http_request_duration_seconds_count {}\n",
            "# TYPE iamrust_auth_failures_total counter\n",
            "iamrust_auth_failures_total {}\n",
            "# TYPE iamrust_messages_sent_total counter\n",
            "iamrust_messages_sent_total {}\n",
            "# TYPE iamrust_message_send_failures_total counter\n",
            "iamrust_message_send_failures_total {}\n",
            "# TYPE iamrust_message_ack_duration_seconds summary\n",
            "iamrust_message_ack_duration_seconds_sum {}\n",
            "iamrust_message_ack_duration_seconds_count {}\n",
            "# TYPE iamrust_websocket_connections_total counter\n",
            "iamrust_websocket_connections_total {}\n",
            "# TYPE iamrust_websocket_connections_active gauge\n",
            "iamrust_websocket_connections_active {}\n",
            "# TYPE iamrust_sync_backlog_events gauge\n",
            "iamrust_sync_backlog_events {}\n",
            "# TYPE iamrust_quarantined_files_total counter\n",
            "iamrust_quarantined_files_total {}\n"
        ),
        state.metrics.requests.load(Ordering::Relaxed),
        state.metrics.client_errors.load(Ordering::Relaxed),
        state.metrics.server_errors.load(Ordering::Relaxed),
        request_latency_seconds,
        state
            .metrics
            .request_latency_samples
            .load(Ordering::Relaxed),
        state.metrics.auth_failures.load(Ordering::Relaxed),
        state.metrics.messages_sent.load(Ordering::Relaxed),
        state.metrics.message_failures.load(Ordering::Relaxed),
        ack_latency_seconds,
        state
            .metrics
            .message_ack_latency_samples
            .load(Ordering::Relaxed),
        state
            .metrics
            .websocket_connections_total
            .load(Ordering::Relaxed),
        state
            .metrics
            .websocket_connections_active
            .load(Ordering::Relaxed),
        state.metrics.sync_backlog_events.load(Ordering::Relaxed),
        state.metrics.quarantined_files.load(Ordering::Relaxed),
    );
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        metrics,
    )
}

async fn openapi() -> Json<Value> {
    Json(crate::openapi::document())
}

async fn register(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Json(request): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), AppError> {
    rate_limit(&state, address.ip(), "register", 5).await?;
    let session = state
        .service
        .register(
            RegisterInput {
                email: request.email,
                username: request.username,
                password: request.password,
                nickname: request.nickname,
                device_name: request.device_name,
                platform: request.platform.unwrap_or_else(|| "unknown".to_owned()),
                app_version: request.app_version.unwrap_or_else(|| "unknown".to_owned()),
            },
            Utc::now(),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(session_response(session))))
}

async fn login(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<SessionResponse>, AppError> {
    rate_limit(&state, address.ip(), "login", 10).await?;
    let device_name = request.device_name.clone();
    let platform = request
        .platform
        .clone()
        .unwrap_or_else(|| "unknown".to_owned());
    let app_version = request
        .app_version
        .clone()
        .unwrap_or_else(|| "unknown".to_owned());
    let second_factor_code = request.second_factor_code;
    match state
        .service
        .login_with_second_factor(
            LoginInput {
                login: request.login,
                password: request.password,
                device_name: request.device_name,
                platform: platform.clone(),
                app_version: app_version.clone(),
            },
            second_factor_code,
            Utc::now(),
        )
        .await
    {
        Ok(session) => {
            let known_device = state
                .service
                .devices(session.profile.id, session.device_id)
                .await
                .into_iter()
                .any(|device| {
                    !device.current && device.name == device_name && device.platform == platform
                });
            if let (Some(mailer), Ok(email)) = (
                state.mailer.clone(),
                state.service.account_email(session.profile.id).await,
            ) && !known_device
            {
                tokio::spawn(async move {
                    if let Err(error) = mailer
                        .send_new_device_login(
                            email,
                            device_name,
                            platform,
                            app_version,
                            Utc::now(),
                        )
                        .await
                    {
                        tracing::warn!(error = %error, "failed to deliver new-device login email");
                    }
                });
            }
            Ok(Json(session_response(session)))
        }
        Err(error) => {
            state.metrics.auth_failures.fetch_add(1, Ordering::Relaxed);
            Err(error.into())
        }
    }
}

async fn begin_qr_login(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Json(request): Json<QrLoginStartRequest>,
) -> Result<(StatusCode, Json<QrLoginStartResponse>), AppError> {
    rate_limit(&state, address.ip(), "qr-login-start", 12).await?;
    let challenge = state
        .service
        .begin_qr_login(
            request.device_name,
            request.platform.unwrap_or_else(|| "unknown".to_owned()),
            request.app_version.unwrap_or_else(|| "unknown".to_owned()),
            Utc::now(),
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(QrLoginStartResponse {
            challenge_id: challenge.challenge_id,
            secret: challenge.secret,
            qr_payload: challenge.qr_payload,
            expires_at: challenge.expires_at,
        }),
    ))
}

async fn approve_qr_login(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(challenge_id): Path<uuid::Uuid>,
    Json(request): Json<QrLoginSecretRequest>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .approve_qr_login(user_id, challenge_id, &request.secret, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn poll_qr_login(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Path(challenge_id): Path<uuid::Uuid>,
    Json(request): Json<QrLoginSecretRequest>,
) -> Result<Json<QrLoginPollResponse>, AppError> {
    rate_limit(&state, address.ip(), "qr-login-poll", 90).await?;
    let Some(session) = state
        .service
        .poll_qr_login(challenge_id, &request.secret, Utc::now())
        .await?
    else {
        return Ok(Json(QrLoginPollResponse {
            status: QrLoginStatus::Pending,
            session: None,
        }));
    };
    let response = session_response(session.clone());
    if let (Some(mailer), Ok(email), Some(device)) = (
        state.mailer.clone(),
        state.service.account_email(session.profile.id).await,
        state
            .service
            .devices(session.profile.id, session.device_id)
            .await
            .into_iter()
            .find(|device| device.current),
    ) {
        tokio::spawn(async move {
            if let Err(error) = mailer
                .send_new_device_login(
                    email,
                    device.name,
                    device.platform,
                    device.app_version,
                    Utc::now(),
                )
                .await
            {
                tracing::warn!(error = %error, "failed to deliver QR-login device email");
            }
        });
    }
    Ok(Json(QrLoginPollResponse {
        status: QrLoginStatus::Ready,
        session: Some(response),
    }))
}

async fn second_factor_status(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Result<Json<SecondFactorStatus>, AppError> {
    Ok(Json(state.service.second_factor_status(user_id).await?))
}

async fn begin_second_factor_setup(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Result<Json<SecondFactorSetupResponse>, AppError> {
    Ok(Json(
        state
            .service
            .begin_second_factor_setup(user_id, Utc::now())
            .await?,
    ))
}

async fn enable_second_factor(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<SecondFactorCodeRequest>,
) -> Result<Json<RecoveryCodesResponse>, AppError> {
    Ok(Json(RecoveryCodesResponse {
        recovery_codes: state
            .service
            .enable_second_factor(user_id, &request.code, Utc::now())
            .await?,
    }))
}

async fn disable_second_factor(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<DisableSecondFactorRequest>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .disable_second_factor(
            user_id,
            &request.current_password,
            &request.code,
            Utc::now(),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn regenerate_recovery_codes(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<RegenerateRecoveryCodesRequest>,
) -> Result<Json<RecoveryCodesResponse>, AppError> {
    Ok(Json(RecoveryCodesResponse {
        recovery_codes: state
            .service
            .regenerate_recovery_codes(
                user_id,
                &request.current_password,
                &request.code,
                Utc::now(),
            )
            .await?,
    }))
}

async fn refresh(
    State(state): State<AppState>,
    Json(request): Json<RefreshRequest>,
) -> Result<Json<SessionResponse>, AppError> {
    let session = state
        .service
        .refresh(&request.refresh_token, Utc::now())
        .await?;
    Ok(Json(session_response(session)))
}

async fn logout(
    State(state): State<AppState>,
    Json(request): Json<LogoutRequest>,
) -> Result<StatusCode, AppError> {
    state.service.logout(&request.refresh_token).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn request_password_reset(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Json(request): Json<PasswordResetRequest>,
) -> Result<StatusCode, AppError> {
    rate_limit(&state, address.ip(), "password-reset", 5).await?;
    if let Some(delivery) = state
        .service
        .request_password_reset(&request.email, Utc::now())
        .await?
    {
        if let Some(mailer) = state.mailer.clone() {
            tokio::spawn(async move {
                if let Err(error) = mailer.send_password_reset(delivery).await {
                    tracing::warn!(error = %error, "failed to deliver password reset email");
                }
            });
        } else {
            tracing::warn!("password reset email transport is unavailable");
        }
    }
    Ok(StatusCode::ACCEPTED)
}

async fn confirm_password_reset(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Json(request): Json<PasswordResetConfirmRequest>,
) -> Result<StatusCode, AppError> {
    rate_limit(&state, address.ip(), "password-reset-confirm", 8).await?;
    state
        .service
        .reset_password(&request.reset_token, request.new_password, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn change_password(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .change_password(user_id, request.current_password, request.new_password)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn devices(
    State(state): State<AppState>,
    CurrentUser(user_id, device_id): CurrentUser,
) -> Json<Vec<iamrust_protocol::DeviceInfo>> {
    Json(state.service.devices(user_id, device_id).await)
}

async fn revoke_device(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(device_id): Path<DeviceId>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .revoke_device(user_id, device_id, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn me(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Result<Json<iamrust_domain::UserProfile>, AppError> {
    Ok(Json(state.service.profile(user_id).await?))
}

async fn update_me(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<UpdateProfileRequest>,
) -> Result<Json<iamrust_domain::UserProfile>, AppError> {
    let avatar_url = request
        .avatar_url
        .map(|value| {
            let url = Url::parse(&value).map_err(|_| AppError::validation("avatar_url"))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(AppError::validation("avatar_url"));
            }
            Ok(url)
        })
        .transpose()?;
    Ok(Json(
        state
            .service
            .update_profile(
                user_id,
                UpdateProfileInput {
                    nickname: request.nickname,
                    signature: request.signature,
                    avatar_url,
                    avatar_attachment_id: request.avatar_attachment_id,
                    gender: request.gender,
                    birthday: request.birthday,
                    region: request.region,
                    presence: request.presence,
                },
                Utc::now(),
            )
            .await?,
    ))
}

async fn profile_privacy(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Json<ProfilePrivacySettings> {
    Json(state.service.profile_privacy(user_id).await)
}

async fn update_profile_privacy(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<ProfilePrivacySettings>,
) -> Result<Json<ProfilePrivacySettings>, AppError> {
    Ok(Json(
        state
            .service
            .update_profile_privacy(user_id, request, Utc::now())
            .await?,
    ))
}

async fn export_me(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Result<Json<PersonalDataExport>, AppError> {
    Ok(Json(
        state
            .service
            .export_personal_data(user_id, Utc::now())
            .await?,
    ))
}

async fn delete_me(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<DeleteAccountRequest>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .delete_account(
            user_id,
            request.current_password,
            request.confirmation,
            Utc::now(),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stickers(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Json<Vec<iamrust_protocol::Sticker>> {
    Json(state.service.stickers(user_id).await)
}

async fn create_sticker(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<CreateStickerRequest>,
) -> Result<(StatusCode, Json<iamrust_protocol::Sticker>), AppError> {
    Ok((
        StatusCode::CREATED,
        Json(
            state
                .service
                .create_sticker(user_id, request, Utc::now())
                .await?,
        ),
    ))
}

async fn delete_sticker(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(sticker_id): Path<uuid::Uuid>,
) -> Result<StatusCode, AppError> {
    state.service.delete_sticker(user_id, sticker_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn bootstrap(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Result<Json<BootstrapResponse>, AppError> {
    Ok(Json(state.service.bootstrap(user_id).await?))
}

#[derive(Debug, Deserialize)]
struct UserSearchQuery {
    username: String,
}

async fn search_users(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    CurrentUser(user_id, _): CurrentUser,
    Query(query): Query<UserSearchQuery>,
) -> Result<Json<Vec<iamrust_domain::UserProfile>>, AppError> {
    rate_limit(&state, address.ip(), "user-search", 60).await?;
    let result = state
        .service
        .search_user_exact(user_id, &query.username)
        .await?;
    Ok(Json(result.into_iter().collect()))
}

async fn friends(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Json<Vec<iamrust_domain::UserProfile>> {
    Json(state.service.friends(user_id).await)
}

async fn friend_settings(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Json<Vec<iamrust_protocol::FriendSettings>> {
    Json(state.service.friend_settings(user_id).await)
}

async fn update_friend_settings(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(friend_id): Path<UserId>,
    Json(request): Json<UpdateFriendSettingsRequest>,
) -> Result<Json<iamrust_protocol::FriendSettings>, AppError> {
    Ok(Json(
        state
            .service
            .update_friend_settings(user_id, friend_id, request, Utc::now())
            .await?,
    ))
}

async fn delete_friend(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(friend_id): Path<UserId>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .delete_friend(user_id, friend_id, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn block_user(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(target_id): Path<UserId>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .block_user(user_id, target_id, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unblock_user(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(target_id): Path<UserId>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .unblock_user(user_id, target_id, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn report_user(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(target_id): Path<UserId>,
    Json(request): Json<ReportUserRequest>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let report_id = state
        .service
        .report_user(user_id, target_id, request, Utc::now())
        .await?;
    Ok((StatusCode::CREATED, Json(json!({ "report_id": report_id }))))
}

async fn friend_requests(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Json<Vec<iamrust_domain::FriendRequest>> {
    Json(state.service.friend_requests(user_id).await)
}

async fn create_friend_request(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<FriendRequestCreate>,
) -> Result<(StatusCode, Json<iamrust_domain::FriendRequest>), AppError> {
    let request = state
        .service
        .send_friend_request(user_id, &request.username, request.message, Utc::now())
        .await?;
    Ok((StatusCode::CREATED, Json(request)))
}

async fn decide_friend_request(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(request_id): Path<FriendRequestId>,
    Json(request): Json<FriendRequestDecisionBody>,
) -> Result<Json<iamrust_domain::FriendRequest>, AppError> {
    Ok(Json(
        state
            .service
            .decide_friend_request(user_id, request_id, request.decision, Utc::now())
            .await?,
    ))
}

async fn conversations(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Json<Vec<iamrust_domain::Conversation>> {
    Json(state.service.conversations(user_id).await)
}

async fn conversation_settings(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
) -> Result<Json<iamrust_protocol::ConversationSettings>, AppError> {
    Ok(Json(
        state
            .service
            .conversation_settings(user_id, conversation_id)
            .await?,
    ))
}

async fn update_conversation_settings(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
    Json(request): Json<UpdateConversationSettingsRequest>,
) -> Result<Json<iamrust_protocol::ConversationSettings>, AppError> {
    Ok(Json(
        state
            .service
            .update_conversation_settings(user_id, conversation_id, request, Utc::now())
            .await?,
    ))
}

async fn mark_all_read(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Result<StatusCode, AppError> {
    state.service.mark_all_read(user_id, Utc::now()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_direct(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<CreateDirectConversationRequest>,
) -> Result<(StatusCode, Json<iamrust_domain::Conversation>), AppError> {
    let conversation = state
        .service
        .create_direct(user_id, request.peer_user_id, Utc::now())
        .await?;
    Ok((StatusCode::CREATED, Json(conversation)))
}

async fn create_group(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<CreateGroupRequest>,
) -> Result<(StatusCode, Json<iamrust_domain::Conversation>), AppError> {
    let conversation = state
        .service
        .create_group(user_id, request.member_ids, request.name, Utc::now())
        .await?;
    Ok((StatusCode::CREATED, Json(conversation)))
}

async fn update_group(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
    Json(request): Json<UpdateGroupRequest>,
) -> Result<Json<iamrust_domain::Conversation>, AppError> {
    Ok(Json(
        state
            .service
            .update_group(user_id, conversation_id, request, Utc::now())
            .await?,
    ))
}

async fn group_settings(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
) -> Result<Json<GroupSettingsResponse>, AppError> {
    Ok(Json(GroupSettingsResponse {
        mute_all: state
            .service
            .group_mute_status(user_id, conversation_id)
            .await?,
    }))
}

async fn add_group_members(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
    Json(request): Json<AddGroupMembersRequest>,
) -> Result<Json<iamrust_domain::Conversation>, AppError> {
    Ok(Json(
        state
            .service
            .add_group_members(user_id, conversation_id, request, Utc::now())
            .await?,
    ))
}

async fn update_group_member(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path((conversation_id, member_id)): Path<(ConversationId, UserId)>,
    Json(request): Json<UpdateGroupMemberRequest>,
) -> Result<Json<iamrust_domain::Conversation>, AppError> {
    Ok(Json(
        state
            .service
            .update_group_member(user_id, conversation_id, member_id, request, Utc::now())
            .await?,
    ))
}

async fn remove_group_member(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path((conversation_id, member_id)): Path<(ConversationId, UserId)>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .remove_group_member(user_id, conversation_id, member_id, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn leave_group(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .leave_group(user_id, conversation_id, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn disband_group(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .disband_group(user_id, conversation_id, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn transfer_group_ownership(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
    Json(request): Json<TransferGroupOwnershipRequest>,
) -> Result<Json<iamrust_domain::Conversation>, AppError> {
    Ok(Json(
        state
            .service
            .transfer_group_ownership(user_id, conversation_id, request, Utc::now())
            .await?,
    ))
}

async fn set_group_mute(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
    Json(request): Json<GroupMuteRequest>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .set_group_mute(user_id, conversation_id, request, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn group_announcements(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
) -> Result<Json<Vec<iamrust_protocol::GroupAnnouncement>>, AppError> {
    Ok(Json(
        state
            .service
            .group_announcements(user_id, conversation_id)
            .await?,
    ))
}

async fn group_files(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
) -> Result<Json<Vec<GroupFileItem>>, AppError> {
    Ok(Json(
        state.service.group_files(user_id, conversation_id).await?,
    ))
}

async fn create_group_announcement(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
    Json(request): Json<CreateGroupAnnouncementRequest>,
) -> Result<(StatusCode, Json<iamrust_protocol::GroupAnnouncement>), AppError> {
    let announcement = state
        .service
        .create_group_announcement(user_id, conversation_id, request, Utc::now())
        .await?;
    Ok((StatusCode::CREATED, Json(announcement)))
}

async fn read_group_announcement(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(announcement_id): Path<uuid::Uuid>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .read_group_announcement(user_id, announcement_id, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn group_join_requests(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
) -> Result<Json<Vec<iamrust_protocol::GroupJoinRequest>>, AppError> {
    Ok(Json(
        state
            .service
            .group_join_requests(user_id, conversation_id)
            .await?,
    ))
}

async fn request_to_join_group(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
    Json(request): Json<CreateGroupJoinRequest>,
) -> Result<(StatusCode, Json<iamrust_protocol::GroupJoinRequest>), AppError> {
    let request = state
        .service
        .request_to_join_group(user_id, conversation_id, request, Utc::now())
        .await?;
    Ok((StatusCode::CREATED, Json(request)))
}

async fn decide_group_join_request(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(request_id): Path<uuid::Uuid>,
    Json(request): Json<DecideGroupJoinRequest>,
) -> Result<Json<iamrust_protocol::GroupJoinRequest>, AppError> {
    Ok(Json(
        state
            .service
            .decide_group_join_request(user_id, request_id, request, Utc::now())
            .await?,
    ))
}

async fn group_polls(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
) -> Result<Json<Vec<iamrust_protocol::GroupPoll>>, AppError> {
    Ok(Json(
        state.service.group_polls(user_id, conversation_id).await?,
    ))
}

async fn create_group_poll(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
    Json(request): Json<CreateGroupPollRequest>,
) -> Result<(StatusCode, Json<iamrust_protocol::GroupPoll>), AppError> {
    let poll = state
        .service
        .create_group_poll(user_id, conversation_id, request, Utc::now())
        .await?;
    Ok((StatusCode::CREATED, Json(poll)))
}

async fn vote_group_poll(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(poll_id): Path<uuid::Uuid>,
    Json(request): Json<VoteGroupPollRequest>,
) -> Result<Json<iamrust_protocol::GroupPoll>, AppError> {
    Ok(Json(
        state
            .service
            .vote_group_poll(user_id, poll_id, request, Utc::now())
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct MessageQuery {
    before: Option<u64>,
    limit: Option<usize>,
}

async fn messages(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
    Query(query): Query<MessageQuery>,
) -> Result<Json<Page<iamrust_domain::Message>>, AppError> {
    let messages = state
        .service
        .messages(
            user_id,
            conversation_id,
            query.before,
            query.limit.unwrap_or(50),
        )
        .await?;
    let next_cursor = messages
        .first()
        .and_then(|message| message.sequence)
        .map(|sequence| sequence.to_string());
    Ok(Json(Page {
        items: messages,
        next_cursor,
    }))
}

async fn send_message(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
    Json(request): Json<SendMessageRequest>,
) -> Result<(StatusCode, Json<iamrust_protocol::MessageAck>), AppError> {
    let started = Instant::now();
    let result = state
        .service
        .send_message_request(user_id, conversation_id, request, Utc::now())
        .await;
    let (_, ack) = match result {
        Ok(value) => value,
        Err(error) => {
            state
                .metrics
                .message_failures
                .fetch_add(1, Ordering::Relaxed);
            return Err(error.into());
        }
    };
    state.metrics.messages_sent.fetch_add(1, Ordering::Relaxed);
    state
        .metrics
        .message_ack_latency_micros
        .fetch_add(micros_u64(started.elapsed()), Ordering::Relaxed);
    state
        .metrics
        .message_ack_latency_samples
        .fetch_add(1, Ordering::Relaxed);
    Ok((StatusCode::CREATED, Json(ack)))
}

async fn message_details(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(message_id): Path<iamrust_domain::MessageId>,
) -> Result<Json<iamrust_protocol::MessageDetails>, AppError> {
    Ok(Json(
        state.service.message_details(user_id, message_id).await?,
    ))
}

async fn translate_message(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    CurrentUser(user_id, _): CurrentUser,
    Path(message_id): Path<iamrust_domain::MessageId>,
    Json(request): Json<TranslateMessageRequest>,
) -> Result<Json<TranslateMessageResponse>, AppError> {
    rate_limit(&state, address.ip(), "translation", 30).await?;
    let target_language = request.target_language.trim().to_ascii_lowercase();
    if !(2..=16).contains(&target_language.len())
        || !target_language
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || byte == b'-')
    {
        return Err(AppError::validation("target_language"));
    }
    let text = state
        .service
        .message_text_for_translation(user_id, message_id)
        .await?;
    let translator = state.translator.as_ref().ok_or_else(|| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            iamrust_protocol::ErrorCode::ServiceUnavailable,
            "error.translation_unavailable",
            None,
            true,
        )
    })?;
    let (translated_text, source_language) = translator
        .translate(&text, &target_language)
        .await
        .map_err(|_| {
            AppError::new(
                StatusCode::BAD_GATEWAY,
                iamrust_protocol::ErrorCode::ServiceUnavailable,
                "error.translation_failed",
                None,
                true,
            )
        })?;
    Ok(Json(TranslateMessageResponse {
        source_language,
        target_language,
        translated_text,
    }))
}

async fn transcribe_message(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    CurrentUser(user_id, _): CurrentUser,
    Path(message_id): Path<iamrust_domain::MessageId>,
) -> Result<Json<TranscribeMessageResponse>, AppError> {
    rate_limit(&state, address.ip(), "transcription", 15).await?;
    let attachment = state
        .service
        .message_audio_for_transcription(user_id, message_id)
        .await?;
    let object_store = state.object_store.as_ref().ok_or_else(|| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            iamrust_protocol::ErrorCode::ServiceUnavailable,
            "error.object_store_unavailable",
            None,
            true,
        )
    })?;
    let transcriber = state.transcriber.as_ref().ok_or_else(|| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            iamrust_protocol::ErrorCode::ServiceUnavailable,
            "error.transcription_unavailable",
            None,
            true,
        )
    })?;
    let bytes = object_store
        .read_object(&attachment.storage_key, 25 * 1024 * 1024, Utc::now())
        .await
        .map_err(|_| {
            AppError::new(
                StatusCode::BAD_GATEWAY,
                iamrust_protocol::ErrorCode::ServiceUnavailable,
                "error.audio_download_failed",
                None,
                true,
            )
        })?;
    let (text, language) = transcriber
        .transcribe(bytes, &attachment.file_name, &attachment.mime_type)
        .await
        .map_err(|_| {
            AppError::new(
                StatusCode::BAD_GATEWAY,
                iamrust_protocol::ErrorCode::ServiceUnavailable,
                "error.transcription_failed",
                None,
                true,
            )
        })?;
    Ok(Json(TranscribeMessageResponse { text, language }))
}

async fn acknowledge_delivery(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(message_id): Path<iamrust_domain::MessageId>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .acknowledge_delivery(user_id, message_id, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn recall_message(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(message_id): Path<iamrust_domain::MessageId>,
) -> Result<Json<iamrust_domain::Message>, AppError> {
    Ok(Json(
        state
            .service
            .recall_message(user_id, message_id, Utc::now())
            .await?,
    ))
}

async fn react_to_message(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(message_id): Path<iamrust_domain::MessageId>,
    Json(request): Json<MessageReactionRequest>,
) -> Result<Json<Vec<iamrust_protocol::MessageReaction>>, AppError> {
    Ok(Json(
        state
            .service
            .react_to_message(
                user_id,
                message_id,
                request.emoji,
                request.active,
                Utc::now(),
            )
            .await?,
    ))
}

async fn set_message_favorite(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(message_id): Path<iamrust_domain::MessageId>,
    Json(request): Json<FavoriteMessageRequest>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .set_message_favorite(user_id, message_id, request.favorite)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn favorite_messages(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Json<Vec<iamrust_domain::Message>> {
    Json(state.service.favorite_messages(user_id).await)
}

async fn forward_messages(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<ForwardMessagesRequest>,
) -> Result<(StatusCode, Json<Vec<iamrust_domain::Message>>), AppError> {
    let messages = state
        .service
        .forward_messages(
            user_id,
            request.message_ids,
            request.target_conversation_id,
            request.mode,
            Utc::now(),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(messages)))
}

async fn schedule_message(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<ScheduleMessageRequest>,
) -> Result<(StatusCode, Json<iamrust_protocol::ScheduledMessageResponse>), AppError> {
    let scheduled = state
        .service
        .schedule_message(user_id, request, Utc::now())
        .await?;
    Ok((StatusCode::ACCEPTED, Json(scheduled)))
}

async fn scheduled_messages(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Json<Vec<iamrust_protocol::ScheduledMessageInfo>> {
    Json(state.service.scheduled_messages(user_id).await)
}

async fn cancel_scheduled_message(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(schedule_id): Path<uuid::Uuid>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .cancel_scheduled_message(user_id, schedule_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn mark_read(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(conversation_id): Path<ConversationId>,
    Json(request): Json<MarkReadRequest>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .mark_read(
            user_id,
            conversation_id,
            request.through_sequence,
            Utc::now(),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct SyncQuery {
    after: Option<u64>,
    limit: Option<usize>,
}

async fn sync(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Query(query): Query<SyncQuery>,
) -> Json<iamrust_protocol::SyncResponse> {
    let response = state
        .service
        .sync(
            user_id,
            query.after.unwrap_or_default(),
            query.limit.unwrap_or(200),
        )
        .await;
    let latest = state.service.latest_cursor().await;
    state.metrics.sync_backlog_events.store(
        latest.saturating_sub(response.next_cursor),
        Ordering::Relaxed,
    );
    Json(response)
}

async fn authorize_upload(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<UploadAuthorizationRequest>,
) -> Result<Json<UploadAuthorizationResponse>, AppError> {
    rate_limit(&state, address.ip(), "upload", 30).await?;
    let authorization = state
        .service
        .authorize_attachment(
            user_id,
            request.file_name,
            request.mime_type,
            request.byte_size,
            request.sha256,
            Utc::now(),
        )
        .await?;
    let object_store = state.object_store.as_ref().ok_or_else(|| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            iamrust_protocol::ErrorCode::ServiceUnavailable,
            "error.object_store_unavailable",
            None,
            true,
        )
    })?;
    let upload = object_store
        .presign_put(
            &authorization.attachment.storage_key,
            &authorization.attachment.mime_type,
            authorization.attachment.sha256.as_deref(),
            Utc::now(),
        )
        .map_err(|_| {
            AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                iamrust_protocol::ErrorCode::ServiceUnavailable,
                "error.object_store_unavailable",
                None,
                true,
            )
        })?;
    Ok(Json(UploadAuthorizationResponse {
        attachment_id: authorization.attachment.id,
        storage_key: authorization.attachment.storage_key,
        upload_url: upload.url,
        expires_at: authorization.expires_at,
        required_headers: upload.required_headers,
    }))
}

async fn complete_upload(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Json(request): Json<CompleteUploadRequest>,
) -> Result<Json<CompleteUploadResponse>, AppError> {
    let now = Utc::now();
    let pending = state
        .service
        .pending_attachment(user_id, request.attachment_id, now)
        .await?;
    let object_store = state.object_store.as_ref().ok_or_else(|| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            iamrust_protocol::ErrorCode::ServiceUnavailable,
            "error.object_store_unavailable",
            None,
            true,
        )
    })?;
    object_store
        .verify_object(
            &pending.storage_key,
            &pending.mime_type,
            pending.byte_size,
            pending.sha256.as_deref(),
            now,
        )
        .await
        .map_err(|_| {
            AppError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                iamrust_protocol::ErrorCode::UnsupportedMediaType,
                "error.upload_verification_failed",
                Some("attachment_id".to_owned()),
                false,
            )
        })?;
    if let Some(scanner) = &state.malware_scanner {
        let bytes = object_store
            .read_object(&pending.storage_key, pending.byte_size, now)
            .await
            .map_err(|_| {
                AppError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    iamrust_protocol::ErrorCode::ServiceUnavailable,
                    "error.malware_scan_download_failed",
                    None,
                    true,
                )
            })?;
        match scanner.scan(&bytes).await {
            Ok(ScanVerdict::Clean) => {}
            Ok(ScanVerdict::Infected { signature }) => {
                state
                    .service
                    .quarantine_attachment(user_id, request.attachment_id, now)
                    .await?;
                state
                    .metrics
                    .quarantined_files
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!(attachment_id = %request.attachment_id, signature, "attachment quarantined");
                return Err(AppError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    iamrust_protocol::ErrorCode::UnsupportedMediaType,
                    "error.malware_detected",
                    Some("attachment_id".to_owned()),
                    false,
                ));
            }
            Err(error) => {
                tracing::warn!(attachment_id = %request.attachment_id, error = %error, "malware scan failed");
                return Err(AppError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    iamrust_protocol::ErrorCode::ServiceUnavailable,
                    "error.malware_scanner_unavailable",
                    None,
                    true,
                ));
            }
        }
    }
    let attachment = state
        .service
        .complete_attachment(user_id, request.attachment_id, now)
        .await?;
    let download = object_store
        .presign_get(&attachment.storage_key, Utc::now())
        .map_err(|_| {
            AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                iamrust_protocol::ErrorCode::ServiceUnavailable,
                "error.object_store_unavailable",
                None,
                true,
            )
        })?;
    Ok(Json(CompleteUploadResponse {
        attachment,
        download_url: download.url,
    }))
}

async fn authorize_download(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
    Path(attachment_id): Path<AttachmentId>,
) -> Result<Json<DownloadAuthorizationResponse>, AppError> {
    let attachment = state
        .service
        .attachment_for_download(user_id, attachment_id)
        .await?;
    let now = Utc::now();
    let download = state
        .object_store
        .as_ref()
        .ok_or_else(|| {
            AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                iamrust_protocol::ErrorCode::ServiceUnavailable,
                "error.object_store_unavailable",
                None,
                true,
            )
        })?
        .presign_get(&attachment.storage_key, now)
        .map_err(|_| {
            AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                iamrust_protocol::ErrorCode::ServiceUnavailable,
                "error.object_store_unavailable",
                None,
                true,
            )
        })?;
    Ok(Json(DownloadAuthorizationResponse {
        download_url: download.url,
        expires_at: now + chrono::Duration::minutes(10),
        attachment,
    }))
}

async fn admin_set_user_suspension(
    State(state): State<AppState>,
    _admin: AdminAccess,
    Path(user_id): Path<UserId>,
    Json(request): Json<AdminSuspendUserRequest>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .admin_set_user_suspended(user_id, request.suspended, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_revoke_user_sessions(
    State(state): State<AppState>,
    _admin: AdminAccess,
    Path(user_id): Path<UserId>,
) -> Result<StatusCode, AppError> {
    state
        .service
        .admin_revoke_user_sessions(user_id, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct AdminAuditQuery {
    limit: Option<usize>,
}

async fn admin_audit(
    State(state): State<AppState>,
    _admin: AdminAccess,
    Query(query): Query<AdminAuditQuery>,
) -> Json<Vec<AdminAuditEntry>> {
    Json(state.service.admin_audit(query.limit.unwrap_or(100)).await)
}

async fn websocket_ticket(
    State(state): State<AppState>,
    CurrentUser(user_id, _): CurrentUser,
) -> Json<WebSocketTicketResponse> {
    let ticket = uuid::Uuid::new_v4().simple().to_string();
    let ttl = Duration::from_secs(30);
    let mut tickets = state.websocket_tickets.lock().await;
    tickets.retain(|_, value| value.expires_at > Instant::now());
    tickets.insert(
        ticket.clone(),
        WebSocketTicket {
            user_id,
            expires_at: Instant::now() + ttl,
        },
    );
    Json(WebSocketTicketResponse {
        ticket,
        expires_at: Utc::now() + chrono::Duration::seconds(30),
    })
}

#[derive(Debug, Deserialize)]
struct WebSocketQuery {
    ticket: String,
}

async fn websocket_upgrade(
    State(state): State<AppState>,
    Query(query): Query<WebSocketQuery>,
    websocket: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let ticket = state
        .websocket_tickets
        .lock()
        .await
        .remove(&query.ticket)
        .filter(|value| value.expires_at > Instant::now())
        .ok_or_else(AppError::unauthorized)?;
    state
        .metrics
        .websocket_connections_total
        .fetch_add(1, Ordering::Relaxed);
    state
        .metrics
        .websocket_connections_active
        .fetch_add(1, Ordering::Relaxed);
    let service = state.service.clone();
    let metrics = state.metrics.clone();
    let connection_id = uuid::Uuid::now_v7();
    Ok(websocket
        .max_message_size(256 * 1024)
        .on_upgrade(move |socket| async move {
            async move {
                websocket::serve(socket, service, ticket.user_id).await;
                metrics
                    .websocket_connections_active
                    .fetch_sub(1, Ordering::Relaxed);
            }
            .instrument(tracing::info_span!(
                "websocket_connection",
                %connection_id,
                user_id = %ticket.user_id
            ))
            .await;
        }))
}

fn session_response(session: AuthenticatedSession) -> SessionResponse {
    SessionResponse {
        access_token: session.access_token,
        refresh_token: session.refresh_token,
        access_expires_at: session.access_expires_at,
        refresh_expires_at: session.refresh_expires_at,
        profile: session.profile,
        device_id: session.device_id,
    }
}

async fn rate_limit(state: &AppState, ip: IpAddr, scope: &str, max: u32) -> Result<(), AppError> {
    state
        .rate_limiter
        .check(format!("{scope}:{ip}"), max, Duration::from_secs(60))
        .await
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::body::to_bytes;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn health_and_openapi_are_available_without_authentication() {
        let app = router(AppState::new(ChatService::new()));
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("I Am Rust API"));
    }

    #[tokio::test]
    async fn protected_route_rejects_missing_token() {
        let app = router(AppState::new(ChatService::new()));
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/me")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn administration_routes_require_the_configured_secret() {
        let app = router(
            AppState::new(ChatService::new())
                .with_admin_token(Some("test-administration-token-32-bytes".to_owned())),
        );
        let request = || {
            axum::http::Request::builder()
                .uri("/api/v1/admin/audit")
                .body(Body::empty())
                .unwrap()
        };
        let response = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/admin/audit")
                    .header("x-admin-token", "test-administration-token-32-bytes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
