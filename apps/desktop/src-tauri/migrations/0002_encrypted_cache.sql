CREATE TABLE drafts_with_encryption (
    conversation_id TEXT PRIMARY KEY REFERENCES cached_conversations(id) ON DELETE CASCADE,
    body TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL,
    CHECK (length(body) <= 32768)
) STRICT;

INSERT INTO drafts_with_encryption(conversation_id, body, updated_at)
SELECT conversation_id, body, updated_at FROM drafts;

DROP TABLE drafts;
ALTER TABLE drafts_with_encryption RENAME TO drafts;
