-- A connection names the kind of backend that serves it and that backend's
-- endpoint, a JSON object only the backend reads, instead of one column per
-- Redis setting. Every connection saved so far is BullMQ on Redis: its
-- endpoint gathers the host, port, database, user, TLS, custom CA and SSH
-- tunnel it was saved with. An SSH tunnel column that does not hold a JSON
-- object was never used (the app read it as no tunnel) and becomes `null`.
--
-- Nothing else changes: the id, the keychain reference (secrets stay in the
-- keychain, never here), the lock, the agent setting and its seal, which
-- covers the id and those two flags only, so it stays valid as it is.
--
-- `endpoint` has a default only because a column added to existing rows
-- needs one; every write sets it.

ALTER TABLE connections ADD COLUMN backend TEXT NOT NULL DEFAULT 'bullmq-redis';
ALTER TABLE connections ADD COLUMN endpoint TEXT NOT NULL DEFAULT '{}';

UPDATE connections SET endpoint = json_object(
    'host', redis_host,
    'port', redis_port,
    'db', redis_db,
    'username', redis_username,
    'tls', json(CASE WHEN tls_enabled != 0 THEN 'true' ELSE 'false' END),
    'tlsCustomCa', tls_custom_ca,
    'sshTunnel', CASE WHEN json_valid(ssh_tunnel_config) THEN
        CASE WHEN json_type(ssh_tunnel_config) = 'object' THEN json(ssh_tunnel_config) END
    END
);

ALTER TABLE connections DROP COLUMN redis_host;
ALTER TABLE connections DROP COLUMN redis_port;
ALTER TABLE connections DROP COLUMN redis_db;
ALTER TABLE connections DROP COLUMN redis_username;
ALTER TABLE connections DROP COLUMN tls_enabled;
ALTER TABLE connections DROP COLUMN tls_custom_ca;
ALTER TABLE connections DROP COLUMN ssh_tunnel_config;
