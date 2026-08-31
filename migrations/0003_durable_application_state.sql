-- Crash-safe application snapshot used by the single-node service while the
-- normalized tables remain available for reporting, retention and migrations.
-- MessagePack is used because the in-memory indexes contain non-string keys.
CREATE TABLE application_state_snapshots (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    format_version integer NOT NULL DEFAULT 1 CHECK (format_version > 0),
    payload bytea NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);
