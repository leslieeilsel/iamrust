PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS local_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS cached_users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    nickname TEXT NOT NULL,
    avatar_url TEXT,
    signature TEXT NOT NULL DEFAULT '',
    presence TEXT NOT NULL DEFAULT 'offline',
    last_seen_at TEXT,
    payload_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS cached_conversations (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('direct', 'group')),
    display_name TEXT NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    muted INTEGER NOT NULL DEFAULT 0 CHECK (muted IN (0, 1)),
    hidden INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1)),
    unread_count INTEGER NOT NULL DEFAULT 0 CHECK (unread_count >= 0),
    last_read_sequence INTEGER NOT NULL DEFAULT 0 CHECK (last_read_sequence >= 0),
    last_message_sequence INTEGER NOT NULL DEFAULT 0 CHECK (last_message_sequence >= 0),
    payload_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS cached_messages (
    id TEXT PRIMARY KEY,
    client_message_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES cached_conversations(id) ON DELETE CASCADE,
    sender_id TEXT NOT NULL,
    sequence INTEGER,
    kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    server_created_at TEXT,
    UNIQUE (conversation_id, sequence),
    UNIQUE (sender_id, client_message_id)
) STRICT;
CREATE INDEX IF NOT EXISTS cached_messages_timeline_idx
    ON cached_messages(conversation_id, sequence DESC, created_at DESC);

CREATE VIRTUAL TABLE IF NOT EXISTS message_search USING fts5(
    message_id UNINDEXED,
    conversation_id UNINDEXED,
    sender_id UNINDEXED,
    body,
    tokenize = 'unicode61'
);

CREATE TABLE IF NOT EXISTS drafts (
    conversation_id TEXT PRIMARY KEY REFERENCES cached_conversations(id) ON DELETE CASCADE,
    body TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL,
    CHECK (length(body) <= 8000)
) STRICT;

CREATE TABLE IF NOT EXISTS outbox (
    client_message_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES cached_conversations(id) ON DELETE CASCADE,
    payload_json TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_attempt_at TEXT NOT NULL,
    last_error_code TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS outbox_ready_idx ON outbox(next_attempt_at, created_at);

CREATE TABLE IF NOT EXISTS pending_transfers (
    id TEXT PRIMARY KEY,
    client_message_id TEXT,
    local_path TEXT NOT NULL,
    temporary_path TEXT,
    direction TEXT NOT NULL CHECK (direction IN ('upload', 'download')),
    byte_size INTEGER NOT NULL CHECK (byte_size > 0),
    transferred_bytes INTEGER NOT NULL DEFAULT 0 CHECK (transferred_bytes >= 0),
    state TEXT NOT NULL,
    sha256 TEXT,
    expires_at TEXT,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS pending_transfers_cleanup_idx ON pending_transfers(expires_at);

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS seen_events (
    event_id TEXT PRIMARY KEY,
    cursor INTEGER NOT NULL UNIQUE,
    seen_at TEXT NOT NULL
) STRICT;

INSERT INTO local_meta(key, value, updated_at)
VALUES ('sync_cursor', '0', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
ON CONFLICT(key) DO NOTHING;
