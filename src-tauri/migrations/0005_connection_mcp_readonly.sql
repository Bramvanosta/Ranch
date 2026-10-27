-- MCP read-only per connection.
--
-- `mcp_readonly` is a nullable override: NULL follows the global default
-- (`kv_settings.mcp_readonly_default`, false when unset), 1 forbids every MCP
-- write tool on this connection, 0 allows them whatever the default says.
-- MCP *read* tools are never restricted.
--
-- Backfill: connections that live in a Production-colored environment become
-- MCP read-only. Until this migration an agent could not write to them
-- without the human unlocking the connection first (they are created locked,
-- by default); a human *approval* authorizes a single write even on
-- a locked connection — so production must start on the safe side here too.

ALTER TABLE connections ADD COLUMN mcp_readonly INTEGER;

UPDATE connections
   SET mcp_readonly = 1
 WHERE environment_id IN (SELECT id FROM environments WHERE color_token = 'env-production');
