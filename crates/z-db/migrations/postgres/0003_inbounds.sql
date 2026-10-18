-- TLS certificates the panel and its inbounds present. PEM text; times are
-- Unix seconds read from the certificate.
CREATE TABLE certificates (
    id         BIGSERIAL PRIMARY KEY,
    name       VARCHAR(128) NOT NULL,
    domains    TEXT NOT NULL,
    cert_pem   TEXT NOT NULL,
    key_pem    TEXT NOT NULL,
    source     VARCHAR(16) NOT NULL,
    not_before BIGINT NOT NULL,
    not_after  BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);

-- Inbounds, kept in the shape of an Xray inbound: `settings`, `stream`
-- (streamSettings) and `sniffing` are JSON. Accounts are not here; they
-- are added from the users (phase 4) when the core's configuration is built.
CREATE TABLE inbounds (
    id         BIGSERIAL PRIMARY KEY,
    tag        VARCHAR(64) NOT NULL UNIQUE,
    remark     VARCHAR(128) NOT NULL,
    enabled    BIGINT NOT NULL DEFAULT 1,
    listen     VARCHAR(64) NOT NULL DEFAULT '',
    port       BIGINT NOT NULL,
    protocol   VARCHAR(32) NOT NULL,
    settings   TEXT NOT NULL,
    stream     TEXT NOT NULL,
    sniffing   TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

-- How clients reach an inbound, for the links and subscriptions built from
-- it: an address (often a CDN or a domain, not the server's own), and what
-- to override on the way.
CREATE TABLE hosts (
    id             BIGSERIAL PRIMARY KEY,
    inbound_id     BIGINT NOT NULL REFERENCES inbounds(id) ON DELETE CASCADE,
    remark         VARCHAR(128) NOT NULL,
    address        VARCHAR(255) NOT NULL,
    port           BIGINT,
    sni            VARCHAR(255) NOT NULL DEFAULT '',
    host           VARCHAR(255) NOT NULL DEFAULT '',
    path           VARCHAR(255) NOT NULL DEFAULT '',
    alpn           VARCHAR(64) NOT NULL DEFAULT '',
    fingerprint    VARCHAR(32) NOT NULL DEFAULT '',
    allow_insecure BIGINT NOT NULL DEFAULT 0,
    enabled        BIGINT NOT NULL DEFAULT 1,
    priority       BIGINT NOT NULL DEFAULT 0,
    created_at     BIGINT NOT NULL
);
CREATE INDEX hosts_inbound ON hosts (inbound_id);
