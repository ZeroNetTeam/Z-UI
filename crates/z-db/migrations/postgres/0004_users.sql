-- Users: the accounts that connect. `username` is also the account's name
-- in the core (its `email`), which its traffic and limits are counted by.
-- Sizes are bytes, speeds bytes per second, times Unix seconds; 0 or NULL
-- means no limit.
CREATE TABLE users (
    id               BIGSERIAL PRIMARY KEY,
    username         VARCHAR(64) NOT NULL UNIQUE,
    -- active, disabled, limited, expired or on_hold.
    status           VARCHAR(16) NOT NULL,
    uuid             VARCHAR(36) NOT NULL,
    password         VARCHAR(64) NOT NULL,
    sub_token        VARCHAR(64) NOT NULL UNIQUE,
    data_limit       BIGINT NOT NULL DEFAULT 0,
    used_traffic     BIGINT NOT NULL DEFAULT 0,
    lifetime_traffic BIGINT NOT NULL DEFAULT 0,
    expire_at        BIGINT,
    -- On hold: the time starts on the first connection (or at the timeout).
    on_hold_duration BIGINT,
    on_hold_timeout  BIGINT,
    -- no_reset, day, week, month or year.
    reset_strategy   VARCHAR(16) NOT NULL DEFAULT 'no_reset',
    last_reset_at    BIGINT,
    ip_limit         BIGINT NOT NULL DEFAULT 0,
    speed_up         BIGINT NOT NULL DEFAULT 0,
    speed_down       BIGINT NOT NULL DEFAULT 0,
    note             TEXT NOT NULL DEFAULT '',
    online_at        BIGINT,
    -- The admin who owns the user (resellers, phase 8).
    admin_id         BIGINT,
    created_at       BIGINT NOT NULL,
    updated_at       BIGINT NOT NULL
);

-- Which inbounds a user may use, by group. A group with `all_inbounds` has
-- every inbound, including those made later.
CREATE TABLE access_groups (
    id           BIGSERIAL PRIMARY KEY,
    name         VARCHAR(64) NOT NULL UNIQUE,
    all_inbounds BIGINT NOT NULL DEFAULT 0,
    created_at   BIGINT NOT NULL
);

CREATE TABLE access_group_inbounds (
    group_id   BIGINT NOT NULL,
    inbound_id BIGINT NOT NULL,
    PRIMARY KEY (group_id, inbound_id)
);

CREATE TABLE user_access_groups (
    user_id  BIGINT NOT NULL,
    group_id BIGINT NOT NULL,
    PRIMARY KEY (user_id, group_id)
);
CREATE INDEX user_access_groups_group ON user_access_groups (group_id);

-- Starting points for new users.
CREATE TABLE user_templates (
    id             BIGSERIAL PRIMARY KEY,
    name           VARCHAR(64) NOT NULL UNIQUE,
    data_limit     BIGINT NOT NULL DEFAULT 0,
    expire_days    BIGINT NOT NULL DEFAULT 0,
    on_hold        BIGINT NOT NULL DEFAULT 0,
    reset_strategy VARCHAR(16) NOT NULL DEFAULT 'no_reset',
    ip_limit       BIGINT NOT NULL DEFAULT 0,
    speed_up       BIGINT NOT NULL DEFAULT 0,
    speed_down     BIGINT NOT NULL DEFAULT 0,
    -- Comma-separated access group ids.
    group_ids      TEXT NOT NULL DEFAULT '',
    created_at     BIGINT NOT NULL
);

-- Traffic per user per hour, for charts.
CREATE TABLE user_usage (
    user_id  BIGINT NOT NULL,
    hour     BIGINT NOT NULL,
    uplink   BIGINT NOT NULL DEFAULT 0,
    downlink BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, hour)
);

-- Every panel starts with one group that holds every inbound.
INSERT INTO access_groups (name, all_inbounds, created_at) VALUES ('default', 1, 0);
