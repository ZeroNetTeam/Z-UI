-- Panel administrators. Flags and times are BIGINT (0/1, Unix seconds) so
-- they read back the same through the Any driver on both databases.
CREATE TABLE admins (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      VARCHAR(64) NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    totp_secret   TEXT,
    is_sudo       BIGINT NOT NULL DEFAULT 0,
    -- Raised whenever existing sessions must stop being valid.
    token_version BIGINT NOT NULL DEFAULT 0,
    created_at    BIGINT NOT NULL,
    last_login_at BIGINT
);
