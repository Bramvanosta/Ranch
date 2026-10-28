-- Seals (HMAC, key in the OS keychain) on what decides an agent's rights:
-- see `core::integrity`. NULL = not sealed, which ranch-mcp treats as
-- tampered.
ALTER TABLE connections ADD COLUMN safety_seal TEXT;
ALTER TABLE mcp_pending_approvals ADD COLUMN decision_seal TEXT;
