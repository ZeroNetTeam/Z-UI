-- Subscriptions: how many devices a user may fetch its subscription on (0
-- for any), when and with what it last fetched it, and the devices seen.
ALTER TABLE users ADD COLUMN hwid_limit BIGINT NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN sub_updated_at BIGINT;
ALTER TABLE users ADD COLUMN sub_user_agent VARCHAR(256) NOT NULL DEFAULT '';
ALTER TABLE user_templates ADD COLUMN hwid_limit BIGINT NOT NULL DEFAULT 0;

-- A device is what a client app calls itself in its x-hwid header.
CREATE TABLE user_devices (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id    BIGINT NOT NULL,
    hwid       VARCHAR(128) NOT NULL,
    platform   VARCHAR(64) NOT NULL DEFAULT '',
    os_version VARCHAR(64) NOT NULL DEFAULT '',
    model      VARCHAR(128) NOT NULL DEFAULT '',
    user_agent VARCHAR(256) NOT NULL DEFAULT '',
    first_seen BIGINT NOT NULL,
    last_seen  BIGINT NOT NULL,
    UNIQUE (user_id, hwid)
);
