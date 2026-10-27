-- Groups become first-class, user-managed entities.
--
-- `collapsed` persists the sidebar's expand/collapse state per group so it
-- survives restarts. Existing groups (including the "Default" group that
-- connection wizard auto-created) default to expanded and are otherwise untouched.

ALTER TABLE groups ADD COLUMN collapsed INTEGER NOT NULL DEFAULT 0;
