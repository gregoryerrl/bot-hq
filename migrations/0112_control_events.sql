-- Plan 5c (Driver event watch): the durable journal a controller reads with
-- `app_control events`. One row per journaled bridge event — a message row
-- (tool rows excluded), a decision parked or resolved, a halt declared or
-- cleared, a session closed, a participant's health, an activity transition,
-- an urgent gate — plus the journal's own markers (`journal_restart`,
-- `journal_gap`). `id` is the cursor: global, monotonic, never reused.
-- `source_session`/`source_slug` name the controller whose action produced the
-- row when the host knows (a delegated message), so a controller can tell its
-- own effects from the world's. Rows older than the retention window or past
-- the row cap are deleted on insert; a read whose cursor predates the oldest
-- retained row says so.
CREATE TABLE control_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at TEXT NOT NULL,
    session_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    agent TEXT,
    ref_id TEXT,
    detail TEXT,
    source_session TEXT,
    source_slug TEXT
);
CREATE INDEX control_events_session ON control_events(session_id, id);

-- A per-declaration identity for the session's halt slot: the stamp alone is
-- millisecond text and two declarations can share one (eyes-2 `04babed4`),
-- so a reader deduping by identity could suppress the newer halt. Minted by
-- the stamped declare writers, cleared with the slot; the banner event
-- carries it and the journal keys `halt_declared` rows on it.
ALTER TABLE sessions ADD COLUMN halt_id TEXT;
