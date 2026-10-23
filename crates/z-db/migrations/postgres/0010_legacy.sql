-- What a migration from another panel (3X-UI, Marzban, PasarGuard) leaves
-- behind so that the subscription links the old panel handed out keep working.

-- The key the old panel signed its links with. A link is not looked up, it is
-- checked against this and names a user.
CREATE TABLE legacy_secrets (
    kind        VARCHAR(16) PRIMARY KEY,
    secret      TEXT NOT NULL,
    imported_at BIGINT NOT NULL
);

-- Which user each of the old panel's users became: a link names the old id
-- or the old username (which a user may no longer have). `valid_after` is the
-- time before which a link was made for someone else or was revoked (the
-- later of the user's creation and its last revoking), and so does not count.
CREATE TABLE legacy_users (
    kind        VARCHAR(16) NOT NULL,
    source_id   BIGINT NOT NULL,
    source_name VARCHAR(190) NOT NULL,
    user_id     BIGINT NOT NULL,
    valid_after BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (kind, source_id)
);
CREATE INDEX legacy_users_user ON legacy_users (user_id);
CREATE INDEX legacy_users_name ON legacy_users (kind, source_name);
