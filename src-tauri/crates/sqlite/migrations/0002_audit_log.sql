-- Audit log (append-only, flushed synchronously).
--
-- Module visibility on `core::audit` enforces that only this module
-- writes here. No `DELETE` path is shipped; deletion of audit data
-- requires explicit filesystem action on ranch.db.
--
-- Connection FK is intentionally LOOSE: a connection can be removed
-- and the audit entries stay readable. The snapshot
-- columns `connection_name` / `env_color` preserve display info.

CREATE TABLE audit_log_entries (
    id              TEXT PRIMARY KEY NOT NULL,
    created_at      TEXT NOT NULL,
    source          TEXT NOT NULL,
    operator_id     TEXT NOT NULL DEFAULT 'local',
    agent_client    TEXT,
    agent_transport TEXT,
    action          TEXT NOT NULL,
    target_kind     TEXT NOT NULL,
    target_payload  TEXT NOT NULL,
    outcome         TEXT NOT NULL,
    outcome_reason  TEXT,
    connection_id   TEXT,
    connection_name TEXT,
    env_color       TEXT
);

CREATE INDEX idx_audit_log_entries_created_at
  ON audit_log_entries(created_at DESC);
CREATE INDEX idx_audit_log_entries_connection_id
  ON audit_log_entries(connection_id);
CREATE INDEX idx_audit_log_entries_action
  ON audit_log_entries(action);
