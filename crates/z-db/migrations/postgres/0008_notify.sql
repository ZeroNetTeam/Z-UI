-- Notifications and the Telegram bot.

-- Things already told: a key per fact (`data_low:<user>:<limit>:<reset>`), so
-- a restart does not say them again and a reset or a raised limit lets the
-- next crossing be told.
CREATE TABLE notify_sent (
    key VARCHAR(190) PRIMARY KEY,
    at  BIGINT NOT NULL
);

-- Which Telegram chat is which admin or user. An admin's chat acts as that
-- admin (with their permissions); a user's chat only reads their own usage.
CREATE TABLE telegram_links (
    chat_id    BIGINT PRIMARY KEY,
    -- `admin` or `user`.
    kind       VARCHAR(8) NOT NULL,
    ref_id     BIGINT NOT NULL,
    -- What Telegram calls the chat, to show who it is.
    name       VARCHAR(128) NOT NULL DEFAULT '',
    created_at BIGINT NOT NULL
);
CREATE INDEX telegram_links_ref ON telegram_links (kind, ref_id);
