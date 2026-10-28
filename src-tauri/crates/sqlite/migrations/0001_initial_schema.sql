-- Ranch initial schema.
--
-- Conventions:
--   - Table names: snake_case, plural.
--   - Column names: snake_case, singular.
--   - Primary keys: TEXT (UUIDv7 string) under column `id`.
--   - Foreign keys: <referent_singular>_id with ON DELETE CASCADE where appropriate.
--   - Timestamps: TEXT (ISO-8601 UTC) — `created_at` always, `updated_at` where mutable.
--   - Booleans: INTEGER (0/1) — sqlite has no native bool; sqlx maps Rust `bool` to this.
--
-- Credentials are NEVER stored in this database. Connection credentials live in
-- the OS keychain only; the `keychain_ref` column on `connections` holds the
-- keychain entry name (= the connection id) for resolution at use time.

CREATE TABLE groups (
    id            TEXT PRIMARY KEY NOT NULL,
    name          TEXT NOT NULL,
    label         TEXT NOT NULL DEFAULT 'Client',
    display_order INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE TABLE environments (
    id            TEXT PRIMARY KEY NOT NULL,
    group_id      TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    label         TEXT NOT NULL DEFAULT 'Environment',
    color_token   TEXT NOT NULL DEFAULT 'env-local',
    display_order INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_environments_group_id ON environments(group_id);

CREATE TABLE connections (
    id                  TEXT PRIMARY KEY NOT NULL,
    environment_id      TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    redis_host          TEXT NOT NULL,
    redis_port          INTEGER NOT NULL,
    redis_db            INTEGER NOT NULL DEFAULT 0,
    redis_username      TEXT,
    tls_enabled         INTEGER NOT NULL DEFAULT 0,
    tls_custom_ca       TEXT,
    ssh_tunnel_config   TEXT,
    keychain_ref        TEXT NOT NULL,
    is_locked           INTEGER NOT NULL DEFAULT 0,
    last_reachable_at   TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);
CREATE INDEX idx_connections_environment_id ON connections(environment_id);

CREATE TABLE kv_settings (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
