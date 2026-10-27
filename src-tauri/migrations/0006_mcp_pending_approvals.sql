-- MCP write approval.
--
-- `ranch-mcp` (spawned by the agent's client) and the Ranch GUI are two
-- processes sharing this database in WAL mode. This table is their queue:
--
--   ranch-mcp  INSERT  status = 'pending'            (then polls the row)
--   GUI        UPDATE  pending  → approved | denied  (the human decides)
--   ranch-mcp  UPDATE  pending  → expired            (timeout / GUI gone)
--   actor      UPDATE  approved → consumed           (single use, then executes)
--
-- Every transition is a conditional `UPDATE … WHERE status = <expected>`; the
-- affected-row count decides the outcome, so a forged, expired, denied or
-- already-consumed id never authorizes a write.
--
-- No foreign key on `connection_id`: the snapshot columns keep a request
-- readable even if its connection is removed meanwhile. Rows are purged after
-- one hour by the GUI.

CREATE TABLE mcp_pending_approvals (
    id                  TEXT PRIMARY KEY NOT NULL,
    created_at          TEXT NOT NULL,
    client_hint         TEXT,
    connection_id       TEXT NOT NULL,
    connection_name     TEXT NOT NULL,
    group_name          TEXT NOT NULL,
    environment_name    TEXT NOT NULL,
    env_color           TEXT NOT NULL,
    tool                TEXT NOT NULL,
    kind                TEXT NOT NULL CHECK (kind IN ('read', 'write')),
    arguments_json      TEXT NOT NULL,
    target_preview_json TEXT,
    status              TEXT NOT NULL
                        CHECK (status IN ('pending', 'approved', 'denied', 'expired', 'consumed')),
    decided_at          TEXT,
    reason              TEXT
);
CREATE INDEX idx_mcp_pending_approvals_status ON mcp_pending_approvals(status, created_at);

-- The approval that authorized an agent write, when there was one.
ALTER TABLE audit_log_entries ADD COLUMN approval_id TEXT;
