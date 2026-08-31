use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use iamrust_protocol::{
    CompleteUploadRequest, DownloadAuthorizationResponse, LoginRequest,
    PasswordResetConfirmRequest, PasswordResetRequest, QrLoginPollResponse, QrLoginSecretRequest,
    QrLoginStartRequest, RegisterRequest, SessionResponse, UploadAuthorizationRequest,
    UploadAuthorizationResponse,
};
use reqwest::{Method, StatusCode, header};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State, ipc::InvokeBody};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};
use url::Url;

const CREDENTIAL_SERVICE: &str = "app.iamrust.desktop";
const CREDENTIAL_ACCOUNT: &str = "refresh-token";
const MAX_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_UPLOAD_BYTES: usize = 100 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Default)]
struct SessionTokens {
    access: Option<String>,
    refresh: Option<String>,
}

pub(crate) struct RemoteApi {
    base_url: Url,
    client: reqwest::Client,
    session: RwLock<SessionTokens>,
    refresh_gate: Mutex<()>,
}

impl fmt::Debug for RemoteApi {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteApi")
            .field("base_url", &self.base_url)
            .field("credentials", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct RemoteResponse {
    status: u16,
    body: Value,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    attachment_id: String,
    received: u64,
    total: u64,
    percent: u8,
}

impl RemoteResponse {
    fn success(status: StatusCode, body: Value) -> Self {
        Self {
            status: status.as_u16(),
            body,
        }
    }

    fn error(status: u16, code: &str, message_key: &str, retryable: bool) -> Self {
        Self {
            status,
            body: json!({
                "code": code,
                "message_key": message_key,
                "field": null,
                "retryable": retryable,
            }),
        }
    }

    fn offline() -> Self {
        Self::error(0, "offline", "network_unavailable", true)
    }

    fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

impl RemoteApi {
    pub(crate) fn new() -> anyhow::Result<Self> {
        let configured =
            std::env::var("IAMRUST_API_URL").unwrap_or_else(|_| "http://127.0.0.1:3780".to_owned());
        let mut base_url = Url::parse(&configured)?;
        anyhow::ensure!(
            matches!(base_url.scheme(), "http" | "https"),
            "IAMRUST_API_URL must use HTTP or HTTPS"
        );
        let loopback = base_url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse()
                    .is_ok_and(|ip: std::net::IpAddr| ip.is_loopback())
        });
        anyhow::ensure!(
            base_url.scheme() == "https" || loopback,
            "non-loopback API endpoints must use HTTPS"
        );
        base_url.set_query(None);
        base_url.set_fragment(None);
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("I-Am-Rust/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            base_url,
            client,
            session: RwLock::new(SessionTokens::default()),
            refresh_gate: Mutex::new(()),
        })
    }

    async fn login(&self, request: LoginRequest) -> RemoteResponse {
        self.authenticate("/api/v1/auth/login", request).await
    }

    async fn register(&self, request: RegisterRequest) -> RemoteResponse {
        self.authenticate("/api/v1/auth/register", request).await
    }

    async fn begin_qr_login(&self, request: QrLoginStartRequest) -> RemoteResponse {
        self.send(
            Method::POST,
            "/api/v1/auth/qr-login",
            Some(json!(request)),
            None,
        )
        .await
    }

    async fn poll_qr_login(
        &self,
        challenge_id: uuid::Uuid,
        request: QrLoginSecretRequest,
    ) -> RemoteResponse {
        let response = self
            .send(
                Method::POST,
                &format!("/api/v1/auth/qr-login/{challenge_id}/poll"),
                Some(json!(request)),
                None,
            )
            .await;
        if !response.is_success() {
            return response;
        }
        let Ok(mut poll) = serde_json::from_value::<QrLoginPollResponse>(response.body) else {
            return RemoteResponse::error(502, "invalid_response", "server_response_invalid", true);
        };
        let Some(session) = poll.session.take() else {
            return RemoteResponse::success(StatusCode::OK, json!(poll));
        };
        match self.accept_session(session).await {
            Ok(public) => RemoteResponse::success(
                StatusCode::OK,
                json!({ "status": poll.status, "session": public }),
            ),
            Err(response) => response,
        }
    }

    async fn authenticate<T: Serialize>(&self, path: &str, request: T) -> RemoteResponse {
        let response = self
            .send(Method::POST, path, Some(json!(request)), None)
            .await;
        if !response.is_success() {
            return response;
        }
        let Ok(session) = serde_json::from_value::<SessionResponse>(response.body) else {
            return RemoteResponse::error(502, "invalid_response", "server_response_invalid", true);
        };
        match self.accept_session(session).await {
            Ok(public) => RemoteResponse::success(StatusCode::OK, public),
            Err(response) => response,
        }
    }

    async fn restore(&self) -> RemoteResponse {
        let token = match load_credential().await {
            Ok(Some(token)) => token,
            Ok(None) => return RemoteResponse::success(StatusCode::NO_CONTENT, Value::Null),
            Err(()) => {
                return RemoteResponse::error(
                    0,
                    "credential_store_unavailable",
                    "credential_store_unavailable",
                    false,
                );
            }
        };
        self.session.write().await.refresh = Some(token);
        let _guard = self.refresh_gate.lock().await;
        match self.refresh_session().await {
            Ok(session) => RemoteResponse::success(StatusCode::OK, public_session(&session)),
            Err(response) => {
                if response.status == StatusCode::UNAUTHORIZED.as_u16() {
                    self.clear_session().await;
                }
                response
            }
        }
    }

    async fn logout(&self) -> RemoteResponse {
        let refresh = self.session.read().await.refresh.clone();
        if let Some(refresh_token) = refresh {
            let _ = self
                .send(
                    Method::POST,
                    "/api/v1/auth/logout",
                    Some(json!({ "refresh_token": refresh_token })),
                    None,
                )
                .await;
        }
        self.clear_session().await;
        RemoteResponse::success(StatusCode::NO_CONTENT, Value::Null)
    }

    async fn clear_session(&self) {
        *self.session.write().await = SessionTokens::default();
        let _ = clear_credential().await;
    }

    async fn accept_session(&self, session: SessionResponse) -> Result<Value, RemoteResponse> {
        if save_credential(session.refresh_token.clone())
            .await
            .is_err()
        {
            *self.session.write().await = SessionTokens::default();
            return Err(RemoteResponse::error(
                0,
                "credential_store_unavailable",
                "credential_store_unavailable",
                false,
            ));
        }
        *self.session.write().await = SessionTokens {
            access: Some(session.access_token.clone()),
            refresh: Some(session.refresh_token.clone()),
        };
        Ok(public_session(&session))
    }

    async fn refresh_session(&self) -> Result<SessionResponse, RemoteResponse> {
        let refresh_token = self.session.read().await.refresh.clone().ok_or_else(|| {
            RemoteResponse::error(
                401,
                "authentication_required",
                "authentication_required",
                false,
            )
        })?;
        let response = self
            .send(
                Method::POST,
                "/api/v1/auth/refresh",
                Some(json!({ "refresh_token": refresh_token })),
                None,
            )
            .await;
        if !response.is_success() {
            return Err(response);
        }
        let session = serde_json::from_value::<SessionResponse>(response.body).map_err(|_| {
            RemoteResponse::error(502, "invalid_response", "server_response_invalid", true)
        })?;
        self.accept_session(session.clone()).await?;
        Ok(session)
    }

    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> RemoteResponse {
        if !is_allowed_api_path(path) {
            return RemoteResponse::error(400, "invalid_request", "api_path_not_allowed", false);
        }
        let access = self.session.read().await.access.clone();
        let Some(access) = access else {
            return RemoteResponse::error(
                401,
                "authentication_required",
                "authentication_required",
                false,
            );
        };
        let response = self
            .send(method.clone(), path, body.clone(), Some(&access))
            .await;
        if response.status != StatusCode::UNAUTHORIZED.as_u16() {
            return response;
        }

        let _guard = self.refresh_gate.lock().await;
        let latest_access = self.session.read().await.access.clone();
        if latest_access.as_deref() == Some(access.as_str())
            && let Err(response) = self.refresh_session().await
        {
            if response.status == StatusCode::UNAUTHORIZED.as_u16() {
                self.clear_session().await;
            }
            return response;
        }
        let Some(access) = self.session.read().await.access.clone() else {
            return RemoteResponse::error(
                401,
                "authentication_required",
                "authentication_required",
                false,
            );
        };
        self.send(method, path, body, Some(&access)).await
    }

    async fn upload(&self, file_name: String, mime_type: String, bytes: Vec<u8>) -> RemoteResponse {
        if bytes.is_empty() || bytes.len() > MAX_UPLOAD_BYTES {
            return RemoteResponse::error(413, "payload_too_large", "upload_size_invalid", false);
        }
        let sha256 = hex(&Sha256::digest(&bytes));
        let authorization = self
            .request(
                Method::POST,
                "/api/v1/uploads/authorize",
                Some(json!(UploadAuthorizationRequest {
                    file_name,
                    mime_type,
                    byte_size: bytes.len() as u64,
                    sha256: Some(sha256),
                })),
            )
            .await;
        if !authorization.is_success() {
            return authorization;
        }
        let Ok(authorization) =
            serde_json::from_value::<UploadAuthorizationResponse>(authorization.body)
        else {
            return RemoteResponse::error(502, "invalid_response", "server_response_invalid", true);
        };
        let Ok(upload_url) = Url::parse(&authorization.upload_url) else {
            return RemoteResponse::error(502, "invalid_response", "upload_url_invalid", true);
        };
        if !is_safe_remote_url(&upload_url) {
            return RemoteResponse::error(502, "invalid_response", "upload_url_invalid", true);
        }
        let mut upload = self.client.put(upload_url).body(bytes);
        for (name, value) in authorization.required_headers {
            let Ok(name) = header::HeaderName::try_from(name) else {
                return RemoteResponse::error(
                    502,
                    "invalid_response",
                    "upload_header_invalid",
                    true,
                );
            };
            let Ok(value) = header::HeaderValue::try_from(value) else {
                return RemoteResponse::error(
                    502,
                    "invalid_response",
                    "upload_header_invalid",
                    true,
                );
            };
            upload = upload.header(name, value);
        }
        match upload.send().await {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => {
                return RemoteResponse::error(
                    response.status().as_u16(),
                    "upload_failed",
                    "upload_failed",
                    response.status().is_server_error(),
                );
            }
            Err(_) => return RemoteResponse::offline(),
        }
        self.request(
            Method::POST,
            "/api/v1/uploads/complete",
            Some(json!(CompleteUploadRequest {
                attachment_id: authorization.attachment_id,
            })),
        )
        .await
    }

    async fn download(
        &self,
        attachment_id: &str,
        directory: &Path,
        app: &AppHandle,
    ) -> RemoteResponse {
        let Ok(parsed_id) = attachment_id
            .parse::<uuid::Uuid>()
            .map(iamrust_domain::AttachmentId::from_uuid)
        else {
            return RemoteResponse::error(400, "invalid_request", "attachment_id_invalid", false);
        };
        let authorization = self
            .request(
                Method::GET,
                &format!("/api/v1/attachments/{parsed_id}/download"),
                None,
            )
            .await;
        if !authorization.is_success() {
            return authorization;
        }
        let Ok(authorization) =
            serde_json::from_value::<DownloadAuthorizationResponse>(authorization.body)
        else {
            return RemoteResponse::error(502, "invalid_response", "server_response_invalid", true);
        };
        if authorization.attachment.id != parsed_id
            || authorization.attachment.byte_size == 0
            || authorization.attachment.byte_size > MAX_DOWNLOAD_BYTES
        {
            return RemoteResponse::error(
                502,
                "invalid_response",
                "download_metadata_invalid",
                false,
            );
        }
        let Ok(download_url) = Url::parse(&authorization.download_url) else {
            return RemoteResponse::error(502, "invalid_response", "download_url_invalid", true);
        };
        if !is_safe_remote_url(&download_url) {
            return RemoteResponse::error(502, "invalid_response", "download_url_invalid", true);
        }
        let file_name = safe_file_name(
            &authorization.attachment.file_name,
            &authorization.attachment.id.to_string(),
        );
        let destination = available_destination(directory, &file_name).await;
        let temporary = directory.join(format!(
            ".iamrust-{}-{}.part",
            authorization.attachment.id,
            uuid::Uuid::now_v7()
        ));
        let result = self
            .download_to_file(
                download_url,
                &temporary,
                authorization.attachment.byte_size,
                authorization.attachment.sha256.as_deref(),
                authorization.attachment.id.to_string(),
                app,
            )
            .await;
        if let Err(response) = result {
            let _ = tokio::fs::remove_file(&temporary).await;
            return response;
        }
        if tokio::fs::rename(&temporary, &destination).await.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
            return RemoteResponse::error(0, "download_failed", "download_move_failed", true);
        }
        RemoteResponse::success(
            StatusCode::OK,
            json!({
                "path": destination.to_string_lossy(),
                "file_name": file_name,
                "byte_size": authorization.attachment.byte_size,
            }),
        )
    }

    async fn download_to_file(
        &self,
        url: Url,
        temporary: &Path,
        expected_size: u64,
        expected_sha256: Option<&str>,
        attachment_id: String,
        app: &AppHandle,
    ) -> Result<(), RemoteResponse> {
        let mut response = self
            .client
            .get(url)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|_| RemoteResponse::offline())?;
        if !response.status().is_success() {
            return Err(RemoteResponse::error(
                response.status().as_u16(),
                "download_failed",
                "download_failed",
                response.status().is_server_error(),
            ));
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temporary)
            .await
            .map_err(|_| {
                RemoteResponse::error(0, "download_failed", "download_file_create_failed", false)
            })?;
        let mut received = 0_u64;
        let mut hasher = Sha256::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| RemoteResponse::offline())?
        {
            received = received.saturating_add(chunk.len() as u64);
            if received > expected_size || received > MAX_DOWNLOAD_BYTES {
                return Err(RemoteResponse::error(
                    502,
                    "download_failed",
                    "download_size_mismatch",
                    false,
                ));
            }
            file.write_all(&chunk).await.map_err(|_| {
                RemoteResponse::error(0, "download_failed", "download_write_failed", true)
            })?;
            hasher.update(&chunk);
            let percent = ((received.saturating_mul(100) / expected_size).min(100)) as u8;
            let _ = app.emit(
                "download-progress",
                DownloadProgress {
                    attachment_id: attachment_id.clone(),
                    received,
                    total: expected_size,
                    percent,
                },
            );
        }
        if received != expected_size
            || expected_sha256
                .is_some_and(|expected| !hex(&hasher.finalize()).eq_ignore_ascii_case(expected))
        {
            return Err(RemoteResponse::error(
                502,
                "download_failed",
                "download_integrity_failed",
                false,
            ));
        }
        file.sync_all()
            .await
            .map_err(|_| RemoteResponse::error(0, "download_failed", "download_write_failed", true))
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        access_token: Option<&str>,
    ) -> RemoteResponse {
        let Ok(url) = self.base_url.join(path.trim_start_matches('/')) else {
            return RemoteResponse::error(400, "invalid_request", "api_path_invalid", false);
        };
        if url.origin() != self.base_url.origin() {
            return RemoteResponse::error(400, "invalid_request", "api_path_invalid", false);
        }
        let mut request = self
            .client
            .request(method, url)
            .header(header::ACCEPT, "application/json");
        if let Some(access_token) = access_token {
            request = request.bearer_auth(access_token);
        }
        if let Some(body) = body {
            let Ok(body) = serde_json::to_vec(&body) else {
                return RemoteResponse::error(
                    400,
                    "invalid_request",
                    "request_body_invalid",
                    false,
                );
            };
            request = request
                .header(header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let Ok(response) = request.send().await else {
            return RemoteResponse::offline();
        };
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE_BYTES)
        {
            return RemoteResponse::error(502, "invalid_response", "response_too_large", true);
        }
        let bytes = match response.bytes().await {
            Ok(bytes) if bytes.len() as u64 <= MAX_RESPONSE_BYTES => bytes,
            Ok(_) => {
                return RemoteResponse::error(502, "invalid_response", "response_too_large", true);
            }
            Err(_) => return RemoteResponse::offline(),
        };
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                json!({
                    "code": "invalid_response",
                    "message_key": "server_response_invalid",
                    "field": null,
                    "retryable": status.is_server_error(),
                })
            })
        };
        RemoteResponse::success(status, body)
    }
}

#[tauri::command]
pub(crate) async fn remote_login(
    state: State<'_, RemoteApi>,
    request: LoginRequest,
) -> Result<RemoteResponse, String> {
    Ok(state.login(request).await)
}

#[tauri::command]
pub(crate) async fn remote_register(
    state: State<'_, RemoteApi>,
    request: RegisterRequest,
) -> Result<RemoteResponse, String> {
    Ok(state.register(request).await)
}

#[tauri::command]
pub(crate) async fn remote_begin_qr_login(
    state: State<'_, RemoteApi>,
    request: QrLoginStartRequest,
) -> Result<RemoteResponse, String> {
    Ok(state.begin_qr_login(request).await)
}

#[tauri::command]
pub(crate) async fn remote_poll_qr_login(
    state: State<'_, RemoteApi>,
    challenge_id: uuid::Uuid,
    request: QrLoginSecretRequest,
) -> Result<RemoteResponse, String> {
    Ok(state.poll_qr_login(challenge_id, request).await)
}

#[tauri::command]
pub(crate) async fn remote_restore(state: State<'_, RemoteApi>) -> Result<RemoteResponse, String> {
    Ok(state.restore().await)
}

#[tauri::command]
pub(crate) async fn remote_logout(state: State<'_, RemoteApi>) -> Result<RemoteResponse, String> {
    Ok(state.logout().await)
}

#[tauri::command]
pub(crate) async fn remote_request_password_reset(
    state: State<'_, RemoteApi>,
    request: PasswordResetRequest,
) -> Result<RemoteResponse, String> {
    Ok(state
        .send(
            Method::POST,
            "/api/v1/auth/password-reset/request",
            Some(json!(request)),
            None,
        )
        .await)
}

#[tauri::command]
pub(crate) async fn remote_confirm_password_reset(
    state: State<'_, RemoteApi>,
    request: PasswordResetConfirmRequest,
) -> Result<RemoteResponse, String> {
    Ok(state
        .send(
            Method::POST,
            "/api/v1/auth/password-reset/confirm",
            Some(json!(request)),
            None,
        )
        .await)
}

#[tauri::command]
pub(crate) async fn remote_request(
    state: State<'_, RemoteApi>,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<RemoteResponse, String> {
    let Ok(method) = Method::from_bytes(method.as_bytes()) else {
        return Ok(RemoteResponse::error(
            400,
            "invalid_request",
            "http_method_invalid",
            false,
        ));
    };
    if !matches!(
        method,
        Method::GET | Method::POST | Method::PATCH | Method::DELETE
    ) {
        return Ok(RemoteResponse::error(
            405,
            "invalid_request",
            "http_method_not_allowed",
            false,
        ));
    }
    Ok(state.request(method, &path, body).await)
}

#[tauri::command]
pub(crate) async fn remote_upload(
    state: State<'_, RemoteApi>,
    request: tauri::ipc::Request<'_>,
) -> Result<RemoteResponse, String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Ok(RemoteResponse::error(
            400,
            "invalid_request",
            "upload_body_invalid",
            false,
        ));
    };
    let Some(file_name) = request
        .headers()
        .get("x-iamrust-file-name")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| percent_decode(value).ok())
    else {
        return Ok(RemoteResponse::error(
            400,
            "invalid_request",
            "upload_name_invalid",
            false,
        ));
    };
    let Some(mime_type) = request
        .headers()
        .get("x-iamrust-mime-type")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
    else {
        return Ok(RemoteResponse::error(
            400,
            "invalid_request",
            "upload_mime_invalid",
            false,
        ));
    };
    let bytes = bytes.clone();
    Ok(state.upload(file_name, mime_type, bytes).await)
}

#[tauri::command]
pub(crate) async fn remote_download_attachment(
    state: State<'_, RemoteApi>,
    app: AppHandle,
    attachment_id: String,
    directory: Option<String>,
) -> Result<RemoteResponse, String> {
    let Ok(directory) = resolve_download_directory(&app, directory).await else {
        return Ok(RemoteResponse::error(
            400,
            "invalid_request",
            "download_directory_invalid",
            false,
        ));
    };
    Ok(state.download(&attachment_id, &directory, &app).await)
}

#[tauri::command]
pub(crate) async fn reveal_download(
    app: AppHandle,
    path: String,
    directory: Option<String>,
) -> Result<(), String> {
    let directory = resolve_download_directory(&app, directory)
        .await
        .map_err(|()| "download directory invalid".to_owned())?;
    let path = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| "downloaded file does not exist".to_owned())?;
    if !path.starts_with(&directory)
        || !tokio::fs::metadata(&path)
            .await
            .is_ok_and(|metadata| metadata.is_file())
    {
        return Err("downloaded file is outside the configured directory".to_owned());
    }
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = tokio::process::Command::new("open");
        command.arg("-R").arg(&path);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = tokio::process::Command::new("explorer.exe");
        command.arg(format!("/select,{}", path.display()));
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = tokio::process::Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(&directory));
        command
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|_| "failed to reveal downloaded file".to_owned())
}

async fn resolve_download_directory(
    app: &AppHandle,
    directory: Option<String>,
) -> Result<PathBuf, ()> {
    let directory = match directory.filter(|value| !value.trim().is_empty()) {
        Some(value) => PathBuf::from(value),
        None => app.path().download_dir().map_err(|_| ())?,
    };
    if !directory.is_absolute() {
        return Err(());
    }
    let directory = tokio::fs::canonicalize(directory).await.map_err(|_| ())?;
    if !tokio::fs::metadata(&directory)
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return Err(());
    }
    Ok(directory)
}

fn public_session(session: &SessionResponse) -> Value {
    json!({
        "access_expires_at": session.access_expires_at,
        "refresh_expires_at": session.refresh_expires_at,
        "profile": session.profile,
        "device_id": session.device_id,
    })
}

fn is_allowed_api_path(path: &str) -> bool {
    if !path.starts_with("/api/v1/")
        || path.len() > 2_048
        || path.contains("..")
        || path.contains('\\')
        || path.contains('#')
        || path.to_ascii_lowercase().contains("%2e")
        || path.to_ascii_lowercase().contains("%2f")
        || path.to_ascii_lowercase().contains("%5c")
        || path.chars().any(char::is_control)
    {
        return false;
    }
    let route = path
        .trim_start_matches("/api/v1/")
        .split(['/', '?'])
        .next()
        .unwrap_or_default();
    matches!(
        route,
        "attachments"
            | "blocks"
            | "bootstrap"
            | "conversations"
            | "devices"
            | "friend-requests"
            | "friends"
            | "group-announcements"
            | "group-join-requests"
            | "groups"
            | "me"
            | "messages"
            | "polls"
            | "reports"
            | "scheduled-messages"
            | "sync"
            | "uploads"
            | "users"
            | "ws-ticket"
    ) || path == "/api/v1/auth/change-password"
        || (path.starts_with("/api/v1/auth/qr-login/") && path.ends_with("/approve"))
}

fn is_safe_remote_url(url: &Url) -> bool {
    if url.scheme() == "https" {
        return true;
    }
    url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        })
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        },
    )
}

fn percent_decode(value: &str) -> Result<String, ()> {
    let mut bytes = Vec::with_capacity(value.len());
    let raw = value.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%' {
            if index + 2 >= raw.len() {
                return Err(());
            }
            let high = hex_digit(raw[index + 1]).ok_or(())?;
            let low = hex_digit(raw[index + 2]).ok_or(())?;
            bytes.push(high * 16 + low);
            index += 3;
        } else {
            bytes.push(raw[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).map_err(|_| ())
}

fn safe_file_name(value: &str, fallback: &str) -> String {
    let candidate = Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| {
            !name.is_empty()
                && *name != "."
                && *name != ".."
                && name.chars().count() <= 240
                && !name.chars().any(char::is_control)
        });
    candidate.map_or_else(|| format!("attachment-{fallback}"), ToOwned::to_owned)
}

async fn available_destination(directory: &Path, file_name: &str) -> PathBuf {
    let requested = directory.join(file_name);
    if !tokio::fs::try_exists(&requested).await.unwrap_or(true) {
        return requested;
    }
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("attachment");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 1..10_000_u32 {
        let candidate_name = extension.map_or_else(
            || format!("{stem} ({index})"),
            |extension| format!("{stem} ({index}).{extension}"),
        );
        let candidate = directory.join(candidate_name);
        if !tokio::fs::try_exists(&candidate).await.unwrap_or(true) {
            return candidate;
        }
    }
    directory.join(format!("{stem}-{}", uuid::Uuid::now_v7()))
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

async fn save_credential(token: String) -> Result<(), ()> {
    tauri::async_runtime::spawn_blocking(move || {
        keyring::Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT)
            .map_err(|_| ())?
            .set_password(&token)
            .map_err(|_| ())
    })
    .await
    .map_err(|_| ())?
}

async fn load_credential() -> Result<Option<String>, ()> {
    tauri::async_runtime::spawn_blocking(|| {
        let entry = keyring::Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT).map_err(|_| ())?;
        match entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(()),
        }
    })
    .await
    .map_err(|_| ())?
}

async fn clear_credential() -> Result<(), ()> {
    tauri::async_runtime::spawn_blocking(|| {
        let entry = keyring::Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT).map_err(|_| ())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(()),
        }
    })
    .await
    .map_err(|_| ())?
}

#[cfg(test)]
mod tests {
    use super::{is_allowed_api_path, percent_decode};

    #[test]
    fn bridge_rejects_auth_and_path_escape_routes() {
        assert!(is_allowed_api_path("/api/v1/conversations?limit=20"));
        assert!(is_allowed_api_path("/api/v1/auth/change-password"));
        assert!(!is_allowed_api_path("/api/v1/auth/refresh"));
        assert!(!is_allowed_api_path("/api/v1/../auth/login"));
        assert!(!is_allowed_api_path("https://attacker.example/api/v1/me"));
    }

    #[test]
    fn percent_decodes_utf8_file_names() {
        assert_eq!(
            percent_decode("Rust%20%E8%81%8A%E5%A4%A9.png"),
            Ok("Rust 聊天.png".to_owned())
        );
        assert_eq!(percent_decode("bad%"), Err(()));
    }
}
