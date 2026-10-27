-- MCP activity log.
--
-- One row per MCP tool call, reads included, written by `ranch-mcp`; the GUI
-- only reads (and can clear). This is NOT the audit log: `audit_log_entries`
-- stays the append-only, non-purgeable record of destructive operations from
-- both surfaces. The activity log answers a different question —
-- "what did the agent look at and try to do in that session?" — and is
-- capped (oldest rows go first) so chatty agents cannot grow the database
-- without bound.
--
-- No foreign key on `connection_id`: snapshot columns keep a row readable
-- after its connection is removed.

CREATE TABLE mcp_activity_events (
    id              TEXT PRIMARY KEY NOT NULL,
    session_id      TEXT NOT NULL,
    timestamp       TEXT NOT NULL,
    client_hint     TEXT,
    tool            TEXT NOT NULL,
    kind            TEXT NOT NULL CHECK (kind IN ('read', 'write')),
    connection_id   TEXT,
    connection_name TEXT,
    target_json     TEXT,
    status          TEXT NOT NULL CHECK (status IN (
                        'success', 'error', 'blocked_readonly',
                        'denied', 'timeout', 'host_unavailable')),
    duration_ms     INTEGER NOT NULL,
    error           TEXT,
    approval_id     TEXT
);
CREATE INDEX idx_mcp_activity_events_timestamp ON mcp_activity_events(timestamp DESC);
CREATE INDEX idx_mcp_activity_events_session ON mcp_activity_events(session_id, timestamp);
