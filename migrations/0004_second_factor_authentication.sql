-- Optional TOTP second factor and one-use recovery codes. Secrets are expected
-- to be envelope-encrypted by the application before normalized persistence.
CREATE TABLE user_second_factors (
    user_id uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    encrypted_totp_secret bytea NOT NULL,
    key_version smallint NOT NULL DEFAULT 1 CHECK (key_version > 0),
    enabled_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE second_factor_recovery_codes (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash bytea NOT NULL,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, code_hash)
);
CREATE INDEX second_factor_recovery_active_idx
    ON second_factor_recovery_codes(user_id, created_at)
    WHERE consumed_at IS NULL;

CREATE TABLE qr_login_challenges (
    id uuid PRIMARY KEY,
    secret_hash bytea NOT NULL UNIQUE,
    approved_user_id uuid REFERENCES users(id) ON DELETE CASCADE,
    device_name varchar(120) NOT NULL,
    platform varchar(32) NOT NULL,
    app_version varchar(32) NOT NULL,
    expires_at timestamptz NOT NULL,
    approved_at timestamptz,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX qr_login_challenges_expiry_idx ON qr_login_challenges(expires_at)
    WHERE consumed_at IS NULL;
