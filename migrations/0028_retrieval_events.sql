-- 0028_retrieval_events.sql — append-only log of cl_retrieve calls.
--
-- Stage 4b measurement: every ranked CL retrieval writes one row so we can
-- answer "is the CL helping" with data — tokens-per-task to acquire context,
-- stale-hit rate, and empty-return rate. Logging is best-effort (a failed
-- insert never blocks a retrieval).
--
-- No foreign keys: this is immutable, append-only telemetry. session_id, agent
-- and project_id are opaque audit strings — a retrieval against '_globals' (not
-- a projects row) must log, an insert must never fail because a session row is
-- absent, and pruning a session must NOT rewrite historical token accounting via
-- cascade. session_id/agent are nullable (host/UI-invoked retrievals have none).

CREATE TABLE retrieval_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id      TEXT,                       -- audit only; null for host/UI calls
    agent           TEXT,                       -- 'brian' | 'rain' | null
    project_id      TEXT NOT NULL,
    query           TEXT NOT NULL,
    atom_count      INTEGER NOT NULL,           -- atoms returned
    tokens_returned INTEGER NOT NULL,           -- sum of estimate_tokens over bodies
    budget_tokens   INTEGER NOT NULL,           -- the cap requested
    stale_count     INTEGER NOT NULL DEFAULT 0, -- returned atoms flagged ⚠ (code drift)
    returned_atoms  TEXT,                        -- JSON: [{file_path,heading_path,tokens,stale}]
    used_atoms      TEXT,                        -- reserved; not populated in v1
    created_at      TEXT NOT NULL
);

-- Project-scoped time series (tokens-per-task, stale-hit rate over a window).
CREATE INDEX idx_retrieval_events_project_time
    ON retrieval_events(project_id, created_at);

-- Per-session rollup (tokens-per-task to acquire context within a session).
CREATE INDEX idx_retrieval_events_session
    ON retrieval_events(session_id, created_at);
