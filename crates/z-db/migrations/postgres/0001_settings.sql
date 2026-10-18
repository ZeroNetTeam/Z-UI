-- Panel settings as key/value text. Written in SQL both SQLite and
-- PostgreSQL accept, since one set of migrations serves both.
CREATE TABLE IF NOT EXISTS settings (
    key   VARCHAR(128) PRIMARY KEY,
    value TEXT NOT NULL
);
