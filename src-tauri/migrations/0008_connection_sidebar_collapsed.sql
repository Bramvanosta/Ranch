-- The sidebar remembers which connections are folded, like it
-- already does for groups (0004). A folded connection hides its queues and
-- keeps its failure count on the row. UI state only: agents never see it.

ALTER TABLE connections ADD COLUMN sidebar_collapsed INTEGER NOT NULL DEFAULT 0;
