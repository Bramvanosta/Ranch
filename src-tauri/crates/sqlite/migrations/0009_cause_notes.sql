-- Resolution notes from agents: an agent's
-- diagnosis of a failure cause, stored next to the cause it explains. A cause
-- is identified by its connection, queue and signature (the reason with its
-- values replaced, `failure_signature`). Notes change no queue state, so they
-- need no approval; they respect AI features and the read-only flag.
--
-- No foreign key on `connection_id`, as for approvals: a note stays readable
-- in Agents › Notes after its connection is removed.

CREATE TABLE cause_notes (
    id              TEXT PRIMARY KEY NOT NULL,
    connection_id   TEXT NOT NULL,
    queue_prefix    TEXT NOT NULL,
    queue_name      TEXT NOT NULL,
    signature       TEXT NOT NULL,
    text            TEXT NOT NULL,
    -- JSON array of strings: what to do, in order.
    steps_json      TEXT NOT NULL DEFAULT '[]',
    client_hint     TEXT,
    created_at      TEXT NOT NULL,
    -- JSON array of job ids the agent read to write the note.
    jobs_read_json  TEXT NOT NULL DEFAULT '[]',
    dismissed_at    TEXT
);

CREATE INDEX cause_notes_by_cause ON cause_notes (connection_id, queue_name, signature);
