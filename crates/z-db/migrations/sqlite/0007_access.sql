-- Who may do what: roles, resellers' quotas, API keys and the audit log.

-- A role is a set of permissions, each `<section>.read` or `<section>.write`
-- (comma-separated). `own_users_only` makes the admin see and change only
-- the users they made: a reseller.
CREATE TABLE roles (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    name           VARCHAR(64) NOT NULL UNIQUE,
    permissions    TEXT NOT NULL DEFAULT '',
    own_users_only BIGINT NOT NULL DEFAULT 0,
    created_at     BIGINT NOT NULL
);

-- A sudo admin has no role and may do everything. Any other admin has the
-- permissions of their role. `user_limit` and `data_quota` (bytes handed out
-- as users' data limits) cap what the admin may give away; 0 for no cap.
-- `group_ids` are the access groups they may give users; empty for all.
ALTER TABLE admins ADD COLUMN role_id BIGINT;
ALTER TABLE admins ADD COLUMN enabled BIGINT NOT NULL DEFAULT 1;
ALTER TABLE admins ADD COLUMN user_limit BIGINT NOT NULL DEFAULT 0;
ALTER TABLE admins ADD COLUMN data_quota BIGINT NOT NULL DEFAULT 0;
ALTER TABLE admins ADD COLUMN group_ids TEXT NOT NULL DEFAULT '';
ALTER TABLE admins ADD COLUMN note VARCHAR(512) NOT NULL DEFAULT '';

CREATE INDEX users_admin ON users (admin_id);

-- An API key acts as its admin, with at most the permissions it names. Only
-- a hash of it is kept: the key itself is shown once, when it is made.
CREATE TABLE api_keys (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    admin_id     BIGINT NOT NULL,
    name         VARCHAR(64) NOT NULL,
    -- The first characters of the key, to tell keys apart in a list.
    prefix       VARCHAR(16) NOT NULL,
    key_hash     VARCHAR(64) NOT NULL UNIQUE,
    -- `*` for whatever the admin may, or comma-separated permissions.
    permissions  TEXT NOT NULL DEFAULT '*',
    -- Comma-separated addresses or CIDR ranges; empty for from anywhere.
    allowed_ips  TEXT NOT NULL DEFAULT '',
    expires_at   BIGINT,
    last_used_at BIGINT,
    last_used_ip VARCHAR(64),
    created_at   BIGINT NOT NULL
);
CREATE INDEX api_keys_admin ON api_keys (admin_id);

-- What admins did. The name is kept as text: the admin may be deleted later.
CREATE TABLE audit_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    at         BIGINT NOT NULL,
    admin_id   BIGINT,
    admin_name VARCHAR(64) NOT NULL,
    -- `session`, or `key:<name>` for a request made with an API key.
    via        VARCHAR(80) NOT NULL DEFAULT 'session',
    ip         VARCHAR(64) NOT NULL DEFAULT '',
    action     VARCHAR(64) NOT NULL,
    target     VARCHAR(128) NOT NULL DEFAULT '',
    detail     TEXT NOT NULL DEFAULT '',
    -- The HTTP status the request got back.
    status     BIGINT NOT NULL DEFAULT 200
);
CREATE INDEX audit_log_at ON audit_log (at);
CREATE INDEX audit_log_admin ON audit_log (admin_id);
