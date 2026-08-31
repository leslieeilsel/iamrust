use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Nonce},
};
use chrono::Utc;
use data_encoding::BASE64URL_NOPAD;
use iamrust_domain::{ConversationKind, Message, SyncEvent};
use iamrust_protocol::BootstrapResponse;
use rand::{Rng as _, RngExt as _};
use serde::{Serialize, de::DeserializeOwned};
use sqlx::{
    Row, Sqlite, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use tokio::sync::RwLock;

const CREDENTIAL_SERVICE: &str = "app.iamrust.desktop";
const CACHE_KEY_ACCOUNT: &str = "local-cache-key-v1";
const ENCRYPTED_PREFIX: &str = "enc:v1:";

/// Resolves the per-user data directory without depending on a UI framework.
/// `IAMRUST_DATA_DIR` is intended for portable builds and automated tests.
pub fn default_data_directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("IAMRUST_DATA_DIR") {
        let path = PathBuf::from(path);
        if path.as_os_str().is_empty() {
            return Err("IAMRUST_DATA_DIR cannot be empty".to_owned());
        }
        return Ok(path);
    }
    platform_data_directory()
}

#[cfg(target_os = "windows")]
fn platform_data_directory() -> Result<PathBuf, String> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("I Am Rust"))
        .ok_or_else(|| "Windows application data directory is unavailable".to_owned())
}

#[cfg(target_os = "macos")]
fn platform_data_directory() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join("Library/Application Support/I Am Rust"))
        .ok_or_else(|| "macOS application data directory is unavailable".to_owned())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_data_directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(path).join("i-am-rust"));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join(".local/share/i-am-rust"))
        .ok_or_else(|| "Unix application data directory is unavailable".to_owned())
}

#[cfg(not(any(target_os = "windows", unix)))]
fn platform_data_directory() -> Result<PathBuf, String> {
    Err("application data directory is unavailable on this platform".to_owned())
}

#[derive(Clone)]
pub struct LocalStore {
    pool: SqlitePool,
    path: PathBuf,
    encryption_key: Arc<RwLock<Option<[u8; 32]>>>,
}

impl fmt::Debug for LocalStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalStore")
            .field("path", &self.path)
            .field("encryption_key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheStats {
    pub database_bytes: u64,
    pub media_bytes: u64,
    pub message_count: i64,
    pub pending_outbox_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutboxItem {
    pub client_message_id: String,
    pub conversation_id: String,
    pub payload_json: String,
    pub attempt_count: i64,
    pub next_attempt_at: String,
    pub last_error_code: Option<String>,
}

impl LocalStore {
    pub async fn open_default() -> Result<Self, String> {
        let directory = default_data_directory()?;
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|_| "failed to create local data directory".to_owned())?;
        Self::open(&directory.join("iamrust.sqlite3")).await
    }

    pub async fn open(path: &Path) -> Result<Self, String> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(|_| "local database unavailable".to_owned())?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|_| "local database migration failed".to_owned())?;
        let encryption_enabled = sqlx::query_scalar::<_, String>(
            "SELECT value FROM local_meta WHERE key = 'cache_encryption'",
        )
        .fetch_optional(&pool)
        .await
        .map_err(|_| "failed to read local encryption state".to_owned())?
        .is_some_and(|value| value == "v1");
        let encryption_key = if encryption_enabled {
            Some(
                load_cache_key()
                    .await?
                    .ok_or_else(|| "encrypted local cache key is unavailable".to_owned())?,
            )
        } else {
            None
        };
        Ok(Self {
            pool,
            path: path.to_owned(),
            encryption_key: Arc::new(RwLock::new(encryption_key)),
        })
    }

    pub async fn cache_stats(&self) -> Result<CacheStats, String> {
        let message_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM cached_messages")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| "failed to count cached messages".to_owned())?;
        let pending_outbox_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM outbox")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| "failed to count pending messages".to_owned())?;
        let database_bytes = tokio::fs::metadata(&self.path)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        Ok(CacheStats {
            database_bytes,
            media_bytes: 0,
            message_count,
            pending_outbox_count,
        })
    }

    pub async fn cache_bootstrap(&self, value: &BootstrapResponse) -> Result<(), String> {
        let key = *self.encryption_key.read().await;
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| "local transaction failed".to_owned())?;
        let now = Utc::now().to_rfc3339();
        cache_profiles(&mut transaction, value, key.as_ref(), &now).await?;
        cache_conversations(&mut transaction, value, key.as_ref(), &now).await?;
        let snapshot = serde_json::to_string(value)
            .map_err(|_| "bootstrap serialization failed".to_owned())?;
        let snapshot = protect(&snapshot, key.as_ref())?;
        sqlx::query(
            r"INSERT INTO local_meta(key, value, updated_at) VALUES ('bootstrap_snapshot', ?, ?)
               ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(snapshot)
        .bind(&now)
        .execute(&mut *transaction)
        .await
        .map_err(|_| "failed to cache bootstrap".to_owned())?;
        sqlx::query(
            r"INSERT INTO local_meta(key, value, updated_at) VALUES ('sync_cursor', ?, ?)
               ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(value.cursor.to_string())
        .bind(&now)
        .execute(&mut *transaction)
        .await
        .map_err(|_| "failed to cache sync cursor".to_owned())?;
        transaction
            .commit()
            .await
            .map_err(|_| "failed to commit local cache".to_owned())
    }

    pub async fn load_bootstrap(&self) -> Result<Option<BootstrapResponse>, String> {
        let key = *self.encryption_key.read().await;
        let raw = sqlx::query_scalar::<_, String>(
            "SELECT value FROM local_meta WHERE key = 'bootstrap_snapshot'",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| "failed to read local cache".to_owned())?;
        raw.map(|value| {
            let value = unprotect(&value, key.as_ref())?;
            serde_json::from_str(&value).map_err(|_| "local cache is invalid".to_owned())
        })
        .transpose()
    }

    pub async fn cache_messages(&self, messages: &[Message]) -> Result<(), String> {
        let key = *self.encryption_key.read().await;
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| "local transaction failed".to_owned())?;
        for message in messages {
            let payload = serde_json::to_string(message)
                .map_err(|_| "message serialization failed".to_owned())?;
            let payload = protect(&payload, key.as_ref())?;
            let kind = match &message.content {
                iamrust_domain::MessageContent::Text { .. } => "text",
                iamrust_domain::MessageContent::Image { .. } => "image",
                iamrust_domain::MessageContent::File { .. } => "file",
                iamrust_domain::MessageContent::Audio { .. } => "audio",
                iamrust_domain::MessageContent::Sticker { .. } => "sticker",
                iamrust_domain::MessageContent::ForwardBundle { .. } => "forward_bundle",
                iamrust_domain::MessageContent::System { .. } => "system",
            };
            sqlx::query(
                r"DELETE FROM message_search
                   WHERE message_id IN (
                     SELECT id FROM cached_messages
                     WHERE sender_id = ? AND client_message_id = ? AND id != ?
                   )",
            )
            .bind(message.sender_id.to_string())
            .bind(message.client_message_id.to_string())
            .bind(message.id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to update message identity".to_owned())?;
            sqlx::query(
                r"DELETE FROM cached_messages
                   WHERE sender_id = ? AND client_message_id = ? AND id != ?",
            )
            .bind(message.sender_id.to_string())
            .bind(message.client_message_id.to_string())
            .bind(message.id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to update message identity".to_owned())?;
            sqlx::query(
                r"INSERT INTO cached_messages
                   (id, client_message_id, conversation_id, sender_id, sequence, kind, payload_json,
                    status, created_at, server_created_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT(id) DO UPDATE SET
                     sequence = excluded.sequence,
                     payload_json = excluded.payload_json,
                     status = excluded.status,
                     server_created_at = excluded.server_created_at",
            )
            .bind(message.id.to_string())
            .bind(message.client_message_id.to_string())
            .bind(message.conversation_id.to_string())
            .bind(message.sender_id.to_string())
            .bind(
                message
                    .sequence
                    .map(i64::try_from)
                    .transpose()
                    .map_err(|_| "message sequence is too large".to_owned())?,
            )
            .bind(kind)
            .bind(&payload)
            .bind(format!("{:?}", message.status).to_ascii_lowercase())
            .bind(message.created_at.to_rfc3339())
            .bind(message.server_created_at.map(|value| value.to_rfc3339()))
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to cache message".to_owned())?;
            sqlx::query("DELETE FROM message_search WHERE message_id = ?")
                .bind(message.id.to_string())
                .execute(&mut *transaction)
                .await
                .map_err(|_| "failed to update message index".to_owned())?;
            if key.is_none()
                && let iamrust_domain::MessageContent::Text { text } = &message.content
            {
                sqlx::query(
                    "INSERT INTO message_search(message_id, conversation_id, sender_id, body) VALUES (?, ?, ?, ?)",
                )
                .bind(message.id.to_string())
                .bind(message.conversation_id.to_string())
                .bind(message.sender_id.to_string())
                .bind(text)
                .execute(&mut *transaction)
                .await
                .map_err(|_| "failed to update message index".to_owned())?;
            }
        }
        transaction
            .commit()
            .await
            .map_err(|_| "failed to commit message cache".to_owned())
    }

    pub async fn load_messages(&self, conversation_id: &str) -> Result<Vec<Message>, String> {
        validate_uuid(conversation_id)?;
        let key = *self.encryption_key.read().await;
        let rows = sqlx::query(
            "SELECT payload_json FROM cached_messages WHERE conversation_id = ? ORDER BY sequence, created_at",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| "failed to read cached messages".to_owned())?;
        rows.into_iter()
            .map(|row| {
                let payload = unprotect(row.get::<&str, _>("payload_json"), key.as_ref())?;
                serde_json::from_str(&payload).map_err(|_| "cached message is invalid".to_owned())
            })
            .collect()
    }

    pub async fn search_messages(&self, query: &str, limit: usize) -> Result<Vec<Message>, String> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 200 || !(1..=100).contains(&limit) {
            return Err("message search query is invalid".to_owned());
        }
        let key = *self.encryption_key.read().await;
        let rows = sqlx::query(
            r"SELECT payload_json FROM cached_messages
               WHERE kind IN ('text', 'system')
               ORDER BY COALESCE(sequence, 9223372036854775807) DESC, created_at DESC
               LIMIT 10000",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|_| "failed to search cached messages".to_owned())?;
        let query = query.to_lowercase();
        let mut matches = Vec::with_capacity(limit);
        for row in rows {
            let payload = unprotect(row.get::<&str, _>("payload_json"), key.as_ref())?;
            let message: Message = serde_json::from_str(&payload)
                .map_err(|_| "cached message is invalid".to_owned())?;
            let (iamrust_domain::MessageContent::Text { text: body }
            | iamrust_domain::MessageContent::System { text: body }) = &message.content
            else {
                continue;
            };
            if body.to_lowercase().contains(&query) {
                matches.push(message);
                if matches.len() == limit {
                    break;
                }
            }
        }
        Ok(matches)
    }

    pub async fn save_draft(&self, conversation_id: &str, body: &str) -> Result<(), String> {
        validate_uuid(conversation_id)?;
        if body.chars().count() > 8_000 {
            return Err("draft is too large".to_owned());
        }
        let key = *self.encryption_key.read().await;
        let protected = protect(body, key.as_ref())?;
        sqlx::query(
            r"INSERT INTO drafts(conversation_id, body, updated_at) VALUES (?, ?, ?)
               ON CONFLICT(conversation_id) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at",
        )
        .bind(conversation_id)
        .bind(protected)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map(|_| ())
            .map_err(|_| "failed to save draft".to_owned())
    }

    pub async fn load_draft(&self, conversation_id: &str) -> Result<String, String> {
        validate_uuid(conversation_id)?;
        let key = *self.encryption_key.read().await;
        let body =
            sqlx::query_scalar::<_, String>("SELECT body FROM drafts WHERE conversation_id = ?")
                .bind(conversation_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| "failed to load draft".to_owned())?;
        body.map_or_else(|| Ok(String::new()), |body| unprotect(&body, key.as_ref()))
    }

    pub async fn enqueue_outbox(
        &self,
        client_message_id: &str,
        conversation_id: &str,
        payload_json: &str,
    ) -> Result<(), String> {
        validate_uuid(client_message_id)?;
        validate_uuid(conversation_id)?;
        if payload_json.len() > 1_048_576
            || serde_json::from_str::<serde_json::Value>(payload_json).is_err()
        {
            return Err("outbox payload is invalid".to_owned());
        }
        let key = *self.encryption_key.read().await;
        let payload_json = protect(payload_json, key.as_ref())?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r"INSERT INTO outbox
               (client_message_id, conversation_id, payload_json, next_attempt_at, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT(client_message_id) DO UPDATE SET
                 payload_json = excluded.payload_json,
                 updated_at = excluded.updated_at",
        )
        .bind(client_message_id)
        .bind(conversation_id)
        .bind(payload_json)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|_| "failed to enqueue message".to_owned())
    }

    pub async fn ready_outbox(&self) -> Result<Vec<OutboxItem>, String> {
        let key = *self.encryption_key.read().await;
        let rows = sqlx::query(
            r"SELECT client_message_id, conversation_id, payload_json, attempt_count,
                      next_attempt_at, last_error_code
               FROM outbox WHERE next_attempt_at <= ? ORDER BY created_at LIMIT 100",
        )
        .bind(Utc::now().to_rfc3339())
        .fetch_all(&self.pool)
        .await
        .map_err(|_| "failed to load outbox".to_owned())?;
        rows.into_iter()
            .map(|row| {
                Ok(OutboxItem {
                    client_message_id: row.get("client_message_id"),
                    conversation_id: row.get("conversation_id"),
                    payload_json: unprotect(row.get::<&str, _>("payload_json"), key.as_ref())?,
                    attempt_count: row.get("attempt_count"),
                    next_attempt_at: row.get("next_attempt_at"),
                    last_error_code: row.get("last_error_code"),
                })
            })
            .collect()
    }

    pub async fn acknowledge_outbox(&self, client_message_id: &str) -> Result<(), String> {
        validate_uuid(client_message_id)?;
        sqlx::query("DELETE FROM outbox WHERE client_message_id = ?")
            .bind(client_message_id)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|_| "failed to acknowledge outbox message".to_owned())
    }

    pub async fn retry_outbox_now(&self, client_message_id: &str) -> Result<(), String> {
        validate_uuid(client_message_id)?;
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            r"UPDATE outbox
               SET next_attempt_at = ?, last_error_code = NULL, updated_at = ?
               WHERE client_message_id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(client_message_id)
        .execute(&self.pool)
        .await
        .map_err(|_| "failed to retry outbox message".to_owned())?;
        if result.rows_affected() == 0 {
            Err("outbox message not found".to_owned())
        } else {
            Ok(())
        }
    }

    pub async fn discard_pending_message(&self, client_message_id: &str) -> Result<(), String> {
        validate_uuid(client_message_id)?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| "local transaction failed".to_owned())?;
        sqlx::query(
            r"DELETE FROM message_search
               WHERE message_id IN (
                 SELECT id FROM cached_messages WHERE client_message_id = ?
               )",
        )
        .bind(client_message_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| "failed to remove message search entry".to_owned())?;
        sqlx::query("DELETE FROM cached_messages WHERE client_message_id = ?")
            .bind(client_message_id)
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to remove cached message".to_owned())?;
        sqlx::query("DELETE FROM outbox WHERE client_message_id = ?")
            .bind(client_message_id)
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to remove outbox message".to_owned())?;
        transaction
            .commit()
            .await
            .map_err(|_| "failed to commit local transaction".to_owned())
    }

    pub async fn record_outbox_failure(
        &self,
        client_message_id: &str,
        error_code: &str,
    ) -> Result<(), String> {
        validate_uuid(client_message_id)?;
        if error_code.is_empty()
            || error_code.len() > 96
            || !error_code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return Err("outbox error code is invalid".to_owned());
        }
        let attempt_count = sqlx::query_scalar::<_, i64>(
            "SELECT attempt_count FROM outbox WHERE client_message_id = ?",
        )
        .bind(client_message_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| "failed to read outbox message".to_owned())?
        .ok_or_else(|| "outbox message not found".to_owned())?;
        let next_attempt = attempt_count.saturating_add(1);
        let exponent = u32::try_from(next_attempt.clamp(0, 8)).unwrap_or(8);
        let base_seconds = 1_i64 << exponent;
        let jitter = rand::rng().random_range(0..=base_seconds / 2);
        let next_attempt_at =
            Utc::now() + chrono::Duration::seconds((base_seconds + jitter).min(300));
        sqlx::query(
            r"UPDATE outbox
               SET attempt_count = ?, next_attempt_at = ?, last_error_code = ?, updated_at = ?
               WHERE client_message_id = ?",
        )
        .bind(next_attempt)
        .bind(next_attempt_at.to_rfc3339())
        .bind(error_code)
        .bind(Utc::now().to_rfc3339())
        .bind(client_message_id)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|_| "failed to reschedule outbox message".to_owned())
    }

    pub async fn save_setting<T: Serialize + ?Sized>(
        &self,
        key: &str,
        value: &T,
    ) -> Result<(), String> {
        validate_setting_key(key)?;
        let value =
            serde_json::to_string(value).map_err(|_| "setting serialization failed".to_owned())?;
        if value.len() > 65_536 {
            return Err("setting is too large".to_owned());
        }
        sqlx::query(
            r"INSERT INTO app_settings(key, value_json, updated_at) VALUES (?, ?, ?)
               ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(value)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|_| "failed to save setting".to_owned())
    }

    pub async fn load_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, String> {
        validate_setting_key(key)?;
        let value =
            sqlx::query_scalar::<_, String>("SELECT value_json FROM app_settings WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| "failed to load setting".to_owned())?;
        value
            .map(|value| {
                serde_json::from_str(&value).map_err(|_| "stored setting is invalid".to_owned())
            })
            .transpose()
    }

    pub async fn sync_cursor(&self) -> Result<u64, String> {
        let value = sqlx::query_scalar::<_, String>(
            "SELECT value FROM local_meta WHERE key = 'sync_cursor'",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| "failed to load sync cursor".to_owned())?
        .unwrap_or_else(|| "0".to_owned());
        value
            .parse()
            .map_err(|_| "stored sync cursor is invalid".to_owned())
    }

    pub async fn record_sync_event(&self, event: &SyncEvent) -> Result<bool, String> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| "local sync transaction failed".to_owned())?;
        let seen =
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM seen_events WHERE event_id = ?")
                .bind(event.id.to_string())
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| "failed to check sync event".to_owned())?
                > 0;
        if seen {
            transaction
                .rollback()
                .await
                .map_err(|_| "failed to close sync transaction".to_owned())?;
            return Ok(false);
        }
        let cursor = sqlx::query_scalar::<_, String>(
            "SELECT value FROM local_meta WHERE key = 'sync_cursor'",
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| "failed to load sync cursor".to_owned())?
        .unwrap_or_else(|| "0".to_owned())
        .parse::<u64>()
        .map_err(|_| "stored sync cursor is invalid".to_owned())?;
        if event.cursor <= cursor {
            transaction
                .rollback()
                .await
                .map_err(|_| "failed to close sync transaction".to_owned())?;
            return Ok(false);
        }
        sqlx::query("INSERT INTO seen_events(event_id, cursor, seen_at) VALUES (?, ?, ?)")
            .bind(event.id.to_string())
            .bind(i64::try_from(event.cursor).map_err(|_| "sync cursor is too large".to_owned())?)
            .bind(Utc::now().to_rfc3339())
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to record sync event".to_owned())?;
        sqlx::query(
            r"INSERT INTO local_meta(key, value, updated_at) VALUES ('sync_cursor', ?, ?)
               ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(event.cursor.to_string())
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *transaction)
        .await
        .map_err(|_| "failed to advance sync cursor".to_owned())?;
        let prune_before = event.cursor.saturating_sub(50_000);
        sqlx::query("DELETE FROM seen_events WHERE cursor < ?")
            .bind(i64::try_from(prune_before).unwrap_or(i64::MAX))
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to prune sync history".to_owned())?;
        transaction
            .commit()
            .await
            .map_err(|_| "failed to commit sync cursor".to_owned())?;
        Ok(true)
    }

    pub async fn encryption_enabled(&self) -> bool {
        self.encryption_key.read().await.is_some()
    }

    pub async fn set_encryption_enabled(&self, enabled: bool) -> Result<bool, String> {
        if self.encryption_enabled().await == enabled {
            return Ok(enabled);
        }
        if enabled {
            let mut key = [0_u8; 32];
            rand::rng().fill_bytes(&mut key);
            save_cache_key(key).await?;
            if let Err(error) = encrypt_existing_cache(&self.pool, &key).await {
                let _clear_result = clear_cache_key().await;
                return Err(error);
            }
            *self.encryption_key.write().await = Some(key);
            return Ok(true);
        }

        let key = self
            .encryption_key
            .read()
            .await
            .as_ref()
            .copied()
            .ok_or_else(|| "local cache encryption key is unavailable".to_owned())?;
        decrypt_existing_cache(&self.pool, &key).await?;
        *self.encryption_key.write().await = None;
        clear_cache_key().await?;
        Ok(false)
    }

    pub async fn clear_account_cache(&self) -> Result<(), String> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| "local transaction failed".to_owned())?;
        for statement in [
            "DELETE FROM message_search",
            "DELETE FROM outbox",
            "DELETE FROM drafts",
            "DELETE FROM cached_messages",
            "DELETE FROM cached_conversations",
            "DELETE FROM cached_users",
            "DELETE FROM seen_events",
            "DELETE FROM local_meta WHERE key NOT IN ('sync_cursor', 'cache_encryption')",
            "UPDATE local_meta SET value = '0', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE key = 'sync_cursor'",
        ] {
            sqlx::query(statement)
                .execute(&mut *transaction)
                .await
                .map_err(|_| "failed to clear local cache".to_owned())?;
        }
        transaction
            .commit()
            .await
            .map_err(|_| "failed to commit cache cleanup".to_owned())
    }
}

async fn cache_profiles(
    transaction: &mut Transaction<'_, Sqlite>,
    value: &BootstrapResponse,
    key: Option<&[u8; 32]>,
    now: &str,
) -> Result<(), String> {
    for profile in std::iter::once(&value.profile).chain(value.friends.iter()) {
        let payload = serde_json::to_string(profile)
            .map_err(|_| "profile serialization failed".to_owned())?;
        let payload = protect(&payload, key)?;
        sqlx::query(
            r"INSERT INTO cached_users
               (id, username, nickname, avatar_url, signature, presence, last_seen_at, payload_json, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                 username = excluded.username,
                 nickname = excluded.nickname,
                 avatar_url = excluded.avatar_url,
                 signature = excluded.signature,
                 presence = excluded.presence,
                 last_seen_at = excluded.last_seen_at,
                 payload_json = excluded.payload_json,
                 updated_at = excluded.updated_at",
        )
        .bind(profile.id.to_string())
        .bind(if key.is_some() { "" } else { &profile.username })
        .bind(if key.is_some() { "" } else { &profile.nickname })
        .bind(if key.is_some() {
            None
        } else {
            profile.avatar_url.as_ref().map(ToString::to_string)
        })
        .bind(if key.is_some() { "" } else { &profile.signature })
        .bind(format!("{:?}", profile.presence).to_ascii_lowercase())
        .bind(profile.last_seen_at.map(|value| value.to_rfc3339()))
        .bind(payload)
        .bind(now)
        .execute(&mut **transaction)
        .await
        .map_err(|_| "failed to cache profile".to_owned())?;
    }
    Ok(())
}

async fn cache_conversations(
    transaction: &mut Transaction<'_, Sqlite>,
    value: &BootstrapResponse,
    key: Option<&[u8; 32]>,
    now: &str,
) -> Result<(), String> {
    for conversation in &value.conversations {
        let payload = serde_json::to_string(conversation)
            .map_err(|_| "conversation serialization failed".to_owned())?;
        let payload = protect(&payload, key)?;
        let kind = match &conversation.kind {
            ConversationKind::Direct { .. } => "direct",
            ConversationKind::Group { .. } => "group",
        };
        sqlx::query(
            r"INSERT INTO cached_conversations
               (id, kind, display_name, pinned, muted, payload_json, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                 kind = excluded.kind,
                 display_name = excluded.display_name,
                 pinned = excluded.pinned,
                 muted = excluded.muted,
                 payload_json = excluded.payload_json,
                 updated_at = excluded.updated_at",
        )
        .bind(conversation.id.to_string())
        .bind(kind)
        .bind(if key.is_some() {
            ""
        } else {
            &conversation.name
        })
        .bind(conversation.pinned)
        .bind(conversation.muted)
        .bind(payload)
        .bind(now)
        .execute(&mut **transaction)
        .await
        .map_err(|_| "failed to cache conversation".to_owned())?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn encrypt_existing_cache(pool: &SqlitePool, key: &[u8; 32]) -> Result<(), String> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| "local encryption transaction failed".to_owned())?;
    for row in sqlx::query("SELECT id, payload_json FROM cached_users")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "failed to read cached profiles for encryption".to_owned())?
    {
        sqlx::query(
            "UPDATE cached_users SET username = '', nickname = '', avatar_url = NULL, signature = '', payload_json = ? WHERE id = ?",
        )
        .bind(protect(row.get("payload_json"), Some(key))?)
        .bind(row.get::<String, _>("id"))
        .execute(&mut *transaction)
        .await
        .map_err(|_| "failed to encrypt cached profile".to_owned())?;
    }
    for row in sqlx::query("SELECT id, payload_json FROM cached_conversations")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "failed to read cached conversations for encryption".to_owned())?
    {
        sqlx::query(
            "UPDATE cached_conversations SET display_name = '', payload_json = ? WHERE id = ?",
        )
        .bind(protect(row.get("payload_json"), Some(key))?)
        .bind(row.get::<String, _>("id"))
        .execute(&mut *transaction)
        .await
        .map_err(|_| "failed to encrypt cached conversation".to_owned())?;
    }
    for row in sqlx::query("SELECT id, payload_json FROM cached_messages")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "failed to read cached messages for encryption".to_owned())?
    {
        sqlx::query("UPDATE cached_messages SET payload_json = ? WHERE id = ?")
            .bind(protect(row.get("payload_json"), Some(key))?)
            .bind(row.get::<String, _>("id"))
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to encrypt cached message".to_owned())?;
    }
    for row in sqlx::query("SELECT conversation_id, body FROM drafts")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "failed to read drafts for encryption".to_owned())?
    {
        sqlx::query("UPDATE drafts SET body = ? WHERE conversation_id = ?")
            .bind(protect(row.get("body"), Some(key))?)
            .bind(row.get::<String, _>("conversation_id"))
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to encrypt draft".to_owned())?;
    }
    for row in sqlx::query("SELECT client_message_id, payload_json FROM outbox")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "failed to read outbox for encryption".to_owned())?
    {
        sqlx::query("UPDATE outbox SET payload_json = ? WHERE client_message_id = ?")
            .bind(protect(row.get("payload_json"), Some(key))?)
            .bind(row.get::<String, _>("client_message_id"))
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to encrypt outbox item".to_owned())?;
    }
    if let Some(snapshot) = sqlx::query_scalar::<_, String>(
        "SELECT value FROM local_meta WHERE key = 'bootstrap_snapshot'",
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| "failed to read bootstrap cache for encryption".to_owned())?
    {
        sqlx::query("UPDATE local_meta SET value = ? WHERE key = 'bootstrap_snapshot'")
            .bind(protect(&snapshot, Some(key))?)
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to encrypt bootstrap cache".to_owned())?;
    }
    sqlx::query("DELETE FROM message_search")
        .execute(&mut *transaction)
        .await
        .map_err(|_| "failed to clear plaintext message index".to_owned())?;
    sqlx::query(
        r"INSERT INTO local_meta(key, value, updated_at) VALUES ('cache_encryption', 'v1', ?)
           ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *transaction)
    .await
    .map_err(|_| "failed to save local encryption state".to_owned())?;
    transaction
        .commit()
        .await
        .map_err(|_| "failed to commit local encryption".to_owned())
}

#[allow(clippy::too_many_lines)]
async fn decrypt_existing_cache(pool: &SqlitePool, key: &[u8; 32]) -> Result<(), String> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| "local decryption transaction failed".to_owned())?;
    for row in sqlx::query("SELECT id, payload_json FROM cached_users")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "failed to read encrypted profiles".to_owned())?
    {
        let payload = unprotect(row.get("payload_json"), Some(key))?;
        let profile: iamrust_domain::UserProfile = serde_json::from_str(&payload)
            .map_err(|_| "encrypted profile cache is invalid".to_owned())?;
        sqlx::query(
            "UPDATE cached_users SET username = ?, nickname = ?, avatar_url = ?, signature = ?, presence = ?, last_seen_at = ?, payload_json = ? WHERE id = ?",
        )
        .bind(&profile.username)
        .bind(&profile.nickname)
        .bind(profile.avatar_url.as_ref().map(ToString::to_string))
        .bind(&profile.signature)
        .bind(format!("{:?}", profile.presence).to_ascii_lowercase())
        .bind(profile.last_seen_at.map(|value| value.to_rfc3339()))
        .bind(payload)
        .bind(row.get::<String, _>("id"))
        .execute(&mut *transaction)
        .await
        .map_err(|_| "failed to decrypt cached profile".to_owned())?;
    }
    for row in sqlx::query("SELECT id, payload_json FROM cached_conversations")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "failed to read encrypted conversations".to_owned())?
    {
        let payload = unprotect(row.get("payload_json"), Some(key))?;
        let conversation: iamrust_domain::Conversation = serde_json::from_str(&payload)
            .map_err(|_| "encrypted conversation cache is invalid".to_owned())?;
        sqlx::query(
            "UPDATE cached_conversations SET display_name = ?, payload_json = ? WHERE id = ?",
        )
        .bind(&conversation.name)
        .bind(payload)
        .bind(row.get::<String, _>("id"))
        .execute(&mut *transaction)
        .await
        .map_err(|_| "failed to decrypt cached conversation".to_owned())?;
    }
    sqlx::query("DELETE FROM message_search")
        .execute(&mut *transaction)
        .await
        .map_err(|_| "failed to rebuild message index".to_owned())?;
    for row in sqlx::query("SELECT id, payload_json FROM cached_messages")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "failed to read encrypted messages".to_owned())?
    {
        let payload = unprotect(row.get("payload_json"), Some(key))?;
        let message: Message = serde_json::from_str(&payload)
            .map_err(|_| "encrypted message cache is invalid".to_owned())?;
        sqlx::query("UPDATE cached_messages SET payload_json = ? WHERE id = ?")
            .bind(&payload)
            .bind(row.get::<String, _>("id"))
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to decrypt cached message".to_owned())?;
        if let iamrust_domain::MessageContent::Text { text } = message.content {
            sqlx::query(
                "INSERT INTO message_search(message_id, conversation_id, sender_id, body) VALUES (?, ?, ?, ?)",
            )
            .bind(message.id.to_string())
            .bind(message.conversation_id.to_string())
            .bind(message.sender_id.to_string())
            .bind(text)
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to restore message index".to_owned())?;
        }
    }
    for row in sqlx::query("SELECT conversation_id, body FROM drafts")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "failed to read encrypted drafts".to_owned())?
    {
        sqlx::query("UPDATE drafts SET body = ? WHERE conversation_id = ?")
            .bind(unprotect(row.get("body"), Some(key))?)
            .bind(row.get::<String, _>("conversation_id"))
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to decrypt draft".to_owned())?;
    }
    for row in sqlx::query("SELECT client_message_id, payload_json FROM outbox")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| "failed to read encrypted outbox".to_owned())?
    {
        sqlx::query("UPDATE outbox SET payload_json = ? WHERE client_message_id = ?")
            .bind(unprotect(row.get("payload_json"), Some(key))?)
            .bind(row.get::<String, _>("client_message_id"))
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to decrypt outbox item".to_owned())?;
    }
    if let Some(snapshot) = sqlx::query_scalar::<_, String>(
        "SELECT value FROM local_meta WHERE key = 'bootstrap_snapshot'",
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| "failed to read encrypted bootstrap cache".to_owned())?
    {
        sqlx::query("UPDATE local_meta SET value = ? WHERE key = 'bootstrap_snapshot'")
            .bind(unprotect(&snapshot, Some(key))?)
            .execute(&mut *transaction)
            .await
            .map_err(|_| "failed to decrypt bootstrap cache".to_owned())?;
    }
    sqlx::query("DELETE FROM local_meta WHERE key = 'cache_encryption'")
        .execute(&mut *transaction)
        .await
        .map_err(|_| "failed to clear local encryption state".to_owned())?;
    transaction
        .commit()
        .await
        .map_err(|_| "failed to commit local decryption".to_owned())
}

fn protect(value: &str, key: Option<&[u8; 32]>) -> Result<String, String> {
    let Some(key) = key else {
        return Ok(value.to_owned());
    };
    if value.starts_with(ENCRYPTED_PREFIX) {
        return Ok(value.to_owned());
    }
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| "local encryption initialization failed".to_owned())?;
    let mut nonce = [0_u8; 12];
    rand::rng().fill_bytes(&mut nonce);
    let nonce_array: &Nonce<Aes256Gcm> = (&nonce).into();
    let ciphertext = cipher
        .encrypt(nonce_array, value.as_bytes())
        .map_err(|_| "failed to encrypt local content".to_owned())?;
    let mut payload = Vec::with_capacity(nonce.len() + ciphertext.len());
    payload.extend_from_slice(&nonce);
    payload.extend_from_slice(&ciphertext);
    Ok(format!(
        "{ENCRYPTED_PREFIX}{}",
        BASE64URL_NOPAD.encode(&payload)
    ))
}

fn unprotect(value: &str, key: Option<&[u8; 32]>) -> Result<String, String> {
    let Some(encoded) = value.strip_prefix(ENCRYPTED_PREFIX) else {
        return Ok(value.to_owned());
    };
    let key =
        key.ok_or_else(|| "local cache is encrypted but its key is unavailable".to_owned())?;
    let payload = BASE64URL_NOPAD
        .decode(encoded.as_bytes())
        .map_err(|_| "encrypted local content is invalid".to_owned())?;
    if payload.len() < 28 {
        return Err("encrypted local content is invalid".to_owned());
    }
    let (nonce, ciphertext) = payload.split_at(12);
    let nonce: &[u8; 12] = nonce
        .try_into()
        .map_err(|_| "encrypted local content is invalid".to_owned())?;
    let nonce: &Nonce<Aes256Gcm> = nonce.into();
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| "local decryption initialization failed".to_owned())?;
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "failed to decrypt local content".to_owned())?;
    String::from_utf8(plaintext).map_err(|_| "decrypted local content is invalid".to_owned())
}

async fn save_cache_key(key: [u8; 32]) -> Result<(), String> {
    let encoded = BASE64URL_NOPAD.encode(&key);
    tokio::task::spawn_blocking(move || {
        keyring::Entry::new(CREDENTIAL_SERVICE, CACHE_KEY_ACCOUNT)
            .map_err(|_| "operating-system credential store is unavailable".to_owned())?
            .set_password(&encoded)
            .map_err(|_| "failed to save local cache key".to_owned())
    })
    .await
    .map_err(|_| "local cache key task failed".to_owned())?
}

async fn load_cache_key() -> Result<Option<[u8; 32]>, String> {
    let encoded = tokio::task::spawn_blocking(|| {
        let entry = keyring::Entry::new(CREDENTIAL_SERVICE, CACHE_KEY_ACCOUNT)
            .map_err(|_| "operating-system credential store is unavailable".to_owned())?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("failed to read local cache key".to_owned()),
        }
    })
    .await
    .map_err(|_| "local cache key task failed".to_owned())??;
    encoded
        .map(|encoded| {
            let decoded = BASE64URL_NOPAD
                .decode(encoded.as_bytes())
                .map_err(|_| "local cache key is invalid".to_owned())?;
            decoded
                .try_into()
                .map_err(|_| "local cache key is invalid".to_owned())
        })
        .transpose()
}

async fn clear_cache_key() -> Result<(), String> {
    tokio::task::spawn_blocking(|| {
        let entry = keyring::Entry::new(CREDENTIAL_SERVICE, CACHE_KEY_ACCOUNT)
            .map_err(|_| "operating-system credential store is unavailable".to_owned())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("failed to remove local cache key".to_owned()),
        }
    })
    .await
    .map_err(|_| "local cache key task failed".to_owned())?
}

fn validate_uuid(value: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| "invalid identifier".to_owned())
}

fn validate_setting_key(key: &str) -> Result<(), String> {
    if (1..=96).contains(&key.len())
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Ok(())
    } else {
        Err("setting key is invalid".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use iamrust_domain::{Conversation, ConversationId, ConversationKind, MessageId};
    use iamrust_protocol::BootstrapResponse;
    use serde_json::json;

    use super::*;

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn migrations_cache_bootstrap_and_persist_outbox() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("cache.sqlite3");
        let store = LocalStore::open(&database_path).await.unwrap();
        let me = iamrust_test_support::user("alice");
        let friend = iamrust_test_support::user("bob");
        let conversation = Conversation {
            id: ConversationId::new(),
            kind: ConversationKind::Direct {
                peer_user_id: friend.id,
            },
            name: String::new(),
            avatar_url: None,
            avatar_attachment_id: None,
            members: BTreeMap::new(),
            muted: false,
            pinned: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let bootstrap = BootstrapResponse {
            profile: me.clone(),
            profile_privacy: iamrust_domain::ProfilePrivacySettings::default(),
            friends: vec![friend],
            friend_settings: vec![],
            friend_requests: vec![],
            friend_request_profiles: vec![],
            conversations: vec![conversation.clone()],
            conversation_states: vec![],
            cursor: 12,
            server_features: json!({}),
        };
        store.cache_bootstrap(&bootstrap).await.unwrap();
        assert_eq!(
            store.load_bootstrap().await.unwrap(),
            Some(bootstrap.clone())
        );
        assert_eq!(store.sync_cursor().await.unwrap(), 12);
        let sync_event = SyncEvent {
            id: uuid::Uuid::now_v7(),
            cursor: 13,
            kind: iamrust_domain::EventKind::PresenceUpdated,
            payload: json!({}),
            created_at: Utc::now(),
        };
        assert!(store.record_sync_event(&sync_event).await.unwrap());
        assert!(!store.record_sync_event(&sync_event).await.unwrap());
        assert_eq!(store.sync_cursor().await.unwrap(), 13);
        store
            .save_draft(&conversation.id.to_string(), "local draft")
            .await
            .unwrap();
        assert_eq!(
            store
                .load_draft(&conversation.id.to_string())
                .await
                .unwrap(),
            "local draft"
        );
        store.save_setting("ui.scale", &1.25_f64).await.unwrap();
        assert_eq!(
            store.load_setting::<f64>("ui.scale").await.unwrap(),
            Some(1.25)
        );

        let mut pending = Message::pending(
            MessageId::new(),
            conversation.id,
            me.id,
            iamrust_domain::MessageContent::Text {
                text: "hello".to_owned(),
            },
            Utc::now(),
        )
        .unwrap();
        store.cache_messages(&[pending.clone()]).await.unwrap();
        pending.id = MessageId::new();
        pending.mark_sent(1, Utc::now()).unwrap();
        store.cache_messages(&[pending.clone()]).await.unwrap();
        let cached_messages = store
            .load_messages(&conversation.id.to_string())
            .await
            .unwrap();
        assert_eq!(cached_messages, vec![pending]);
        assert_eq!(store.search_messages("hello", 20).await.unwrap().len(), 1);
        assert!(
            store
                .search_messages("missing", 20)
                .await
                .unwrap()
                .is_empty()
        );

        let message_id = MessageId::new();
        store
            .enqueue_outbox(
                &message_id.to_string(),
                &conversation.id.to_string(),
                &json!({ "content": { "type": "text", "data": { "text": "hello" } }, "reply_to": null }).to_string(),
            )
            .await
            .unwrap();
        store
            .record_outbox_failure(&message_id.to_string(), "offline")
            .await
            .unwrap();
        let attempts = sqlx::query_scalar::<_, i64>(
            "SELECT attempt_count FROM outbox WHERE client_message_id = ?",
        )
        .bind(message_id.to_string())
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(attempts, 1);
        store
            .retry_outbox_now(&message_id.to_string())
            .await
            .unwrap();
        let key = [7_u8; 32];
        encrypt_existing_cache(&store.pool, &key).await.unwrap();
        *store.encryption_key.write().await = Some(key);
        let raw_snapshot = sqlx::query_scalar::<_, String>(
            "SELECT value FROM local_meta WHERE key = 'bootstrap_snapshot'",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert!(raw_snapshot.starts_with(ENCRYPTED_PREFIX));
        assert!(!raw_snapshot.contains("alice"));
        assert_eq!(
            store.load_bootstrap().await.unwrap(),
            Some(bootstrap.clone())
        );
        assert_eq!(store.search_messages("hello", 20).await.unwrap().len(), 1);
        assert_eq!(store.ready_outbox().await.unwrap().len(), 1);
        decrypt_existing_cache(&store.pool, &key).await.unwrap();
        *store.encryption_key.write().await = None;
        drop(store);

        let store = LocalStore::open(&database_path).await.unwrap();
        assert_eq!(store.load_bootstrap().await.unwrap(), Some(bootstrap));
        assert_eq!(store.ready_outbox().await.unwrap().len(), 1);
        store
            .acknowledge_outbox(&message_id.to_string())
            .await
            .unwrap();
        assert!(store.ready_outbox().await.unwrap().is_empty());

        let discard_id = MessageId::new();
        let discard = Message::pending(
            discard_id,
            conversation.id,
            me.id,
            iamrust_domain::MessageContent::Text {
                text: "discard me".to_owned(),
            },
            Utc::now(),
        )
        .unwrap();
        store.cache_messages(&[discard]).await.unwrap();
        store
            .enqueue_outbox(
                &discard_id.to_string(),
                &conversation.id.to_string(),
                &json!({ "client_message_id": discard_id }).to_string(),
            )
            .await
            .unwrap();
        store
            .discard_pending_message(&discard_id.to_string())
            .await
            .unwrap();
        assert!(
            store
                .load_messages(&conversation.id.to_string())
                .await
                .unwrap()
                .iter()
                .all(|message| message.client_message_id != discard_id)
        );
        assert!(store.ready_outbox().await.unwrap().is_empty());
    }

    #[test]
    fn encrypted_values_round_trip_without_leaking_plaintext() {
        let key = [9_u8; 32];
        let encrypted = protect("secret message body", Some(&key)).unwrap();
        assert!(encrypted.starts_with(ENCRYPTED_PREFIX));
        assert!(!encrypted.contains("secret"));
        assert_eq!(
            unprotect(&encrypted, Some(&key)).unwrap(),
            "secret message body"
        );
        assert!(unprotect(&encrypted, Some(&[8_u8; 32])).is_err());
    }

    #[test]
    fn aes_gcm_0_10_ciphertext_remains_readable() {
        let key = std::array::from_fn(|index| index.try_into().unwrap());
        let encrypted = format!(
            "{ENCRYPTED_PREFIX}AAECAwQFBgcICQoLK2exeqac73jsIv_unJ8ZAfaz9vYhPx3US990dpcoEGlv1w"
        );
        assert_eq!(
            unprotect(&encrypted, Some(&key)).unwrap(),
            "legacy-cache-value"
        );
    }
}
