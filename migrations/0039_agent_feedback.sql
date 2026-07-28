-- Agent-filed feedback about bot-hq ITSELF, not about the repo a session is
-- working in. A session on any project can hit friction with the tool (a gate
-- that reads badly, a workflow that wastes a round-trip) or think of an
-- improvement; without somewhere to put it, that observation dies with the
-- session. A later bot-hq session reads this table and works the queue.
--
-- Deliberately NOT scoped to a session for reading: filed from anywhere, read
-- from here. session_id/project are provenance, so the reader can go back to
-- the originating conversation for context.
CREATE TABLE agent_feedback (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL,
    -- The project the FILING session was working on ("bcc-data-hub-ingest"),
    -- not the project the feedback is about — that is always bot-hq.
    project     TEXT,
    agent       TEXT NOT NULL,
    -- 'issue' (something is broken/annoying) | 'idea' (something could be better)
    kind        TEXT NOT NULL,
    title       TEXT NOT NULL,
    body        TEXT NOT NULL,
    -- 'open' | 'done' | 'dismissed'
    status      TEXT NOT NULL DEFAULT 'open',
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- The reader's query is "what's still open, newest first".
CREATE INDEX idx_agent_feedback_open
    ON agent_feedback (status, id DESC);
