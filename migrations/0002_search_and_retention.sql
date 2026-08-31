CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE INDEX users_username_trgm_idx ON users USING gin ((username::text) gin_trgm_ops)
    WHERE account_state = 'active';
CREATE INDEX users_nickname_trgm_idx ON users USING gin (nickname gin_trgm_ops)
    WHERE account_state = 'active';
CREATE INDEX conversations_name_trgm_idx ON conversations USING gin (name gin_trgm_ops)
    WHERE deleted_at IS NULL;
CREATE INDEX messages_text_trgm_idx ON messages USING gin ((content->>'text') gin_trgm_ops)
    WHERE kind = 'text' AND deleted_at IS NULL;

-- Cleanup jobs may delete rows only after the application-specific retention window.
CREATE INDEX sync_events_expiry_idx ON sync_events(expires_at);
CREATE INDEX audit_log_expiry_idx ON audit_log(expires_at);
CREATE INDEX users_deletion_queue_idx ON users(deleted_at) WHERE account_state = 'deleting';
