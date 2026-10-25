-- A link that lets an admin set a password: for admins brought over from
-- another panel, whose old passwords are not carried. Only the hash of the
-- token is kept.
CREATE TABLE admin_invites (
    token_hash VARCHAR(64) PRIMARY KEY,
    admin_id   BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);
CREATE INDEX admin_invites_admin ON admin_invites (admin_id);
