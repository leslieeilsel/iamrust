-- I Am Rust server schema. All timestamps are UTC `timestamptz` values.
CREATE EXTENSION IF NOT EXISTS citext;
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TYPE account_state AS ENUM ('active', 'suspended', 'deleting', 'deleted');
CREATE TYPE presence_state AS ENUM ('online', 'away', 'busy', 'invisible', 'offline');
CREATE TYPE friendship_request_state AS ENUM ('pending', 'accepted', 'rejected', 'cancelled');
CREATE TYPE conversation_kind AS ENUM ('direct', 'group');
CREATE TYPE member_role AS ENUM ('member', 'administrator', 'owner');
CREATE TYPE message_kind AS ENUM ('text', 'image', 'file', 'audio', 'video', 'system');
CREATE TYPE message_state AS ENUM ('sent', 'recalled', 'deleted');
CREATE TYPE receipt_kind AS ENUM ('delivered', 'read');
CREATE TYPE attachment_state AS ENUM ('authorized', 'uploaded', 'quarantined', 'available', 'deleted');

CREATE TABLE users (
    id uuid PRIMARY KEY,
    email citext NOT NULL UNIQUE,
    username citext NOT NULL UNIQUE,
    nickname varchar(48) NOT NULL,
    avatar_key text,
    signature varchar(160) NOT NULL DEFAULT '',
    gender varchar(32),
    birthday date,
    region varchar(96),
    presence presence_state NOT NULL DEFAULT 'offline',
    account_state account_state NOT NULL DEFAULT 'active',
    last_seen_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz,
    CONSTRAINT username_syntax CHECK (username::text ~ '^[A-Za-z0-9_]{3,32}$'),
    CONSTRAINT nickname_not_blank CHECK (length(btrim(nickname)) > 0)
);

CREATE TABLE credentials (
    user_id uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    password_hash text NOT NULL,
    password_changed_at timestamptz NOT NULL DEFAULT now(),
    failed_attempts integer NOT NULL DEFAULT 0 CHECK (failed_attempts >= 0),
    locked_until timestamptz,
    hash_version smallint NOT NULL DEFAULT 1
);

CREATE TABLE devices (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name varchar(120) NOT NULL,
    platform varchar(32) NOT NULL DEFAULT 'unknown',
    app_version varchar(32) NOT NULL DEFAULT 'unknown',
    push_token_hash bytea,
    last_ip inet,
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz
);
CREATE INDEX devices_user_active_idx ON devices(user_id, last_seen_at DESC) WHERE revoked_at IS NULL;

CREATE TABLE refresh_tokens (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_id uuid NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    family_id uuid NOT NULL,
    token_hash bytea NOT NULL UNIQUE,
    parent_token_id uuid REFERENCES refresh_tokens(id) ON DELETE SET NULL,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX refresh_tokens_family_idx ON refresh_tokens(family_id);
CREATE INDEX refresh_tokens_user_active_idx ON refresh_tokens(user_id, expires_at) WHERE revoked_at IS NULL;

CREATE TABLE password_reset_challenges (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash bytea NOT NULL,
    expires_at timestamptz NOT NULL,
    attempts smallint NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 10),
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX password_reset_active_idx ON password_reset_challenges(user_id, expires_at DESC)
    WHERE consumed_at IS NULL;

CREATE TABLE user_privacy_settings (
    user_id uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    gender_visibility varchar(16) NOT NULL DEFAULT 'friends' CHECK (gender_visibility IN ('everyone', 'friends', 'nobody')),
    birthday_visibility varchar(16) NOT NULL DEFAULT 'friends' CHECK (birthday_visibility IN ('everyone', 'friends', 'nobody')),
    region_visibility varchar(16) NOT NULL DEFAULT 'friends' CHECK (region_visibility IN ('everyone', 'friends', 'nobody')),
    presence_visibility varchar(16) NOT NULL DEFAULT 'friends' CHECK (presence_visibility IN ('everyone', 'friends', 'nobody')),
    read_receipts_enabled boolean NOT NULL DEFAULT true,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE friend_requests (
    id uuid PRIMARY KEY,
    sender_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    recipient_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    message varchar(120) NOT NULL DEFAULT '',
    state friendship_request_state NOT NULL DEFAULT 'pending',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT friend_request_not_self CHECK (sender_id <> recipient_id)
);
CREATE UNIQUE INDEX friend_requests_pending_pair_idx
    ON friend_requests(sender_id, recipient_id) WHERE state = 'pending';
CREATE INDEX friend_requests_inbox_idx ON friend_requests(recipient_id, updated_at DESC);

CREATE TABLE friendships (
    lower_user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    upper_user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    lower_remark varchar(48),
    upper_remark varchar(48),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (lower_user_id, upper_user_id),
    CONSTRAINT friendship_canonical_order CHECK (lower_user_id < upper_user_id)
);
CREATE INDEX friendships_upper_idx ON friendships(upper_user_id);

CREATE TABLE user_blocks (
    blocker_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blocked_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason varchar(120),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (blocker_id, blocked_id),
    CONSTRAINT user_block_not_self CHECK (blocker_id <> blocked_id)
);

CREATE TABLE conversations (
    id uuid PRIMARY KEY,
    kind conversation_kind NOT NULL,
    name varchar(80) NOT NULL DEFAULT '',
    avatar_key text,
    next_sequence bigint NOT NULL DEFAULT 1 CHECK (next_sequence > 0),
    created_by uuid NOT NULL REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz,
    CONSTRAINT direct_name_empty CHECK (kind <> 'direct' OR name = '')
);

CREATE TABLE direct_conversations (
    conversation_id uuid PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    lower_user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    upper_user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    UNIQUE (lower_user_id, upper_user_id),
    CONSTRAINT direct_canonical_order CHECK (lower_user_id < upper_user_id)
);

CREATE TABLE conversation_members (
    conversation_id uuid NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role member_role NOT NULL DEFAULT 'member',
    group_nickname varchar(48),
    muted_until timestamptz,
    joined_at timestamptz NOT NULL DEFAULT now(),
    left_at timestamptz,
    PRIMARY KEY (conversation_id, user_id)
);
CREATE INDEX conversation_members_user_idx ON conversation_members(user_id, conversation_id)
    WHERE left_at IS NULL;
CREATE UNIQUE INDEX one_group_owner_idx ON conversation_members(conversation_id)
    WHERE role = 'owner' AND left_at IS NULL;

CREATE TABLE member_states (
    conversation_id uuid NOT NULL,
    user_id uuid NOT NULL,
    pinned boolean NOT NULL DEFAULT false,
    muted boolean NOT NULL DEFAULT false,
    hidden boolean NOT NULL DEFAULT false,
    manually_unread boolean NOT NULL DEFAULT false,
    last_read_sequence bigint NOT NULL DEFAULT 0 CHECK (last_read_sequence >= 0),
    draft text NOT NULL DEFAULT '',
    draft_updated_at timestamptz,
    label varchar(48),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (conversation_id, user_id),
    FOREIGN KEY (conversation_id, user_id)
        REFERENCES conversation_members(conversation_id, user_id) ON DELETE CASCADE,
    CONSTRAINT draft_limit CHECK (length(draft) <= 8000)
);

CREATE TABLE messages (
    id uuid PRIMARY KEY,
    client_message_id uuid NOT NULL,
    conversation_id uuid NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    sender_id uuid NOT NULL REFERENCES users(id),
    sequence bigint NOT NULL CHECK (sequence > 0),
    kind message_kind NOT NULL,
    content jsonb NOT NULL,
    state message_state NOT NULL DEFAULT 'sent',
    reply_to uuid REFERENCES messages(id) ON DELETE SET NULL,
    scheduled_for timestamptz,
    created_at timestamptz NOT NULL,
    edited_at timestamptz,
    recalled_at timestamptz,
    deleted_at timestamptz,
    UNIQUE (conversation_id, sequence),
    UNIQUE (sender_id, client_message_id),
    CONSTRAINT message_content_object CHECK (jsonb_typeof(content) = 'object'),
    CONSTRAINT message_text_limit CHECK (kind <> 'text' OR length(content->>'text') BETWEEN 1 AND 8000)
);
CREATE INDEX messages_timeline_idx ON messages(conversation_id, sequence DESC) WHERE deleted_at IS NULL;
CREATE INDEX messages_sender_idx ON messages(sender_id, created_at DESC);

CREATE TABLE attachments (
    id uuid PRIMARY KEY,
    message_id uuid REFERENCES messages(id) ON DELETE CASCADE,
    owner_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind message_kind NOT NULL CHECK (kind IN ('image', 'file', 'audio', 'video')),
    file_name varchar(255) NOT NULL,
    mime_type varchar(127) NOT NULL,
    byte_size bigint NOT NULL CHECK (byte_size BETWEEN 1 AND 104857600),
    sha256 bytea,
    storage_key text NOT NULL UNIQUE,
    thumbnail_key text,
    state attachment_state NOT NULL DEFAULT 'authorized',
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    expires_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT attachment_file_name_safe CHECK (file_name !~ '[/\\]' AND file_name NOT IN ('.', '..'))
);
CREATE INDEX attachments_owner_idx ON attachments(owner_id, created_at DESC);
CREATE INDEX attachments_cleanup_idx ON attachments(expires_at) WHERE state IN ('authorized', 'quarantined');

CREATE TABLE message_receipts (
    message_id uuid NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind receipt_kind NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (message_id, user_id, kind)
);

CREATE TABLE message_reactions (
    message_id uuid NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    emoji varchar(32) NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (message_id, user_id, emoji),
    CONSTRAINT emoji_not_blank CHECK (length(btrim(emoji)) > 0)
);

CREATE TABLE favorite_messages (
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    message_id uuid NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, message_id)
);

CREATE TABLE group_announcements (
    id uuid PRIMARY KEY,
    conversation_id uuid NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    author_id uuid NOT NULL REFERENCES users(id),
    content varchar(4000) NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE group_announcement_reads (
    announcement_id uuid NOT NULL REFERENCES group_announcements(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    read_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (announcement_id, user_id)
);

CREATE TABLE group_join_requests (
    id uuid PRIMARY KEY,
    conversation_id uuid NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    applicant_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    message varchar(120) NOT NULL DEFAULT '',
    state friendship_request_state NOT NULL DEFAULT 'pending',
    reviewed_by uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX group_join_pending_idx ON group_join_requests(conversation_id, applicant_id)
    WHERE state = 'pending';

CREATE TABLE polls (
    id uuid PRIMARY KEY,
    conversation_id uuid NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    creator_id uuid NOT NULL REFERENCES users(id),
    question varchar(240) NOT NULL,
    multiple_choice boolean NOT NULL DEFAULT false,
    closes_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE poll_options (
    id uuid PRIMARY KEY,
    poll_id uuid NOT NULL REFERENCES polls(id) ON DELETE CASCADE,
    label varchar(160) NOT NULL,
    position smallint NOT NULL,
    UNIQUE (poll_id, position)
);
CREATE TABLE poll_votes (
    poll_id uuid NOT NULL REFERENCES polls(id) ON DELETE CASCADE,
    option_id uuid NOT NULL REFERENCES poll_options(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (poll_id, option_id, user_id)
);

CREATE TABLE sync_events (
    cursor bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    event_id uuid NOT NULL UNIQUE,
    kind varchar(64) NOT NULL,
    payload jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL DEFAULT (now() + interval '90 days'),
    CONSTRAINT sync_payload_object CHECK (jsonb_typeof(payload) = 'object')
);
CREATE TABLE sync_event_recipients (
    cursor bigint NOT NULL REFERENCES sync_events(cursor) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (cursor, user_id)
);
CREATE INDEX sync_recipient_cursor_idx ON sync_event_recipients(user_id, cursor);

CREATE TABLE audit_log (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    actor_id uuid REFERENCES users(id) ON DELETE SET NULL,
    action varchar(96) NOT NULL,
    target_type varchar(64),
    target_id uuid,
    outcome varchar(32) NOT NULL,
    correlation_id uuid NOT NULL,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    ip_hash bytea,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL DEFAULT (now() + interval '365 days')
);
CREATE INDEX audit_log_lookup_idx ON audit_log(target_type, target_id, created_at DESC);
CREATE INDEX audit_log_actor_idx ON audit_log(actor_id, created_at DESC);

CREATE TABLE call_sessions (
    id uuid PRIMARY KEY,
    conversation_id uuid NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    initiator_id uuid NOT NULL REFERENCES users(id),
    media_kind varchar(16) NOT NULL CHECK (media_kind IN ('audio', 'video', 'screen')),
    state varchar(24) NOT NULL CHECK (state IN ('ringing', 'active', 'declined', 'busy', 'ended', 'failed')),
    started_at timestamptz NOT NULL DEFAULT now(),
    answered_at timestamptz,
    ended_at timestamptz,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE FUNCTION set_updated_at() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END
$$;

CREATE TRIGGER users_set_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER friend_requests_set_updated_at BEFORE UPDATE ON friend_requests
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER conversations_set_updated_at BEFORE UPDATE ON conversations
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER attachments_set_updated_at BEFORE UPDATE ON attachments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER group_announcements_set_updated_at BEFORE UPDATE ON group_announcements
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER group_join_requests_set_updated_at BEFORE UPDATE ON group_join_requests
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE FUNCTION enforce_message_membership() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM conversation_members
        WHERE conversation_id = NEW.conversation_id
          AND user_id = NEW.sender_id
          AND left_at IS NULL
          AND (muted_until IS NULL OR muted_until <= NEW.created_at)
    ) THEN
        RAISE EXCEPTION 'sender is not an active conversation member' USING ERRCODE = '42501';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER messages_enforce_membership BEFORE INSERT ON messages
    FOR EACH ROW EXECUTE FUNCTION enforce_message_membership();
