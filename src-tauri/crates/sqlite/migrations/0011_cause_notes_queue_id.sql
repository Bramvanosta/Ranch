-- A cause note names its queue by the queue's id, as the backend that
-- listed the queue made it, instead of a key prefix and a name. Every note
-- stored so far is on a BullMQ queue, whose id is `{prefix}:{name}`. The
-- name stays, for display.

ALTER TABLE cause_notes ADD COLUMN queue_id TEXT NOT NULL DEFAULT '';
UPDATE cause_notes SET queue_id = queue_prefix || ':' || queue_name;

DROP INDEX cause_notes_by_cause;
CREATE INDEX cause_notes_by_cause ON cause_notes (connection_id, queue_id, signature);

ALTER TABLE cause_notes DROP COLUMN queue_prefix;
