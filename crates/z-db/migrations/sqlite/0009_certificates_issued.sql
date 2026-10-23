-- Certificates the panel issues and renews itself (phase 10). An uploaded or
-- self-signed one has no method and does not renew.
ALTER TABLE certificates ADD COLUMN auto_renew BIGINT NOT NULL DEFAULT 0;
-- How it was issued: `http-01`, `tls-alpn-01` or `dns-01`.
ALTER TABLE certificates ADD COLUMN method VARCHAR(16) NOT NULL DEFAULT '';
-- Who issued it: `letsencrypt`, `letsencrypt_staging`, `zerossl`, or the
-- directory URL of another certificate authority.
ALTER TABLE certificates ADD COLUMN ca VARCHAR(255) NOT NULL DEFAULT '';
-- What the last attempt to issue or renew said (empty when it worked), and when.
ALTER TABLE certificates ADD COLUMN last_error TEXT NOT NULL DEFAULT '';
ALTER TABLE certificates ADD COLUMN last_attempt_at BIGINT NOT NULL DEFAULT 0;
