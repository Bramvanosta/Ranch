-- Smoke-test polish fix.
--
-- `environments.label` is an editable display name
-- (defaults to 'Environment'). `upsert_env_for_color`
-- created envs with the literal label 'Environment' regardless of the
-- color_token. Users ended up with "Environment" / "Environment" in
-- the sidebar instead of "Local" / "Staging".
--
-- New envs created after this migration get `label = name` (where
-- `name` is derived from the color_token — see
-- `default_env_name_for_color` in commands/connections.rs).
--
-- This migration backfills the existing rows: ONLY where the label is
-- still the literal default 'Environment'. Rows the user has already
-- renamed are left alone.

UPDATE environments
SET label = name
WHERE label = 'Environment';
